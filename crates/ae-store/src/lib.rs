#![forbid(unsafe_code)]

//! SQLite registry: the only production state writer.
//!
//! One connection, one writer: every mutation goes through a BEGIN IMMEDIATE
//! transaction on this connection, so stale writers, duplicate events and
//! digest collisions fail closed instead of silently overwriting winners.
//! Identity-bearing data is stored as canonical binary bytes; JSON is used
//! only for debugging provenance columns.

pub mod continuity_vault;
pub use continuity_vault::{
    locate_vault, RebirthActionV1, RebirthAuditReceiptV1, RebirthChallengeV1,
    RebirthChildStageRequestV1, RebirthCommitPermitV1, RebirthCurrentV1, RebirthFaultV1,
    RebirthLifecycleError, RebirthOutcomeV1, RebirthPreflightV1, RebirthPrepareRequestV1,
    RebirthPrepareResponseV1, RebirthResponseEnvelopeV1, RebirthResponseStateV1,
    RebirthStagedChildV1, UserAuthorizedRebirthV1, VaultLifecycle, VaultLocateError, VaultLocation,
    VaultMode,
};
pub mod continuity_migration;
pub use continuity_migration::{
    migrate_continuity, open_current_generation, ContinuityAuthority, ContinuityMigrationDecision,
    ContinuityMigrationError, ContinuityMigrationFault, ContinuityMigrationReceipt,
    CurrentContinuityGeneration,
};
pub mod legacy_discovery;
pub use legacy_discovery::{
    discover_legacy, validate_legacy_candidate, verify_candidate, CandidateFences, Discovery,
    DiscoveryRejectCode, DiscoverySources, LegacyCandidate,
};

