#![deny(unsafe_code)]

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
    RebirthStagedChildV1, SeedClearCommitPermitV1, SeedClearStagedChildV1, SeedConfigAckResultV1,
    SeedConfigAckStateV1, SeedConfigLifecycleError, SeedConfigObservationV1, SeedConfigOriginV1,
    SeedConfigPreflightV1, SeedConfigReconcileRequestV1, SeedConfigReconcileResultV1,
    SeedConfigStateV1, SeedConfigWritebackAckV1, SeedConfigWritebackV1, UserAuthorizedRebirthV1,
    VaultLifecycle, VaultLocateError, VaultLocation, VaultMode,
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
#[allow(unsafe_code)]
pub mod semantic_outbox_crypto;
pub use semantic_outbox_crypto::{
    SemanticOutboxCryptoError, SemanticOutboxCryptoStatusV1, SemanticOutboxCryptoStatusValueV1,
    SemanticOutboxKeyAuthorityV1, SEMANTIC_OUTBOX_ENVELOPE_OVERHEAD_BYTES_V1,
    SEMANTIC_OUTBOX_KEY_VERSION_V1, SEMANTIC_OUTBOX_MAX_AAD_BYTES_V1,
    SEMANTIC_OUTBOX_MAX_ENVELOPE_BYTES_V1, SEMANTIC_OUTBOX_MAX_PLAINTEXT_BYTES_V1,
};
mod semantic_field_attestation;

use ae_authority::authority_projection_digest;
use ae_context_projector::{
    project_committed_receipt, ContextProjectionStateV1, DeliveryOutcome as ContextDeliveryOutcome,
    ReceiptCommitStatus, ReceiptEnvelopeV1, ValidatedCommittedReceiptV1,
};
use ae_continuum::{CommitEnvelope, JournalRow};
use ae_contracts::{
    phase0_canonical_formula_digest_v1, wire, CanonicalEvent, CommitStatus, Digest,
    GenesisManifest, GenesisReceipt, GenesisStatus, PerceptionProposalV1, PersonaSourceRef,
    ScopeRef, TransitionReceipt,
};
use ae_neurofield::{
    graph_digest, initial_state_from_manifest, state_digest, NeuralField, SparseGraph,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use sha2::{Digest as Sha2Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub const LEASE_TTL_MS: u64 = 120_000;
/// Namespace shared by the runtime and store for the per-incarnation semantic
/// lane.  Keeping it at the durable boundary lets the store distinguish that
/// lane from an arbitrary relation-scoped event before it accepts the one
/// Phase-0 formula transition.
pub const SEMANTIC_LANE_NAMESPACE_DOMAIN_V1: &[u8] = b"astr-embodiment/semantic-lane-namespace-v1";

const PHASE0_FORMULA_TRANSITION_MAGIC_V1: &[u8] = b"AE-P0FT1\0";
const PHASE0_FORMULA_TRANSITION_SCHEMA_V1: u16 = 1;
const PHASE0_FORMULA_TRANSITION_KIND_V1: u8 = 1;
const LEGACY_SEMANTIC_FORMULA_UPGRADE_MAGIC_V1: &[u8] = b"AE-LSU1\0";
const LEGACY_SEMANTIC_FORMULA_UPGRADE_SCHEMA_V1: u16 = 1;
const LEGACY_SEMANTIC_FORMULA_UPGRADE_KIND_V1: u8 = 1;
const LEGACY_SEMANTIC_FIELD_DOMAIN_UPGRADE_SCHEMA_V1: u16 = 2;
const LEGACY_SEMANTIC_FIELD_DOMAIN_UPGRADE_KIND_V1: u8 = 2;
const LEGACY_SEMANTIC_FORMULA_UPGRADE_ID_DOMAIN_V1: &[u8] =
    b"astr-embodiment/legacy-semantic-formula-upgrade-v1";
const LEGACY_SEMANTIC_FIELD_DOMAIN_UPGRADE_ID_DOMAIN_V1: &[u8] =
    b"astr-embodiment/legacy-semantic-field-domain-upgrade-v1";
const FIELD_MIGRATION_BACKUP_MANIFEST_MAGIC_V2: &[u8] = b"AE-FMP2\0";
pub const JOINT_MAX_LINEAR_FXP6_V1: u8 = 1;
pub const LEGACY_FIELD_FXP6_SCALE: u32 = 1_000_000;
const AESEM2_SNAPSHOT_MAGIC: &[u8] = b"AESEM2\0";
const REVISION_RANGE_FENCE: &str = "revision_range";

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
    #[error("field migration backup failed: {context}")]
    FieldMigrationBackup { context: &'static str },
    #[error("continuity duplicate does not match the complete stored authority")]
    ContinuityDuplicateMismatch,
    #[error("continuity duplicate points at an incomplete authority bundle")]
    ContinuityIncomplete,
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

    /// Parses a stored lease-status string.
    ///
    /// Kept for compatibility with the public store API; `FromStr` below
    /// provides the standard parsing interface without removing this constructor.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Option<Self> {
        <Self as std::str::FromStr>::from_str(value).ok()
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

impl std::str::FromStr for LeaseStatus {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "claimed" => Ok(Self::Claimed),
            "compiling" => Ok(Self::Compiling),
            "validating" => Ok(Self::Validating),
            "developing" => Ok(Self::Developing),
            "committed" => Ok(Self::Committed),
            "failed" => Ok(Self::Failed),
            "retry_wait" => Ok(Self::RetryWait),
            _ => Err(()),
        }
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

/// The Store is the cross-process authority, so an identical applied event is
/// materially different from a newly inserted transition even though both
/// return the same sealed journal row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContinuityCommitOutcomeV1 {
    Inserted { revision: u64, row: JournalRow },
    ExistingIdentical { revision: u64, row: JournalRow },
}

impl ContinuityCommitOutcomeV1 {
    pub fn revision(&self) -> u64 {
        match self {
            Self::Inserted { revision, .. } | Self::ExistingIdentical { revision, .. } => *revision,
        }
    }

    pub fn row(&self) -> &JournalRow {
        match self {
            Self::Inserted { row, .. } | Self::ExistingIdentical { row, .. } => row,
        }
    }
}

/// Canonical graph delta for the only allowed Genesis-to-Phase-0 formula
/// change.  The receipt digest makes the delta explicitly bind to the receipt
/// that is appended to the Continuum chain; the remaining fields bind it to
/// the prior Genesis graph authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Phase0FormulaTransitionV1 {
    scope_digest: Digest,
    event_digest: Digest,
    receipt_digest: Digest,
    base_revision: u64,
    next_revision: u64,
    base_graph_digest: Digest,
    from_formula_digest: Digest,
    to_formula_digest: Digest,
}

struct Phase0FormulaTransitionInput<'a> {
    delta_bytes: &'a [u8],
    event: &'a CanonicalEvent,
    event_scope: &'a ScopeRef,
    receipt: &'a TransitionReceipt,
    current_revision: u64,
    current_graph: Digest,
    current_formula: Digest,
    incoming_formula: Digest,
}

/// Receipt for the only permitted non-empty semantic-lane formula change:
/// an authenticated AESEM2 state under the active incarnation's original
/// formula becomes the `state_before` of one current Phase-0 transition.
///
/// The receipt is stored both as the graph/journal delta and in the dedicated
/// unique upgrade registry inside the same SQLite transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LegacySemanticFormulaUpgradeReceiptV1 {
    pub scope_digest: Digest,
    pub event_digest: Digest,
    pub receipt_digest: Digest,
    pub base_revision: u64,
    pub next_revision: u64,
    pub source_state_digest: Digest,
    pub target_state_before: Digest,
    pub source_graph_digest: Digest,
    pub prior_chain_digest: Digest,
    pub from_formula_digest: Digest,
    pub to_formula_digest: Digest,
    /// Present only for the closed AESEM2 finite-field migration.  Older
    /// formula-only receipts retain their exact schema-1 bytes.
    pub field_domain: Option<LegacySemanticFieldDomainUpgradeV1>,
    pub migration_id: Digest,
}

/// Aggregate facts for the frozen `JOINT_MAX_LINEAR_FXP6_V1` transform.  The
/// receipt deliberately stores no node index, raw vector, text, scope value,
/// secret, or filesystem data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LegacySemanticFieldDomainUpgradeV1 {
    pub algorithm: u8,
    pub fxp6_scale: u32,
    pub source_common_max: i64,
    pub out_of_range_count: u32,
    pub potential_out_of_range_count: u32,
    pub excitation_out_of_range_count: u32,
    pub signal_mass_before: i128,
    pub signal_mass_after: i128,
}

struct LegacySemanticFormulaUpgradeInput<'a> {
    bundle: &'a ContinuityCommitBundleV1,
    delta_bytes: &'a [u8],
    event: &'a CanonicalEvent,
    event_scope: &'a ScopeRef,
    receipt: &'a TransitionReceipt,
    current_revision: u64,
    current_state_digest: Digest,
    current_state_bytes: &'a [u8],
    current_graph_digest: Digest,
    current_formula_digest: Digest,
    incoming_formula_digest: Digest,
    prior_chain_digest: Digest,
}

enum FormulaTransitionAdmission {
    Phase0,
    LegacySemantic(Box<LegacySemanticAdmissionV1>),
}

#[derive(Clone)]
struct ActiveSemanticIdentityV1 {
    incarnation_id: Digest,
    manifest_digest: Digest,
    formula_digest: Digest,
    initial_snapshot_digest: Digest,
    baseline_field: NeuralField,
    baseline_graph: SparseGraph,
}

struct AttestedFieldDomainUpgradeV1 {
    upgrade: LegacySemanticFormulaUpgradeReceiptV1,
    identity: ActiveSemanticIdentityV1,
}

enum LegacySemanticAdmissionV1 {
    FormulaOnly(Box<LegacySemanticFormulaUpgradeReceiptV1>),
    FieldDomain(Box<AttestedFieldDomainUpgradeV1>),
}

#[derive(Clone)]
struct FieldMigrationPreimageBackupV1 {
    migration_id: Digest,
    scope_digest: Digest,
    source_revision: u64,
    source_state_digest: Digest,
    source_formula_digest: Digest,
    source_graph_digest: Digest,
    incarnation_id: Digest,
    manifest_digest: Digest,
    package_identity: String,
    build_identity: String,
    capture_method: FieldMigrationBackupCaptureMethodV1,
    byte_len: u64,
    sha256: Digest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum FieldMigrationBackupCaptureMethodV1 {
    SqliteBackupApi = 1,
}

impl FieldMigrationBackupCaptureMethodV1 {
    const fn as_u8(self) -> u8 {
        self as u8
    }
}

impl Phase0FormulaTransitionV1 {
    fn canonical_bytes(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            PHASE0_FORMULA_TRANSITION_MAGIC_V1.len() + 2 + 1 + (32 * 6) + (8 * 2),
        );
        out.extend_from_slice(PHASE0_FORMULA_TRANSITION_MAGIC_V1);
        out.extend_from_slice(&PHASE0_FORMULA_TRANSITION_SCHEMA_V1.to_le_bytes());
        out.push(PHASE0_FORMULA_TRANSITION_KIND_V1);
        for digest in [self.scope_digest, self.event_digest, self.receipt_digest] {
            out.extend_from_slice(&digest);
        }
        out.extend_from_slice(&self.base_revision.to_le_bytes());
        out.extend_from_slice(&self.next_revision.to_le_bytes());
        for digest in [
            self.base_graph_digest,
            self.from_formula_digest,
            self.to_formula_digest,
        ] {
            out.extend_from_slice(&digest);
        }
        out
    }

    fn decode(bytes: &[u8]) -> Result<Self, StoreError> {
        if bytes.len() < PHASE0_FORMULA_TRANSITION_MAGIC_V1.len()
            || &bytes[..PHASE0_FORMULA_TRANSITION_MAGIC_V1.len()]
                != PHASE0_FORMULA_TRANSITION_MAGIC_V1
        {
            return Err(StoreError::ContinuityFence("formula_transition_magic"));
        }
        let mut reader = wire::Reader::new(&bytes[PHASE0_FORMULA_TRANSITION_MAGIC_V1.len()..]);
        let schema_version = reader
            .u16()
            .map_err(|_| StoreError::ContinuityFence("formula_transition_decode"))?;
        let kind = reader
            .u8()
            .map_err(|_| StoreError::ContinuityFence("formula_transition_decode"))?;
        if schema_version != PHASE0_FORMULA_TRANSITION_SCHEMA_V1
            || kind != PHASE0_FORMULA_TRANSITION_KIND_V1
        {
            return Err(StoreError::ContinuityFence("formula_transition_schema"));
        }
        let scope_digest = reader
            .digest()
            .map_err(|_| StoreError::ContinuityFence("formula_transition_decode"))?;
        let event_digest = reader
            .digest()
            .map_err(|_| StoreError::ContinuityFence("formula_transition_decode"))?;
        let receipt_digest = reader
            .digest()
            .map_err(|_| StoreError::ContinuityFence("formula_transition_decode"))?;
        let base_revision = reader
            .u64()
            .map_err(|_| StoreError::ContinuityFence("formula_transition_decode"))?;
        let next_revision = reader
            .u64()
            .map_err(|_| StoreError::ContinuityFence("formula_transition_decode"))?;
        let base_graph_digest = reader
            .digest()
            .map_err(|_| StoreError::ContinuityFence("formula_transition_decode"))?;
        let from_formula_digest = reader
            .digest()
            .map_err(|_| StoreError::ContinuityFence("formula_transition_decode"))?;
        let to_formula_digest = reader
            .digest()
            .map_err(|_| StoreError::ContinuityFence("formula_transition_decode"))?;
        reader
            .finish()
            .map_err(|_| StoreError::ContinuityFence("formula_transition_decode"))?;
        Ok(Self {
            scope_digest,
            event_digest,
            receipt_digest,
            base_revision,
            next_revision,
            base_graph_digest,
            from_formula_digest,
            to_formula_digest,
        })
    }
}