use ae_continuum::{CommitEnvelope, JournalRow};
use ae_contracts::{
    canonical_one_learning_compensation_policy_digest_v1, learning_compensation_formula_digest_v1,
    wire, CanonicalEvent, CommitStatus, Digest, GenesisManifest, GenesisReceipt, GenesisStatus,
    LearningCompensationEnqueueReceiptV1, LearningCompensationTerminalReceiptV1,
    LearningCompensationTerminalStatusV1, PersonaSourceRef, ScopeRef,
    SemanticLearningCompensationEnqueueV1, LEARNING_COMPENSATION_ENQUEUE_RECEIPT_SCHEMA_V1,
    LEARNING_COMPENSATION_TERMINAL_SCHEMA_V1,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub const LEASE_TTL_MS: u64 = 120_000;

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("storage io error: {context}: {source}")]
    Io {
        context: &'static str,
        source: std::io::Error,
    },
    #[error("sqlite error: {0}")]
    Sqlite(String),
    #[error("SEED_DIGEST_COLLISION: manifest digest exists with different canonical bytes")]
    SeedDigestCollision,
    #[error("manifest digest does not match its canonical bytes")]
    ManifestDigestMismatch,
    #[error("seed code digest does not match the manifest digest")]
    SeedCodeMismatch,
    #[error("genesis lease not found")]
    LeaseNotFound,
    #[error("genesis lease conflict: stale epoch or invalid status")]
    LeaseConflict,
    #[error("genesis lease in flight")]
    LeaseInFlight,
    #[error("incarnation identity conflict")]
    IncarnationConflict,
    #[error("active binding already points at a different incarnation")]
    BindingConflict,
    #[error("stale base revision: expected {expected}, found {actual}")]
    StaleRevision { expected: u64, actual: u64 },
    #[error("duplicate event: already applied at revision {0}")]
    DuplicateEvent(u64),
    #[error("no committed genesis for this scope")]
    GenesisNotFound,
    #[error("snapshot not found")]
    SnapshotNotFound,
    #[error("continuity bundle fence failed: {0}")]
    ContinuityFence(&'static str),
    #[error("continuity duplicate does not match the complete stored authority")]
    ContinuityDuplicateMismatch,
    #[error("continuity duplicate points at an incomplete authority bundle")]
    ContinuityIncomplete,
    #[error("learning compensation job not found")]
    LearningCompensationJobNotFound,
    #[error("learning compensation lease or lifecycle conflict")]
    LearningCompensationConflict,
    #[error("learning compensation job is already claimed")]
    LearningCompensationInFlight,
    #[error("learning compensation durable payload is invalid")]
    LearningCompensationInvalid,
    #[error("store is closed")]
    Closed,
}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        StoreError::Sqlite(error.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaseStatus {
    Claimed,
    Compiling,
    Validating,
    Developing,
    Committed,
    Failed,
    RetryWait,
}

impl LeaseStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            LeaseStatus::Claimed => "claimed",
            LeaseStatus::Compiling => "compiling",
            LeaseStatus::Validating => "validating",
            LeaseStatus::Developing => "developing",
            LeaseStatus::Committed => "committed",
            LeaseStatus::Failed => "failed",
            LeaseStatus::RetryWait => "retry_wait",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "claimed" => LeaseStatus::Claimed,
            "compiling" => LeaseStatus::Compiling,
            "validating" => LeaseStatus::Validating,
            "developing" => LeaseStatus::Developing,
            "committed" => LeaseStatus::Committed,
            "failed" => LeaseStatus::Failed,
            "retry_wait" => LeaseStatus::RetryWait,
            _ => return None,
        })
    }

    pub fn is_in_flight(self) -> bool {
        matches!(
            self,
            LeaseStatus::Claimed
                | LeaseStatus::Compiling
                | LeaseStatus::Validating
                | LeaseStatus::Developing
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaseRow {
    pub scope_key: Digest,
    pub lease_epoch: u64,
    pub status: LeaseStatus,
    pub nonce_digest: Option<Digest>,
    pub manifest_digest: Option<Digest>,
    pub incarnation_id: Option<Digest>,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClaimOutcome {
    Committed,
    Claimed { lease_epoch: u64, nonce: Digest },
    InFlight,
}

/// Everything the runtime must hand over to atomically close one birth.
#[derive(Clone, Debug)]
pub struct GenesisCommit {
    pub scope_key: Digest,
    pub lease_epoch: u64,
    pub nonce_digest: Digest,
    pub manifest: GenesisManifest,
    pub manifest_body: Vec<u8>,
    pub seed_code_digest: Digest,
    pub incarnation_id: Digest,
    pub formula_digest: Digest,
    pub source: PersonaSourceRef,
    pub compiler_protocol_digest: Digest,
    pub compiler_model_digest: Digest,
    pub compiled_at_ms: u64,
    pub receipt: GenesisReceipt,
    pub initial_snapshot_digest: Digest,
    pub state_bytes: Vec<u8>,
    pub graph_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommittedGenesis {
    pub receipt: GenesisReceipt,
    pub manifest: GenesisManifest,
    pub source: PersonaSourceRef,
    pub canonical_bytes: Vec<u8>,
    pub incarnation_nonce: Digest,
    pub born_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingRow {
    pub bot_token: [u8; 16],
    pub persona_token: [u8; 16],
    pub incarnation_id: Digest,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotRow {
    pub revision: u64,
    pub scope_digest: Digest,
    pub state_digest: Digest,
    pub state_bytes: Vec<u8>,
}

/// Stable digest boundary for the opaque, de-identified context projection
/// state. The context projector and runtime call this helper rather than
/// duplicating the domain-separation formula.
pub const CONTINUITY_CONTEXT_DIGEST_DOMAIN: &[u8] = b"AE-CONTEXT-PROJECTION-STATE-V1";

pub fn continuity_context_digest(canonical_state_bytes: &[u8]) -> Digest {
    wire::domain_hash(CONTINUITY_CONTEXT_DIGEST_DOMAIN, &[canonical_state_bytes])
}

/// Opaque state checkpoint that belongs to one successful transition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotCommitV1 {
    pub state_digest: Digest,
    pub state_bytes: Vec<u8>,
}

/// Opaque graph replay checkpoint that belongs to one successful transition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphCommitV1 {
    pub base_graph_digest: Digest,
    pub graph_digest: Digest,
    pub formula_digest: Digest,
    pub delta_bytes: Vec<u8>,
    pub replay_state_bytes: Vec<u8>,
}

/// Opaque, de-identified context projection checkpoint. `relation_hmac` is
/// produced by the projector and retained only as its fixed-width ordering key;
/// ae-store never receives the projection key or raw relation content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextCommitV1 {
    pub relation_scope_token: [u8; 16],
    pub relation_hmac: Digest,
    pub source_continuum_revision: u64,
    pub context_digest: Digest,
    pub canonical_state_bytes: Vec<u8>,
}

/// One committed opaque context checkpoint returned by its raw storage key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextCommitRowV1 {
    pub scope_digest: Digest,
    pub relation_scope_token: [u8; 16],
    pub relation_hmac: Digest,
    pub revision: u64,
    pub context_digest: Digest,
    pub canonical_state_bytes: Vec<u8>,
}

/// Every durable mutation that must share a single Continuum authority
/// transaction. The public boundary contains no SQLite handles or projector
/// crate types.
#[derive(Clone, Debug)]
pub struct ContinuityCommitBundleV1 {
    pub envelope: CommitEnvelope,
    pub snapshot: SnapshotCommitV1,
    pub graph: GraphCommitV1,
    pub context: ContextCommitV1,
}

// ---------------------------------------------------------------- learning compensation

/// Text-free durable lifecycle.  These values are deliberately separate from
/// Genesis leases: a teacher job never becomes a persona authority lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LearningCompensationJobStatusV1 {
    Pending,
    Claimed,
    Committed,
    NoChange,
    Rejected,
    Abandoned,
    Expired,
}

impl LearningCompensationJobStatusV1 {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Claimed => "claimed",
            Self::Committed => "committed",
            Self::NoChange => "no_change",
            Self::Rejected => "rejected",
            Self::Abandoned => "abandoned",
            Self::Expired => "expired",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "claimed" => Self::Claimed,
            "committed" => Self::Committed,
            "no_change" => Self::NoChange,
            "rejected" => Self::Rejected,
            "abandoned" => Self::Abandoned,
            "expired" => Self::Expired,
            _ => return None,
        })
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Committed | Self::NoChange | Self::Rejected | Self::Abandoned | Self::Expired
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewLearningCompensationJobV1 {
    pub job_id: Digest,
    pub scope_digest: Digest,
    pub source_event_digest: Digest,
    pub source_text_digest: Digest,
    pub source_base_revision: u64,
    pub request_digest: Digest,
    pub request_bytes: Vec<u8>,
    pub policy_digest: Digest,
    pub schema_digest: Digest,
    pub formula_digest: Digest,
    pub telemetry_digest: Digest,
    pub checkpoint_digest: Digest,
    /// A durable acceptance attestation. It is stored independently from a
    /// later apply/no-change/terminal outcome receipt.
    pub enqueue_receipt_bytes: Vec<u8>,
    pub enqueue_receipt_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LearningCompensationJobRowV1 {
    pub job_id: Digest,
    pub scope_digest: Digest,
    pub source_event_digest: Digest,
    pub source_text_digest: Digest,
    pub source_base_revision: u64,
    pub request_digest: Digest,
    pub request_bytes: Vec<u8>,
    pub policy_digest: Digest,
    pub schema_digest: Digest,
    pub formula_digest: Digest,
    pub telemetry_digest: Digest,
    pub checkpoint_digest: Digest,
    pub status: LearningCompensationJobStatusV1,
    pub lease_token: Option<Digest>,
    pub lease_epoch: u64,
    pub terminal_reason_digest: Option<Digest>,
    pub receipt_bytes: Option<Vec<u8>>,
    pub receipt_digest: Option<Digest>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LearningCompensationEnqueueReceiptRowV1 {
    pub job_id: Digest,
    pub receipt_bytes: Vec<u8>,
    pub receipt_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LearningCompensationEnqueueOutcomeV1 {
    Queued {
        job: LearningCompensationJobRowV1,
        enqueue_receipt: LearningCompensationEnqueueReceiptRowV1,
    },
    Replayed {
        job: LearningCompensationJobRowV1,
        enqueue_receipt: LearningCompensationEnqueueReceiptRowV1,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LearningCompensationStateV1 {
    pub scope_digest: Digest,
    pub checkpoint_revision: u64,
    pub u_bytes: Vec<u8>,
    pub compensation_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LearningCompensationCommitV1 {
    pub job_id: Digest,
    pub lease_token: Digest,
    pub expected_request_digest: Digest,
    /// CAS fence over the independently committed semantic journal cursor.
    /// Runtime has already verified the telemetry/checkpoint payload for this
    /// exact cursor before entering this SQLite transaction.
    pub expected_semantic_revision: u64,
    pub expected_formula_digest: Digest,
    pub expected_telemetry_digest: Digest,
    pub expected_checkpoint_digest: Digest,
    pub next_checkpoint_revision: u64,
    pub u_bytes: Vec<u8>,
    pub compensation_digest: Digest,
    pub receipt_bytes: Vec<u8>,
    pub receipt_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LearningCompensationClaimBindingV1 {
    pub job_id: Digest,
    pub lease_token: Digest,
    pub base_revision: u64,
    pub formula_digest: Digest,
    pub telemetry_digest: Digest,
    pub checkpoint_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LearningCompensationTerminalCommitV1 {
    pub job_id: Digest,
    pub lease_token: Digest,
    pub status: LearningCompensationJobStatusV1,
    pub reason_digest: Digest,
    pub checkpoint_digest: Digest,
    pub receipt_bytes: Vec<u8>,
    pub receipt_digest: Digest,
}

type StoredJournalColumns = (i64, String, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>);
type StoredGraphColumns = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>);
type StoredContextColumns = (Vec<u8>, Vec<u8>, i64, Vec<u8>, Vec<u8>);
type StoredContextDuplicateColumns = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>);
type StoredLearningCompensationJobColumns = (
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    i64,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    String,
    Option<Vec<u8>>,
    i64,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
);

pub struct Store {
    conn: Option<Connection>,
}

fn blob<const N: usize>(value: [u8; N]) -> Vec<u8> {
    value.to_vec()
}

fn digest_from_blob(bytes: &[u8], fence: &'static str) -> Result<Digest, StoreError> {
    if bytes.len() != 32 {
        return Err(StoreError::ContinuityFence(fence));
    }
    let mut digest = [0u8; 32];
    digest.copy_from_slice(bytes);
    Ok(digest)
}

fn token_from_blob(bytes: &[u8], fence: &'static str) -> Result<[u8; 16], StoreError> {
    if bytes.len() != 16 {
        return Err(StoreError::ContinuityFence(fence));
    }
    let mut token = [0u8; 16];
    token.copy_from_slice(bytes);
    Ok(token)
}

fn learning_compensation_job_from_columns(
    columns: StoredLearningCompensationJobColumns,
) -> Result<LearningCompensationJobRowV1, StoreError> {
    let (
        job_id,
        scope_digest,
        source_event_digest,
        source_text_digest,
        source_base_revision,
        request_digest,
        request_bytes,
        policy_digest,
        schema_digest,
        formula_digest,
        telemetry_digest,
        checkpoint_digest,
        status,
        lease_token,
        lease_epoch,
        terminal_reason_digest,
        receipt_bytes,
        receipt_digest,
    ) = columns;
    if source_base_revision < 0 || lease_epoch < 0 {
        return Err(StoreError::LearningCompensationInvalid);
    }
    Ok(LearningCompensationJobRowV1 {
        job_id: digest_from_blob(&job_id, "learning_job_id")?,
        scope_digest: digest_from_blob(&scope_digest, "learning_scope")?,
        source_event_digest: digest_from_blob(&source_event_digest, "learning_source_event")?,
        source_text_digest: digest_from_blob(&source_text_digest, "learning_source_text")?,
        source_base_revision: source_base_revision as u64,
        request_digest: digest_from_blob(&request_digest, "learning_request")?,
        request_bytes,
        policy_digest: digest_from_blob(&policy_digest, "learning_policy")?,
        schema_digest: digest_from_blob(&schema_digest, "learning_schema")?,
        formula_digest: digest_from_blob(&formula_digest, "learning_formula")?,
        telemetry_digest: digest_from_blob(&telemetry_digest, "learning_telemetry")?,
        checkpoint_digest: digest_from_blob(&checkpoint_digest, "learning_checkpoint")?,
        status: LearningCompensationJobStatusV1::from_str(&status)
            .ok_or(StoreError::LearningCompensationInvalid)?,
        lease_token: lease_token
            .as_deref()
            .map(|value| digest_from_blob(value, "learning_lease"))
            .transpose()?,
        lease_epoch: lease_epoch as u64,
        terminal_reason_digest: terminal_reason_digest
            .as_deref()
            .map(|value| digest_from_blob(value, "learning_terminal_reason"))
            .transpose()?,
        receipt_bytes,
        receipt_digest: receipt_digest
            .as_deref()
            .map(|value| digest_from_blob(value, "learning_receipt"))
            .transpose()?,
    })
}

fn scope_from_event(event: &CanonicalEvent) -> &ScopeRef {
    match event {
        CanonicalEvent::UserStimulus(value) => &value.scope,
        CanonicalEvent::UserReaction(value) => &value.scope,
        CanonicalEvent::CorrectionClaim(value) => &value.scope,
        CanonicalEvent::CorrectionVerdict(value) => &value.scope,
        CanonicalEvent::SelfActionCandidate(value) => &value.scope,
        CanonicalEvent::DeliveryOutcome(value) => &value.scope,
        CanonicalEvent::SettlementEvidence(value) => &value.scope,
        CanonicalEvent::TimeAdvance(value) => &value.scope,
        CanonicalEvent::AdminAction(value) => &value.scope,
    }
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| StoreError::Io {
                context: "creating store directory",
                source,
            })?;
        }
        let mut conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Self::migrate(&mut conn)?;
        Ok(Self { conn: Some(conn) })
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        let mut conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Self::migrate(&mut conn)?;
        Ok(Self { conn: Some(conn) })
    }

    fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS genesis_manifests (
                manifest_digest BLOB PRIMARY KEY,
                seed_code_digest BLOB NOT NULL UNIQUE,
                canonical_bytes BLOB NOT NULL,
                source_json TEXT NOT NULL,
                compiler_protocol_digest BLOB NOT NULL,
                compiler_model_digest BLOB NOT NULL,
                compiled_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS genesis_leases (
                scope_key BLOB PRIMARY KEY,
                lease_epoch INTEGER NOT NULL,
                status TEXT NOT NULL,
                nonce_digest BLOB,
                manifest_digest BLOB,
                incarnation_id BLOB,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS incarnations (
                incarnation_id BLOB PRIMARY KEY,
                seed_code_digest BLOB NOT NULL,
                manifest_digest BLOB NOT NULL,
                formula_digest BLOB NOT NULL,
                parent_incarnation_id BLOB,
                nonce_digest BLOB NOT NULL,
                status TEXT NOT NULL,
                initial_snapshot_digest BLOB NOT NULL,
                graph_digest BLOB NOT NULL,
                development_seed_digest BLOB NOT NULL,
                persona_source_digest BLOB NOT NULL,
                compiler_protocol_digest BLOB NOT NULL,
                compiler_model_digest BLOB NOT NULL,
                equilibrium_residual INTEGER NOT NULL,
                energy_residual INTEGER NOT NULL,
                capacity_residual INTEGER NOT NULL,
                sample_fit_residual INTEGER NOT NULL,
                born_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS active_bindings (
                bot_token BLOB NOT NULL,
                persona_token BLOB NOT NULL,
                incarnation_id BLOB NOT NULL,
                revision INTEGER NOT NULL,
                PRIMARY KEY (bot_token, persona_token)
            );
            CREATE TABLE IF NOT EXISTS journal (
                revision INTEGER PRIMARY KEY AUTOINCREMENT,
                logical_revision INTEGER NOT NULL,
                scope_digest BLOB NOT NULL,
                base_revision INTEGER NOT NULL,
                event_kind TEXT NOT NULL,
                event_bytes BLOB NOT NULL,
                event_digest BLOB NOT NULL,
                receipt_bytes BLOB NOT NULL,
                chain_digest BLOB NOT NULL,
                committed_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS applied_events (
                scope_digest BLOB NOT NULL,
                event_digest BLOB NOT NULL,
                revision INTEGER NOT NULL,
                PRIMARY KEY (scope_digest, event_digest)
            );
            CREATE TABLE IF NOT EXISTS snapshots (
                revision INTEGER NOT NULL,
                scope_digest BLOB NOT NULL,
                state_digest BLOB NOT NULL,
                state_bytes BLOB NOT NULL,
                PRIMARY KEY (revision, scope_digest)
            );
            CREATE TABLE IF NOT EXISTS graph_commits (
                scope_digest BLOB NOT NULL,
                revision INTEGER NOT NULL,
                base_graph_digest BLOB NOT NULL CHECK (length(base_graph_digest) = 32),
                graph_digest BLOB NOT NULL CHECK (length(graph_digest) = 32),
                formula_digest BLOB NOT NULL CHECK (length(formula_digest) = 32),
                delta_bytes BLOB NOT NULL,
                replay_state_bytes BLOB NOT NULL,
                PRIMARY KEY (scope_digest, revision)
            );
            CREATE TABLE IF NOT EXISTS context_commits (
                scope_digest BLOB NOT NULL,
                relation_scope_token BLOB NOT NULL CHECK (length(relation_scope_token) = 16),
                relation_hmac BLOB NOT NULL CHECK (length(relation_hmac) = 32),
                revision INTEGER NOT NULL,
                context_digest BLOB NOT NULL CHECK (length(context_digest) = 32),
                canonical_state_bytes BLOB NOT NULL,
                PRIMARY KEY (scope_digest, relation_scope_token, revision)
            );
            -- Text is intentionally absent.  All vectors are canonical
            -- fixed-point bytes inside request/receipt/checkpoint payloads.
            CREATE TABLE IF NOT EXISTS learning_compensation_jobs (
                job_id BLOB PRIMARY KEY CHECK (length(job_id) = 32),
                scope_digest BLOB NOT NULL CHECK (length(scope_digest) = 32),
                source_event_digest BLOB NOT NULL CHECK (length(source_event_digest) = 32),
                source_text_digest BLOB NOT NULL CHECK (length(source_text_digest) = 32),
                source_base_revision INTEGER NOT NULL,
                request_digest BLOB NOT NULL CHECK (length(request_digest) = 32),
                request_bytes BLOB NOT NULL,
                policy_digest BLOB NOT NULL CHECK (length(policy_digest) = 32),
                schema_digest BLOB NOT NULL CHECK (length(schema_digest) = 32),
                formula_digest BLOB NOT NULL CHECK (length(formula_digest) = 32),
                telemetry_digest BLOB NOT NULL CHECK (length(telemetry_digest) = 32),
                checkpoint_digest BLOB NOT NULL CHECK (length(checkpoint_digest) = 32),
                status TEXT NOT NULL,
                lease_token BLOB CHECK (lease_token IS NULL OR length(lease_token) = 32),
                lease_epoch INTEGER NOT NULL,
                terminal_reason_digest BLOB CHECK (terminal_reason_digest IS NULL OR length(terminal_reason_digest) = 32),
                receipt_bytes BLOB,
                receipt_digest BLOB CHECK (receipt_digest IS NULL OR length(receipt_digest) = 32),
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );
            CREATE UNIQUE INDEX IF NOT EXISTS learning_compensation_job_identity
                ON learning_compensation_jobs (scope_digest, source_event_digest, source_text_digest, policy_digest, schema_digest, formula_digest);
            -- Acceptance is a first-class durable receipt. It is intentionally
            -- split from the mutable job row's eventual apply/terminal receipt
            -- so `QUEUED` never masquerades as a completed outcome.
            CREATE TABLE IF NOT EXISTS learning_compensation_enqueue_receipts (
                job_id BLOB PRIMARY KEY CHECK (length(job_id) = 32),
                receipt_bytes BLOB NOT NULL,
                receipt_digest BLOB NOT NULL UNIQUE CHECK (length(receipt_digest) = 32)
            );
            CREATE TABLE IF NOT EXISTS learning_compensation_claim_bindings (
                job_id BLOB PRIMARY KEY CHECK (length(job_id) = 32),
                lease_token BLOB NOT NULL CHECK (length(lease_token) = 32),
                base_revision INTEGER NOT NULL,
                formula_digest BLOB NOT NULL CHECK (length(formula_digest) = 32),
                telemetry_digest BLOB NOT NULL CHECK (length(telemetry_digest) = 32),
                checkpoint_digest BLOB NOT NULL CHECK (length(checkpoint_digest) = 32)
            );
            CREATE TABLE IF NOT EXISTS learning_compensation_state (
                scope_digest BLOB PRIMARY KEY CHECK (length(scope_digest) = 32),
                checkpoint_revision INTEGER NOT NULL,
                u_bytes BLOB NOT NULL,
                compensation_digest BLOB NOT NULL CHECK (length(compensation_digest) = 32)
            );
            CREATE TABLE IF NOT EXISTS learning_compensation_checkpoints (
                scope_digest BLOB NOT NULL CHECK (length(scope_digest) = 32),
                checkpoint_revision INTEGER NOT NULL,
                job_id BLOB NOT NULL UNIQUE CHECK (length(job_id) = 32),
                compensation_digest BLOB NOT NULL CHECK (length(compensation_digest) = 32),
                receipt_digest BLOB NOT NULL CHECK (length(receipt_digest) = 32),
                u_bytes BLOB NOT NULL,
                receipt_bytes BLOB NOT NULL,
                PRIMARY KEY (scope_digest, checkpoint_revision)
            );
            "#,
        )?;
        if !Self::journal_has_logical_revision(&tx)? {
            tx.execute(
                "ALTER TABLE journal ADD COLUMN logical_revision INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
            Self::backfill_scope_local_revisions(&tx)?;
        }
        tx.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS journal_scope_logical_revision ON journal (scope_digest, logical_revision);",
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO meta (key, value) VALUES ('schema_version', X'01')",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn journal_has_logical_revision(tx: &Transaction<'_>) -> Result<bool, StoreError> {
        let mut statement = tx.prepare("PRAGMA table_info(journal)")?;
        let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
        for column in columns {
            if column? == "logical_revision" {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn backfill_scope_local_revisions(tx: &Transaction<'_>) -> Result<(), StoreError> {
        let invalid_applied_events: i64 = tx.query_row(
            "SELECT COUNT(*) FROM applied_events AS ae LEFT JOIN journal AS j ON j.scope_digest = ae.scope_digest AND j.event_digest = ae.event_digest AND j.revision = ae.revision WHERE j.revision IS NULL",
            [],
            |row| row.get(0),
        )?;
        if invalid_applied_events != 0 {
            return Err(StoreError::Sqlite(
                "legacy applied event does not map to its journal row".to_owned(),
            ));
        }

        let invalid_snapshots: i64 = tx.query_row(
            "SELECT COUNT(*) FROM snapshots AS s WHERE s.revision <> 0 AND NOT EXISTS (SELECT 1 FROM journal AS j WHERE j.scope_digest = s.scope_digest AND j.revision = s.revision)",
            [],
            |row| row.get(0),
        )?;
        if invalid_snapshots != 0 {
            return Err(StoreError::Sqlite(
                "legacy snapshot does not map to its journal row".to_owned(),
            ));
        }

        let mappings = {
            let mut statement = tx.prepare(
                "SELECT revision, scope_digest FROM journal ORDER BY scope_digest ASC, revision ASC",
            )?;
            let mut rows = statement.query([])?;
            let mut mappings = Vec::new();
            let mut previous_scope: Option<Vec<u8>> = None;
            let mut logical_revision = 0_i64;
            while let Some(row) = rows.next()? {
                let physical_revision: i64 = row.get(0)?;
                let scope_digest: Vec<u8> = row.get(1)?;
                if previous_scope.as_ref() == Some(&scope_digest) {
                    logical_revision += 1;
                } else {
                    logical_revision = 1;
                    previous_scope = Some(scope_digest);
                }
                mappings.push((physical_revision, logical_revision));
            }
            mappings
        };
        for (physical_revision, logical_revision) in mappings {
            tx.execute(
                "UPDATE journal SET logical_revision = ?1 WHERE revision = ?2",
                params![logical_revision, physical_revision],
            )?;
        }

        tx.execute(
            "UPDATE applied_events SET revision = (SELECT logical_revision FROM journal WHERE journal.scope_digest = applied_events.scope_digest AND journal.event_digest = applied_events.event_digest AND journal.revision = applied_events.revision)",
            [],
        )?;
        tx.execute(
            "UPDATE snapshots SET revision = (SELECT logical_revision FROM journal AS j WHERE j.scope_digest = snapshots.scope_digest AND j.revision = snapshots.revision) WHERE revision <> 0",
            [],
        )?;
        Ok(())
    }

    fn connection(&self) -> Result<&Connection, StoreError> {
        self.conn.as_ref().ok_or(StoreError::Closed)
    }

    // ------------------------------------------------------------- leases

    /// Claim (or join) the durable Genesis lease for one scope key.
    /// The persisted birth nonce wins: a retry after a crash replays the
    /// original birth transaction instead of starting a second one.
    pub fn claim_lease(
        &mut self,
        scope_key: &Digest,
        offered_nonce: Option<Digest>,
    ) -> Result<ClaimOutcome, StoreError> {
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        let now = now_ms();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let existing = tx
            .query_row(
                "SELECT lease_epoch, status, nonce_digest, updated_at_ms FROM genesis_leases WHERE scope_key = ?1",
                params![blob(*scope_key)],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<Vec<u8>>>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?;

        let outcome = match existing {
            None => {
                let nonce = offered_nonce.ok_or(StoreError::LeaseNotFound)?;
                tx.execute(
                    "INSERT INTO genesis_leases (scope_key, lease_epoch, status, nonce_digest, manifest_digest, incarnation_id, created_at_ms, updated_at_ms) VALUES (?1, 1, 'claimed', ?2, NULL, NULL, ?3, ?3)",
                    params![blob(*scope_key), blob(nonce), now as i64],
                )?;
                ClaimOutcome::Claimed {
                    lease_epoch: 1,
                    nonce,
                }
            }
            Some((epoch, status_text, stored_nonce, updated_at)) => {
                let status =
                    LeaseStatus::from_str(&status_text).ok_or(StoreError::LeaseConflict)?;
                if status == LeaseStatus::Committed {
                    ClaimOutcome::Committed
                } else if status.is_in_flight() && (now as i64 - updated_at) < LEASE_TTL_MS as i64 {
                    ClaimOutcome::InFlight
                } else {
                    // Stale in-flight lease (crash recovery) or a failed/retry-wait
                    // lease: take over, reusing the persisted birth nonce.
                    let stored = stored_nonce.map(|b| {
                        let mut digest = [0u8; 32];
                        digest.copy_from_slice(&b);
                        digest
                    });
                    let nonce = stored.or(offered_nonce).ok_or(StoreError::LeaseNotFound)?;
                    let new_epoch = (epoch + 1) as i64;
                    tx.execute(
                        "UPDATE genesis_leases SET lease_epoch = ?2, status = 'claimed', nonce_digest = ?3, manifest_digest = NULL, incarnation_id = NULL, updated_at_ms = ?4 WHERE scope_key = ?1",
                        params![blob(*scope_key), new_epoch, blob(nonce), now as i64],
                    )?;
                    ClaimOutcome::Claimed {
                        lease_epoch: new_epoch as u64,
                        nonce,
                    }
                }
            }
        };
        tx.commit()?;
        Ok(outcome)
    }

    pub fn lookup_lease(&self, scope_key: &Digest) -> Result<Option<LeaseRow>, StoreError> {
        let conn = self.connection()?;
        let row = conn
            .query_row(
                "SELECT lease_epoch, status, nonce_digest, manifest_digest, incarnation_id, updated_at_ms FROM genesis_leases WHERE scope_key = ?1",
                params![blob(*scope_key)],
                |row| {
                    let nonce = row
                        .get::<_, Option<Vec<u8>>>(2)?
                        .map(|b| {
                            let mut digest = [0u8; 32];
                            digest.copy_from_slice(&b);
                            digest
                        });
                    let manifest = row
                        .get::<_, Option<Vec<u8>>>(3)?
                        .map(|b| {
                            let mut digest = [0u8; 32];
                            digest.copy_from_slice(&b);
                            digest
                        });
                    let incarnation = row
                        .get::<_, Option<Vec<u8>>>(4)?
                        .map(|b| {
                            let mut digest = [0u8; 32];
                            digest.copy_from_slice(&b);
                            digest
                        });
                    Ok(LeaseRow {
                        scope_key: *scope_key,
                        lease_epoch: row.get::<_, i64>(0)? as u64,
                        status: LeaseStatus::from_str(&row.get::<_, String>(1)?)
                            .unwrap_or(LeaseStatus::Failed),
                        nonce_digest: nonce,
                        manifest_digest: manifest,
                        incarnation_id: incarnation,
                        updated_at_ms: row.get::<_, i64>(5)? as u64,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    // ------------------------------------------------------------ manifests

    /// Register a canonical Manifest. Collision policy: same digest with
    /// different bytes fails closed with SeedDigestCollision; identical
    /// content is idempotent.
    pub fn register_manifest(
        &mut self,
        manifest: &GenesisManifest,
        manifest_body: &[u8],
        source: &PersonaSourceRef,
        compiler_protocol_digest: &Digest,
        compiler_model_digest: &Digest,
        compiled_at_ms: u64,
    ) -> Result<(), StoreError> {
        let recomputed = wire::decode_manifest_body(manifest_body)
            .map_err(|_| StoreError::ManifestDigestMismatch)?;
        let expected_digest = wire::manifest_body_digest(&recomputed);
        if expected_digest != manifest.manifest_digest {
            return Err(StoreError::ManifestDigestMismatch);
        }
        let seed = ae_genesis::derive_seed_code_digest(&expected_digest);
        if seed != ae_genesis::derive_seed_code_digest(&manifest.manifest_digest) {
            return Err(StoreError::SeedCodeMismatch);
        }
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<Vec<u8>> = tx
            .query_row(
                "SELECT canonical_bytes FROM genesis_manifests WHERE manifest_digest = ?1",
                params![blob(manifest.manifest_digest)],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(stored) = existing {
            if stored != manifest_body {
                return Err(StoreError::SeedDigestCollision);
            }
            return Ok(());
        }
        let source_json = serde_json::to_string(source)
            .map_err(|error| StoreError::Sqlite(format!("source serialization failed: {error}")))?;
        tx.execute(
            "INSERT INTO genesis_manifests (manifest_digest, seed_code_digest, canonical_bytes, source_json, compiler_protocol_digest, compiler_model_digest, compiled_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                blob(manifest.manifest_digest),
                blob(seed),
                manifest_body.to_vec(),
                source_json,
                blob(*compiler_protocol_digest),
                blob(*compiler_model_digest),
                compiled_at_ms as i64,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    // ------------------------------------------------------- genesis commit

    /// Atomically close one birth: verify the lease epoch, register the
    /// Manifest, insert the incarnation, bind the persona and write the
    /// revision-0 snapshot. Stale epochs update zero rows and fail.
    pub fn commit_genesis(&mut self, commit: &GenesisCommit) -> Result<(), StoreError> {
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let lease_status: Option<(i64, String)> = tx
            .query_row(
                "SELECT lease_epoch, status FROM genesis_leases WHERE scope_key = ?1",
                params![blob(commit.scope_key)],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        match lease_status {
            None => return Err(StoreError::LeaseNotFound),
            Some((epoch, status_text)) => {
                let status =
                    LeaseStatus::from_str(&status_text).ok_or(StoreError::LeaseConflict)?;
                if epoch != commit.lease_epoch as i64 || !status.is_in_flight() {
                    return Err(StoreError::LeaseConflict);
                }
            }
        }

        // Identity verification before any write.
        let recomputed = wire::decode_manifest_body(&commit.manifest_body)
            .map_err(|_| StoreError::ManifestDigestMismatch)?;
        let expected_digest = wire::manifest_body_digest(&recomputed);
        if expected_digest != commit.manifest.manifest_digest {
            return Err(StoreError::ManifestDigestMismatch);
        }
        let expected_seed = ae_genesis::derive_seed_code_digest(&expected_digest);
        if expected_seed != commit.seed_code_digest {
            return Err(StoreError::SeedCodeMismatch);
        }
        if commit.receipt.seed_code_digest != commit.seed_code_digest
            || commit.receipt.manifest_digest != expected_digest
            || commit.receipt.status != GenesisStatus::Committed
        {
            return Err(StoreError::IncarnationConflict);
        }

        // Manifest collision check: same digest must be byte-identical.
        let existing_bytes: Option<Vec<u8>> = tx
            .query_row(
                "SELECT canonical_bytes FROM genesis_manifests WHERE manifest_digest = ?1",
                params![blob(expected_digest)],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(stored) = existing_bytes {
            if stored != commit.manifest_body {
                return Err(StoreError::SeedDigestCollision);
            }
        } else {
            let source_json = serde_json::to_string(&commit.source).map_err(|error| {
                StoreError::Sqlite(format!("source serialization failed: {error}"))
            })?;
            tx.execute(
                "INSERT INTO genesis_manifests (manifest_digest, seed_code_digest, canonical_bytes, source_json, compiler_protocol_digest, compiler_model_digest, compiled_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    blob(expected_digest),
                    blob(commit.seed_code_digest),
                    commit.manifest_body.clone(),
                    source_json,
                    blob(commit.compiler_protocol_digest),
                    blob(commit.compiler_model_digest),
                    commit.compiled_at_ms as i64,
                ],
            )?;
        }

        // Incarnation row: idempotent only for the identical birth transaction;
        // a differing row behind the same incarnation id fails closed.
        let existing_incarnation: Option<(Vec<u8>, Vec<u8>)> = tx
            .query_row(
                "SELECT seed_code_digest, manifest_digest FROM incarnations WHERE incarnation_id = ?1",
                params![blob(commit.incarnation_id)],
                |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?)),
            )
            .optional()?;
        if let Some((stored_seed, stored_manifest)) = existing_incarnation {
            if stored_seed != commit.seed_code_digest.to_vec()
                || stored_manifest != expected_digest.to_vec()
            {
                return Err(StoreError::IncarnationConflict);
            }
        } else {
            tx.execute(
                "INSERT INTO incarnations (incarnation_id, seed_code_digest, manifest_digest, formula_digest, parent_incarnation_id, nonce_digest, status, initial_snapshot_digest, graph_digest, development_seed_digest, persona_source_digest, compiler_protocol_digest, compiler_model_digest, equilibrium_residual, energy_residual, capacity_residual, sample_fit_residual, born_at_ms) VALUES (?1, ?2, ?3, ?4, NULL, ?5, 'active', ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                params![
                    blob(commit.incarnation_id),
                    blob(commit.seed_code_digest),
                    blob(expected_digest),
                    blob(commit.formula_digest),
                    blob(commit.nonce_digest),
                    blob(commit.initial_snapshot_digest),
                    blob(commit.graph_digest),
                    blob(commit.receipt.development_seed_digest),
                    blob(commit.receipt.persona_source_digest),
                    blob(commit.compiler_protocol_digest),
                    blob(commit.compiler_model_digest),
                    commit.receipt.equilibrium_residual.raw(),
                    commit.receipt.energy_residual.raw(),
                    commit.receipt.capacity_residual.raw(),
                    commit.receipt.sample_fit_residual.raw(),
                    commit.compiled_at_ms as i64,
                ],
            )?;
        }

        // Active binding: one active incarnation per (Bot, Persona).
        let existing_binding: Option<Vec<u8>> = tx
            .query_row(
                "SELECT incarnation_id FROM active_bindings WHERE bot_token = ?1 AND persona_token = ?2",
                params![
                    blob(commit.source.scope.bot_token),
                    blob(commit.source.scope.persona_token),
                ],
                |row| row.get(0),
            )
            .optional()?;
        match existing_binding {
            None => {
                tx.execute(
                    "INSERT INTO active_bindings (bot_token, persona_token, incarnation_id, revision) VALUES (?1, ?2, ?3, 1)",
                    params![
                        blob(commit.source.scope.bot_token),
                        blob(commit.source.scope.persona_token),
                        blob(commit.incarnation_id),
                    ],
                )?;
            }
            Some(stored) => {
                if stored != commit.incarnation_id.to_vec() {
                    return Err(StoreError::BindingConflict);
                }
            }
        }

        // Revision-0 snapshot for this persona commit lane.
        let scope_digest = wire::persona_scope_digest(
            &commit.source.scope.bot_token,
            &commit.source.scope.persona_token,
            None,
        );
        tx.execute(
            "INSERT OR IGNORE INTO snapshots (revision, scope_digest, state_digest, state_bytes) VALUES (0, ?1, ?2, ?3)",
            params![
                blob(scope_digest),
                blob(commit.initial_snapshot_digest),
                commit.state_bytes.clone(),
            ],
        )?;

        // Close the lease.
        tx.execute(
            "UPDATE genesis_leases SET status = 'committed', manifest_digest = ?2, incarnation_id = ?3, updated_at_ms = ?4 WHERE scope_key = ?1 AND lease_epoch = ?5",
            params![
                blob(commit.scope_key),
                blob(expected_digest),
                blob(commit.incarnation_id),
                now_ms() as i64,
                commit.lease_epoch as i64,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn lookup_committed_genesis(
        &self,
        scope_key: &Digest,
    ) -> Result<Option<CommittedGenesis>, StoreError> {
        let conn = self.connection()?;
        let lease = conn
            .query_row(
                "SELECT status, manifest_digest, incarnation_id, nonce_digest, updated_at_ms FROM genesis_leases WHERE scope_key = ?1",
                params![blob(*scope_key)],
                |row| {
                    let manifest = row.get::<_, Option<Vec<u8>>>(1)?;
                    let incarnation = row.get::<_, Option<Vec<u8>>>(2)?;
                    let nonce = row.get::<_, Option<Vec<u8>>>(3)?;
                    Ok((
                        row.get::<_, String>(0)?,
                        manifest,
                        incarnation,
                        nonce,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            status,
            Some(manifest_digest_bytes),
            Some(incarnation_bytes),
            nonce_bytes,
            born_at,
        )) = lease
        else {
            return Ok(None);
        };
        if status != "committed" {
            return Ok(None);
        }
        let mut manifest_digest = [0u8; 32];
        manifest_digest.copy_from_slice(&manifest_digest_bytes);
        let mut incarnation_id = [0u8; 32];
        incarnation_id.copy_from_slice(&incarnation_bytes);
        let mut incarnation_nonce = [0u8; 32];
        if let Some(bytes) = nonce_bytes {
            incarnation_nonce.copy_from_slice(&bytes);
        }

        let row = conn
            .query_row(
                "SELECT seed_code_digest, canonical_bytes, source_json, compiler_protocol_digest, compiler_model_digest, compiled_at_ms FROM genesis_manifests WHERE manifest_digest = ?1",
                params![blob(manifest_digest)],
                |row| {
                    let mut seed = [0u8; 32];
                    let seed_bytes: Vec<u8> = row.get(0)?;
                    seed.copy_from_slice(&seed_bytes);
                    let canonical: Vec<u8> = row.get(1)?;
                    let source_json: String = row.get(2)?;
                    let mut protocol = [0u8; 32];
                    let protocol_bytes: Vec<u8> = row.get(3)?;
                    protocol.copy_from_slice(&protocol_bytes);
                    let mut model = [0u8; 32];
                    let model_bytes: Vec<u8> = row.get(4)?;
                    model.copy_from_slice(&model_bytes);
                    Ok((seed, canonical, source_json, protocol, model, row.get::<_, i64>(5)?))
                },
            )
            .optional()?;
        let Some((seed_code_digest, canonical_bytes, source_json, protocol, model, _compiled_at)) =
            row
        else {
            return Ok(None);
        };
        let source: PersonaSourceRef = serde_json::from_str(&source_json).map_err(|error| {
            StoreError::Sqlite(format!("source deserialization failed: {error}"))
        })?;

        let incarnation = conn
            .query_row(
                "SELECT formula_digest, initial_snapshot_digest, graph_digest, development_seed_digest, persona_source_digest, equilibrium_residual, energy_residual, capacity_residual, sample_fit_residual FROM incarnations WHERE incarnation_id = ?1",
                params![blob(incarnation_id)],
                |row| {
                    let mut formula = [0u8; 32];
                    let bytes: Vec<u8> = row.get(0)?;
                    formula.copy_from_slice(&bytes);
                    let mut snapshot = [0u8; 32];
                    let bytes: Vec<u8> = row.get(1)?;
                    snapshot.copy_from_slice(&bytes);
                    let mut graph = [0u8; 32];
                    let bytes: Vec<u8> = row.get(2)?;
                    graph.copy_from_slice(&bytes);
                    let mut development = [0u8; 32];
                    let bytes: Vec<u8> = row.get(3)?;
                    development.copy_from_slice(&bytes);
                    let mut persona_source = [0u8; 32];
                    let bytes: Vec<u8> = row.get(4)?;
                    persona_source.copy_from_slice(&bytes);
                    Ok((
                        formula,
                        snapshot,
                        graph,
                        development,
                        persona_source,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                    ))
                },
            )
            .optional()?;
        let Some((formula, snapshot, graph, development, persona_source, eq, en, cap, fit)) =
            incarnation
        else {
            return Ok(None);
        };

        let manifest = wire::decode_manifest_body(&canonical_bytes).map_err(|_| {
            StoreError::Sqlite("stored canonical manifest bytes are invalid".to_string())
        })?;
        let receipt = GenesisReceipt {
            schema_version: 1,
            seed_code_digest,
            manifest_digest,
            incarnation_id,
            formula_digest: formula,
            persona_source_digest: persona_source,
            compiler_protocol_digest: protocol,
            compiler_model_digest: model,
            development_seed_digest: development,
            initial_snapshot_digest: snapshot,
            graph_digest: graph,
            equilibrium_residual: ae_fixed::Fixed::from_raw(eq),
            energy_residual: ae_fixed::Fixed::from_raw(en),
            capacity_residual: ae_fixed::Fixed::from_raw(cap),
            sample_fit_residual: ae_fixed::Fixed::from_raw(fit),
            status: GenesisStatus::Committed,
        };
        Ok(Some(CommittedGenesis {
            receipt,
            manifest,
            source,
            canonical_bytes,
            incarnation_nonce,
            born_at_ms: born_at as u64,
        }))
    }

    // ------------------------------------------------------------ bindings

    pub fn lookup_binding(
        &self,
        bot_token: &[u8; 16],
        persona_token: &[u8; 16],
    ) -> Result<Option<BindingRow>, StoreError> {
        let conn = self.connection()?;
        let row = conn
            .query_row(
                "SELECT incarnation_id, revision FROM active_bindings WHERE bot_token = ?1 AND persona_token = ?2",
                params![blob(*bot_token), blob(*persona_token)],
                |row| {
                    let bytes: Vec<u8> = row.get(0)?;
                    let mut incarnation = [0u8; 32];
                    incarnation.copy_from_slice(&bytes);
                    Ok(BindingRow {
                        bot_token: *bot_token,
                        persona_token: *persona_token,
                        incarnation_id: incarnation,
                        revision: row.get::<_, i64>(1)? as u64,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// Resolve the committed genesis for a persona binding by
    /// (Bot, Persona) tokens, joining bindings -> incarnations -> manifests.
    pub fn lookup_bound_genesis(
        &self,
        bot_token: &[u8; 16],
        persona_token: &[u8; 16],
    ) -> Result<Option<CommittedGenesis>, StoreError> {
        let Some(binding) = self.lookup_binding(bot_token, persona_token)? else {
            return Ok(None);
        };
        let conn = self.connection()?;
        let Some((manifest_digest_bytes, nonce_bytes, formula_bytes, snapshot_bytes, graph_bytes, dev_bytes, persona_bytes, protocol_bytes, model_bytes, eq, en, cap, fit, born_at, seed_bytes, canonical, source_json)) =
            conn.query_row(
                "SELECT i.manifest_digest, i.nonce_digest, i.formula_digest, i.initial_snapshot_digest, i.graph_digest, i.development_seed_digest, i.persona_source_digest, i.compiler_protocol_digest, i.compiler_model_digest, i.equilibrium_residual, i.energy_residual, i.capacity_residual, i.sample_fit_residual, i.born_at_ms, m.seed_code_digest, m.canonical_bytes, m.source_json FROM incarnations i JOIN genesis_manifests m ON i.manifest_digest = m.manifest_digest WHERE i.incarnation_id = ?1",
                params![blob(binding.incarnation_id)],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, Vec<u8>>(4)?,
                        row.get::<_, Vec<u8>>(5)?,
                        row.get::<_, Vec<u8>>(6)?,
                        row.get::<_, Vec<u8>>(7)?,
                        row.get::<_, Vec<u8>>(8)?,
                        row.get::<_, i64>(9)?,
                        row.get::<_, i64>(10)?,
                        row.get::<_, i64>(11)?,
                        row.get::<_, i64>(12)?,
                        row.get::<_, i64>(13)?,
                        row.get::<_, Vec<u8>>(14)?,
                        row.get::<_, Vec<u8>>(15)?,
                        row.get::<_, String>(16)?,
                    ))
                },
            ).optional()?
        else {
            return Ok(None);
        };
        let mut manifest_digest = [0u8; 32];
        manifest_digest.copy_from_slice(&manifest_digest_bytes);
        let mut incarnation_nonce = [0u8; 32];
        incarnation_nonce.copy_from_slice(&nonce_bytes);
        let mut formula_digest = [0u8; 32];
        formula_digest.copy_from_slice(&formula_bytes);
        let mut initial_snapshot_digest = [0u8; 32];
        initial_snapshot_digest.copy_from_slice(&snapshot_bytes);
        let mut graph_digest = [0u8; 32];
        graph_digest.copy_from_slice(&graph_bytes);
        let mut development_seed_digest = [0u8; 32];
        development_seed_digest.copy_from_slice(&dev_bytes);
        let mut persona_source_digest = [0u8; 32];
        persona_source_digest.copy_from_slice(&persona_bytes);
        let mut compiler_protocol_digest = [0u8; 32];
        compiler_protocol_digest.copy_from_slice(&protocol_bytes);
        let mut compiler_model_digest = [0u8; 32];
        compiler_model_digest.copy_from_slice(&model_bytes);
        let mut seed_code_digest = [0u8; 32];
        seed_code_digest.copy_from_slice(&seed_bytes);
        let source: PersonaSourceRef = serde_json::from_str(&source_json).map_err(|error| {
            StoreError::Sqlite(format!("source deserialization failed: {error}"))
        })?;
        let manifest = wire::decode_manifest_body(&canonical).map_err(|_| {
            StoreError::Sqlite("stored canonical manifest bytes are invalid".to_string())
        })?;
        let receipt = GenesisReceipt {
            schema_version: 1,
            seed_code_digest,
            manifest_digest,
            incarnation_id: binding.incarnation_id,
            formula_digest,
            persona_source_digest,
            compiler_protocol_digest,
            compiler_model_digest,
            development_seed_digest,
            initial_snapshot_digest,
            graph_digest,
            equilibrium_residual: ae_fixed::Fixed::from_raw(eq),
            energy_residual: ae_fixed::Fixed::from_raw(en),
            capacity_residual: ae_fixed::Fixed::from_raw(cap),
            sample_fit_residual: ae_fixed::Fixed::from_raw(fit),
            status: GenesisStatus::Committed,
        };
        Ok(Some(CommittedGenesis {
            receipt,
            manifest,
            source,
            canonical_bytes: canonical,
            incarnation_nonce,
            born_at_ms: born_at as u64,
        }))
    }

    // -------------------------------------------------------------- journal

    pub fn current_revision(&self, scope_digest: &Digest) -> Result<u64, StoreError> {
        let conn = self.connection()?;
        let revision: i64 = conn.query_row(
            "SELECT COALESCE(MAX(logical_revision), 0) FROM journal WHERE scope_digest = ?1",
            params![blob(*scope_digest)],
            |row| row.get(0),
        )?;
        Ok(revision as u64)
    }

    pub fn last_chain_digest(&self, scope_digest: &Digest) -> Result<Option<Digest>, StoreError> {
        let conn = self.connection()?;
        let bytes: Option<Vec<u8>> = conn
            .query_row(
            "SELECT chain_digest FROM journal WHERE scope_digest = ?1 ORDER BY logical_revision DESC LIMIT 1",
                params![blob(*scope_digest)],
                |row| row.get(0),
            )
            .optional()?;
        Ok(bytes.map(|b| {
            let mut digest = [0u8; 32];
            digest.copy_from_slice(&b);
            digest
        }))
    }

    pub fn lookup_event(
        &self,
        scope_digest: &Digest,
        event_digest: &Digest,
    ) -> Result<Option<JournalRow>, StoreError> {
        let conn = self.connection()?;
        let revision: Option<i64> = conn
            .query_row(
                "SELECT revision FROM applied_events WHERE scope_digest = ?1 AND event_digest = ?2",
                params![blob(*scope_digest), blob(*event_digest)],
                |row| row.get(0),
            )
            .optional()?;
        let Some(revision) = revision else {
            return Ok(None);
        };
        self.read_journal_row(scope_digest, revision as u64)
    }

    fn read_journal_row(
        &self,
        scope_digest: &Digest,
        revision: u64,
    ) -> Result<Option<JournalRow>, StoreError> {
        let conn = self.connection()?;
        conn.query_row(
            "SELECT base_revision, event_kind, event_bytes, event_digest, receipt_bytes, chain_digest FROM journal WHERE scope_digest = ?1 AND logical_revision = ?2",
            params![blob(*scope_digest), revision as i64],
            |row| {
                let mut event_digest = [0u8; 32];
                let bytes: Vec<u8> = row.get(3)?;
                event_digest.copy_from_slice(&bytes);
                let mut chain = [0u8; 32];
                let bytes: Vec<u8> = row.get(5)?;
                chain.copy_from_slice(&bytes);
                Ok(JournalRow {
                    revision,
                    scope_digest: *scope_digest,
                    base_revision: row.get::<_, i64>(0)? as u64,
                    event_kind: row.get(1)?,
                    event_bytes: row.get(2)?,
                    event_digest,
                    receipt_bytes: row.get(4)?,
                    chain_digest: chain,
                })
            },
        )
        .optional()
        .map_err(StoreError::from)
    }

    pub fn read_journal(&self, scope_digest: &Digest) -> Result<Vec<JournalRow>, StoreError> {
        let conn = self.connection()?;
        let mut statement = conn.prepare(
            "SELECT logical_revision, base_revision, event_kind, event_bytes, event_digest, receipt_bytes, chain_digest FROM journal WHERE scope_digest = ?1 ORDER BY logical_revision ASC",
        )?;
        let rows = statement
            .query_map(params![blob(*scope_digest)], |row| {
                let mut event_digest = [0u8; 32];
                let bytes: Vec<u8> = row.get(4)?;
                event_digest.copy_from_slice(&bytes);
                let mut chain = [0u8; 32];
                let bytes: Vec<u8> = row.get(6)?;
                chain.copy_from_slice(&bytes);
                Ok(JournalRow {
                    revision: row.get::<_, i64>(0)? as u64,
                    scope_digest: *scope_digest,
                    base_revision: row.get::<_, i64>(1)? as u64,
                    event_kind: row.get(2)?,
                    event_bytes: row.get(3)?,
                    event_digest,
                    receipt_bytes: row.get(5)?,
                    chain_digest: chain,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn validate_continuity_payloads(
        bundle: &ContinuityCommitBundleV1,
    ) -> Result<CanonicalEvent, StoreError> {
        let event = wire::decode_event(&bundle.envelope.event_bytes)
            .map_err(|_| StoreError::ContinuityFence("event_decode"))?;
        let event_digest = wire::event_digest(&event);
        if event_digest != bundle.envelope.receipt.event_digest {
            return Err(StoreError::ContinuityFence("event_digest"));
        }
        if bundle.envelope.event_kind != wire::event_kind_name(&event) {
            return Err(StoreError::ContinuityFence("event_kind"));
        }
        if bundle.envelope.receipt.status != CommitStatus::Committed {
            return Err(StoreError::ContinuityFence("receipt_status"));
        }
        if bundle.envelope.receipt.next_revision
            != bundle
                .envelope
                .receipt
                .base_revision
                .checked_add(1)
                .ok_or(StoreError::ContinuityFence("revision_overflow"))?
        {
            return Err(StoreError::ContinuityFence("receipt_revision"));
        }

        let scope = scope_from_event(&event);
        let expected_scope = wire::persona_scope_digest(
            &scope.bot_token,
            &scope.persona_token,
            scope.relation_token.as_ref(),
        );
        if bundle.envelope.receipt.scope_digest != expected_scope {
            return Err(StoreError::ContinuityFence("scope"));
        }
        let expected_relation_token = scope.relation_token.unwrap_or(scope.session_token);
        if bundle.context.relation_scope_token != expected_relation_token {
            return Err(StoreError::ContinuityFence("context_relation_scope"));
        }
        if bundle.snapshot.state_digest != bundle.envelope.receipt.state_after {
            return Err(StoreError::ContinuityFence("snapshot_receipt_digest"));
        }
        if bundle.graph.graph_digest != bundle.envelope.receipt.graph_after {
            return Err(StoreError::ContinuityFence("graph_receipt_digest"));
        }
        if bundle.graph.formula_digest != bundle.envelope.receipt.formula_digest {
            return Err(StoreError::ContinuityFence("graph_formula"));
        }
        if bundle.graph.delta_bytes != bundle.envelope.delta_bytes {
            return Err(StoreError::ContinuityFence("graph_delta"));
        }
        if bundle.context.source_continuum_revision != bundle.envelope.receipt.next_revision {
            return Err(StoreError::ContinuityFence("context_revision"));
        }
        if continuity_context_digest(&bundle.context.canonical_state_bytes)
            != bundle.context.context_digest
        {
            return Err(StoreError::ContinuityFence("context_digest"));
        }
        Ok(event)
    }

    fn read_journal_row_tx(
        tx: &Transaction<'_>,
        scope_digest: &Digest,
        revision: u64,
    ) -> Result<Option<JournalRow>, StoreError> {
        let stored: Option<StoredJournalColumns> = tx
            .query_row(
                "SELECT base_revision, event_kind, event_bytes, event_digest, receipt_bytes, chain_digest FROM journal WHERE scope_digest = ?1 AND logical_revision = ?2",
                params![blob(*scope_digest), revision as i64],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            base_revision,
            event_kind,
            event_bytes,
            event_digest,
            receipt_bytes,
            chain_digest,
        )) = stored
        else {
            return Ok(None);
        };
        Ok(Some(JournalRow {
            revision,
            scope_digest: *scope_digest,
            base_revision: base_revision as u64,
            event_kind,
            event_bytes,
            event_digest: digest_from_blob(&event_digest, "stored_event_digest")?,
            receipt_bytes,
            chain_digest: digest_from_blob(&chain_digest, "stored_chain_digest")?,
        }))
    }

    fn current_snapshot_digest_tx(
        tx: &Transaction<'_>,
        scope_digest: &Digest,
    ) -> Result<Option<Digest>, StoreError> {
        let bytes: Option<Vec<u8>> = tx
            .query_row(
                "SELECT state_digest FROM snapshots WHERE scope_digest = ?1 ORDER BY revision DESC LIMIT 1",
                params![blob(*scope_digest)],
                |row| row.get(0),
            )
            .optional()?;
        bytes
            .map(|value| digest_from_blob(&value, "stored_snapshot_digest"))
            .transpose()
    }

    fn current_graph_authority_tx(
        tx: &Transaction<'_>,
        scope_digest: &Digest,
        event_scope: &ScopeRef,
    ) -> Result<Option<(Digest, Digest)>, StoreError> {
        let latest: Option<(Vec<u8>, Vec<u8>)> = tx
            .query_row(
                "SELECT graph_digest, formula_digest FROM graph_commits WHERE scope_digest = ?1 ORDER BY revision DESC LIMIT 1",
                params![blob(*scope_digest)],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let authority = match latest {
            Some(value) => Some(value),
            // Semantic Phase 0 uses a relation-scoped continuity namespace
            // with its own formula and graph lineage.  It must materialize its
            // first graph against that empty lineage, never borrow the persona
            // genesis graph/formula authority.
            None if event_scope.relation_token.is_some() => None,
            None => tx
                .query_row(
                    "SELECT i.graph_digest, i.formula_digest FROM active_bindings AS b JOIN incarnations AS i ON i.incarnation_id = b.incarnation_id WHERE b.bot_token = ?1 AND b.persona_token = ?2",
                    params![
                        blob(event_scope.bot_token),
                        blob(event_scope.persona_token),
                    ],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?,
        };
        authority
            .map(|(graph_digest, formula_digest)| {
                Ok((
                    digest_from_blob(&graph_digest, "stored_graph_digest")?,
                    digest_from_blob(&formula_digest, "stored_graph_formula")?,
                ))
            })
            .transpose()
    }

    fn last_chain_digest_tx(
        tx: &Transaction<'_>,
        scope_digest: &Digest,
    ) -> Result<Option<Digest>, StoreError> {
        let bytes: Option<Vec<u8>> = tx
            .query_row(
                "SELECT chain_digest FROM journal WHERE scope_digest = ?1 ORDER BY logical_revision DESC LIMIT 1",
                params![blob(*scope_digest)],
                |row| row.get(0),
            )
            .optional()?;
        bytes
            .map(|value| digest_from_blob(&value, "stored_chain_digest"))
            .transpose()
    }

    fn duplicate_bundle_is_identical_tx(
        tx: &Transaction<'_>,
        bundle: &ContinuityCommitBundleV1,
        row: &JournalRow,
    ) -> Result<bool, StoreError> {
        let receipt_bytes = wire::encode_transition_receipt(&bundle.envelope.receipt);
        let expected_chain = ae_continuum::chain_link(
            &bundle.envelope.chain_seed,
            &bundle.envelope.event_bytes,
            &receipt_bytes,
        );
        if row.revision != bundle.envelope.receipt.next_revision
            || row.base_revision != bundle.envelope.receipt.base_revision
            || row.event_kind != bundle.envelope.event_kind
            || row.event_bytes != bundle.envelope.event_bytes
            || row.event_digest != bundle.envelope.receipt.event_digest
            || row.receipt_bytes != receipt_bytes
            || row.chain_digest != expected_chain
        {
            return Ok(false);
        }

        let snapshot: Option<(Vec<u8>, Vec<u8>)> = tx
            .query_row(
                "SELECT state_digest, state_bytes FROM snapshots WHERE scope_digest = ?1 AND revision = ?2",
                params![blob(bundle.envelope.receipt.scope_digest), row.revision as i64],
                |stored| Ok((stored.get(0)?, stored.get(1)?)),
            )
            .optional()?;
        if snapshot
            != Some((
                blob(bundle.snapshot.state_digest),
                bundle.snapshot.state_bytes.clone(),
            ))
        {
            return Ok(false);
        }

        let graph: Option<StoredGraphColumns> = tx
            .query_row(
                "SELECT base_graph_digest, graph_digest, formula_digest, delta_bytes, replay_state_bytes FROM graph_commits WHERE scope_digest = ?1 AND revision = ?2",
                params![blob(bundle.envelope.receipt.scope_digest), row.revision as i64],
                |stored| {
                    Ok((
                        stored.get(0)?,
                        stored.get(1)?,
                        stored.get(2)?,
                        stored.get(3)?,
                        stored.get(4)?,
                    ))
                },
            )
            .optional()?;
        if graph
            != Some((
                blob(bundle.graph.base_graph_digest),
                blob(bundle.graph.graph_digest),
                blob(bundle.graph.formula_digest),
                bundle.graph.delta_bytes.clone(),
                bundle.graph.replay_state_bytes.clone(),
            ))
        {
            return Ok(false);
        }

        let context: Option<StoredContextDuplicateColumns> = tx
            .query_row(
                "SELECT relation_hmac, context_digest, canonical_state_bytes, relation_scope_token FROM context_commits WHERE scope_digest = ?1 AND relation_scope_token = ?2 AND revision = ?3",
                params![
                    blob(bundle.envelope.receipt.scope_digest),
                    blob(bundle.context.relation_scope_token),
                    row.revision as i64,
                ],
                |stored| Ok((stored.get(0)?, stored.get(1)?, stored.get(2)?, stored.get(3)?)),
            )
            .optional()?;
        Ok(context
            == Some((
                blob(bundle.context.relation_hmac),
                blob(bundle.context.context_digest),
                bundle.context.canonical_state_bytes.clone(),
                blob(bundle.context.relation_scope_token),
            )))
    }

    /// Atomically append one transition and its state, graph and context
    /// projections. Any validation or SQLite failure occurs before the one
    /// commit boundary, so an observer sees the old complete authority or the
    /// new complete authority and never a cross-domain partial write.
    pub fn commit_continuity_bundle(
        &mut self,
        bundle: &ContinuityCommitBundleV1,
    ) -> Result<(u64, JournalRow), StoreError> {
        let event = Self::validate_continuity_payloads(bundle)?;
        let event_scope = scope_from_event(&event);
        let scope_digest = bundle.envelope.receipt.scope_digest;
        let receipt_bytes = wire::encode_transition_receipt(&bundle.envelope.receipt);
        let chain_digest = ae_continuum::chain_link(
            &bundle.envelope.chain_seed,
            &bundle.envelope.event_bytes,
            &receipt_bytes,
        );

        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let duplicate: Option<i64> = tx
            .query_row(
                "SELECT revision FROM applied_events WHERE scope_digest = ?1 AND event_digest = ?2",
                params![
                    blob(scope_digest),
                    blob(bundle.envelope.receipt.event_digest)
                ],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(revision) = duplicate {
            let revision = revision as u64;
            let row = Self::read_journal_row_tx(&tx, &scope_digest, revision)?
                .ok_or(StoreError::ContinuityIncomplete)?;
            if !Self::duplicate_bundle_is_identical_tx(&tx, bundle, &row)? {
                return Err(StoreError::ContinuityDuplicateMismatch);
            }
            tx.commit()?;
            return Ok((revision, row));
        }

        let current = tx.query_row(
            "SELECT COALESCE(MAX(logical_revision), 0) FROM journal WHERE scope_digest = ?1",
            params![blob(scope_digest)],
            |row| row.get::<_, i64>(0),
        )? as u64;
        if bundle.envelope.receipt.base_revision != current {
            return Err(StoreError::StaleRevision {
                expected: bundle.envelope.receipt.base_revision,
                actual: current,
            });
        }
        let expected_next = current
            .checked_add(1)
            .ok_or(StoreError::ContinuityFence("revision_overflow"))?;
        if bundle.envelope.receipt.next_revision != expected_next {
            return Err(StoreError::StaleRevision {
                expected: bundle.envelope.receipt.next_revision,
                actual: expected_next,
            });
        }

        match Self::current_snapshot_digest_tx(&tx, &scope_digest)? {
            Some(current_state) => {
                if bundle.envelope.receipt.state_before != current_state {
                    return Err(StoreError::ContinuityFence("state_before"));
                }
            }
            None if current != 0 => return Err(StoreError::ContinuityFence("missing_snapshot")),
            None => {}
        }
        match Self::current_graph_authority_tx(&tx, &scope_digest, event_scope)? {
            Some((current_graph, current_formula)) => {
                if bundle.graph.base_graph_digest != current_graph {
                    return Err(StoreError::ContinuityFence("graph_base"));
                }
                if bundle.graph.formula_digest != current_formula {
                    return Err(StoreError::ContinuityFence("graph_current_formula"));
                }
            }
            None if current != 0 => return Err(StoreError::ContinuityFence("missing_graph")),
            None => {}
        }
        if let Some(last_chain) = Self::last_chain_digest_tx(&tx, &scope_digest)? {
            if bundle.envelope.chain_seed != last_chain {
                return Err(StoreError::ContinuityFence("chain_seed"));
            }
        }

        tx.execute(
            "INSERT INTO journal (logical_revision, scope_digest, base_revision, event_kind, event_bytes, event_digest, receipt_bytes, chain_digest, committed_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                expected_next as i64,
                blob(scope_digest),
                bundle.envelope.receipt.base_revision as i64,
                bundle.envelope.event_kind.clone(),
                bundle.envelope.event_bytes.clone(),
                blob(bundle.envelope.receipt.event_digest),
                receipt_bytes.clone(),
                blob(chain_digest),
                now_ms() as i64,
            ],
        )?;
        tx.execute(
            "INSERT INTO applied_events (scope_digest, event_digest, revision) VALUES (?1, ?2, ?3)",
            params![
                blob(scope_digest),
                blob(bundle.envelope.receipt.event_digest),
                expected_next as i64,
            ],
        )?;
        tx.execute(
            "INSERT INTO snapshots (revision, scope_digest, state_digest, state_bytes) VALUES (?1, ?2, ?3, ?4)",
            params![
                expected_next as i64,
                blob(scope_digest),
                blob(bundle.snapshot.state_digest),
                bundle.snapshot.state_bytes.clone(),
            ],
        )?;
        tx.execute(
            "INSERT INTO graph_commits (scope_digest, revision, base_graph_digest, graph_digest, formula_digest, delta_bytes, replay_state_bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                blob(scope_digest),
                expected_next as i64,
                blob(bundle.graph.base_graph_digest),
                blob(bundle.graph.graph_digest),
                blob(bundle.graph.formula_digest),
                bundle.graph.delta_bytes.clone(),
                bundle.graph.replay_state_bytes.clone(),
            ],
        )?;
        tx.execute(
            "INSERT INTO context_commits (scope_digest, relation_scope_token, relation_hmac, revision, context_digest, canonical_state_bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                blob(scope_digest),
                blob(bundle.context.relation_scope_token),
                blob(bundle.context.relation_hmac),
                expected_next as i64,
                blob(bundle.context.context_digest),
                bundle.context.canonical_state_bytes.clone(),
            ],
        )?;
        tx.commit()?;

        let row = JournalRow {
            revision: expected_next,
            scope_digest,
            base_revision: bundle.envelope.receipt.base_revision,
            event_kind: bundle.envelope.event_kind.clone(),
            event_bytes: bundle.envelope.event_bytes.clone(),
            event_digest: bundle.envelope.receipt.event_digest,
            receipt_bytes,
            chain_digest,
        };
        Ok((expected_next, row))
    }

    /// Return the newest fully committed opaque context projection for one raw
    /// relation storage token. Digest validation happens again on read so a
    /// corrupt row can never become a projection input after restart.
    pub fn read_context_commit(
        &self,
        scope_digest: &Digest,
        relation_scope_token: &[u8; 16],
    ) -> Result<Option<ContextCommitRowV1>, StoreError> {
        let conn = self.connection()?;
        let stored: Option<StoredContextColumns> = conn
            .query_row(
                "SELECT relation_scope_token, relation_hmac, revision, context_digest, canonical_state_bytes FROM context_commits WHERE scope_digest = ?1 AND relation_scope_token = ?2 ORDER BY revision DESC LIMIT 1",
                params![blob(*scope_digest), blob(*relation_scope_token)],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .optional()?;
        let Some((stored_token, relation_hmac, revision, context_digest, canonical_state_bytes)) =
            stored
        else {
            return Ok(None);
        };
        let context_digest = digest_from_blob(&context_digest, "stored_context_digest")?;
        if continuity_context_digest(&canonical_state_bytes) != context_digest {
            return Err(StoreError::ContinuityFence("stored_context_digest"));
        }
        Ok(Some(ContextCommitRowV1 {
            scope_digest: *scope_digest,
            relation_scope_token: token_from_blob(&stored_token, "stored_relation_scope_token")?,
            relation_hmac: digest_from_blob(&relation_hmac, "stored_relation_hmac")?,
            revision: revision as u64,
            context_digest,
            canonical_state_bytes,
        }))
    }

    /// CAS commit of one journal entry. The caller supplies the chain seed
    /// (genesis snapshot digest for the first entry, previous chain digest
    /// afterwards); the store verifies it against its own last chain digest
    /// and appends atomically. Duplicate events update zero rows and fail.
    pub fn commit_journal(
        &mut self,
        envelope: &CommitEnvelope,
    ) -> Result<(u64, JournalRow), StoreError> {
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let current = tx.query_row(
            "SELECT COALESCE(MAX(logical_revision), 0) FROM journal WHERE scope_digest = ?1",
            params![blob(envelope.receipt.scope_digest)],
            |row| row.get::<_, i64>(0),
        )? as u64;
        if envelope.receipt.base_revision != current {
            return Err(StoreError::StaleRevision {
                expected: envelope.receipt.base_revision,
                actual: current,
            });
        }
        if envelope.receipt.next_revision != current + 1 {
            return Err(StoreError::StaleRevision {
                expected: envelope.receipt.next_revision,
                actual: current + 1,
            });
        }

        let event = wire::decode_event(&envelope.event_bytes)
            .map_err(|error| StoreError::Sqlite(format!("event decode failed: {error}")))?;
        let event_digest = wire::event_digest(&event);
        if event_digest != envelope.receipt.event_digest {
            return Err(StoreError::StaleRevision {
                expected: 0,
                actual: 0,
            });
        }

        let duplicate: Option<i64> = tx
            .query_row(
                "SELECT revision FROM applied_events WHERE scope_digest = ?1 AND event_digest = ?2",
                params![blob(envelope.receipt.scope_digest), blob(event_digest)],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(revision) = duplicate {
            return Err(StoreError::DuplicateEvent(revision as u64));
        }

        let last_chain: Option<Vec<u8>> = tx
            .query_row(
            "SELECT chain_digest FROM journal WHERE scope_digest = ?1 ORDER BY logical_revision DESC LIMIT 1",
                params![blob(envelope.receipt.scope_digest)],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(bytes) = last_chain {
            if bytes != envelope.chain_seed.to_vec() {
                return Err(StoreError::StaleRevision {
                    expected: 0,
                    actual: 0,
                });
            }
        }

        let receipt_bytes = wire::encode_transition_receipt(&envelope.receipt);
        let chain_digest =
            ae_continuum::chain_link(&envelope.chain_seed, &envelope.event_bytes, &receipt_bytes);
        tx.execute(
            "INSERT INTO journal (logical_revision, scope_digest, base_revision, event_kind, event_bytes, event_digest, receipt_bytes, chain_digest, committed_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                (current + 1) as i64,
                blob(envelope.receipt.scope_digest),
                envelope.receipt.base_revision as i64,
                envelope.event_kind.clone(),
                envelope.event_bytes.clone(),
                blob(event_digest),
                receipt_bytes.clone(),
                blob(chain_digest),
                now_ms() as i64,
            ],
        )?;
        let revision = current + 1;
        tx.execute(
            "INSERT INTO applied_events (scope_digest, event_digest, revision) VALUES (?1, ?2, ?3)",
            params![
                blob(envelope.receipt.scope_digest),
                blob(event_digest),
                revision as i64,
            ],
        )?;
        tx.commit()?;
        let row = JournalRow {
            revision,
            scope_digest: envelope.receipt.scope_digest,
            base_revision: envelope.receipt.base_revision,
            event_kind: envelope.event_kind.clone(),
            event_bytes: envelope.event_bytes.clone(),
            event_digest,
            receipt_bytes,
            chain_digest,
        };
        Ok((revision, row))
    }

    // ------------------------------------------------ learning compensation

    fn learning_compensation_enqueue_receipt_tx(
        tx: &Transaction<'_>,
        job_id: &Digest,
    ) -> Result<Option<LearningCompensationEnqueueReceiptRowV1>, StoreError> {
        let stored: Option<(Vec<u8>, Vec<u8>)> = tx
            .query_row(
                "SELECT receipt_bytes, receipt_digest FROM learning_compensation_enqueue_receipts WHERE job_id = ?1",
                params![blob(*job_id)],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        stored
            .map(|(receipt_bytes, receipt_digest)| {
                if receipt_bytes.is_empty() {
                    return Err(StoreError::LearningCompensationInvalid);
                }
                Ok(LearningCompensationEnqueueReceiptRowV1 {
                    job_id: *job_id,
                    receipt_bytes,
                    receipt_digest: digest_from_blob(&receipt_digest, "learning_enqueue_receipt")?,
                })
            })
            .transpose()
    }

    fn learning_compensation_job_tx(
        tx: &Transaction<'_>,
        job_id: &Digest,
    ) -> Result<Option<LearningCompensationJobRowV1>, StoreError> {
        let stored: Option<StoredLearningCompensationJobColumns> = tx
            .query_row(
                "SELECT job_id, scope_digest, source_event_digest, source_text_digest, source_base_revision, request_digest, request_bytes, policy_digest, schema_digest, formula_digest, telemetry_digest, checkpoint_digest, status, lease_token, lease_epoch, terminal_reason_digest, receipt_bytes, receipt_digest FROM learning_compensation_jobs WHERE job_id = ?1",
                params![blob(*job_id)],
                |row| {
                    Ok((
                        row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?,
                        row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?,
                        row.get(10)?, row.get(11)?, row.get(12)?, row.get(13)?, row.get(14)?,
                        row.get(15)?, row.get(16)?, row.get(17)?,
                    ))
                },
            )
            .optional()?;
        stored
            .map(learning_compensation_job_from_columns)
            .transpose()
    }

    fn learning_compensation_job_by_identity_tx(
        tx: &Transaction<'_>,
        job: &NewLearningCompensationJobV1,
    ) -> Result<Option<LearningCompensationJobRowV1>, StoreError> {
        let stored: Option<StoredLearningCompensationJobColumns> = tx
            .query_row(
                "SELECT job_id, scope_digest, source_event_digest, source_text_digest, source_base_revision, request_digest, request_bytes, policy_digest, schema_digest, formula_digest, telemetry_digest, checkpoint_digest, status, lease_token, lease_epoch, terminal_reason_digest, receipt_bytes, receipt_digest FROM learning_compensation_jobs WHERE scope_digest = ?1 AND source_event_digest = ?2 AND source_text_digest = ?3 AND policy_digest = ?4 AND schema_digest = ?5 AND formula_digest = ?6",
                params![
                    blob(job.scope_digest),
                    blob(job.source_event_digest),
                    blob(job.source_text_digest),
                    blob(job.policy_digest),
                    blob(job.schema_digest),
                    blob(job.formula_digest),
                ],
                |row| {
                    Ok((
                        row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?,
                        row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?,
                        row.get(10)?, row.get(11)?, row.get(12)?, row.get(13)?, row.get(14)?,
                        row.get(15)?, row.get(16)?, row.get(17)?,
                    ))
                },
            )
            .optional()?;
        stored
            .map(learning_compensation_job_from_columns)
            .transpose()
    }

    fn learning_compensation_job_matches(
        row: &LearningCompensationJobRowV1,
        job: &NewLearningCompensationJobV1,
    ) -> bool {
        row.job_id == job.job_id
            && row.scope_digest == job.scope_digest
            && row.source_event_digest == job.source_event_digest
            && row.source_text_digest == job.source_text_digest
            && row.source_base_revision == job.source_base_revision
            && row.request_digest == job.request_digest
            && row.request_bytes == job.request_bytes
            && row.policy_digest == job.policy_digest
            && row.schema_digest == job.schema_digest
            && row.formula_digest == job.formula_digest
            && row.telemetry_digest == job.telemetry_digest
            && row.checkpoint_digest == job.checkpoint_digest
    }

    fn learning_compensation_enqueue_receipt_matches_job(
        job: &NewLearningCompensationJobV1,
    ) -> bool {
        let request =
            match SemanticLearningCompensationEnqueueV1::decode_canonical_bytes(&job.request_bytes)
            {
                Some(value) => value,
                None => return false,
            };
        let receipt = match LearningCompensationEnqueueReceiptV1::decode_canonical_bytes(
            &job.enqueue_receipt_bytes,
        ) {
            Some(value) => value,
            None => return false,
        };
        receipt.receipt_digest == job.enqueue_receipt_digest
            && receipt.job_id == job.job_id
            && receipt.source_event_digest == job.source_event_digest
            && receipt.source_text_digest == job.source_text_digest
            && receipt.source_revision == job.source_base_revision
            && receipt.request_digest == job.request_digest
            && receipt.formula_digest == job.formula_digest
            && receipt.local_estimator_formula_digest == request.local_estimator_formula_digest
            && receipt.learning_formula_digest
                == learning_compensation_formula_digest_v1(
                    &request.formula_digest,
                    &request.local_estimator_formula_digest,
                    &request.policy_digest,
                )
            && receipt.source_telemetry_digest == job.telemetry_digest
            && receipt.source_checkpoint_digest == job.checkpoint_digest
            && receipt.policy_digest == job.policy_digest
            && receipt.provider_digest == request.provider_digest
            && receipt.model_digest == request.model_digest
            && receipt.prompt_digest == request.prompt_digest
            && receipt.schema_digest == job.schema_digest
    }

    fn learning_compensation_enqueue_receipt_matches_row(
        row: &LearningCompensationJobRowV1,
        persisted: &LearningCompensationEnqueueReceiptRowV1,
    ) -> bool {
        let request =
            match SemanticLearningCompensationEnqueueV1::decode_canonical_bytes(&row.request_bytes)
            {
                Some(value) => value,
                None => return false,
            };
        let receipt = match LearningCompensationEnqueueReceiptV1::decode_canonical_bytes(
            &persisted.receipt_bytes,
        ) {
            Some(value) => value,
            None => return false,
        };
        persisted.job_id == row.job_id
            && persisted.receipt_digest == receipt.receipt_digest
            && receipt.job_id == row.job_id
            && receipt.source_event_digest == row.source_event_digest
            && receipt.source_text_digest == row.source_text_digest
            && receipt.source_revision == row.source_base_revision
            && receipt.request_digest == row.request_digest
            && receipt.formula_digest == row.formula_digest
            && receipt.local_estimator_formula_digest == request.local_estimator_formula_digest
            && receipt.learning_formula_digest
                == learning_compensation_formula_digest_v1(
                    &request.formula_digest,
                    &request.local_estimator_formula_digest,
                    &request.policy_digest,
                )
            && receipt.source_telemetry_digest == row.telemetry_digest
            && receipt.source_checkpoint_digest == row.checkpoint_digest
            && receipt.policy_digest == row.policy_digest
            && receipt.provider_digest == request.provider_digest
            && receipt.model_digest == request.model_digest
            && receipt.prompt_digest == request.prompt_digest
            && receipt.schema_digest == row.schema_digest
    }

    pub fn enqueue_learning_compensation_job_v1(
        &mut self,
        job: &NewLearningCompensationJobV1,
    ) -> Result<LearningCompensationEnqueueOutcomeV1, StoreError> {
        if job.source_base_revision == 0
            || job.request_bytes.is_empty()
            || job.enqueue_receipt_bytes.is_empty()
            || [
                &job.job_id,
                &job.scope_digest,
                &job.source_event_digest,
                &job.source_text_digest,
                &job.request_digest,
                &job.policy_digest,
                &job.schema_digest,
                &job.formula_digest,
                &job.telemetry_digest,
                &job.checkpoint_digest,
                &job.enqueue_receipt_digest,
            ]
            .into_iter()
            .any(|digest| digest.iter().all(|byte| *byte == 0))
        {
            return Err(StoreError::LearningCompensationInvalid);
        }
        if !Self::learning_compensation_enqueue_receipt_matches_job(job) {
            return Err(StoreError::LearningCompensationInvalid);
        }
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(row) = Self::learning_compensation_job_tx(&tx, &job.job_id)? {
            if !Self::learning_compensation_job_matches(&row, job) {
                return Err(StoreError::LearningCompensationConflict);
            }
            let enqueue_receipt = Self::learning_compensation_enqueue_receipt_tx(&tx, &job.job_id)?
                .ok_or(StoreError::LearningCompensationInvalid)?;
            if enqueue_receipt.receipt_bytes != job.enqueue_receipt_bytes
                || enqueue_receipt.receipt_digest != job.enqueue_receipt_digest
            {
                return Err(StoreError::LearningCompensationConflict);
            }
            tx.commit()?;
            return Ok(LearningCompensationEnqueueOutcomeV1::Replayed {
                job: row,
                enqueue_receipt,
            });
        }
        if let Some(row) = Self::learning_compensation_job_by_identity_tx(&tx, job)? {
            if !Self::learning_compensation_job_matches(&row, job) {
                return Err(StoreError::LearningCompensationConflict);
            }
            let enqueue_receipt = Self::learning_compensation_enqueue_receipt_tx(&tx, &row.job_id)?
                .ok_or(StoreError::LearningCompensationInvalid)?;
            if enqueue_receipt.receipt_bytes != job.enqueue_receipt_bytes
                || enqueue_receipt.receipt_digest != job.enqueue_receipt_digest
            {
                return Err(StoreError::LearningCompensationConflict);
            }
            tx.commit()?;
            return Ok(LearningCompensationEnqueueOutcomeV1::Replayed {
                job: row,
                enqueue_receipt,
            });
        }
        let now = now_ms();
        tx.execute(
            "INSERT INTO learning_compensation_jobs (job_id, scope_digest, source_event_digest, source_text_digest, source_base_revision, request_digest, request_bytes, policy_digest, schema_digest, formula_digest, telemetry_digest, checkpoint_digest, status, lease_token, lease_epoch, terminal_reason_digest, receipt_bytes, receipt_digest, created_at_ms, updated_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'pending', NULL, 0, NULL, NULL, NULL, ?13, ?13)",
            params![
                blob(job.job_id), blob(job.scope_digest), blob(job.source_event_digest),
                blob(job.source_text_digest), job.source_base_revision as i64,
                blob(job.request_digest), job.request_bytes.clone(), blob(job.policy_digest),
                blob(job.schema_digest), blob(job.formula_digest), blob(job.telemetry_digest),
                blob(job.checkpoint_digest), now as i64,
            ],
        )?;
        tx.execute(
            "INSERT INTO learning_compensation_enqueue_receipts (job_id, receipt_bytes, receipt_digest) VALUES (?1, ?2, ?3)",
            params![
                blob(job.job_id),
                job.enqueue_receipt_bytes.clone(),
                blob(job.enqueue_receipt_digest),
            ],
        )?;
        let row = Self::learning_compensation_job_tx(&tx, &job.job_id)?
            .ok_or(StoreError::LearningCompensationInvalid)?;
        let enqueue_receipt = Self::learning_compensation_enqueue_receipt_tx(&tx, &job.job_id)?
            .ok_or(StoreError::LearningCompensationInvalid)?;
        tx.commit()?;
        Ok(LearningCompensationEnqueueOutcomeV1::Queued {
            job: row,
            enqueue_receipt,
        })
    }

    pub fn read_learning_compensation_job_v1(
        &self,
        job_id: &Digest,
    ) -> Result<Option<LearningCompensationJobRowV1>, StoreError> {
        let conn = self.connection()?;
        let stored: Option<StoredLearningCompensationJobColumns> = conn
            .query_row(
                "SELECT job_id, scope_digest, source_event_digest, source_text_digest, source_base_revision, request_digest, request_bytes, policy_digest, schema_digest, formula_digest, telemetry_digest, checkpoint_digest, status, lease_token, lease_epoch, terminal_reason_digest, receipt_bytes, receipt_digest FROM learning_compensation_jobs WHERE job_id = ?1",
                params![blob(*job_id)],
                |row| {
                    Ok((
                        row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?,
                        row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?,
                        row.get(10)?, row.get(11)?, row.get(12)?, row.get(13)?, row.get(14)?,
                        row.get(15)?, row.get(16)?, row.get(17)?,
                    ))
                },
            )
            .optional()?;
        stored
            .map(learning_compensation_job_from_columns)
            .transpose()
    }

    pub fn read_learning_compensation_enqueue_receipt_v1(
        &self,
        job_id: &Digest,
    ) -> Result<Option<LearningCompensationEnqueueReceiptRowV1>, StoreError> {
        let conn = self.connection()?;
        let stored: Option<(Vec<u8>, Vec<u8>)> = conn
            .query_row(
                "SELECT receipt_bytes, receipt_digest FROM learning_compensation_enqueue_receipts WHERE job_id = ?1",
                params![blob(*job_id)],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        stored
            .map(|(receipt_bytes, receipt_digest)| {
                if receipt_bytes.is_empty() {
                    return Err(StoreError::LearningCompensationInvalid);
                }
                Ok(LearningCompensationEnqueueReceiptRowV1 {
                    job_id: *job_id,
                    receipt_bytes,
                    receipt_digest: digest_from_blob(&receipt_digest, "learning_enqueue_receipt")?,
                })
            })
            .transpose()
    }

    pub fn claim_learning_compensation_job_v1(
        &mut self,
        job_id: &Digest,
        previous_lease_token: Option<Digest>,
    ) -> Result<LearningCompensationJobRowV1, StoreError> {
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = Self::learning_compensation_job_tx(&tx, job_id)?
            .ok_or(StoreError::LearningCompensationJobNotFound)?;
        if row.status == LearningCompensationJobStatusV1::Pending {
            if previous_lease_token.is_some() {
                return Err(StoreError::LearningCompensationConflict);
            }
            let lease_epoch = row
                .lease_epoch
                .checked_add(1)
                .ok_or(StoreError::LearningCompensationInvalid)?;
            let epoch = lease_epoch.to_le_bytes();
            let now = now_ms().to_le_bytes();
            let lease_token = wire::domain_hash(
                b"astr-embodiment/phase0-learning-lease-v1",
                &[&row.job_id, &row.request_digest, &epoch, &now],
            );
            let changed = tx.execute(
                "UPDATE learning_compensation_jobs SET status = 'claimed', lease_token = ?2, lease_epoch = ?3, updated_at_ms = ?4 WHERE job_id = ?1 AND status = 'pending'",
                params![blob(*job_id), blob(lease_token), lease_epoch as i64, now_ms() as i64],
            )?;
            if changed != 1 {
                return Err(StoreError::LearningCompensationConflict);
            }
        } else if row.status == LearningCompensationJobStatusV1::Claimed {
            if row.lease_token != previous_lease_token {
                return Err(StoreError::LearningCompensationConflict);
            }
            let lease_epoch = row
                .lease_epoch
                .checked_add(1)
                .ok_or(StoreError::LearningCompensationInvalid)?;
            let epoch = lease_epoch.to_le_bytes();
            let now = now_ms().to_le_bytes();
            let lease_token = wire::domain_hash(
                b"astr-embodiment/phase0-learning-lease-v1",
                &[&row.job_id, &row.request_digest, &epoch, &now],
            );
            let changed = tx.execute(
                "UPDATE learning_compensation_jobs SET lease_token = ?2, lease_epoch = ?3, updated_at_ms = ?4 WHERE job_id = ?1 AND status = 'claimed' AND lease_token = ?5",
                params![blob(*job_id), blob(lease_token), lease_epoch as i64, now_ms() as i64, blob(previous_lease_token.ok_or(StoreError::LearningCompensationConflict)?)],
            )?;
            if changed != 1 {
                return Err(StoreError::LearningCompensationConflict);
            }
        }
        if matches!(
            row.status,
            LearningCompensationJobStatusV1::Pending | LearningCompensationJobStatusV1::Claimed
        ) {
            tx.execute(
                "DELETE FROM learning_compensation_claim_bindings WHERE job_id = ?1",
                params![blob(*job_id)],
            )?;
        }
        let claimed = Self::learning_compensation_job_tx(&tx, job_id)?
            .ok_or(StoreError::LearningCompensationInvalid)?;
        tx.commit()?;
        Ok(claimed)
    }

    /// Persist the exact telemetry cursor carried by a lease. This makes
    /// apply and terminalization reject a host-supplied checkpoint from any
    /// other claim attempt.
    pub fn bind_learning_compensation_claim_v1(
        &mut self,
        binding: &LearningCompensationClaimBindingV1,
    ) -> Result<(), StoreError> {
        if binding.base_revision == 0
            || [
                &binding.job_id,
                &binding.lease_token,
                &binding.formula_digest,
                &binding.telemetry_digest,
                &binding.checkpoint_digest,
            ]
            .into_iter()
            .any(|digest| digest.iter().all(|byte| *byte == 0))
        {
            return Err(StoreError::LearningCompensationInvalid);
        }
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = Self::learning_compensation_job_tx(&tx, &binding.job_id)?
            .ok_or(StoreError::LearningCompensationJobNotFound)?;
        if row.status != LearningCompensationJobStatusV1::Claimed
            || row.lease_token != Some(binding.lease_token)
        {
            return Err(StoreError::LearningCompensationConflict);
        }
        let semantic_revision: i64 = tx.query_row(
            "SELECT COALESCE(MAX(revision), 0) FROM journal WHERE scope_digest = ?1",
            params![blob(row.scope_digest)],
            |value| value.get(0),
        )?;
        if semantic_revision < 0 || semantic_revision as u64 != binding.base_revision {
            return Err(StoreError::LearningCompensationConflict);
        }
        tx.execute(
            "INSERT INTO learning_compensation_claim_bindings (job_id, lease_token, base_revision, formula_digest, telemetry_digest, checkpoint_digest) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(job_id) DO UPDATE SET lease_token = excluded.lease_token, base_revision = excluded.base_revision, formula_digest = excluded.formula_digest, telemetry_digest = excluded.telemetry_digest, checkpoint_digest = excluded.checkpoint_digest",
            params![blob(binding.job_id), blob(binding.lease_token), binding.base_revision as i64, blob(binding.formula_digest), blob(binding.telemetry_digest), blob(binding.checkpoint_digest)],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// A restarted native process no longer has the in-memory raw text needed
    /// to ask a teacher. Persisted PENDING/CLAIMED jobs therefore atomically
    /// become receipt-backed `ABANDONED_INPUT_UNAVAILABLE` terminals. The
    /// receipt is derived solely from the durable text-free request body, so a
    /// later enqueue/claim can deterministically surface the same seal.
    pub fn abandon_unavailable_learning_compensation_jobs_v1(&mut self) -> Result<u64, StoreError> {
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let reason = wire::domain_hash(
            b"astr-embodiment/phase0-learning-restart-abandon-v1",
            &[b"ABANDONED_INPUT_UNAVAILABLE"],
        );
        let stored_rows: Vec<StoredLearningCompensationJobColumns> = {
            let mut statement = tx.prepare(
                "SELECT job_id, scope_digest, source_event_digest, source_text_digest, source_base_revision, request_digest, request_bytes, policy_digest, schema_digest, formula_digest, telemetry_digest, checkpoint_digest, status, lease_token, lease_epoch, terminal_reason_digest, receipt_bytes, receipt_digest FROM learning_compensation_jobs WHERE status IN ('pending', 'claimed') ORDER BY job_id",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                    row.get(11)?,
                    row.get(12)?,
                    row.get(13)?,
                    row.get(14)?,
                    row.get(15)?,
                    row.get(16)?,
                    row.get(17)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let rows = stored_rows
            .into_iter()
            .map(learning_compensation_job_from_columns)
            .collect::<Result<Vec<_>, _>>()?;
        let recovered = rows.len();
        for row in rows {
            let request =
                SemanticLearningCompensationEnqueueV1::decode_canonical_bytes(&row.request_bytes)
                    .ok_or(StoreError::LearningCompensationInvalid)?;
            if row.source_event_digest != request.source_event_digest
                || row.source_text_digest != request.source_text_digest
                || row.source_base_revision != request.source_revision
                || row.request_digest != request.request_digest(&row.scope_digest)
                || row.policy_digest != request.policy_digest
                || request.policy_digest != canonical_one_learning_compensation_policy_digest_v1()
                || row.schema_digest != request.schema_digest
                || row.formula_digest != request.formula_digest
                || row.telemetry_digest != request.source_telemetry_digest
                || row.checkpoint_digest != request.source_checkpoint_digest
            {
                return Err(StoreError::LearningCompensationInvalid);
            }
            let enqueue_receipt = match Self::learning_compensation_enqueue_receipt_tx(
                &tx,
                &row.job_id,
            )? {
                Some(value) => value,
                None => {
                    // v3.1 introduced a distinct durable enqueue attestation.
                    // An older text-free PENDING/CLAIMED row can be migrated
                    // deterministically from its sealed request before restart
                    // recovery derives the terminal receipt; malformed existing
                    // receipts are never replaced.
                    let receipt = LearningCompensationEnqueueReceiptV1 {
                        schema: LEARNING_COMPENSATION_ENQUEUE_RECEIPT_SCHEMA_V1.to_owned(),
                        job_id: row.job_id,
                        source_event_digest: request.source_event_digest,
                        source_text_digest: request.source_text_digest,
                        source_revision: request.source_revision,
                        request_digest: row.request_digest,
                        formula_digest: request.formula_digest,
                        local_estimator_formula_digest: request.local_estimator_formula_digest,
                        learning_formula_digest: learning_compensation_formula_digest_v1(
                            &request.formula_digest,
                            &request.local_estimator_formula_digest,
                            &request.policy_digest,
                        ),
                        source_telemetry_digest: request.source_telemetry_digest,
                        source_checkpoint_digest: request.source_checkpoint_digest,
                        policy_digest: request.policy_digest,
                        provider_digest: request.provider_digest,
                        model_digest: request.model_digest,
                        prompt_digest: request.prompt_digest,
                        schema_digest: request.schema_digest,
                        receipt_digest: [0; 32],
                    }
                    .seal();
                    if !receipt.validate() {
                        return Err(StoreError::LearningCompensationInvalid);
                    }
                    let mut receipt_bytes = receipt.canonical_bytes_without_receipt_digest();
                    receipt_bytes.extend_from_slice(&receipt.receipt_digest);
                    tx.execute(
                        "INSERT INTO learning_compensation_enqueue_receipts (job_id, receipt_bytes, receipt_digest) VALUES (?1, ?2, ?3)",
                        params![blob(row.job_id), receipt_bytes.clone(), blob(receipt.receipt_digest)],
                    )?;
                    LearningCompensationEnqueueReceiptRowV1 {
                        job_id: row.job_id,
                        receipt_bytes,
                        receipt_digest: receipt.receipt_digest,
                    }
                }
            };
            if !Self::learning_compensation_enqueue_receipt_matches_row(&row, &enqueue_receipt) {
                return Err(StoreError::LearningCompensationInvalid);
            }
            let receipt = LearningCompensationTerminalReceiptV1 {
                schema: LEARNING_COMPENSATION_TERMINAL_SCHEMA_V1.to_owned(),
                job_id: row.job_id,
                status: LearningCompensationTerminalStatusV1::AbandonedInputUnavailable,
                source_event_digest: request.source_event_digest,
                source_text_digest: request.source_text_digest,
                source_revision: request.source_revision,
                request_digest: row.request_digest,
                formula_digest: request.formula_digest,
                local_estimator_formula_digest: request.local_estimator_formula_digest,
                learning_formula_digest: learning_compensation_formula_digest_v1(
                    &request.formula_digest,
                    &request.local_estimator_formula_digest,
                    &request.policy_digest,
                ),
                policy_digest: request.policy_digest,
                provider_digest: request.provider_digest,
                model_digest: request.model_digest,
                prompt_digest: request.prompt_digest,
                schema_digest: request.schema_digest,
                reason_digest: reason,
                checkpoint_digest: request.source_checkpoint_digest,
                receipt_digest: [0; 32],
            }
            .seal();
            if !receipt.validate() {
                return Err(StoreError::LearningCompensationInvalid);
            }
            let mut receipt_bytes = receipt.canonical_bytes_without_receipt_digest();
            receipt_bytes.extend_from_slice(&receipt.receipt_digest);
            let changed = tx.execute(
                "UPDATE learning_compensation_jobs SET status = 'abandoned', lease_token = NULL, terminal_reason_digest = ?2, receipt_bytes = ?3, receipt_digest = ?4, updated_at_ms = ?5 WHERE job_id = ?1 AND status IN ('pending', 'claimed')",
                params![blob(row.job_id), blob(reason), receipt_bytes, blob(receipt.receipt_digest), now_ms() as i64],
            )?;
            if changed != 1 {
                return Err(StoreError::LearningCompensationConflict);
            }
            tx.execute(
                "DELETE FROM learning_compensation_claim_bindings WHERE job_id = ?1",
                params![blob(row.job_id)],
            )?;
        }
        tx.execute(
            "DELETE FROM learning_compensation_claim_bindings WHERE job_id NOT IN (SELECT job_id FROM learning_compensation_jobs WHERE status = 'claimed')",
            [],
        )?;
        tx.commit()?;
        u64::try_from(recovered).map_err(|_| StoreError::LearningCompensationInvalid)
    }

    pub fn read_learning_compensation_state_v1(
        &self,
        scope_digest: &Digest,
    ) -> Result<Option<LearningCompensationStateV1>, StoreError> {
        let conn = self.connection()?;
        let stored: Option<(i64, Vec<u8>, Vec<u8>)> = conn
            .query_row(
                "SELECT checkpoint_revision, u_bytes, compensation_digest FROM learning_compensation_state WHERE scope_digest = ?1",
                params![blob(*scope_digest)],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        stored
            .map(|(checkpoint_revision, u_bytes, compensation_digest)| {
                if checkpoint_revision < 0 || u_bytes.is_empty() {
                    return Err(StoreError::LearningCompensationInvalid);
                }
                Ok(LearningCompensationStateV1 {
                    scope_digest: *scope_digest,
                    checkpoint_revision: checkpoint_revision as u64,
                    u_bytes,
                    compensation_digest: digest_from_blob(
                        &compensation_digest,
                        "learning_state_digest",
                    )?,
                })
            })
            .transpose()
    }

    pub fn commit_learning_compensation_v1(
        &mut self,
        commit: &LearningCompensationCommitV1,
    ) -> Result<LearningCompensationJobRowV1, StoreError> {
        if commit.u_bytes.is_empty()
            || commit.receipt_bytes.is_empty()
            || [
                &commit.job_id,
                &commit.lease_token,
                &commit.expected_request_digest,
                &commit.expected_formula_digest,
                &commit.expected_telemetry_digest,
                &commit.expected_checkpoint_digest,
                &commit.compensation_digest,
                &commit.receipt_digest,
            ]
            .into_iter()
            .any(|digest| digest.iter().all(|byte| *byte == 0))
        {
            return Err(StoreError::LearningCompensationInvalid);
        }
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = Self::learning_compensation_job_tx(&tx, &commit.job_id)?
            .ok_or(StoreError::LearningCompensationJobNotFound)?;
        if row.status == LearningCompensationJobStatusV1::Committed {
            if row.request_digest != commit.expected_request_digest {
                return Err(StoreError::LearningCompensationConflict);
            }
            tx.commit()?;
            return Ok(row);
        }
        if row.status != LearningCompensationJobStatusV1::Claimed
            || row.lease_token != Some(commit.lease_token)
            || row.request_digest != commit.expected_request_digest
        {
            return Err(StoreError::LearningCompensationConflict);
        }
        let binding: (Vec<u8>, i64, Vec<u8>, Vec<u8>, Vec<u8>) = tx
            .query_row(
                "SELECT lease_token, base_revision, formula_digest, telemetry_digest, checkpoint_digest FROM learning_compensation_claim_bindings WHERE job_id = ?1",
                params![blob(commit.job_id)],
                |value| Ok((value.get(0)?, value.get(1)?, value.get(2)?, value.get(3)?, value.get(4)?)),
            )
            .optional()?
            .ok_or(StoreError::LearningCompensationConflict)?;
        if binding.1 < 0
            || digest_from_blob(&binding.0, "learning_claim_lease")? != commit.lease_token
            || binding.1 as u64 != commit.expected_semantic_revision
            || digest_from_blob(&binding.2, "learning_claim_formula")?
                != commit.expected_formula_digest
            || digest_from_blob(&binding.3, "learning_claim_telemetry")?
                != commit.expected_telemetry_digest
            || digest_from_blob(&binding.4, "learning_claim_checkpoint")?
                != commit.expected_checkpoint_digest
        {
            return Err(StoreError::LearningCompensationConflict);
        }
        let semantic_revision: i64 = tx.query_row(
            "SELECT COALESCE(MAX(revision), 0) FROM journal WHERE scope_digest = ?1",
            params![blob(row.scope_digest)],
            |value| value.get(0),
        )?;
        if semantic_revision < 0 || semantic_revision as u64 != commit.expected_semantic_revision {
            return Err(StoreError::LearningCompensationConflict);
        }
        let current_checkpoint: i64 = tx.query_row(
            "SELECT COALESCE((SELECT checkpoint_revision FROM learning_compensation_state WHERE scope_digest = ?1), 0)",
            params![blob(row.scope_digest)],
            |value| value.get(0),
        )?;
        if current_checkpoint < 0
            || commit.next_checkpoint_revision
                != (current_checkpoint as u64)
                    .checked_add(1)
                    .ok_or(StoreError::LearningCompensationInvalid)?
        {
            return Err(StoreError::LearningCompensationConflict);
        }
        tx.execute(
            "INSERT INTO learning_compensation_state (scope_digest, checkpoint_revision, u_bytes, compensation_digest) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(scope_digest) DO UPDATE SET checkpoint_revision = excluded.checkpoint_revision, u_bytes = excluded.u_bytes, compensation_digest = excluded.compensation_digest",
            params![blob(row.scope_digest), commit.next_checkpoint_revision as i64, commit.u_bytes.clone(), blob(commit.compensation_digest)],
        )?;
        tx.execute(
            "INSERT INTO learning_compensation_checkpoints (scope_digest, checkpoint_revision, job_id, compensation_digest, receipt_digest, u_bytes, receipt_bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![blob(row.scope_digest), commit.next_checkpoint_revision as i64, blob(commit.job_id), blob(commit.compensation_digest), blob(commit.receipt_digest), commit.u_bytes.clone(), commit.receipt_bytes.clone()],
        )?;
        let changed = tx.execute(
            "UPDATE learning_compensation_jobs SET status = 'committed', lease_token = NULL, receipt_bytes = ?2, receipt_digest = ?3, updated_at_ms = ?4 WHERE job_id = ?1 AND status = 'claimed' AND lease_token = ?5",
            params![blob(commit.job_id), commit.receipt_bytes.clone(), blob(commit.receipt_digest), now_ms() as i64, blob(commit.lease_token)],
        )?;
        if changed != 1 {
            return Err(StoreError::LearningCompensationConflict);
        }
        tx.execute(
            "DELETE FROM learning_compensation_claim_bindings WHERE job_id = ?1",
            params![blob(commit.job_id)],
        )?;
        let committed = Self::learning_compensation_job_tx(&tx, &commit.job_id)?
            .ok_or(StoreError::LearningCompensationInvalid)?;
        tx.commit()?;
        Ok(committed)
    }

    pub fn terminalize_learning_compensation_v1(
        &mut self,
        terminal: &LearningCompensationTerminalCommitV1,
    ) -> Result<LearningCompensationJobRowV1, StoreError> {
        if !matches!(
            terminal.status,
            LearningCompensationJobStatusV1::NoChange
                | LearningCompensationJobStatusV1::Rejected
                | LearningCompensationJobStatusV1::Abandoned
                | LearningCompensationJobStatusV1::Expired
        ) || terminal.receipt_bytes.is_empty()
            || [
                &terminal.job_id,
                &terminal.lease_token,
                &terminal.reason_digest,
                &terminal.checkpoint_digest,
                &terminal.receipt_digest,
            ]
            .into_iter()
            .any(|digest| digest.iter().all(|byte| *byte == 0))
        {
            return Err(StoreError::LearningCompensationInvalid);
        }
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = Self::learning_compensation_job_tx(&tx, &terminal.job_id)?
            .ok_or(StoreError::LearningCompensationJobNotFound)?;
        if row.status != LearningCompensationJobStatusV1::Claimed
            || row.lease_token != Some(terminal.lease_token)
        {
            return Err(StoreError::LearningCompensationConflict);
        }
        let binding: (Vec<u8>, Vec<u8>) = tx
            .query_row(
                "SELECT lease_token, checkpoint_digest FROM learning_compensation_claim_bindings WHERE job_id = ?1",
                params![blob(terminal.job_id)],
                |value| Ok((value.get(0)?, value.get(1)?)),
            )
            .optional()?
            .ok_or(StoreError::LearningCompensationConflict)?;
        if digest_from_blob(&binding.0, "learning_terminal_lease")? != terminal.lease_token
            || digest_from_blob(&binding.1, "learning_terminal_checkpoint")?
                != terminal.checkpoint_digest
        {
            return Err(StoreError::LearningCompensationConflict);
        }
        let changed = tx.execute(
            "UPDATE learning_compensation_jobs SET status = ?2, lease_token = NULL, terminal_reason_digest = ?3, receipt_bytes = ?4, receipt_digest = ?5, updated_at_ms = ?6 WHERE job_id = ?1 AND status = 'claimed' AND lease_token = ?7",
            params![blob(terminal.job_id), terminal.status.as_str(), blob(terminal.reason_digest), terminal.receipt_bytes.clone(), blob(terminal.receipt_digest), now_ms() as i64, blob(terminal.lease_token)],
        )?;
        if changed != 1 {
            return Err(StoreError::LearningCompensationConflict);
        }
        tx.execute(
            "DELETE FROM learning_compensation_claim_bindings WHERE job_id = ?1",
            params![blob(terminal.job_id)],
        )?;
        let terminalized = Self::learning_compensation_job_tx(&tx, &terminal.job_id)?
            .ok_or(StoreError::LearningCompensationInvalid)?;
        tx.commit()?;
        Ok(terminalized)
    }

    // ------------------------------------------------------------ snapshots

    pub fn write_snapshot(
        &mut self,
        scope_digest: &Digest,
        revision: u64,
        state_digest: &Digest,
        state_bytes: &[u8],
    ) -> Result<(), StoreError> {
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        conn.execute(
            "INSERT OR REPLACE INTO snapshots (revision, scope_digest, state_digest, state_bytes) VALUES (?1, ?2, ?3, ?4)",
            params![
                revision as i64,
                blob(*scope_digest),
                blob(*state_digest),
                state_bytes.to_vec(),
            ],
        )?;
        Ok(())
    }

    pub fn read_snapshot(
        &self,
        scope_digest: &Digest,
        revision: u64,
    ) -> Result<Option<SnapshotRow>, StoreError> {
        let conn = self.connection()?;
        conn.query_row(
            "SELECT state_digest, state_bytes FROM snapshots WHERE scope_digest = ?1 AND revision = ?2",
            params![blob(*scope_digest), revision as i64],
            |row| {
                let bytes: Vec<u8> = row.get(0)?;
                let mut state_digest = [0u8; 32];
                state_digest.copy_from_slice(&bytes);
                Ok(SnapshotRow {
                    revision,
                    scope_digest: *scope_digest,
                    state_digest,
                    state_bytes: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(StoreError::from)
    }

    pub fn flush(&mut self) -> Result<(), StoreError> {
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        conn.pragma_update(None, "wal_checkpoint", "TRUNCATE")?;
        Ok(())
    }

    pub fn close(mut self) -> Result<(), StoreError> {
        if let Some(conn) = self.conn.take() {
            conn.pragma_update(None, "wal_checkpoint", "TRUNCATE")?;
            drop(conn);
        }
        Ok(())
    }

    // ---------------------------------------------------- test diagnostics

    pub fn count_leases(&self) -> Result<u64, StoreError> {
        let conn = self.connection()?;
        Ok(
            conn.query_row("SELECT COUNT(*) FROM genesis_leases", [], |row| {
                row.get::<_, i64>(0)
            })? as u64,
        )
    }

    pub fn count_incarnations(&self) -> Result<u64, StoreError> {
        let conn = self.connection()?;
        Ok(
            conn.query_row("SELECT COUNT(*) FROM incarnations", [], |row| {
                row.get::<_, i64>(0)
            })? as u64,
        )
    }

    pub fn count_journal(&self) -> Result<u64, StoreError> {
        let conn = self.connection()?;
        Ok(conn.query_row("SELECT COUNT(*) FROM journal", [], |row| {
            row.get::<_, i64>(0)
        })? as u64)
    }

    pub fn count_manifests(&self) -> Result<u64, StoreError> {
        let conn = self.connection()?;
        Ok(
            conn.query_row("SELECT COUNT(*) FROM genesis_manifests", [], |row| {
                row.get::<_, i64>(0)
            })? as u64,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ae_contracts::{
        wire, AllostaticSetpoints, EpistemicPriors, ExpressionPhenotype, GenesisStatus,
        PersonaScopeRef, PersonaSelectionKind, PersonalityVector, SocialPriors,
    };
    use ae_fixed::Fixed;

    fn test_manifest(seed: u8) -> GenesisManifest {
        let mut manifest = GenesisManifest {
            schema_version: 1,
            traits: PersonalityVector {
                baseline_warmth: Fixed::from_raw(600_000 + i64::from(seed)),
                ..PersonalityVector::default()
            },
            expression: ExpressionPhenotype::default(),
            allostasis: AllostaticSetpoints::default(),
            epistemic: EpistemicPriors::default(),
            social: SocialPriors::default(),
            manifest_digest: [0; 32],
        };
        manifest.manifest_digest = wire::manifest_body_digest(&manifest);
        manifest
    }

    fn source(bot: u8, persona: u8) -> PersonaSourceRef {
        PersonaSourceRef {
            scope: PersonaScopeRef {
                bot_token: [bot; 16],
                persona_token: [persona; 16],
            },
            source_digest: [3; 32],
            capability_digest: [4; 32],
            selection: PersonaSelectionKind::Conversation,
            prompt_chars: 1,
            begin_dialog_count: 0,
            mood_dialog_count: 0,
        }
    }

    fn commit(seed: u8, epoch: u64, nonce: [u8; 32]) -> GenesisCommit {
        let manifest = test_manifest(seed);
        let source = source(seed, seed.wrapping_add(1));
        let scope_key = ae_genesis::genesis_scope_key(
            &source.scope.bot_token,
            &source.scope.persona_token,
            &source.source_digest,
            &[9; 32],
        );
        let seed_code = ae_genesis::derive_seed_code_digest(&manifest.manifest_digest);
        let incarnation = [seed.wrapping_add(7); 32];
        let receipt = GenesisReceipt {
            schema_version: 1,
            seed_code_digest: seed_code,
            manifest_digest: manifest.manifest_digest,
            incarnation_id: incarnation,
            formula_digest: [9; 32],
            persona_source_digest: source.source_digest,
            compiler_protocol_digest: [10; 32],
            compiler_model_digest: [11; 32],
            development_seed_digest: [12; 32],
            initial_snapshot_digest: [13; 32],
            graph_digest: [14; 32],
            equilibrium_residual: Fixed::ZERO,
            energy_residual: Fixed::ZERO,
            capacity_residual: Fixed::ZERO,
            sample_fit_residual: Fixed::ZERO,
            status: GenesisStatus::Committed,
        };
        GenesisCommit {
            scope_key,
            lease_epoch: epoch,
            nonce_digest: nonce,
            manifest_body: wire::encode_manifest_body(&manifest),
            seed_code_digest: seed_code,
            incarnation_id: incarnation,
            formula_digest: [9; 32],
            source,
            compiler_protocol_digest: [10; 32],
            compiler_model_digest: [11; 32],
            compiled_at_ms: 1,
            receipt,
            initial_snapshot_digest: [13; 32],
            state_bytes: vec![seed; 64],
            graph_digest: [14; 32],
            manifest,
        }
    }

    fn create_legacy_revision_tables(conn: &Connection) {
        conn.execute_batch(
            r#"
            CREATE TABLE meta (
                key TEXT PRIMARY KEY,
                value BLOB NOT NULL
            );
            CREATE TABLE journal (
                revision INTEGER PRIMARY KEY AUTOINCREMENT,
                scope_digest BLOB NOT NULL,
                base_revision INTEGER NOT NULL,
                event_kind TEXT NOT NULL,
                event_bytes BLOB NOT NULL,
                event_digest BLOB NOT NULL,
                receipt_bytes BLOB NOT NULL,
                chain_digest BLOB NOT NULL,
                committed_at_ms INTEGER NOT NULL
            );
            CREATE TABLE applied_events (
                scope_digest BLOB NOT NULL,
                event_digest BLOB NOT NULL,
                revision INTEGER NOT NULL,
                PRIMARY KEY (scope_digest, event_digest)
            );
            CREATE TABLE snapshots (
                revision INTEGER NOT NULL,
                scope_digest BLOB NOT NULL,
                state_digest BLOB NOT NULL,
                state_bytes BLOB NOT NULL,
                PRIMARY KEY (revision, scope_digest)
            );
            CREATE TABLE active_bindings (
                bot_token BLOB NOT NULL,
                persona_token BLOB NOT NULL,
                incarnation_id BLOB NOT NULL,
                revision INTEGER NOT NULL,
                PRIMARY KEY (bot_token, persona_token)
            );
            "#,
        )
        .unwrap();
    }

    fn insert_legacy_journal_row(
        conn: &Connection,
        scope_digest: Digest,
        event_digest: Digest,
        base_revision: u64,
    ) -> u64 {
        conn.execute(
            "INSERT INTO journal (scope_digest, base_revision, event_kind, event_bytes, event_digest, receipt_bytes, chain_digest, committed_at_ms) VALUES (?1, ?2, 'legacy', ?3, ?4, ?5, ?6, 1)",
            params![
                blob(scope_digest),
                base_revision as i64,
                vec![event_digest[0]; 32],
                blob(event_digest),
                vec![event_digest[0].wrapping_add(1); 32],
                vec![event_digest[0].wrapping_add(2); 32],
            ],
        )
        .unwrap();
        let physical_revision = conn.last_insert_rowid() as u64;
        conn.execute(
            "INSERT INTO applied_events (scope_digest, event_digest, revision) VALUES (?1, ?2, ?3)",
            params![
                blob(scope_digest),
                blob(event_digest),
                physical_revision as i64,
            ],
        )
        .unwrap();
        physical_revision
    }

    #[test]
    fn legacy_migration_remaps_related_rows_idempotently() {
        let mut conn = Connection::open_in_memory().unwrap();
        create_legacy_revision_tables(&conn);
        let scope_a = [31; 32];
        let scope_b = [32; 32];
        let event_a1 = [41; 32];
        let event_b1 = [42; 32];
        let event_a2 = [43; 32];
        let event_b2 = [44; 32];
        let a1 = insert_legacy_journal_row(&conn, scope_a, event_a1, 0);
        let b1 = insert_legacy_journal_row(&conn, scope_b, event_b1, 0);
        let a2 = insert_legacy_journal_row(&conn, scope_a, event_a2, 1);
        let b2 = insert_legacy_journal_row(&conn, scope_b, event_b2, 1);
        assert_eq!((a1, b1, a2, b2), (1, 2, 3, 4));

        let snapshot_bytes = vec![91, 92, 93];
        conn.execute(
            "INSERT INTO snapshots (revision, scope_digest, state_digest, state_bytes) VALUES (?1, ?2, ?3, ?4)",
            params![
                a2 as i64,
                blob(scope_a),
                blob([94; 32]),
                snapshot_bytes.clone(),
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO active_bindings (bot_token, persona_token, incarnation_id, revision) VALUES (?1, ?2, ?3, 77)",
            params![blob([71; 16]), blob([72; 16]), blob([73; 32])],
        )
        .unwrap();
        let receipt_before: Vec<u8> = conn
            .query_row(
                "SELECT receipt_bytes FROM journal WHERE revision = ?1",
                params![a2 as i64],
                |row| row.get(0),
            )
            .unwrap();
        let chain_before: Vec<u8> = conn
            .query_row(
                "SELECT chain_digest FROM journal WHERE revision = ?1",
                params![a2 as i64],
                |row| row.get(0),
            )
            .unwrap();

        Store::migrate(&mut conn).unwrap();
        let mut store = Store { conn: Some(conn) };
        assert_eq!(
            store
                .read_journal(&scope_a)
                .unwrap()
                .iter()
                .map(|row| row.revision)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(
            store
                .read_journal(&scope_b)
                .unwrap()
                .iter()
                .map(|row| row.revision)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(
            store
                .lookup_event(&scope_b, &event_b1)
                .unwrap()
                .unwrap()
                .revision,
            1
        );
        assert_eq!(
            store
                .read_snapshot(&scope_a, 2)
                .unwrap()
                .unwrap()
                .state_bytes,
            snapshot_bytes
        );
        assert!(store.read_snapshot(&scope_a, 3).unwrap().is_none());
        assert_eq!(
            store
                .lookup_binding(&[71; 16], &[72; 16])
                .unwrap()
                .unwrap()
                .revision,
            77
        );
        let physical_a = {
            let conn = store.conn.as_ref().unwrap();
            let mut statement = conn
                .prepare(
                    "SELECT revision FROM journal WHERE scope_digest = ?1 ORDER BY revision ASC",
                )
                .unwrap();
            statement
                .query_map(params![blob(scope_a)], |row| row.get::<_, i64>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(physical_a, vec![1, 3]);
        let receipt_after: Vec<u8> = store
            .conn
            .as_ref()
            .unwrap()
            .query_row(
                "SELECT receipt_bytes FROM journal WHERE revision = ?1",
                params![a2 as i64],
                |row| row.get(0),
            )
            .unwrap();
        let chain_after: Vec<u8> = store
            .conn
            .as_ref()
            .unwrap()
            .query_row(
                "SELECT chain_digest FROM journal WHERE revision = ?1",
                params![a2 as i64],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(receipt_after, receipt_before);
        assert_eq!(chain_after, chain_before);

        Store::migrate(store.conn.as_mut().unwrap()).unwrap();
        assert_eq!(store.current_revision(&scope_a).unwrap(), 2);
        assert_eq!(store.current_revision(&scope_b).unwrap(), 2);
        assert_eq!(
            store
                .lookup_event(&scope_a, &event_a2)
                .unwrap()
                .unwrap()
                .revision,
            2
        );
    }

    #[test]
    fn legacy_migration_failure_rolls_back_atomically() {
        let mut conn = Connection::open_in_memory().unwrap();
        create_legacy_revision_tables(&conn);
        let scope_digest = [81; 32];
        let event_digest = [82; 32];
        insert_legacy_journal_row(&conn, scope_digest, event_digest, 0);
        conn.execute("UPDATE applied_events SET revision = 99", [])
            .unwrap();

        assert!(matches!(
            Store::migrate(&mut conn),
            Err(StoreError::Sqlite(_))
        ));
        let logical_revision_exists = {
            let mut statement = conn.prepare("PRAGMA table_info(journal)").unwrap();
            let exists = statement
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .map(|column| column.unwrap())
                .any(|column| column == "logical_revision");
            exists
        };
        assert!(!logical_revision_exists);
        let applied_revision: i64 = conn
            .query_row("SELECT revision FROM applied_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(applied_revision, 99);
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM journal", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            1
        );
    }

    #[test]
    fn migrations_are_idempotent() {
        let store = Store::open_in_memory().unwrap();
        let again = Store::open_in_memory().unwrap();
        assert_eq!(store.count_leases().unwrap(), 0);
        assert_eq!(again.count_leases().unwrap(), 0);
    }

    #[test]
    fn lease_claim_and_commit_round_trip() {
        let mut store = Store::open_in_memory().unwrap();
        let commit = commit(1, 0, [21; 32]);
        let outcome = store
            .claim_lease(&commit.scope_key, Some([21; 32]))
            .unwrap();
        let ClaimOutcome::Claimed { lease_epoch, nonce } = outcome else {
            panic!("expected claim");
        };
        assert_eq!(nonce, [21; 32]);
        let mut commit = commit;
        commit.lease_epoch = lease_epoch;
        store.commit_genesis(&commit).unwrap();
        assert_eq!(store.count_incarnations().unwrap(), 1);
        assert_eq!(store.count_leases().unwrap(), 1);

        let committed = store
            .lookup_committed_genesis(&commit.scope_key)
            .unwrap()
            .unwrap();
        assert_eq!(committed.receipt.incarnation_id, commit.incarnation_id);
        assert_eq!(committed.receipt.seed_code_digest, commit.seed_code_digest);
        assert_eq!(
            committed.manifest.schema_version,
            commit.manifest.schema_version
        );
        assert_eq!(committed.manifest.traits, commit.manifest.traits);
        assert_eq!(committed.manifest.expression, commit.manifest.expression);
        assert_eq!(committed.manifest.allostasis, commit.manifest.allostasis);
        assert_eq!(committed.manifest.epistemic, commit.manifest.epistemic);
        assert_eq!(committed.manifest.social, commit.manifest.social);
        assert_eq!(
            committed.receipt.manifest_digest,
            commit.manifest.manifest_digest
        );
        assert_eq!(committed.incarnation_nonce, [21; 32]);

        // A second claim joins the committed lease.
        assert_eq!(
            store.claim_lease(&commit.scope_key, None).unwrap(),
            ClaimOutcome::Committed
        );

        // Binding exists.
        let binding = store
            .lookup_binding(
                &commit.source.scope.bot_token,
                &commit.source.scope.persona_token,
            )
            .unwrap()
            .unwrap();
        assert_eq!(binding.incarnation_id, commit.incarnation_id);
    }

    #[test]
    fn stale_epoch_cannot_commit() {
        let mut store = Store::open_in_memory().unwrap();
        let initial_commit = commit(1, 0, [21; 32]);
        let ClaimOutcome::Claimed { lease_epoch, .. } = store
            .claim_lease(&initial_commit.scope_key, Some([21; 32]))
            .unwrap()
        else {
            panic!()
        };

        // Simulate the first holder crashing: the lease goes stale, a second
        // process takes over (bumping the epoch) without committing yet.
        store
            .conn
            .as_mut()
            .unwrap()
            .execute(
                "UPDATE genesis_leases SET updated_at_ms = 0 WHERE scope_key = ?1",
                params![blob(initial_commit.scope_key)],
            )
            .unwrap();
        let ClaimOutcome::Claimed {
            lease_epoch: newer, ..
        } = store.claim_lease(&initial_commit.scope_key, None).unwrap()
        else {
            panic!("second claim should take over a stale lease")
        };
        assert_ne!(lease_epoch, newer);
        let mut stale_commit = initial_commit;
        stale_commit.lease_epoch = lease_epoch;
        assert!(matches!(
            store.commit_genesis(&stale_commit).unwrap_err(),
            StoreError::LeaseConflict
        ));
        assert_eq!(store.count_incarnations().unwrap(), 0);

        // The newer epoch can commit.
        let mut current_commit = commit(1, 0, [21; 32]);
        current_commit.lease_epoch = newer;
        store.commit_genesis(&current_commit).unwrap();
        assert_eq!(store.count_incarnations().unwrap(), 1);
    }

    #[test]
    fn in_flight_lease_blocks_second_claim() {
        let mut store = Store::open_in_memory().unwrap();
        let commit = commit(1, 0, [21; 32]);
        let _ = store
            .claim_lease(&commit.scope_key, Some([21; 32]))
            .unwrap();
        // Fresh in-flight lease: no takeover.
        assert_eq!(
            store
                .claim_lease(&commit.scope_key, Some([22; 32]))
                .unwrap(),
            ClaimOutcome::InFlight
        );
        // No second birth row appeared.
        assert_eq!(store.count_leases().unwrap(), 1);
    }

    #[test]
    fn digest_collision_fails_closed() {
        let mut store = Store::open_in_memory().unwrap();
        let commit = commit(1, 0, [21; 32]);
        let ClaimOutcome::Claimed { lease_epoch, .. } = store
            .claim_lease(&commit.scope_key, Some([21; 32]))
            .unwrap()
        else {
            panic!()
        };
        let mut commit = commit;
        commit.lease_epoch = lease_epoch;
        store.commit_genesis(&commit).unwrap();

        // A forged commit: same manifest_digest, different canonical bytes.
        let mut forged = commit.clone();
        forged.scope_key = ae_genesis::genesis_scope_key(&[1; 16], &[2; 16], &[98; 32], &[9; 32]);
        let ClaimOutcome::Claimed {
            lease_epoch: forged_epoch,
            ..
        } = store
            .claim_lease(&forged.scope_key, Some([22; 32]))
            .unwrap()
        else {
            panic!("expected forged scope lease");
        };
        forged.lease_epoch = forged_epoch;
        forged.manifest_body[10] ^= 0x01;
        // Force the store's decode check to pass is impossible: the bytes no
        // longer match the digest, so this must fail BEFORE the collision
        // check with ManifestDigestMismatch. To exercise the byte-compare
        // itself, use register_manifest directly with forged bytes.
        assert!(matches!(
            store.commit_genesis(&forged).unwrap_err(),
            StoreError::ManifestDigestMismatch
        ));

        let manifest = test_manifest(1);
        let mut wrong_bytes = wire::encode_manifest_body(&manifest);
        wrong_bytes[20] ^= 0x02;
        let err = store
            .register_manifest(
                &manifest,
                &wrong_bytes,
                &source(1, 2),
                &[1; 32],
                &[2; 32],
                1,
            )
            .unwrap_err();
        assert!(matches!(err, StoreError::ManifestDigestMismatch));
    }

    #[test]
    fn stored_collision_fails_closed_with_byte_compare() {
        let mut store = Store::open_in_memory().unwrap();
        let manifest = test_manifest(1);
        let digest = manifest.manifest_digest;

        // Forge a stored row: the digest points at bytes that do NOT belong
        // to this manifest (simulating a corrupted or colliding registry).
        let mut forged_bytes = wire::encode_manifest_body(&manifest);
        forged_bytes[2] ^= 0x40;
        let conn = store.conn.as_mut().unwrap();
        conn.execute(
            "INSERT INTO genesis_manifests (manifest_digest, seed_code_digest, canonical_bytes, source_json, compiler_protocol_digest, compiler_model_digest, compiled_at_ms) VALUES (?1, ?2, ?3, '{}', X'00', X'00', 1)",
            params![
                blob(digest),
                blob([0; 32]),
                forged_bytes.clone(),
            ],
        )
        .unwrap();

        // A legitimate write with the same digest but the CORRECT bytes must
        // fail closed with SeedDigestCollision, never silently overwrite.
        let correct_bytes = wire::encode_manifest_body(&manifest);
        let err = store
            .register_manifest(
                &manifest,
                &correct_bytes,
                &source(1, 2),
                &[1; 32],
                &[2; 32],
                1,
            )
            .unwrap_err();
        assert!(matches!(err, StoreError::SeedDigestCollision));
    }

    #[test]
    fn journal_cas_and_duplicate_events() {
        let mut store = Store::open_in_memory().unwrap();
        let commit = commit(1, 0, [21; 32]);
        let ClaimOutcome::Claimed { lease_epoch, .. } = store
            .claim_lease(&commit.scope_key, Some([21; 32]))
            .unwrap()
        else {
            panic!()
        };
        let mut commit = commit;
        commit.lease_epoch = lease_epoch;
        store.commit_genesis(&commit).unwrap();

        let scope_digest = wire::persona_scope_digest(
            &commit.source.scope.bot_token,
            &commit.source.scope.persona_token,
            None,
        );
        let chain_seed = commit.initial_snapshot_digest;
        let event = ae_contracts::CanonicalEvent::TimeAdvance(ae_contracts::TimeAdvance {
            event_id: [42; 16],
            scope: ae_contracts::ScopeRef {
                bot_token: commit.source.scope.bot_token,
                persona_token: commit.source.scope.persona_token,
                relation_token: None,
                session_token: [5; 16],
            },
            elapsed_ms: 7,
        });
        let event_bytes = wire::encode_event(&event);
        let event_digest = wire::event_digest(&event);
        let receipt = ae_contracts::TransitionReceipt {
            schema_version: 1,
            formula_digest: [9; 32],
            scope_digest,
            event_digest,
            authority_digest: [15; 32],
            base_revision: 0,
            next_revision: 1,
            state_before: [13; 32],
            state_after: [13; 32],
            graph_after: [14; 32],
            action_contract: None,
            active_nodes: 16_384,
            active_edges: 0,
            residuals: ae_contracts::InvariantResiduals::default(),
            status: ae_contracts::CommitStatus::Committed,
        };
        let envelope = CommitEnvelope {
            event_kind: "time_advance".to_string(),
            event_bytes,
            receipt,
            chain_seed,
            delta_bytes: vec![],
        };
        let (revision, row) = store.commit_journal(&envelope).unwrap();
        assert_eq!(revision, 1);
        assert_eq!(
            row.chain_digest,
            ae_continuum::chain_link(
                &chain_seed,
                &envelope.event_bytes,
                &wire::encode_transition_receipt(&envelope.receipt)
            )
        );
        assert_eq!(store.current_revision(&scope_digest).unwrap(), 1);

        // Identical event bytes/digest remain isolated by receipt scope.
        let isolated_scope = [98; 32];
        let mut same_digest_other_scope = envelope.clone();
        same_digest_other_scope.receipt.scope_digest = isolated_scope;
        same_digest_other_scope.receipt.base_revision = 0;
        same_digest_other_scope.receipt.next_revision = 1;
        same_digest_other_scope.chain_seed = [97; 32];
        let (isolated_revision, _) = store.commit_journal(&same_digest_other_scope).unwrap();
        assert_eq!(isolated_revision, 1);
        assert_eq!(
            store
                .lookup_event(&isolated_scope, &event_digest)
                .unwrap()
                .unwrap()
                .revision,
            1
        );

        // Replaying the original receipt after the revision advances is
        // rejected by the CAS guard before duplicate lookup.
        assert!(matches!(
            store.commit_journal(&envelope).unwrap_err(),
            StoreError::StaleRevision {
                expected: 0,
                actual: 1
            }
        ));
        assert_eq!(store.count_journal().unwrap(), 2);

        // A duplicate with the current base revision reaches duplicate
        // detection and is rejected without writing.
        let mut duplicate = envelope.clone();
        duplicate.receipt.base_revision = 1;
        duplicate.receipt.next_revision = 2;
        assert!(matches!(
            store.commit_journal(&duplicate).unwrap_err(),
            StoreError::DuplicateEvent(1)
        ));
        assert_eq!(store.count_journal().unwrap(), 2);

        // Stale base revision is rejected.
        let mut stale = envelope.clone();
        stale.receipt.base_revision = 5;
        assert!(matches!(
            store.commit_journal(&stale).unwrap_err(),
            StoreError::StaleRevision {
                expected: 5,
                actual: 1
            }
        ));
        assert_eq!(store.count_journal().unwrap(), 2);

        // Duplicate lookup returns the original row.
        let found = store
            .lookup_event(&scope_digest, &event_digest)
            .unwrap()
            .unwrap();
        assert_eq!(found.revision, 1);

        // Replay reads the same row set.
        let rows = store.read_journal(&scope_digest).unwrap();
        assert_eq!(rows.len(), 1);
        let report = ae_continuum::verify_replay(chain_seed, &rows);
        assert!(report.ok, "{:?}", report.first_error);
    }

    #[test]
    fn crash_recovery_reopens_and_replays() {
        let dir = std::env::temp_dir().join(format!("ae-store-crash-{}", std::process::id()));
        let path = dir.join("store.db");
        std::fs::create_dir_all(&dir).unwrap();

        let mut store = Store::open(&path).unwrap();
        let commit = commit(1, 0, [21; 32]);
        let ClaimOutcome::Claimed { lease_epoch, .. } = store
            .claim_lease(&commit.scope_key, Some([21; 32]))
            .unwrap()
        else {
            panic!()
        };
        let mut commit = commit;
        commit.lease_epoch = lease_epoch;
        store.commit_genesis(&commit).unwrap();
        drop(store); // crash without flush

        let mut store = Store::open(&path).unwrap();
        let committed = store
            .lookup_committed_genesis(&commit.scope_key)
            .unwrap()
            .unwrap();
        assert_eq!(committed.receipt.incarnation_id, commit.incarnation_id);
        assert_eq!(store.count_incarnations().unwrap(), 1);
        // Re-committing the same birth is idempotent via lookup, not a
        // second row.
        assert_eq!(
            store.claim_lease(&commit.scope_key, None).unwrap(),
            ClaimOutcome::Committed
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persisted_nonce_is_reused_after_takeover() {
        let mut store = Store::open_in_memory().unwrap();
        let commit = commit(1, 0, [21; 32]);
        let _ = store
            .claim_lease(&commit.scope_key, Some([21; 32]))
            .unwrap();
        // Simulate the lease going stale (crash), then a retry with a
        // different nonce: the persisted nonce must win.
        let conn = store.conn.as_mut().unwrap();
        conn.execute(
            "UPDATE genesis_leases SET updated_at_ms = 0 WHERE scope_key = ?1",
            params![blob(commit.scope_key)],
        )
        .unwrap();
        let outcome = store
            .claim_lease(&commit.scope_key, Some([99; 32]))
            .unwrap();
        let ClaimOutcome::Claimed { nonce, .. } = outcome else {
            panic!()
        };
        assert_eq!(nonce, [21; 32]);
    }
}