impl LegacySemanticFormulaUpgradeReceiptV1 {
    fn expected_migration_id(&self) -> Digest {
        match self.field_domain {
            None => wire::domain_hash(
                LEGACY_SEMANTIC_FORMULA_UPGRADE_ID_DOMAIN_V1,
                &[
                    &self.scope_digest,
                    &self.event_digest,
                    &self.receipt_digest,
                    &self.base_revision.to_le_bytes(),
                    &self.next_revision.to_le_bytes(),
                    &self.source_state_digest,
                    &self.target_state_before,
                    &self.source_graph_digest,
                    &self.prior_chain_digest,
                    &self.from_formula_digest,
                    &self.to_formula_digest,
                ],
            ),
            Some(field_domain) => wire::domain_hash(
                LEGACY_SEMANTIC_FIELD_DOMAIN_UPGRADE_ID_DOMAIN_V1,
                &[
                    &self.scope_digest,
                    &self.event_digest,
                    &self.receipt_digest,
                    &self.base_revision.to_le_bytes(),
                    &self.next_revision.to_le_bytes(),
                    &self.source_state_digest,
                    &self.target_state_before,
                    &self.source_graph_digest,
                    &self.prior_chain_digest,
                    &self.from_formula_digest,
                    &self.to_formula_digest,
                    &[field_domain.algorithm],
                    &field_domain.fxp6_scale.to_le_bytes(),
                    &field_domain.source_common_max.to_le_bytes(),
                    &field_domain.out_of_range_count.to_le_bytes(),
                    &field_domain.potential_out_of_range_count.to_le_bytes(),
                    &field_domain.excitation_out_of_range_count.to_le_bytes(),
                    &field_domain.signal_mass_before.to_le_bytes(),
                    &field_domain.signal_mass_after.to_le_bytes(),
                ],
            ),
        }
    }

    /// Construct the canonical receipt tied to the regular transition receipt
    /// that advances the semantic lane by exactly one logical revision.
    pub fn from_transition_receipt(
        receipt: &TransitionReceipt,
        source_state_digest: Digest,
        source_graph_digest: Digest,
        from_formula_digest: Digest,
        prior_chain_digest: Digest,
    ) -> Self {
        let receipt_digest = wire::receipt_digest(receipt);
        let to_formula_digest = receipt.formula_digest;
        let mut upgrade = Self {
            scope_digest: receipt.scope_digest,
            event_digest: receipt.event_digest,
            receipt_digest,
            base_revision: receipt.base_revision,
            next_revision: receipt.next_revision,
            source_state_digest,
            target_state_before: receipt.state_before,
            source_graph_digest,
            prior_chain_digest,
            from_formula_digest,
            to_formula_digest,
            field_domain: None,
            migration_id: [0; 32],
        };
        upgrade.migration_id = upgrade.expected_migration_id();
        upgrade
    }

    /// Construct the sole receipt shape permitted to carry an authenticated
    /// AESEM2 field-domain migration with its following Phase-0 event.
    pub fn from_transition_receipt_with_field_domain(
        receipt: &TransitionReceipt,
        source_state_digest: Digest,
        source_graph_digest: Digest,
        from_formula_digest: Digest,
        prior_chain_digest: Digest,
        field_domain: LegacySemanticFieldDomainUpgradeV1,
    ) -> Self {
        let mut upgrade = Self::from_transition_receipt(
            receipt,
            source_state_digest,
            source_graph_digest,
            from_formula_digest,
            prior_chain_digest,
        );
        upgrade.field_domain = Some(field_domain);
        upgrade.migration_id = upgrade.expected_migration_id();
        upgrade
    }

    /// Canonical opaque bytes persisted with the transition and the upgrade
    /// registry.  No unbound JSON or caller-supplied formula is accepted.
    pub fn canonical_bytes(self) -> Vec<u8> {
        let has_field_domain = self.field_domain.is_some();
        let mut out = Vec::with_capacity(
            LEGACY_SEMANTIC_FORMULA_UPGRADE_MAGIC_V1.len()
                + 2
                + 1
                + (32 * 10)
                + (8 * 2)
                + if has_field_domain {
                    1 + 4 + 8 + (4 * 3) + (16 * 2)
                } else {
                    0
                },
        );
        out.extend_from_slice(LEGACY_SEMANTIC_FORMULA_UPGRADE_MAGIC_V1);
        out.extend_from_slice(
            &(if has_field_domain {
                LEGACY_SEMANTIC_FIELD_DOMAIN_UPGRADE_SCHEMA_V1
            } else {
                LEGACY_SEMANTIC_FORMULA_UPGRADE_SCHEMA_V1
            })
            .to_le_bytes(),
        );
        out.push(if has_field_domain {
            LEGACY_SEMANTIC_FIELD_DOMAIN_UPGRADE_KIND_V1
        } else {
            LEGACY_SEMANTIC_FORMULA_UPGRADE_KIND_V1
        });
        for digest in [self.scope_digest, self.event_digest, self.receipt_digest] {
            out.extend_from_slice(&digest);
        }
        out.extend_from_slice(&self.base_revision.to_le_bytes());
        out.extend_from_slice(&self.next_revision.to_le_bytes());
        for digest in [
            self.source_state_digest,
            self.target_state_before,
            self.source_graph_digest,
            self.prior_chain_digest,
            self.from_formula_digest,
            self.to_formula_digest,
        ] {
            out.extend_from_slice(&digest);
        }
        if let Some(field_domain) = self.field_domain {
            out.push(field_domain.algorithm);
            out.extend_from_slice(&field_domain.fxp6_scale.to_le_bytes());
            out.extend_from_slice(&field_domain.source_common_max.to_le_bytes());
            out.extend_from_slice(&field_domain.out_of_range_count.to_le_bytes());
            out.extend_from_slice(&field_domain.potential_out_of_range_count.to_le_bytes());
            out.extend_from_slice(&field_domain.excitation_out_of_range_count.to_le_bytes());
            out.extend_from_slice(&field_domain.signal_mass_before.to_le_bytes());
            out.extend_from_slice(&field_domain.signal_mass_after.to_le_bytes());
        }
        out.extend_from_slice(&self.migration_id);
        out
    }

    fn decode(bytes: &[u8]) -> Result<Self, StoreError> {
        if bytes.len() < LEGACY_SEMANTIC_FORMULA_UPGRADE_MAGIC_V1.len()
            || &bytes[..LEGACY_SEMANTIC_FORMULA_UPGRADE_MAGIC_V1.len()]
                != LEGACY_SEMANTIC_FORMULA_UPGRADE_MAGIC_V1
        {
            return Err(StoreError::ContinuityFence("legacy_upgrade_magic"));
        }
        let mut reader =
            wire::Reader::new(&bytes[LEGACY_SEMANTIC_FORMULA_UPGRADE_MAGIC_V1.len()..]);
        let schema_version = reader
            .u16()
            .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
        let kind = reader
            .u8()
            .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
        let is_formula_only = schema_version == LEGACY_SEMANTIC_FORMULA_UPGRADE_SCHEMA_V1
            && kind == LEGACY_SEMANTIC_FORMULA_UPGRADE_KIND_V1;
        let is_field_domain = schema_version == LEGACY_SEMANTIC_FIELD_DOMAIN_UPGRADE_SCHEMA_V1
            && kind == LEGACY_SEMANTIC_FIELD_DOMAIN_UPGRADE_KIND_V1;
        if !is_formula_only && !is_field_domain {
            return Err(StoreError::ContinuityFence("legacy_upgrade_schema"));
        }
        let scope_digest = reader
            .digest()
            .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
        let event_digest = reader
            .digest()
            .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
        let receipt_digest = reader
            .digest()
            .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
        let base_revision = reader
            .u64()
            .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
        let next_revision = reader
            .u64()
            .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
        let source_state_digest = reader
            .digest()
            .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
        let target_state_before = reader
            .digest()
            .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
        let source_graph_digest = reader
            .digest()
            .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
        let prior_chain_digest = reader
            .digest()
            .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
        let from_formula_digest = reader
            .digest()
            .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
        let to_formula_digest = reader
            .digest()
            .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
        let field_domain = if is_field_domain {
            let algorithm = reader
                .u8()
                .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
            let fxp6_scale = reader
                .u32()
                .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
            let source_common_max = i64::from_le_bytes(
                reader
                    .u64()
                    .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?
                    .to_le_bytes(),
            );
            let out_of_range_count = reader
                .u32()
                .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
            let potential_out_of_range_count = reader
                .u32()
                .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
            let excitation_out_of_range_count = reader
                .u32()
                .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
            let read_i128 = |reader: &mut wire::Reader<'_>| -> Result<i128, StoreError> {
                let low = reader
                    .u64()
                    .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
                let high = reader
                    .u64()
                    .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
                let mut bytes = [0_u8; 16];
                bytes[..8].copy_from_slice(&low.to_le_bytes());
                bytes[8..].copy_from_slice(&high.to_le_bytes());
                Ok(i128::from_le_bytes(bytes))
            };
            let signal_mass_before = read_i128(&mut reader)?;
            let signal_mass_after = read_i128(&mut reader)?;
            let field_domain = LegacySemanticFieldDomainUpgradeV1 {
                algorithm,
                fxp6_scale,
                source_common_max,
                out_of_range_count,
                potential_out_of_range_count,
                excitation_out_of_range_count,
                signal_mass_before,
                signal_mass_after,
            };
            if field_domain.algorithm != JOINT_MAX_LINEAR_FXP6_V1
                || field_domain.fxp6_scale != LEGACY_FIELD_FXP6_SCALE
                || field_domain.source_common_max <= i64::from(LEGACY_FIELD_FXP6_SCALE)
                || field_domain.out_of_range_count == 0
                || field_domain.out_of_range_count
                    != field_domain
                        .potential_out_of_range_count
                        .checked_add(field_domain.excitation_out_of_range_count)
                        .ok_or(StoreError::ContinuityFence("legacy_upgrade_field_domain"))?
                || field_domain.signal_mass_before <= 0
                || field_domain.signal_mass_after < 0
            {
                return Err(StoreError::ContinuityFence("legacy_upgrade_field_domain"));
            }
            Some(field_domain)
        } else {
            None
        };
        let migration_id = reader
            .digest()
            .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
        reader
            .finish()
            .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
        let upgrade = Self {
            scope_digest,
            event_digest,
            receipt_digest,
            base_revision,
            next_revision,
            source_state_digest,
            target_state_before,
            source_graph_digest,
            prior_chain_digest,
            from_formula_digest,
            to_formula_digest,
            field_domain,
            migration_id,
        };
        if upgrade.migration_id != upgrade.expected_migration_id() {
            return Err(StoreError::ContinuityFence("legacy_upgrade_id"));
        }
        Ok(upgrade)
    }
}

/// Produce the receipt-bound delta that upgrades an empty semantic lane from
/// its active Genesis formula to the Phase-0 native dynamics formula.
pub fn phase0_formula_transition_delta_v1(
    receipt: &TransitionReceipt,
    base_graph_digest: Digest,
    genesis_formula_digest: Digest,
) -> Vec<u8> {
    Phase0FormulaTransitionV1 {
        scope_digest: receipt.scope_digest,
        event_digest: receipt.event_digest,
        receipt_digest: wire::receipt_digest(receipt),
        base_revision: receipt.base_revision,
        next_revision: receipt.next_revision,
        base_graph_digest,
        from_formula_digest: genesis_formula_digest,
        to_formula_digest: phase0_canonical_formula_digest_v1(&genesis_formula_digest),
    }
    .canonical_bytes()
}

type StoredJournalColumns = (i64, String, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>);
type StoredJournalListColumns = (i64, i64, String, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>);
type StoredGraphColumns = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>);
type StoredContextColumns = (Vec<u8>, Vec<u8>, i64, Vec<u8>, Vec<u8>);
type StoredContextDuplicateColumns = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>);
type StoredActiveSemanticIdentityColumns = (
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
);
type StoredFieldMigrationBackupColumns = (
    Vec<u8>,
    i64,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    i64,
    Vec<u8>,
    Vec<u8>,
);

pub struct Store {
    conn: Option<Connection>,
    database_path: Option<PathBuf>,
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

fn revision_from_sqlite(value: i64) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| StoreError::ContinuityFence(REVISION_RANGE_FENCE))
}

fn revision_to_sqlite(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::ContinuityFence(REVISION_RANGE_FENCE))
}

fn next_sqlite_revision(current: u64) -> Result<(u64, i64), StoreError> {
    let next = current
        .checked_add(1)
        .ok_or(StoreError::ContinuityFence(REVISION_RANGE_FENCE))?;
    Ok((next, revision_to_sqlite(next)?))
}

fn is_formula_transition_delta(bytes: &[u8]) -> bool {
    bytes.starts_with(PHASE0_FORMULA_TRANSITION_MAGIC_V1)
        || bytes.starts_with(LEGACY_SEMANTIC_FORMULA_UPGRADE_MAGIC_V1)
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
        Ok(Self {
            conn: Some(conn),
            database_path: Some(path.to_path_buf()),
        })
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        let mut conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Self::migrate(&mut conn)?;
        Ok(Self {
            conn: Some(conn),
            database_path: None,
        })
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
            CREATE TABLE IF NOT EXISTS legacy_semantic_formula_upgrades (
                scope_digest BLOB NOT NULL CHECK (length(scope_digest) = 32),
                from_formula_digest BLOB NOT NULL CHECK (length(from_formula_digest) = 32),
                to_formula_digest BLOB NOT NULL CHECK (length(to_formula_digest) = 32),
                base_revision INTEGER NOT NULL,
                next_revision INTEGER NOT NULL,
                event_digest BLOB NOT NULL CHECK (length(event_digest) = 32),
                receipt_digest BLOB NOT NULL CHECK (length(receipt_digest) = 32),
                source_state_digest BLOB NOT NULL CHECK (length(source_state_digest) = 32),
                target_state_before BLOB NOT NULL CHECK (length(target_state_before) = 32),
                source_graph_digest BLOB NOT NULL CHECK (length(source_graph_digest) = 32),
                prior_chain_digest BLOB NOT NULL CHECK (length(prior_chain_digest) = 32),
                migration_id BLOB NOT NULL CHECK (length(migration_id) = 32),
                upgrade_bytes BLOB NOT NULL,
                PRIMARY KEY (scope_digest, from_formula_digest, to_formula_digest),
                UNIQUE (scope_digest, next_revision),
                UNIQUE (migration_id)
            );
            CREATE TABLE IF NOT EXISTS field_migration_preimage_backups (
                migration_id BLOB PRIMARY KEY CHECK (length(migration_id) = 32),
                scope_digest BLOB NOT NULL CHECK (length(scope_digest) = 32),
                source_revision INTEGER NOT NULL,
                source_state_digest BLOB NOT NULL CHECK (length(source_state_digest) = 32),
                source_formula_digest BLOB NOT NULL CHECK (length(source_formula_digest) = 32),
                source_graph_digest BLOB NOT NULL CHECK (length(source_graph_digest) = 32),
                incarnation_id BLOB NOT NULL CHECK (length(incarnation_id) = 32),
                manifest_digest BLOB NOT NULL CHECK (length(manifest_digest) = 32),
                byte_len INTEGER NOT NULL,
                sha256 BLOB NOT NULL CHECK (length(sha256) = 32),
                manifest_bytes BLOB NOT NULL
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
                    let new_epoch = epoch + 1;
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
        let row: Option<(Vec<u8>, i64)> = conn
            .query_row(
                "SELECT incarnation_id, revision FROM active_bindings WHERE bot_token = ?1 AND persona_token = ?2",
                params![blob(*bot_token), blob(*persona_token)],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((incarnation_id, revision)) = row else {
            return Ok(None);
        };
        Ok(Some(BindingRow {
            bot_token: *bot_token,
            persona_token: *persona_token,
            incarnation_id: digest_from_blob(&incarnation_id, "binding_incarnation")?,
            revision: revision_from_sqlite(revision)?,
        }))
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
        revision_from_sqlite(revision)
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
        self.read_journal_row(scope_digest, revision_from_sqlite(revision)?)
    }

    fn read_journal_row(
        &self,
        scope_digest: &Digest,
        revision: u64,
    ) -> Result<Option<JournalRow>, StoreError> {
        let conn = self.connection()?;
        let revision_sql = revision_to_sqlite(revision)?;
        let stored: Option<StoredJournalColumns> = conn
            .query_row(
                "SELECT base_revision, event_kind, event_bytes, event_digest, receipt_bytes, chain_digest FROM journal WHERE scope_digest = ?1 AND logical_revision = ?2",
                params![blob(*scope_digest), revision_sql],
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
            base_revision: revision_from_sqlite(base_revision)?,
            event_kind,
            event_bytes,
            event_digest: digest_from_blob(&event_digest, "stored_event_digest")?,
            receipt_bytes,
            chain_digest: digest_from_blob(&chain_digest, "stored_chain_digest")?,
        }))
    }

    pub fn read_journal(&self, scope_digest: &Digest) -> Result<Vec<JournalRow>, StoreError> {
        let conn = self.connection()?;
        let mut statement = conn.prepare(
            "SELECT logical_revision, base_revision, event_kind, event_bytes, event_digest, receipt_bytes, chain_digest FROM journal WHERE scope_digest = ?1 ORDER BY logical_revision ASC",
        )?;
        let stored = statement
            .query_map(params![blob(*scope_digest)], |row| {
                Ok::<StoredJournalListColumns, rusqlite::Error>((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        stored
            .into_iter()
            .map(
                |(
                    revision,
                    base_revision,
                    event_kind,
                    event_bytes,
                    event_digest,
                    receipt_bytes,
                    chain_digest,
                )| {
                    Ok(JournalRow {
                        revision: revision_from_sqlite(revision)?,
                        scope_digest: *scope_digest,
                        base_revision: revision_from_sqlite(base_revision)?,
                        event_kind,
                        event_bytes,
                        event_digest: digest_from_blob(&event_digest, "stored_event_digest")?,
                        receipt_bytes,
                        chain_digest: digest_from_blob(&chain_digest, "stored_chain_digest")?,
                    })
                },
            )
            .collect()
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
        let revision_sql = revision_to_sqlite(revision)?;
        let stored: Option<StoredJournalColumns> = tx
            .query_row(
                "SELECT base_revision, event_kind, event_bytes, event_digest, receipt_bytes, chain_digest FROM journal WHERE scope_digest = ?1 AND logical_revision = ?2",
                params![blob(*scope_digest), revision_sql],
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
            base_revision: revision_from_sqlite(base_revision)?,
            event_kind,
            event_bytes,
            event_digest: digest_from_blob(&event_digest, "stored_event_digest")?,
            receipt_bytes,
            chain_digest: digest_from_blob(&chain_digest, "stored_chain_digest")?,
        }))
    }

    fn current_snapshot_tx(
        tx: &Transaction<'_>,
        scope_digest: &Digest,
    ) -> Result<Option<(Digest, Vec<u8>)>, StoreError> {
        let snapshot: Option<(Vec<u8>, Vec<u8>)> = tx
            .query_row(
                "SELECT state_digest, state_bytes FROM snapshots WHERE scope_digest = ?1 ORDER BY revision DESC LIMIT 1",
                params![blob(*scope_digest)],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        snapshot
            .map(|(state_digest, state_bytes)| {
                Ok((
                    digest_from_blob(&state_digest, "stored_snapshot_digest")?,
                    state_bytes,
                ))
            })
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

    fn active_semantic_storage_scope_matches_tx(
        tx: &Transaction<'_>,
        event_scope: &ScopeRef,
        expected_genesis_formula: Digest,
    ) -> Result<bool, StoreError> {
        let active: Option<(Vec<u8>, Vec<u8>)> = tx
            .query_row(
                "SELECT i.incarnation_id, i.formula_digest FROM active_bindings AS b JOIN incarnations AS i ON i.incarnation_id = b.incarnation_id WHERE b.bot_token = ?1 AND b.persona_token = ?2",
                params![
                    blob(event_scope.bot_token),
                    blob(event_scope.persona_token),
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((incarnation_id, genesis_formula)) = active else {
            return Ok(false);
        };
        let incarnation_id = digest_from_blob(&incarnation_id, "semantic_incarnation")?;
        let genesis_formula = digest_from_blob(&genesis_formula, "semantic_formula")?;
        if genesis_formula != expected_genesis_formula {
            return Ok(false);
        }
        let root_scope =
            wire::persona_scope_digest(&event_scope.bot_token, &event_scope.persona_token, None);
        let binding = wire::domain_hash(
            SEMANTIC_LANE_NAMESPACE_DOMAIN_V1,
            &[&root_scope, &incarnation_id, &genesis_formula],
        );
        let mut expected_relation = [0u8; 16];
        expected_relation.copy_from_slice(&binding[..16]);
        let mut expected_session = [0u8; 16];
        expected_session.copy_from_slice(&binding[16..]);
        Ok(
            event_scope.relation_token.as_ref() == Some(&expected_relation)
                && event_scope.session_token == expected_session,
        )
    }

    fn active_semantic_identity_tx(
        tx: &Transaction<'_>,
        event_scope: &ScopeRef,
    ) -> Result<Option<ActiveSemanticIdentityV1>, StoreError> {
        let stored: Option<StoredActiveSemanticIdentityColumns> = tx
            .query_row(
                "SELECT i.incarnation_id, i.formula_digest, i.initial_snapshot_digest, i.graph_digest, i.development_seed_digest, i.manifest_digest, m.canonical_bytes FROM active_bindings AS b JOIN incarnations AS i ON i.incarnation_id = b.incarnation_id JOIN genesis_manifests AS m ON m.manifest_digest = i.manifest_digest WHERE b.bot_token = ?1 AND b.persona_token = ?2",
                params![blob(event_scope.bot_token), blob(event_scope.persona_token)],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            incarnation_id,
            formula_digest,
            initial_snapshot_digest,
            graph_digest_value,
            development_seed_digest,
            manifest_digest,
            manifest_bytes,
        )) = stored
        else {
            return Ok(None);
        };
        let incarnation_id = digest_from_blob(&incarnation_id, "semantic_incarnation")?;
        let formula_digest = digest_from_blob(&formula_digest, "semantic_formula")?;
        let initial_snapshot_digest =
            digest_from_blob(&initial_snapshot_digest, "semantic_initial_snapshot")?;
        let graph_digest_value = digest_from_blob(&graph_digest_value, "semantic_initial_graph")?;
        let development_seed_digest =
            digest_from_blob(&development_seed_digest, "semantic_development_seed")?;
        let manifest_digest = digest_from_blob(&manifest_digest, "semantic_manifest")?;
        let manifest = wire::decode_manifest_body(&manifest_bytes)
            .map_err(|_| StoreError::ContinuityFence("semantic_manifest_wire"))?;
        if wire::encode_manifest_body(&manifest) != manifest_bytes
            || wire::manifest_body_digest(&manifest) != manifest_digest
        {
            return Err(StoreError::ContinuityFence("semantic_manifest_closure"));
        }
        let (baseline_field, baseline_graph) =
            initial_state_from_manifest(&manifest, &formula_digest, &development_seed_digest);
        if !baseline_field.validate()
            || !baseline_graph.validate()
            || state_digest(&baseline_field, &formula_digest) != initial_snapshot_digest
            || graph_digest(&baseline_graph) != graph_digest_value
        {
            return Err(StoreError::ContinuityFence("semantic_genesis_closure"));
        }
        Ok(Some(ActiveSemanticIdentityV1 {
            incarnation_id,
            manifest_digest,
            formula_digest,
            initial_snapshot_digest,
            baseline_field,
            baseline_graph,
        }))
    }

    fn snapshot_at_tx(
        tx: &Transaction<'_>,
        scope_digest: &Digest,
        revision: u64,
    ) -> Result<Option<(Digest, Vec<u8>)>, StoreError> {
        let revision_sql = revision_to_sqlite(revision)?;
        let stored: Option<(Vec<u8>, Vec<u8>)> = tx
            .query_row(
                "SELECT state_digest, state_bytes FROM snapshots WHERE scope_digest = ?1 AND revision = ?2",
                params![blob(*scope_digest), revision_sql],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        stored
            .map(|(digest, bytes)| {
                Ok((
                    digest_from_blob(&digest, "semantic_snapshot_digest")?,
                    bytes,
                ))
            })
            .transpose()
    }

    fn graph_at_tx(
        tx: &Transaction<'_>,
        scope_digest: &Digest,
        revision: u64,
    ) -> Result<Option<GraphCommitV1>, StoreError> {
        let revision_sql = revision_to_sqlite(revision)?;
        let stored: Option<StoredGraphColumns> = tx
            .query_row(
                "SELECT base_graph_digest, graph_digest, formula_digest, delta_bytes, replay_state_bytes FROM graph_commits WHERE scope_digest = ?1 AND revision = ?2",
                params![blob(*scope_digest), revision_sql],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        stored
            .map(
                |(
                    base_graph_digest,
                    graph_digest_value,
                    formula_digest,
                    delta_bytes,
                    replay_state_bytes,
                )| {
                    Ok(GraphCommitV1 {
                        base_graph_digest: digest_from_blob(
                            &base_graph_digest,
                            "semantic_graph_base_digest",
                        )?,
                        graph_digest: digest_from_blob(
                            &graph_digest_value,
                            "semantic_graph_digest",
                        )?,
                        formula_digest: digest_from_blob(
                            &formula_digest,
                            "semantic_graph_formula",
                        )?,
                        delta_bytes,
                        replay_state_bytes,
                    })
                },
            )
            .transpose()
    }

    fn context_at_tx(
        tx: &Transaction<'_>,
        scope_digest: &Digest,
        relation_scope_token: &[u8; 16],
        revision: u64,
    ) -> Result<Option<ContextCommitV1>, StoreError> {
        let revision_sql = revision_to_sqlite(revision)?;
        let stored: Option<StoredContextDuplicateColumns> = tx
            .query_row(
                "SELECT relation_hmac, context_digest, canonical_state_bytes, relation_scope_token FROM context_commits WHERE scope_digest = ?1 AND relation_scope_token = ?2 AND revision = ?3",
                params![blob(*scope_digest), blob(*relation_scope_token), revision_sql],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        stored
            .map(
                |(relation_hmac, context_digest, canonical_state_bytes, stored_relation_token)| {
                    Ok(ContextCommitV1 {
                        relation_scope_token: token_from_blob(
                            &stored_relation_token,
                            "semantic_context_relation",
                        )?,
                        relation_hmac: digest_from_blob(&relation_hmac, "semantic_context_hmac")?,
                        source_continuum_revision: revision,
                        context_digest: digest_from_blob(
                            &context_digest,
                            "semantic_context_digest",
                        )?,
                        canonical_state_bytes,
                    })
                },
            )
            .transpose()
    }

    fn context_closes(context: &ContextCommitV1) -> Result<(), StoreError> {
        if continuity_context_digest(&context.canonical_state_bytes) != context.context_digest {
            return Err(StoreError::ContinuityFence("semantic_context_digest"));
        }
        let decoded = ContextProjectionStateV1::try_from_canonical_state_bytes(
            &context.canonical_state_bytes,
        )
        .map_err(|_| StoreError::ContinuityFence("semantic_context_wire"))?;
        if decoded.canonical_state_bytes() != context.canonical_state_bytes
            || decoded.relation_hmac() != context.relation_hmac
        {
            return Err(StoreError::ContinuityFence("semantic_context_canonical"));
        }
        Ok(())
    }

    fn committed_context_receipt(
        event: &CanonicalEvent,
        relation_scope_token: [u8; 16],
        source_continuum_revision: u64,
    ) -> Result<ValidatedCommittedReceiptV1, StoreError> {
        let (event_id, dimensions_fxp6, unresolved_boundary, unresolved_repair, delivery_outcome) =
            match event {
                CanonicalEvent::UserStimulus(stimulus) => {
                    let dimensions = &stimulus.evidence.dimensions;
                    let bounded = |value: ae_fixed::Fixed| {
                        value
                            .raw()
                            .clamp(0, ValidatedCommittedReceiptV1::MAX_DIMENSION_FXP6)
                    };
                    (
                        stimulus.event_id,
                        [
                            bounded(dimensions.positive),
                            bounded(dimensions.affiliation),
                            bounded(dimensions.harm),
                            bounded(dimensions.boundary),
                            bounded(dimensions.repair),
                            bounded(dimensions.repetition),
                            bounded(dimensions.new_information),
                            bounded(dimensions.constraint_instability),
                            bounded(dimensions.epistemic_conflict),
                            bounded(dimensions.self_responsibility),
                            bounded(dimensions.other_responsibility),
                            bounded(dimensions.hostility),
                            bounded(dimensions.publicness),
                            bounded(dimensions.engagement),
                            bounded(dimensions.rejection),
                        ],
                        dimensions.boundary.raw() > 0,
                        dimensions.repair.raw() > 0,
                        ContextDeliveryOutcome::Pending,
                    )
                }
                CanonicalEvent::DeliveryOutcome(outcome) => (
                    outcome.event_id,
                    [0; 15],
                    false,
                    false,
                    if outcome.delivered {
                        ContextDeliveryOutcome::Delivered
                    } else {
                        ContextDeliveryOutcome::Failed
                    },
                ),
                CanonicalEvent::TimeAdvance(advance) => (
                    advance.event_id,
                    [0; 15],
                    false,
                    false,
                    ContextDeliveryOutcome::Pending,
                ),
                _ => return Err(StoreError::ContinuityFence("field_upgrade_context")),
            };
        ValidatedCommittedReceiptV1::try_from_envelope(ReceiptEnvelopeV1 {
            commit_status: ReceiptCommitStatus::Committed,
            event_id,
            relation_token: relation_scope_token,
            source_continuum_revision,
            dimensions_fxp6,
            unresolved_boundary,
            unresolved_repair,
            repetition_increment: 1,
            delivery_outcome,
        })
        .map_err(|_| StoreError::ContinuityFence("field_upgrade_context"))
    }

    fn context_matches_projection(
        context: &ContextCommitV1,
        previous_context_state: Option<&[u8]>,
        event: &CanonicalEvent,
        relation_scope_token: [u8; 16],
        source_continuum_revision: u64,
        fence: &'static str,
    ) -> Result<Vec<u8>, StoreError> {
        Self::context_closes(context)?;
        let receipt =
            Self::committed_context_receipt(event, relation_scope_token, source_continuum_revision)
                .map_err(|_| StoreError::ContinuityFence(fence))?;
        let expected = project_committed_receipt(previous_context_state, &receipt)
            .map_err(|_| StoreError::ContinuityFence(fence))?;
        let expected_bytes = expected.canonical_state_bytes();
        if context.relation_scope_token != relation_scope_token
            || context.source_continuum_revision != source_continuum_revision
            || context.relation_hmac != expected.relation_hmac()
            || context.context_digest != continuity_context_digest(&expected_bytes)
            || context.canonical_state_bytes != expected_bytes
        {
            return Err(StoreError::ContinuityFence(fence));
        }
        Ok(expected_bytes)
    }

    fn attest_field_domain_upgrade_tx(
        tx: &Transaction<'_>,
        input: &LegacySemanticFormulaUpgradeInput<'_>,
        upgrade: LegacySemanticFormulaUpgradeReceiptV1,
    ) -> Result<AttestedFieldDomainUpgradeV1, StoreError> {
        let Some(field_domain) = upgrade.field_domain else {
            return Err(StoreError::ContinuityFence("field_upgrade_metadata"));
        };
        let identity = Self::active_semantic_identity_tx(tx, input.event_scope)?
            .ok_or(StoreError::ContinuityFence("semantic_identity"))?;
        if identity.formula_digest != input.current_formula_digest
            || !Self::active_semantic_storage_scope_matches_tx(
                tx,
                input.event_scope,
                identity.formula_digest,
            )?
        {
            return Err(StoreError::ContinuityFence("semantic_identity"));
        }
        let relation_scope_token = input
            .event_scope
            .relation_token
            .ok_or(StoreError::ContinuityFence("semantic_relation"))?;
        let mut replay_field = identity.baseline_field.clone();
        let replay_graph = identity.baseline_graph.clone();
        let mut chain_seed = identity.initial_snapshot_digest;
        let mut replay_context_state: Option<Vec<u8>> = None;
        let mut latest_snapshot: Option<semantic_field_attestation::DecodedSemanticSnapshotV2> =
            None;
        for revision in 1..=input.current_revision {
            let row = Self::read_journal_row_tx(tx, &input.receipt.scope_digest, revision)?
                .ok_or(StoreError::ContinuityFence("semantic_history_journal"))?;
            if row.revision != revision
                || row.base_revision.checked_add(1) != Some(revision)
                || row.base_revision != revision - 1
            {
                return Err(StoreError::ContinuityFence("semantic_history_revision"));
            }
            let event = wire::decode_event(&row.event_bytes)
                .map_err(|_| StoreError::ContinuityFence("semantic_history_event"))?;
            if wire::encode_event(&event) != row.event_bytes
                || wire::event_digest(&event) != row.event_digest
                || row.event_kind != wire::event_kind_name(&event)
            {
                return Err(StoreError::ContinuityFence("semantic_history_event"));
            }
            let ae_contracts::CanonicalEvent::UserStimulus(stimulus) = &event else {
                return Err(StoreError::ContinuityFence("semantic_history_event"));
            };
            if stimulus.scope != *input.event_scope
                || stimulus.causal.base_revision != row.base_revision
                || stimulus.evidence.schema_version != PerceptionProposalV1::SCHEMA_VERSION
            {
                return Err(StoreError::ContinuityFence("semantic_history_event"));
            }
            let source_receipt = wire::decode_transition_receipt(&row.receipt_bytes)
                .map_err(|_| StoreError::ContinuityFence("semantic_history_receipt"))?;
            if wire::encode_transition_receipt(&source_receipt) != row.receipt_bytes
                || source_receipt.schema_version != 1
                || source_receipt.status != CommitStatus::Committed
                || source_receipt.action_contract.is_some()
                || source_receipt.scope_digest != input.receipt.scope_digest
                || source_receipt.event_digest != row.event_digest
                || source_receipt.formula_digest != identity.formula_digest
                || source_receipt.base_revision != row.base_revision
                || source_receipt.next_revision != revision
                || source_receipt.authority_digest
                    != authority_projection_digest(&ae_contracts::CanonicalEvent::UserStimulus(
                        stimulus.clone(),
                    ))
                || row.chain_digest
                    != ae_continuum::chain_link(&chain_seed, &row.event_bytes, &row.receipt_bytes)
            {
                return Err(StoreError::ContinuityFence("semantic_history_receipt"));
            }
            let (snapshot_digest, snapshot_bytes) =
                Self::snapshot_at_tx(tx, &input.receipt.scope_digest, revision)?
                    .ok_or(StoreError::ContinuityFence("semantic_history_snapshot"))?;
            let snapshot = semantic_field_attestation::decode_semantic_snapshot_v2(
                &snapshot_bytes,
                &identity.formula_digest,
                &snapshot_digest,
                &source_receipt.graph_after,
                &source_receipt,
            )?;
            if !semantic_field_attestation::p_and_e_within_legacy_revision_bound(
                &snapshot.field,
                revision,
            ) {
                return Err(StoreError::ContinuityFence("semantic_history_range"));
            }
            let graph = Self::graph_at_tx(tx, &input.receipt.scope_digest, revision)?
                .ok_or(StoreError::ContinuityFence("semantic_history_graph"))?;
            if graph.base_graph_digest != graph_digest(&replay_graph)
                || graph.graph_digest != graph_digest(&snapshot.graph)
                || graph.formula_digest != identity.formula_digest
                || !graph.delta_bytes.is_empty()
                || graph.replay_state_bytes != snapshot.graph.canonical_bytes()
            {
                return Err(StoreError::ContinuityFence("semantic_history_graph"));
            }
            let context = Self::context_at_tx(
                tx,
                &input.receipt.scope_digest,
                &relation_scope_token,
                revision,
            )?
            .ok_or(StoreError::ContinuityFence("semantic_history_context"))?;
            let expected_context_bytes = Self::context_matches_projection(
                &context,
                replay_context_state.as_deref(),
                &event,
                relation_scope_token,
                revision,
                "semantic_history_context",
            )?;
            let replay = semantic_field_attestation::replay_legacy_aesem2_transition_v1(
                &replay_field,
                &identity.baseline_field,
                &stimulus.evidence.dimensions,
                stimulus.evidence.estimator_confidence,
            )?;
            if state_digest(&replay_field, &identity.formula_digest) != source_receipt.state_before
                || state_digest(&replay.next_field, &identity.formula_digest)
                    != source_receipt.state_after
                || state_digest(&snapshot.field, &identity.formula_digest)
                    != source_receipt.state_after
                || graph_digest(&snapshot.graph) != graph_digest(&replay_graph)
                || source_receipt.graph_after != graph_digest(&replay_graph)
                || source_receipt.active_nodes != replay.active_nodes
                || source_receipt.active_edges != 0
                || source_receipt.residuals != ae_contracts::InvariantResiduals::default()
            {
                return Err(StoreError::ContinuityFence("semantic_history_replay"));
            }
            replay_field = replay.next_field;
            chain_seed = row.chain_digest;
            replay_context_state = Some(expected_context_bytes);
            latest_snapshot = Some(snapshot);
        }
        let latest_snapshot =
            latest_snapshot.ok_or(StoreError::ContinuityFence("semantic_history_snapshot"))?;
        if state_digest(&replay_field, &identity.formula_digest) != input.current_state_digest
            || state_digest(&latest_snapshot.field, &identity.formula_digest)
                != input.current_state_digest
            || graph_digest(&latest_snapshot.graph) != input.current_graph_digest
            || graph_digest(&replay_graph) != input.current_graph_digest
            || chain_seed != input.prior_chain_digest
            || identity.formula_digest != input.current_formula_digest
        {
            return Err(StoreError::ContinuityFence("semantic_history_source"));
        }
        let Some((normalized, recomputed_field_domain)) =
            semantic_field_attestation::normalize_legacy_aesem2_field_domain_v1(
                &latest_snapshot.field,
            )?
        else {
            return Err(StoreError::ContinuityFence("field_upgrade_not_needed"));
        };
        if recomputed_field_domain != field_domain
            || state_digest(&normalized, &input.incoming_formula_digest)
                != input.receipt.state_before
        {
            return Err(StoreError::ContinuityFence("field_upgrade_transform"));
        }
        let expected_upgrade =
            LegacySemanticFormulaUpgradeReceiptV1::from_transition_receipt_with_field_domain(
                input.receipt,
                input.current_state_digest,
                input.current_graph_digest,
                input.current_formula_digest,
                input.prior_chain_digest,
                recomputed_field_domain,
            );
        if upgrade != expected_upgrade
            || input.incoming_formula_digest
                != phase0_canonical_formula_digest_v1(&input.current_formula_digest)
            || input.bundle.graph.replay_state_bytes.is_empty()
        {
            return Err(StoreError::ContinuityFence("field_upgrade_receipt"));
        }
        let incoming_graph = semantic_field_attestation::verify_semantic_snapshot_v3(
            &input.bundle.snapshot.state_bytes,
            &input.incoming_formula_digest,
            &input.bundle.snapshot.state_digest,
            &input.bundle.graph.graph_digest,
            input.receipt,
        )?;
        if incoming_graph != input.bundle.graph.replay_state_bytes {
            return Err(StoreError::ContinuityFence("field_upgrade_graph"));
        }
        Self::context_matches_projection(
            &input.bundle.context,
            replay_context_state.as_deref(),
            input.event,
            relation_scope_token,
            input.receipt.next_revision,
            "field_upgrade_context",
        )?;
        Ok(AttestedFieldDomainUpgradeV1 { upgrade, identity })
    }

    fn field_migration_backup_package_identity() -> String {
        format!("{}@{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
    }

    fn field_migration_backup_build_identity() -> String {
        format!(
            "{}@{};target={}-{};manifest=AE-FMP2",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
        )
    }

    fn field_migration_backup_manifest(backup: &FieldMigrationPreimageBackupV1) -> Vec<u8> {
        let package_identity = backup.package_identity.as_bytes();
        let build_identity = backup.build_identity.as_bytes();
        let package_len = u8::try_from(package_identity.len())
            .expect("field migration backup package identity is bounded");
        let build_len = u8::try_from(build_identity.len())
            .expect("field migration backup build identity is bounded");
        let mut bytes = Vec::with_capacity(
            8 + 3 + package_identity.len() + build_identity.len() + (32 * 8) + 17,
        );
        bytes.extend_from_slice(FIELD_MIGRATION_BACKUP_MANIFEST_MAGIC_V2);
        bytes.push(package_len);
        bytes.extend_from_slice(package_identity);
        bytes.push(build_len);
        bytes.extend_from_slice(build_identity);
        bytes.push(backup.capture_method.as_u8());
        bytes.extend_from_slice(&backup.migration_id);
        bytes.extend_from_slice(&backup.scope_digest);
        bytes.extend_from_slice(&backup.source_revision.to_le_bytes());
        for digest in [
            backup.source_state_digest,
            backup.source_formula_digest,
            backup.source_graph_digest,
            backup.incarnation_id,
            backup.manifest_digest,
        ] {
            bytes.extend_from_slice(&digest);
        }
        bytes.extend_from_slice(&backup.byte_len.to_le_bytes());
        bytes.extend_from_slice(&backup.sha256);
        bytes.push(1);
        bytes
    }

    fn sha256_file(path: &Path) -> Result<(u64, Digest), StoreError> {
        let mut file = File::open(path).map_err(|source| StoreError::Io {
            context: "opening field migration backup",
            source,
        })?;
        let mut hasher = Sha256::new();
        let mut bytes = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).map_err(|source| StoreError::Io {
                context: "reading field migration backup",
                source,
            })?;
            if read == 0 {
                break;
            }
            bytes = bytes
                .checked_add(
                    u64::try_from(read)
                        .map_err(|_| StoreError::ContinuityFence("field_backup_length"))?,
                )
                .ok_or(StoreError::ContinuityFence("field_backup_length"))?;
            hasher.update(&buffer[..read]);
        }
        let mut digest = [0_u8; 32];
        digest.copy_from_slice(&hasher.finalize());
        Ok((bytes, digest))
    }

    fn field_migration_backup_paths(
        database_path: &Path,
        migration_id: &Digest,
        source_revision: u64,
    ) -> Result<(PathBuf, PathBuf, PathBuf, PathBuf), StoreError> {
        let parent = database_path
            .parent()
            .ok_or(StoreError::ContinuityFence("field_backup_path"))?;
        let root = parent.join(".astr-embodiment-field-migration-preimages");
        fs::create_dir_all(&root).map_err(|source| StoreError::Io {
            context: "creating field migration backup directory",
            source,
        })?;
        let stem = format!(
            "migration-{}-r{}",
            ae_contracts::hex::encode32(migration_id),
            source_revision
        );
        let database = root.join(format!("{stem}.authority.sqlite"));
        let manifest = root.join(format!("{stem}.manifest"));
        let database_partial = root.join(format!("{stem}.authority.sqlite.partial"));
        let manifest_partial = root.join(format!("{stem}.manifest.partial"));
        Ok((database, manifest, database_partial, manifest_partial))
    }

    fn validate_preimage_backup_files(
        database_path: &Path,
        backup: &FieldMigrationPreimageBackupV1,
    ) -> Result<(), StoreError> {
        let (database, manifest, _, _) = Self::field_migration_backup_paths(
            database_path,
            &backup.migration_id,
            backup.source_revision,
        )?;
        if !database.is_file() || !manifest.is_file() {
            return Err(StoreError::ContinuityFence("field_backup_missing"));
        }
        let (byte_len, sha256) = Self::sha256_file(&database)?;
        if byte_len != backup.byte_len || sha256 != backup.sha256 {
            return Err(StoreError::ContinuityFence("field_backup_hash"));
        }
        let manifest_bytes = fs::read(&manifest).map_err(|source| StoreError::Io {
            context: "reading field migration backup manifest",
            source,
        })?;
        if manifest_bytes != Self::field_migration_backup_manifest(backup) {
            return Err(StoreError::ContinuityFence("field_backup_manifest"));
        }
        Ok(())
    }

    fn field_migration_backup_from_row_tx(
        tx: &Transaction<'_>,
        expected: &FieldMigrationPreimageBackupV1,
    ) -> Result<Option<FieldMigrationPreimageBackupV1>, StoreError> {
        let stored: Option<StoredFieldMigrationBackupColumns> = tx
            .query_row(
                "SELECT scope_digest, source_revision, source_state_digest, source_formula_digest, source_graph_digest, incarnation_id, manifest_digest, byte_len, sha256, manifest_bytes FROM field_migration_preimage_backups WHERE migration_id = ?1",
                params![blob(expected.migration_id)],
                |row| {
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
                    ))
                },
            )
            .optional()?;
        let Some((
            scope_digest,
            source_revision,
            source_state_digest,
            source_formula_digest,
            source_graph_digest,
            incarnation_id,
            manifest_digest,
            byte_len,
            sha256,
            manifest_bytes,
        )) = stored
        else {
            return Ok(None);
        };
        let backup = FieldMigrationPreimageBackupV1 {
            migration_id: expected.migration_id,
            scope_digest: digest_from_blob(&scope_digest, "field_backup_scope")?,
            source_revision: revision_from_sqlite(source_revision)?,
            source_state_digest: digest_from_blob(&source_state_digest, "field_backup_state")?,
            source_formula_digest: digest_from_blob(
                &source_formula_digest,
                "field_backup_formula",
            )?,
            source_graph_digest: digest_from_blob(&source_graph_digest, "field_backup_graph")?,
            incarnation_id: digest_from_blob(&incarnation_id, "field_backup_incarnation")?,
            manifest_digest: digest_from_blob(&manifest_digest, "field_backup_manifest_digest")?,
            package_identity: expected.package_identity.clone(),
            build_identity: expected.build_identity.clone(),
            capture_method: expected.capture_method,
            byte_len: u64::try_from(byte_len)
                .map_err(|_| StoreError::ContinuityFence("field_backup_length"))?,
            sha256: digest_from_blob(&sha256, "field_backup_sha256")?,
        };
        if backup.scope_digest != expected.scope_digest
            || backup.source_revision != expected.source_revision
            || backup.source_state_digest != expected.source_state_digest
            || backup.source_formula_digest != expected.source_formula_digest
            || backup.source_graph_digest != expected.source_graph_digest
            || backup.incarnation_id != expected.incarnation_id
            || backup.manifest_digest != expected.manifest_digest
            || backup.package_identity != expected.package_identity
            || backup.build_identity != expected.build_identity
            || backup.capture_method != expected.capture_method
            || manifest_bytes != Self::field_migration_backup_manifest(&backup)
        {
            return Err(StoreError::ContinuityFence("field_backup_record"));
        }
        Ok(Some(backup))
    }

    fn ensure_field_migration_preimage_backup_tx(
        tx: &Transaction<'_>,
        database_path: Option<&Path>,
        upgrade: &LegacySemanticFormulaUpgradeReceiptV1,
        identity: &ActiveSemanticIdentityV1,
    ) -> Result<FieldMigrationPreimageBackupV1, StoreError> {
        let database_path =
            database_path.ok_or(StoreError::ContinuityFence("field_backup_path"))?;
        let expected = FieldMigrationPreimageBackupV1 {
            migration_id: upgrade.migration_id,
            scope_digest: upgrade.scope_digest,
            source_revision: upgrade.base_revision,
            source_state_digest: upgrade.source_state_digest,
            source_formula_digest: upgrade.from_formula_digest,
            source_graph_digest: upgrade.source_graph_digest,
            incarnation_id: identity.incarnation_id,
            manifest_digest: identity.manifest_digest,
            package_identity: Self::field_migration_backup_package_identity(),
            build_identity: Self::field_migration_backup_build_identity(),
            capture_method: FieldMigrationBackupCaptureMethodV1::SqliteBackupApi,
            byte_len: 0,
            sha256: [0; 32],
        };
        if let Some(existing) = Self::field_migration_backup_from_row_tx(tx, &expected)? {
            Self::validate_preimage_backup_files(database_path, &existing)?;
            return Ok(existing);
        }
        let (database, manifest, database_partial, manifest_partial) =
            Self::field_migration_backup_paths(
                database_path,
                &expected.migration_id,
                expected.source_revision,
            )?;
        if database_partial.exists() || manifest_partial.exists() {
            return Err(StoreError::ContinuityFence("field_backup_incomplete"));
        }
        if database.exists() || manifest.exists() {
            if !database.is_file() || !manifest.is_file() {
                return Err(StoreError::ContinuityFence("field_backup_incomplete"));
            }
            let (byte_len, sha256) = Self::sha256_file(&database)?;
            let completed = FieldMigrationPreimageBackupV1 {
                byte_len,
                sha256,
                ..expected
            };
            // A transaction may fail after the fsync/rename gate but before
            // its SQLite commit.  The completed, identity-bound preimage is
            // intentionally reusable by the exact retry; any partial or
            // mismatched artifact remains a hard refusal.
            Self::validate_preimage_backup_files(database_path, &completed)?;
            return Ok(completed);
        }
        // SQLite's backup API cannot use the same connection while it owns a
        // write transaction.  A second read-only handle observes the exact
        // pre-write image while this `BEGIN IMMEDIATE` transaction excludes
        // competing writers; the complete backup is re-hashed before any
        // authority row is inserted below.
        let source =
            Connection::open_with_flags(database_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|_| StoreError::FieldMigrationBackup {
                    context: "opening read-only authority source",
                })?;
        source
            .backup(rusqlite::DatabaseName::Main, &database_partial, None)
            .map_err(|_| StoreError::FieldMigrationBackup {
                context: "capturing authority preimage",
            })?;
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&database_partial)
            .map_err(|source| StoreError::Io {
                context: "opening field migration backup for sync",
                source,
            })?
            .sync_all()
            .map_err(|source| StoreError::Io {
                context: "syncing field migration backup",
                source,
            })?;
        let (byte_len, sha256) = Self::sha256_file(&database_partial)?;
        let backup = FieldMigrationPreimageBackupV1 {
            byte_len,
            sha256,
            ..expected
        };
        fs::rename(&database_partial, &database).map_err(|source| StoreError::Io {
            context: "finalizing field migration backup",
            source,
        })?;
        let manifest_bytes = Self::field_migration_backup_manifest(&backup);
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&manifest_partial)
            .map_err(|source| StoreError::Io {
                context: "creating field migration backup manifest",
                source,
            })?;
        use std::io::Write;
        output
            .write_all(&manifest_bytes)
            .map_err(|source| StoreError::Io {
                context: "writing field migration backup manifest",
                source,
            })?;
        output.sync_all().map_err(|source| StoreError::Io {
            context: "syncing field migration backup manifest",
            source,
        })?;
        drop(output);
        fs::rename(&manifest_partial, &manifest).map_err(|source| StoreError::Io {
            context: "finalizing field migration backup manifest",
            source,
        })?;
        Self::validate_preimage_backup_files(database_path, &backup)?;
        Ok(backup)
    }

    fn insert_field_migration_preimage_backup_tx(
        tx: &Transaction<'_>,
        backup: &FieldMigrationPreimageBackupV1,
    ) -> Result<(), StoreError> {
        tx.execute(
            "INSERT INTO field_migration_preimage_backups (migration_id, scope_digest, source_revision, source_state_digest, source_formula_digest, source_graph_digest, incarnation_id, manifest_digest, byte_len, sha256, manifest_bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                blob(backup.migration_id),
                blob(backup.scope_digest),
                revision_to_sqlite(backup.source_revision)?,
                blob(backup.source_state_digest),
                blob(backup.source_formula_digest),
                blob(backup.source_graph_digest),
                blob(backup.incarnation_id),
                blob(backup.manifest_digest),
                i64::try_from(backup.byte_len)
                    .map_err(|_| StoreError::ContinuityFence("field_backup_length"))?,
                blob(backup.sha256),
                Self::field_migration_backup_manifest(backup),
            ],
        )?;
        Ok(())
    }

    fn phase0_formula_transition_is_allowed_tx(
        tx: &Transaction<'_>,
        input: Phase0FormulaTransitionInput<'_>,
    ) -> Result<bool, StoreError> {
        let Phase0FormulaTransitionInput {
            delta_bytes,
            event,
            event_scope,
            receipt,
            current_revision,
            current_graph,
            current_formula,
            incoming_formula,
        } = input;
        if current_revision != 0 || !matches!(event, CanonicalEvent::UserStimulus(_)) {
            return Ok(false);
        }
        let existing_graph_commit: Option<i64> = tx
            .query_row(
                "SELECT revision FROM graph_commits WHERE scope_digest = ?1 LIMIT 1",
                params![blob(receipt.scope_digest)],
                |row| row.get(0),
            )
            .optional()?;
        if existing_graph_commit.is_some()
            || !Self::active_semantic_storage_scope_matches_tx(tx, event_scope, current_formula)?
        {
            return Ok(false);
        }
        let transition = Phase0FormulaTransitionV1::decode(delta_bytes)?;
        Ok(transition.scope_digest == receipt.scope_digest
            && transition.event_digest == receipt.event_digest
            && transition.receipt_digest == wire::receipt_digest(receipt)
            && transition.base_revision == receipt.base_revision
            && transition.next_revision == receipt.next_revision
            && transition.base_revision == 0
            && transition.next_revision == 1
            && transition.base_graph_digest == current_graph
            && transition.from_formula_digest == current_formula
            && transition.to_formula_digest == incoming_formula
            && transition.to_formula_digest == receipt.formula_digest
            && transition.to_formula_digest == phase0_canonical_formula_digest_v1(&current_formula)
            && transition.to_formula_digest != transition.from_formula_digest)
    }

    fn legacy_semantic_formula_upgrade_is_allowed_tx(
        tx: &Transaction<'_>,
        input: LegacySemanticFormulaUpgradeInput<'_>,
    ) -> Result<Option<LegacySemanticAdmissionV1>, StoreError> {
        if input.current_revision == 0
            || !matches!(input.event, CanonicalEvent::UserStimulus(_))
            || !input.current_state_bytes.starts_with(AESEM2_SNAPSHOT_MAGIC)
        {
            return Ok(None);
        }
        let upgrade = LegacySemanticFormulaUpgradeReceiptV1::decode(input.delta_bytes)?;
        if upgrade.field_domain.is_some() {
            let attested = Self::attest_field_domain_upgrade_tx(tx, &input, upgrade)?;
            return Ok(Some(LegacySemanticAdmissionV1::FieldDomain(Box::new(
                attested,
            ))));
        }
        if !Self::active_semantic_storage_scope_matches_tx(
            tx,
            input.event_scope,
            input.current_formula_digest,
        )? {
            return Ok(None);
        }
        let current_revision_sql = revision_to_sqlite(input.current_revision)?;
        let source_receipt_bytes: Option<Vec<u8>> = tx
            .query_row(
                "SELECT receipt_bytes FROM journal WHERE scope_digest = ?1 AND logical_revision = ?2",
                params![blob(input.receipt.scope_digest), current_revision_sql],
                |row| row.get(0),
            )
            .optional()?;
        let Some(source_receipt_bytes) = source_receipt_bytes else {
            return Ok(None);
        };
        let source_receipt = wire::decode_transition_receipt(&source_receipt_bytes)
            .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_source_receipt"))?;
        if wire::encode_transition_receipt(&source_receipt) != source_receipt_bytes {
            return Err(StoreError::ContinuityFence("legacy_upgrade_source_receipt"));
        }
        let existing_upgrade: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM legacy_semantic_formula_upgrades WHERE scope_digest = ?1 AND from_formula_digest = ?2 AND to_formula_digest = ?3",
                params![
                    blob(input.receipt.scope_digest),
                    blob(input.current_formula_digest),
                    blob(input.incoming_formula_digest),
                ],
                |row| row.get(0),
            )
            .optional()?;
        if existing_upgrade.is_some() {
            return Ok(None);
        }

        let expected_next_revision = input
            .current_revision
            .checked_add(1)
            .ok_or(StoreError::ContinuityFence("revision_overflow"))?;
        let expected_upgrade = LegacySemanticFormulaUpgradeReceiptV1::from_transition_receipt(
            input.receipt,
            input.current_state_digest,
            input.current_graph_digest,
            input.current_formula_digest,
            input.prior_chain_digest,
        );
        let source_receipt_is_attested_aesem2 = source_receipt.schema_version == 1
            && source_receipt.status == CommitStatus::Committed
            && source_receipt.action_contract.is_none()
            && source_receipt.scope_digest == input.receipt.scope_digest
            && source_receipt.formula_digest == input.current_formula_digest
            && source_receipt.next_revision == input.current_revision
            && source_receipt.base_revision.checked_add(1) == Some(input.current_revision)
            && source_receipt.state_after == input.current_state_digest
            && source_receipt.graph_after == input.current_graph_digest;
        if !source_receipt_is_attested_aesem2
            || upgrade != expected_upgrade
            || upgrade.next_revision != expected_next_revision
            || upgrade.from_formula_digest == upgrade.to_formula_digest
            || upgrade.to_formula_digest
                != phase0_canonical_formula_digest_v1(&upgrade.from_formula_digest)
        {
            return Ok(None);
        }
        Ok(Some(LegacySemanticAdmissionV1::FormulaOnly(Box::new(
            upgrade,
        ))))
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
        let row_revision_sql = revision_to_sqlite(row.revision)?;
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
                params![blob(bundle.envelope.receipt.scope_digest), row_revision_sql],
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
                params![blob(bundle.envelope.receipt.scope_digest), row_revision_sql],
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

        if bundle
            .envelope
            .delta_bytes
            .starts_with(LEGACY_SEMANTIC_FORMULA_UPGRADE_MAGIC_V1)
        {
            let upgrade =
                LegacySemanticFormulaUpgradeReceiptV1::decode(&bundle.envelope.delta_bytes)?;
            let stored_upgrade: Option<Vec<u8>> = tx
                .query_row(
                    "SELECT upgrade_bytes FROM legacy_semantic_formula_upgrades WHERE scope_digest = ?1 AND from_formula_digest = ?2 AND to_formula_digest = ?3",
                    params![
                        blob(upgrade.scope_digest),
                        blob(upgrade.from_formula_digest),
                        blob(upgrade.to_formula_digest),
                    ],
                    |stored| stored.get(0),
                )
                .optional()?;
            if stored_upgrade != Some(bundle.envelope.delta_bytes.clone()) {
                return Ok(false);
            }
        }

        let context: Option<StoredContextDuplicateColumns> = tx
            .query_row(
                "SELECT relation_hmac, context_digest, canonical_state_bytes, relation_scope_token FROM context_commits WHERE scope_digest = ?1 AND relation_scope_token = ?2 AND revision = ?3",
                params![
                    blob(bundle.envelope.receipt.scope_digest),
                    blob(bundle.context.relation_scope_token),
                    row_revision_sql,
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
    ) -> Result<ContinuityCommitOutcomeV1, StoreError> {
        let event = Self::validate_continuity_payloads(bundle)?;
        let event_scope = scope_from_event(&event);
        let scope_digest = bundle.envelope.receipt.scope_digest;
        let receipt_bytes = wire::encode_transition_receipt(&bundle.envelope.receipt);
        let chain_digest = ae_continuum::chain_link(
            &bundle.envelope.chain_seed,
            &bundle.envelope.event_bytes,
            &receipt_bytes,
        );
        let database_path = self.database_path.clone();

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
            let revision = revision_from_sqlite(revision)?;
            let row = Self::read_journal_row_tx(&tx, &scope_digest, revision)?
                .ok_or(StoreError::ContinuityIncomplete)?;
            if !Self::duplicate_bundle_is_identical_tx(&tx, bundle, &row)? {
                return Err(StoreError::ContinuityDuplicateMismatch);
            }
            tx.commit()?;
            return Ok(ContinuityCommitOutcomeV1::ExistingIdentical { revision, row });
        }

        let current_sql: i64 = tx.query_row(
            "SELECT COALESCE(MAX(logical_revision), 0) FROM journal WHERE scope_digest = ?1",
            params![blob(scope_digest)],
            |row| row.get::<_, i64>(0),
        )?;
        let current = revision_from_sqlite(current_sql)?;
        if bundle.envelope.receipt.base_revision != current {
            return Err(StoreError::StaleRevision {
                expected: bundle.envelope.receipt.base_revision,
                actual: current,
            });
        }
        let (expected_next, expected_next_sql) = next_sqlite_revision(current)?;
        if bundle.envelope.receipt.next_revision != expected_next {
            return Err(StoreError::StaleRevision {
                expected: bundle.envelope.receipt.next_revision,
                actual: expected_next,
            });
        }
        let base_revision_sql = revision_to_sqlite(bundle.envelope.receipt.base_revision)?;

        let current_snapshot = Self::current_snapshot_tx(&tx, &scope_digest)?;
        if current_snapshot.is_none() && current != 0 {
            return Err(StoreError::ContinuityFence("missing_snapshot"));
        }
        let current_graph_authority =
            Self::current_graph_authority_tx(&tx, &scope_digest, event_scope)?;
        if current_graph_authority.is_none() && current != 0 {
            return Err(StoreError::ContinuityFence("missing_graph"));
        }
        let last_chain = Self::last_chain_digest_tx(&tx, &scope_digest)?;
        if let Some(last_chain) = last_chain {
            if bundle.envelope.chain_seed != last_chain {
                return Err(StoreError::ContinuityFence("chain_seed"));
            }
        }

        let transition_admission = match current_graph_authority {
            Some((current_graph, current_formula)) => {
                if bundle.graph.base_graph_digest != current_graph {
                    return Err(StoreError::ContinuityFence("graph_base"));
                }
                if bundle.graph.formula_digest == current_formula {
                    if is_formula_transition_delta(&bundle.envelope.delta_bytes) {
                        return Err(StoreError::ContinuityFence("formula_transition_unexpected"));
                    }
                    None
                } else if bundle
                    .envelope
                    .delta_bytes
                    .starts_with(PHASE0_FORMULA_TRANSITION_MAGIC_V1)
                {
                    if !Self::phase0_formula_transition_is_allowed_tx(
                        &tx,
                        Phase0FormulaTransitionInput {
                            delta_bytes: &bundle.envelope.delta_bytes,
                            event: &event,
                            event_scope,
                            receipt: &bundle.envelope.receipt,
                            current_revision: current,
                            current_graph,
                            current_formula,
                            incoming_formula: bundle.graph.formula_digest,
                        },
                    )? {
                        return Err(StoreError::ContinuityFence("graph_current_formula"));
                    }
                    Some(FormulaTransitionAdmission::Phase0)
                } else if bundle
                    .envelope
                    .delta_bytes
                    .starts_with(LEGACY_SEMANTIC_FORMULA_UPGRADE_MAGIC_V1)
                {
                    let Some((current_state_digest, current_state_bytes)) =
                        current_snapshot.as_ref()
                    else {
                        return Err(StoreError::ContinuityFence("state_before"));
                    };
                    let Some(prior_chain_digest) = last_chain else {
                        return Err(StoreError::ContinuityFence("legacy_upgrade_chain"));
                    };
                    let Some(upgrade) = Self::legacy_semantic_formula_upgrade_is_allowed_tx(
                        &tx,
                        LegacySemanticFormulaUpgradeInput {
                            bundle,
                            delta_bytes: &bundle.envelope.delta_bytes,
                            event: &event,
                            event_scope,
                            receipt: &bundle.envelope.receipt,
                            current_revision: current,
                            current_state_digest: *current_state_digest,
                            current_state_bytes,
                            current_graph_digest: current_graph,
                            current_formula_digest: current_formula,
                            incoming_formula_digest: bundle.graph.formula_digest,
                            prior_chain_digest,
                        },
                    )?
                    else {
                        return Err(StoreError::ContinuityFence("graph_current_formula"));
                    };
                    Some(FormulaTransitionAdmission::LegacySemantic(Box::new(
                        upgrade,
                    )))
                } else {
                    return Err(StoreError::ContinuityFence("graph_current_formula"));
                }
            }
            None => {
                if is_formula_transition_delta(&bundle.envelope.delta_bytes) {
                    return Err(StoreError::ContinuityFence("formula_transition_unexpected"));
                }
                None
            }
        };
        // The field-domain branch is deliberately re-attested from durable
        // authority inside this `BEGIN IMMEDIATE` transaction.  The receipt's
        // metadata is only a claim: no caller-provided aggregate can grant an
        // overflow migration.  Before the first write, take and verify the
        // one immutable preimage backup; failure leaves SQLite untouched.
        let (legacy_upgrade, field_migration_backup) = match transition_admission {
            Some(FormulaTransitionAdmission::LegacySemantic(admission)) => match *admission {
                LegacySemanticAdmissionV1::FormulaOnly(upgrade) => (Some(*upgrade), None),
                LegacySemanticAdmissionV1::FieldDomain(attested) => {
                    let AttestedFieldDomainUpgradeV1 { upgrade, identity } = *attested;
                    let backup = Self::ensure_field_migration_preimage_backup_tx(
                        &tx,
                        database_path.as_deref(),
                        &upgrade,
                        &identity,
                    )?;
                    (Some(upgrade), Some(backup))
                }
            },
            Some(FormulaTransitionAdmission::Phase0) | None => (None, None),
        };
        let legacy_upgrade_revisions = legacy_upgrade
            .map(|upgrade| {
                Ok::<(i64, i64), StoreError>((
                    revision_to_sqlite(upgrade.base_revision)?,
                    revision_to_sqlite(upgrade.next_revision)?,
                ))
            })
            .transpose()?;
        if let Some((current_state_digest, _)) = current_snapshot {
            if bundle.envelope.receipt.state_before != current_state_digest
                && legacy_upgrade.is_none()
            {
                return Err(StoreError::ContinuityFence("state_before"));
            }
        }

        if let Some(backup) = field_migration_backup.as_ref() {
            Self::insert_field_migration_preimage_backup_tx(&tx, backup)?;
        }

        tx.execute(
            "INSERT INTO journal (logical_revision, scope_digest, base_revision, event_kind, event_bytes, event_digest, receipt_bytes, chain_digest, committed_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                expected_next_sql,
                blob(scope_digest),
                base_revision_sql,
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
                expected_next_sql,
            ],
        )?;
        tx.execute(
            "INSERT INTO snapshots (revision, scope_digest, state_digest, state_bytes) VALUES (?1, ?2, ?3, ?4)",
            params![
                expected_next_sql,
                blob(scope_digest),
                blob(bundle.snapshot.state_digest),
                bundle.snapshot.state_bytes.clone(),
            ],
        )?;
        tx.execute(
            "INSERT INTO graph_commits (scope_digest, revision, base_graph_digest, graph_digest, formula_digest, delta_bytes, replay_state_bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                blob(scope_digest),
                expected_next_sql,
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
                expected_next_sql,
                blob(bundle.context.context_digest),
                bundle.context.canonical_state_bytes.clone(),
            ],
        )?;
        if let Some((upgrade, (upgrade_base_revision_sql, upgrade_next_revision_sql))) =
            legacy_upgrade.zip(legacy_upgrade_revisions)
        {
            tx.execute(
                "INSERT INTO legacy_semantic_formula_upgrades (scope_digest, from_formula_digest, to_formula_digest, base_revision, next_revision, event_digest, receipt_digest, source_state_digest, target_state_before, source_graph_digest, prior_chain_digest, migration_id, upgrade_bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    blob(upgrade.scope_digest),
                    blob(upgrade.from_formula_digest),
                    blob(upgrade.to_formula_digest),
                    upgrade_base_revision_sql,
                    upgrade_next_revision_sql,
                    blob(upgrade.event_digest),
                    blob(upgrade.receipt_digest),
                    blob(upgrade.source_state_digest),
                    blob(upgrade.target_state_before),
                    blob(upgrade.source_graph_digest),
                    blob(upgrade.prior_chain_digest),
                    blob(upgrade.migration_id),
                    upgrade.canonical_bytes(),
                ],
            )?;
        }
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
        Ok(ContinuityCommitOutcomeV1::Inserted {
            revision: expected_next,
            row,
        })
    }

    /// Read one immutable graph projection at an exact logical revision.  The
    /// caller must still close its replay bytes against the corresponding
    /// snapshot; this method exists so an auditor never substitutes a newer
    /// graph while attesting historic semantic authority.
    pub fn read_graph_commit_at_revision_v1(
        &self,
        scope_digest: &Digest,
        revision: u64,
    ) -> Result<Option<GraphCommitV1>, StoreError> {
        let conn = self.connection()?;
        let revision_sql = revision_to_sqlite(revision)?;
        let stored: Option<StoredGraphColumns> = conn
            .query_row(
                "SELECT base_graph_digest, graph_digest, formula_digest, delta_bytes, replay_state_bytes FROM graph_commits WHERE scope_digest = ?1 AND revision = ?2",
                params![blob(*scope_digest), revision_sql],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        stored
            .map(
                |(
                    base_graph_digest,
                    graph_digest_value,
                    formula_digest,
                    delta_bytes,
                    replay_state_bytes,
                )| {
                    Ok(GraphCommitV1 {
                        base_graph_digest: digest_from_blob(
                            &base_graph_digest,
                            "stored_graph_base_digest",
                        )?,
                        graph_digest: digest_from_blob(&graph_digest_value, "stored_graph_digest")?,
                        formula_digest: digest_from_blob(&formula_digest, "stored_graph_formula")?,
                        delta_bytes,
                        replay_state_bytes,
                    })
                },
            )
            .transpose()
    }

    /// Read one immutable context projection at an exact logical revision and
    /// independently close its canonical state and relation HMAC.
    pub fn read_context_commit_at_revision_v1(
        &self,
        scope_digest: &Digest,
        relation_scope_token: &[u8; 16],
        revision: u64,
    ) -> Result<Option<ContextCommitV1>, StoreError> {
        let conn = self.connection()?;
        let revision_sql = revision_to_sqlite(revision)?;
        let stored: Option<StoredContextDuplicateColumns> = conn
            .query_row(
                "SELECT relation_hmac, context_digest, canonical_state_bytes, relation_scope_token FROM context_commits WHERE scope_digest = ?1 AND relation_scope_token = ?2 AND revision = ?3",
                params![blob(*scope_digest), blob(*relation_scope_token), revision_sql],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((relation_hmac, context_digest, canonical_state_bytes, stored_relation_token)) =
            stored
        else {
            return Ok(None);
        };
        let context = ContextCommitV1 {
            relation_scope_token: token_from_blob(
                &stored_relation_token,
                "stored_context_relation_scope_token",
            )?,
            relation_hmac: digest_from_blob(&relation_hmac, "stored_context_hmac")?,
            source_continuum_revision: revision,
            context_digest: digest_from_blob(&context_digest, "stored_context_digest")?,
            canonical_state_bytes,
        };
        Self::context_closes(&context)?;
        Ok(Some(context))
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
            revision: revision_from_sqlite(revision)?,
            context_digest,
            canonical_state_bytes,
        }))
    }

    /// Read the one immutable receipt that records an AESEM2-to-Phase-0
    /// semantic transition.  The stored primary key and canonical bytes are
    /// rechecked before the receipt is returned to an internal auditor.
    pub fn read_legacy_semantic_formula_upgrade_v1(
        &self,
        scope_digest: &Digest,
        from_formula_digest: &Digest,
        to_formula_digest: &Digest,
    ) -> Result<Option<LegacySemanticFormulaUpgradeReceiptV1>, StoreError> {
        let conn = self.connection()?;
        let bytes: Option<Vec<u8>> = conn
            .query_row(
                "SELECT upgrade_bytes FROM legacy_semantic_formula_upgrades WHERE scope_digest = ?1 AND from_formula_digest = ?2 AND to_formula_digest = ?3",
                params![
                    blob(*scope_digest),
                    blob(*from_formula_digest),
                    blob(*to_formula_digest),
                ],
                |row| row.get(0),
            )
            .optional()?;
        let Some(bytes) = bytes else {
            return Ok(None);
        };
        let receipt = LegacySemanticFormulaUpgradeReceiptV1::decode(&bytes)?;
        if receipt.scope_digest != *scope_digest
            || receipt.from_formula_digest != *from_formula_digest
            || receipt.to_formula_digest != *to_formula_digest
            || receipt.canonical_bytes() != bytes
        {
            return Err(StoreError::ContinuityFence("legacy_upgrade_stored_receipt"));
        }
        Ok(Some(receipt))
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

        let current_sql: i64 = tx.query_row(
            "SELECT COALESCE(MAX(logical_revision), 0) FROM journal WHERE scope_digest = ?1",
            params![blob(envelope.receipt.scope_digest)],
            |row| row.get::<_, i64>(0),
        )?;
        let current = revision_from_sqlite(current_sql)?;
        if envelope.receipt.base_revision != current {
            return Err(StoreError::StaleRevision {
                expected: envelope.receipt.base_revision,
                actual: current,
            });
        }
        let (revision, revision_sql) = next_sqlite_revision(current)?;
        if envelope.receipt.next_revision != revision {
            return Err(StoreError::StaleRevision {
                expected: envelope.receipt.next_revision,
                actual: revision,
            });
        }
        let base_revision_sql = revision_to_sqlite(envelope.receipt.base_revision)?;

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
            return Err(StoreError::DuplicateEvent(revision_from_sqlite(revision)?));
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
                revision_sql,
                blob(envelope.receipt.scope_digest),
                base_revision_sql,
                envelope.event_kind.clone(),
                envelope.event_bytes.clone(),
                blob(event_digest),
                receipt_bytes.clone(),
                blob(chain_digest),
                now_ms() as i64,
            ],
        )?;
        tx.execute(
            "INSERT INTO applied_events (scope_digest, event_digest, revision) VALUES (?1, ?2, ?3)",
            params![
                blob(envelope.receipt.scope_digest),
                blob(event_digest),
                revision_sql,
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

    // ------------------------------------------------------------ snapshots

    pub fn write_snapshot(
        &mut self,
        scope_digest: &Digest,
        revision: u64,
        state_digest: &Digest,
        state_bytes: &[u8],
    ) -> Result<(), StoreError> {
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        let revision_sql = revision_to_sqlite(revision)?;
        conn.execute(
            "INSERT OR REPLACE INTO snapshots (revision, scope_digest, state_digest, state_bytes) VALUES (?1, ?2, ?3, ?4)",
            params![
                revision_sql,
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
        let revision_sql = revision_to_sqlite(revision)?;
        conn.query_row(
            "SELECT state_digest, state_bytes FROM snapshots WHERE scope_digest = ?1 AND revision = ?2",
            params![blob(*scope_digest), revision_sql],
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

    struct ContinuityBundleTestInput<'a> {
        scope: &'a ScopeRef,
        formula_digest: Digest,
        revision: u64,
        marker: u8,
        state_before: Digest,
        base_graph_digest: Digest,
        chain_seed: Digest,
        delta_bytes: Vec<u8>,
    }

    fn continuity_bundle_for_test(
        input: ContinuityBundleTestInput<'_>,
    ) -> ContinuityCommitBundleV1 {
        let ContinuityBundleTestInput {
            scope,
            formula_digest,
            revision,
            marker,
            state_before,
            base_graph_digest,
            chain_seed,
            delta_bytes,
        } = input;
        let event = CanonicalEvent::TimeAdvance(ae_contracts::TimeAdvance {
            event_id: [marker; 16],
            scope: scope.clone(),
            elapsed_ms: u64::from(marker),
        });
        let event_bytes = wire::encode_event(&event);
        let event_digest = wire::event_digest(&event);
        let state_after = [marker.wrapping_add(1); 32];
        let graph_after = [marker.wrapping_add(2); 32];
        let canonical_context_state = vec![0xc0, marker, 0x01];
        let relation_scope_token = scope.relation_token.unwrap_or(scope.session_token);
        let receipt = TransitionReceipt {
            schema_version: 1,
            formula_digest,
            scope_digest: wire::persona_scope_digest(
                &scope.bot_token,
                &scope.persona_token,
                scope.relation_token.as_ref(),
            ),
            event_digest,
            authority_digest: [marker.wrapping_add(3); 32],
            base_revision: revision.checked_sub(1).expect("test revision is nonzero"),
            next_revision: revision,
            state_before,
            state_after,
            graph_after,
            action_contract: None,
            active_nodes: 1,
            active_edges: 0,
            residuals: ae_contracts::InvariantResiduals::default(),
            status: CommitStatus::Committed,
        };
        ContinuityCommitBundleV1 {
            envelope: CommitEnvelope {
                event_kind: wire::event_kind_name(&event).to_owned(),
                event_bytes,
                receipt: receipt.clone(),
                chain_seed,
                delta_bytes: delta_bytes.clone(),
            },
            snapshot: SnapshotCommitV1 {
                state_digest: state_after,
                state_bytes: vec![0xb0, marker],
            },
            graph: GraphCommitV1 {
                base_graph_digest,
                graph_digest: graph_after,
                formula_digest,
                delta_bytes,
                replay_state_bytes: vec![0xe0, marker],
            },
            context: ContextCommitV1 {
                relation_scope_token,
                relation_hmac: [marker.wrapping_add(4); 32],
                source_continuum_revision: receipt.next_revision,
                context_digest: continuity_context_digest(&canonical_context_state),
                canonical_state_bytes: canonical_context_state,
            },
        }
    }

    fn continuity_row_counts(store: &Store, scope_digest: Digest) -> [i64; 6] {
        let conn = store.conn.as_ref().expect("test store remains open");
        let queries = [
            "SELECT COUNT(*) FROM journal WHERE scope_digest = ?1",
            "SELECT COUNT(*) FROM applied_events WHERE scope_digest = ?1",
            "SELECT COUNT(*) FROM snapshots WHERE scope_digest = ?1",
            "SELECT COUNT(*) FROM graph_commits WHERE scope_digest = ?1",
            "SELECT COUNT(*) FROM context_commits WHERE scope_digest = ?1",
            "SELECT COUNT(*) FROM legacy_semantic_formula_upgrades WHERE scope_digest = ?1",
        ];
        let mut counts = [0_i64; 6];
        for (index, query) in queries.iter().enumerate() {
            counts[index] = conn
                .query_row(query, params![blob(scope_digest)], |row| row.get(0))
                .expect("count succeeds");
        }
        counts
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
        let mut store = Store {
            conn: Some(conn),
            database_path: None,
        };
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
    fn phase0_transition_rejects_receipt_bound_arbitrary_formula_without_writes() {
        let mut store = Store::open_in_memory().unwrap();
        let mut genesis = commit(61, 0, [62; 32]);
        let ClaimOutcome::Claimed { lease_epoch, .. } = store
            .claim_lease(&genesis.scope_key, Some(genesis.nonce_digest))
            .unwrap()
        else {
            panic!("expected genesis lease");
        };
        genesis.lease_epoch = lease_epoch;
        store.commit_genesis(&genesis).unwrap();

        let root_scope = wire::persona_scope_digest(
            &genesis.source.scope.bot_token,
            &genesis.source.scope.persona_token,
            None,
        );
        let lane_binding = wire::domain_hash(
            SEMANTIC_LANE_NAMESPACE_DOMAIN_V1,
            &[
                &root_scope,
                &genesis.incarnation_id,
                &genesis.formula_digest,
            ],
        );
        let mut relation_token = [0u8; 16];
        relation_token.copy_from_slice(&lane_binding[..16]);
        let mut session_token = [0u8; 16];
        session_token.copy_from_slice(&lane_binding[16..]);
        let semantic_scope = ScopeRef {
            bot_token: genesis.source.scope.bot_token,
            persona_token: genesis.source.scope.persona_token,
            relation_token: Some(relation_token),
            session_token,
        };
        let event = CanonicalEvent::UserStimulus(ae_contracts::UserStimulus {
            event_id: [63; 16],
            scope: semantic_scope.clone(),
            causal: ae_contracts::CausalRef {
                turn_id: [64; 16],
                action_id: None,
                delivery_id: None,
                claim_id: None,
                base_revision: 0,
            },
            observed_at_ms: 1_700_000_000_300,
            evidence: ae_contracts::SemanticEstimate {
                schema_version: 1,
                dimensions: ae_contracts::EvidenceVector::default(),
                estimator_confidence: Fixed::ONE,
                estimator_digest: [65; 32],
            },
        });
        let scope_digest = wire::persona_scope_digest(
            &semantic_scope.bot_token,
            &semantic_scope.persona_token,
            semantic_scope.relation_token.as_ref(),
        );
        let event_digest = wire::event_digest(&event);
        let arbitrary_formula = [0xa5; 32];
        assert_ne!(arbitrary_formula, genesis.formula_digest);
        assert_ne!(
            arbitrary_formula,
            phase0_canonical_formula_digest_v1(&genesis.formula_digest)
        );
        let receipt = TransitionReceipt {
            schema_version: 1,
            formula_digest: arbitrary_formula,
            scope_digest,
            event_digest,
            authority_digest: [66; 32],
            base_revision: 0,
            next_revision: 1,
            state_before: [67; 32],
            state_after: [68; 32],
            graph_after: [69; 32],
            action_contract: None,
            active_nodes: 1,
            active_edges: 0,
            residuals: ae_contracts::InvariantResiduals::default(),
            status: CommitStatus::Committed,
        };
        let delta_bytes = Phase0FormulaTransitionV1 {
            scope_digest: receipt.scope_digest,
            event_digest: receipt.event_digest,
            receipt_digest: wire::receipt_digest(&receipt),
            base_revision: receipt.base_revision,
            next_revision: receipt.next_revision,
            base_graph_digest: genesis.graph_digest,
            from_formula_digest: genesis.formula_digest,
            to_formula_digest: arbitrary_formula,
        }
        .canonical_bytes();
        let context_bytes = vec![70, 71];
        let bundle = ContinuityCommitBundleV1 {
            envelope: CommitEnvelope {
                event_kind: wire::event_kind_name(&event).to_owned(),
                event_bytes: wire::encode_event(&event),
                receipt: receipt.clone(),
                chain_seed: genesis.initial_snapshot_digest,
                delta_bytes: delta_bytes.clone(),
            },
            snapshot: SnapshotCommitV1 {
                state_digest: receipt.state_after,
                state_bytes: vec![72, 73],
            },
            graph: GraphCommitV1 {
                base_graph_digest: genesis.graph_digest,
                graph_digest: receipt.graph_after,
                formula_digest: receipt.formula_digest,
                delta_bytes,
                replay_state_bytes: vec![74, 75],
            },
            context: ContextCommitV1 {
                relation_scope_token: relation_token,
                relation_hmac: [76; 32],
                source_continuum_revision: 1,
                context_digest: continuity_context_digest(&context_bytes),
                canonical_state_bytes: context_bytes,
            },
        };

        assert!(matches!(
            store.commit_continuity_bundle(&bundle),
            Err(StoreError::ContinuityFence("graph_current_formula"))
        ));
        let conn = store.conn.as_ref().unwrap();
        let persisted_counts = (
            conn.query_row(
                "SELECT COUNT(*) FROM journal WHERE scope_digest = ?1",
                params![blob(scope_digest)],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            conn.query_row(
                "SELECT COUNT(*) FROM applied_events WHERE scope_digest = ?1",
                params![blob(scope_digest)],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            conn.query_row(
                "SELECT COUNT(*) FROM snapshots WHERE scope_digest = ?1",
                params![blob(scope_digest)],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            conn.query_row(
                "SELECT COUNT(*) FROM graph_commits WHERE scope_digest = ?1",
                params![blob(scope_digest)],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            conn.query_row(
                "SELECT COUNT(*) FROM context_commits WHERE scope_digest = ?1",
                params![blob(scope_digest)],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        );
        assert_eq!(persisted_counts, (0, 0, 0, 0, 0));
    }

    #[test]
    fn same_formula_legacy_upgrade_delta_is_rejected_without_registry_or_write() {
        let mut store = Store::open_in_memory().unwrap();
        let scope = ScopeRef {
            bot_token: [81; 16],
            persona_token: [82; 16],
            relation_token: Some([83; 16]),
            session_token: [84; 16],
        };
        let formula = [85; 32];
        let first = continuity_bundle_for_test(ContinuityBundleTestInput {
            scope: &scope,
            formula_digest: formula,
            revision: 1,
            marker: 86,
            state_before: [0; 32],
            base_graph_digest: [0; 32],
            chain_seed: [87; 32],
            delta_bytes: vec![],
        });
        let first_commit = store.commit_continuity_bundle(&first).unwrap();
        let first_row = first_commit.row().clone();
        let mut tagged = continuity_bundle_for_test(ContinuityBundleTestInput {
            scope: &scope,
            formula_digest: formula,
            revision: 2,
            marker: 88,
            state_before: first.snapshot.state_digest,
            base_graph_digest: first.graph.graph_digest,
            chain_seed: first_row.chain_digest,
            delta_bytes: vec![],
        });
        let tagged_delta = LegacySemanticFormulaUpgradeReceiptV1::from_transition_receipt(
            &tagged.envelope.receipt,
            first.snapshot.state_digest,
            first.graph.graph_digest,
            formula,
            first_row.chain_digest,
        )
        .canonical_bytes();
        tagged.envelope.delta_bytes = tagged_delta.clone();
        tagged.graph.delta_bytes = tagged_delta;

        for _ in 0..2 {
            assert!(matches!(
                store.commit_continuity_bundle(&tagged),
                Err(StoreError::ContinuityFence("formula_transition_unexpected"))
            ));
        }

        let scope_digest = tagged.envelope.receipt.scope_digest;
        assert_eq!(store.current_revision(&scope_digest).unwrap(), 1);
        assert_eq!(
            continuity_row_counts(&store, scope_digest),
            [1, 1, 1, 1, 1, 0]
        );
        let stored_delta: Vec<u8> = store
            .conn
            .as_ref()
            .unwrap()
            .query_row(
                "SELECT delta_bytes FROM graph_commits WHERE scope_digest = ?1 AND revision = 1",
                params![blob(scope_digest)],
                |row| row.get(0),
            )
            .unwrap();
        assert!(stored_delta.is_empty());
        assert!(store
            .read_legacy_semantic_formula_upgrade_v1(&scope_digest, &formula, &formula)
            .unwrap()
            .is_none());
    }

    #[test]
    fn sqlite_revision_limit_fails_before_continuity_bundle_writes() {
        let mut store = Store::open_in_memory().unwrap();
        let scope = ScopeRef {
            bot_token: [91; 16],
            persona_token: [92; 16],
            relation_token: Some([93; 16]),
            session_token: [94; 16],
        };
        let formula = [95; 32];
        let revision = u64::try_from(i64::MAX).unwrap();
        let predecessor = continuity_bundle_for_test(ContinuityBundleTestInput {
            scope: &scope,
            formula_digest: formula,
            revision,
            marker: 96,
            state_before: [97; 32],
            base_graph_digest: [98; 32],
            chain_seed: [99; 32],
            delta_bytes: vec![],
        });
        let predecessor_chain = [100; 32];
        let scope_digest = predecessor.envelope.receipt.scope_digest;
        let revision_sql = i64::try_from(revision).unwrap();
        {
            let conn = store.conn.as_mut().unwrap();
            conn.execute(
                "INSERT INTO journal (logical_revision, scope_digest, base_revision, event_kind, event_bytes, event_digest, receipt_bytes, chain_digest, committed_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1)",
                params![
                    revision_sql,
                    blob(scope_digest),
                    i64::try_from(predecessor.envelope.receipt.base_revision).unwrap(),
                    predecessor.envelope.event_kind.clone(),
                    predecessor.envelope.event_bytes.clone(),
                    blob(predecessor.envelope.receipt.event_digest),
                    wire::encode_transition_receipt(&predecessor.envelope.receipt),
                    blob(predecessor_chain),
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO applied_events (scope_digest, event_digest, revision) VALUES (?1, ?2, ?3)",
                params![
                    blob(scope_digest),
                    blob(predecessor.envelope.receipt.event_digest),
                    revision_sql,
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO snapshots (revision, scope_digest, state_digest, state_bytes) VALUES (?1, ?2, ?3, ?4)",
                params![
                    revision_sql,
                    blob(scope_digest),
                    blob(predecessor.snapshot.state_digest),
                    predecessor.snapshot.state_bytes.clone(),
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO graph_commits (scope_digest, revision, base_graph_digest, graph_digest, formula_digest, delta_bytes, replay_state_bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    blob(scope_digest),
                    revision_sql,
                    blob(predecessor.graph.base_graph_digest),
                    blob(predecessor.graph.graph_digest),
                    blob(predecessor.graph.formula_digest),
                    predecessor.graph.delta_bytes.clone(),
                    predecessor.graph.replay_state_bytes.clone(),
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO context_commits (scope_digest, relation_scope_token, relation_hmac, revision, context_digest, canonical_state_bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    blob(scope_digest),
                    blob(predecessor.context.relation_scope_token),
                    blob(predecessor.context.relation_hmac),
                    revision_sql,
                    blob(predecessor.context.context_digest),
                    predecessor.context.canonical_state_bytes.clone(),
                ],
            )
            .unwrap();
        }
        assert_eq!(store.current_revision(&scope_digest).unwrap(), revision);
        let successor_revision = revision.checked_add(1).unwrap();
        let successor = continuity_bundle_for_test(ContinuityBundleTestInput {
            scope: &scope,
            formula_digest: formula,
            revision: successor_revision,
            marker: 101,
            state_before: predecessor.snapshot.state_digest,
            base_graph_digest: predecessor.graph.graph_digest,
            chain_seed: predecessor_chain,
            delta_bytes: vec![],
        });
        let before = continuity_row_counts(&store, scope_digest);

        assert!(matches!(
            store.commit_continuity_bundle(&successor),
            Err(StoreError::ContinuityFence("revision_range"))
        ));
        assert_eq!(continuity_row_counts(&store, scope_digest), before);
        assert_eq!(store.current_revision(&scope_digest).unwrap(), revision);
        let negative_revisions: i64 = store
            .conn
            .as_ref()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM journal WHERE scope_digest = ?1 AND logical_revision < 0",
                params![blob(scope_digest)],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(negative_revisions, 0);
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

    #[test]
    fn field_migration_backup_manifest_uses_the_provenance_bound_v2_wire() {
        let backup = FieldMigrationPreimageBackupV1 {
            migration_id: [1; 32],
            scope_digest: [2; 32],
            source_revision: 7,
            source_state_digest: [3; 32],
            source_formula_digest: [4; 32],
            source_graph_digest: [5; 32],
            incarnation_id: [6; 32],
            manifest_digest: [7; 32],
            package_identity: Store::field_migration_backup_package_identity(),
            build_identity: Store::field_migration_backup_build_identity(),
            capture_method: FieldMigrationBackupCaptureMethodV1::SqliteBackupApi,
            byte_len: 4_096,
            sha256: [8; 32],
        };

        let manifest = Store::field_migration_backup_manifest(&backup);
        assert!(manifest.starts_with(b"AE-FMP2\0"));
        let package_start = 9;
        let package_end = package_start + usize::from(manifest[8]);
        assert_eq!(
            &manifest[package_start..package_end],
            backup.package_identity.as_bytes()
        );
        let build_len_at = package_end;
        let build_start = build_len_at + 1;
        let build_end = build_start + usize::from(manifest[build_len_at]);
        assert_eq!(
            &manifest[build_start..build_end],
            backup.build_identity.as_bytes()
        );
        assert_eq!(manifest[build_end], backup.capture_method.as_u8());
    }
}
