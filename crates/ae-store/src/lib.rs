#![forbid(unsafe_code)]

//! SQLite registry: the only production state writer.
//!
//! One connection, one writer: every mutation goes through a BEGIN IMMEDIATE
//! transaction on this connection, so stale writers, duplicate events and
//! digest collisions fail closed instead of silently overwriting winners.
//! Identity-bearing data is stored as canonical binary bytes; JSON is used
//! only for debugging provenance columns.

use ae_continuum::{CommitEnvelope, JournalRow};
use ae_contracts::{
    wire, CanonicalEvent, Digest, GenesisManifest, GenesisReceipt, GenesisStatus, PersonaSourceRef,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub mod alpha3;
mod autonomy;
mod core_boundary_v9;
mod core_ingress;
mod embodiment_clock;
mod legacy_semantic_schema;
// Store alone owns semantic candidates. The module stays private in every
// build; Task 7 connects through the safe high-level Store method below.
#[allow(dead_code)]
mod semantic;
#[cfg(test)]
mod semantic_atomic_tests;
#[allow(dead_code)]
mod semantic_field_attestation;
pub use autonomy::AUTONOMY_DB_VERSION;
pub use semantic::{
    CommittedSemanticV1, SemanticAppraisalBeginStoreOutcomeV1,
    SemanticAppraisalSettlementStoreOutcomeV1, SemanticAppraisalStoreSettlementV1,
    SemanticCommitDispositionV1, SemanticCommitResultV1, SemanticOriginV1,
};
pub use semantic_field_attestation::{
    decode_canonical_aesem3_blocks, decode_canonical_semantic_snapshot_v3, CanonicalAesem3Blocks,
    CanonicalAesem3Error, DecodedCanonicalSemanticSnapshotV3, LegacySemanticFieldDomainUpgradeV1,
    LegacySemanticFormulaUpgradeReceiptV1, SemanticMigrationOutcomeV1, SemanticMigrationRequestV2,
    JOINT_MAX_LINEAR_FXP6_V1, LEGACY_FIELD_FXP6_SCALE,
};
#[cfg(feature = "migration-test-hooks")]
#[doc(hidden)]
pub use semantic_field_attestation::{
    set_semantic_migration_test_failpoint_v1, set_semantic_migration_test_hook_v1,
    SemanticMigrationTestFailpointV1, SemanticMigrationTestHookV1,
};

pub const LEASE_TTL_MS: u64 = 120_000;

pub(crate) const DIGEST_BYTES: u64 = 32;
pub(crate) const MAX_STORE_DATABASE_BYTES: u64 = 8 * 1024 * 1024 * 1024;
pub(crate) const MAX_JOURNAL_ROWS_PER_SCOPE: u64 = 65_536;
pub(crate) const MAX_JOURNAL_EVENT_KIND_BYTES: u64 = 64;
pub(crate) const MAX_JOURNAL_EVENT_BYTES: u64 = 256 * 1024;
pub(crate) const MAX_JOURNAL_RECEIPT_BYTES: u64 = 64 * 1024;
pub(crate) const MAX_JOURNAL_DELTA_BYTES: u64 = 1024 * 1024;
pub(crate) const MAX_JOURNAL_AGGREGATE_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const MAX_SNAPSHOT_STATE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SQLITE_SCHEMA_IDENTIFIER_BYTES: u64 = 255;
const APPLIED_EVENTS_ORIGIN_LOOKUP_INDEX_V1: &str = "applied_events_origin_lookup_v1";

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
    #[error("revision cannot be represented by SQLite: {revision}")]
    RevisionOutOfRange { revision: u64 },
    #[error("stored SQLite revision is negative: {revision}")]
    InvalidStoredRevision { revision: i64 },
    #[error("stored digest {field} has invalid width: expected 32 bytes, found {actual}")]
    InvalidStoredDigest { field: &'static str, actual: u64 },
    #[error("storage budget exceeded for {resource}: limit {limit}, found {actual}")]
    StorageBudgetExceeded {
        resource: &'static str,
        limit: u64,
        actual: u64,
    },
    #[error("continuity fence closed: {0}")]
    ContinuityFence(&'static str),
    #[error(
        "semantic suffix authority is unavailable: migration boundary {boundary_revision}, current revision {current_revision}"
    )]
    SemanticSuffixAuthorityUnavailable {
        boundary_revision: u64,
        current_revision: u64,
    },
    #[error("field migration preimage backup failed: {context}")]
    FieldMigrationBackup { context: &'static str },
    #[error("store is closed")]
    Closed,
    #[error("autonomy contract conflict: {0}")]
    AutonomyConflict(String),
    #[error("autonomy record not found: {0}")]
    AutonomyNotFound(&'static str),
    #[error("semantic commit is invalid: {0}")]
    SemanticInvalid(&'static str),
    #[error("semantic event identity conflicts with a persisted event")]
    SemanticIdentityConflict,
    #[error("semantic commit row not found")]
    SemanticNotFound,
    #[error("semantic transition authority is not connected")]
    SemanticTransitionAuthorityUnavailable,
    #[error("operating-system entropy is unavailable for perception authorization")]
    PerceptionEntropyUnavailable,
    #[cfg(test)]
    #[error("semantic transaction test fault: {0}")]
    SemanticTestFault(&'static str),
}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        StoreError::Sqlite(error.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct JournalRevision(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SqliteRevision(i64);

impl JournalRevision {
    pub(crate) fn new(revision: u64) -> Self {
        Self(revision)
    }

    pub(crate) fn get(self) -> u64 {
        self.0
    }

    pub(crate) fn to_sqlite(self) -> Result<SqliteRevision, StoreError> {
        SqliteRevision::try_from(self.0)
    }

    pub(crate) fn checked_next(self) -> Result<Self, StoreError> {
        let next = self
            .0
            .checked_add(1)
            .ok_or(StoreError::RevisionOutOfRange { revision: self.0 })?;
        SqliteRevision::try_from(next)?;
        Ok(Self(next))
    }
}

impl SqliteRevision {
    pub(crate) fn get(self) -> i64 {
        self.0
    }
}

impl TryFrom<i64> for JournalRevision {
    type Error = StoreError;

    fn try_from(revision: i64) -> Result<Self, Self::Error> {
        u64::try_from(revision)
            .map(Self)
            .map_err(|_| StoreError::InvalidStoredRevision { revision })
    }
}

impl TryFrom<u64> for SqliteRevision {
    type Error = StoreError;

    fn try_from(revision: u64) -> Result<Self, Self::Error> {
        i64::try_from(revision)
            .map(Self)
            .map_err(|_| StoreError::RevisionOutOfRange { revision })
    }
}

pub(crate) fn enforce_byte_budget(
    resource: &'static str,
    actual: u64,
    limit: u64,
) -> Result<(), StoreError> {
    if actual > limit {
        return Err(StoreError::StorageBudgetExceeded {
            resource,
            limit,
            actual,
        });
    }
    Ok(())
}

pub(crate) fn enforce_read_budget(
    row_resource: &'static str,
    byte_resource: &'static str,
    rows: u64,
    bytes: u64,
    max_rows: u64,
    max_bytes: u64,
) -> Result<(), StoreError> {
    enforce_byte_budget(row_resource, rows, max_rows)?;
    enforce_byte_budget(byte_resource, bytes, max_bytes)
}

pub(crate) fn enforce_database_budget(actual: u64) -> Result<(), StoreError> {
    enforce_byte_budget("store.database_bytes", actual, MAX_STORE_DATABASE_BYTES)
}

pub(crate) fn sqlite_length(raw: i64, resource: &'static str) -> Result<u64, StoreError> {
    u64::try_from(raw).map_err(|_| StoreError::StorageBudgetExceeded {
        resource,
        limit: 0,
        actual: u64::MAX,
    })
}

pub(crate) fn bounded_value<T>(
    value: Option<T>,
    raw_len: i64,
    limit: u64,
    resource: &'static str,
) -> Result<T, StoreError> {
    let actual = sqlite_length(raw_len, resource)?;
    enforce_byte_budget(resource, actual, limit)?;
    value.ok_or(StoreError::StorageBudgetExceeded {
        resource,
        limit,
        actual,
    })
}

/// Decode a value selected through a SQL pre-allocation type/length gate.
/// A negative length is the query's sentinel for a wrong SQLite storage class;
/// SQLite's `length()` cannot produce a negative value for a valid TEXT/BLOB.
pub(crate) fn bounded_typed_value<T>(
    value: Option<T>,
    raw_len: i64,
    limit: u64,
    resource: &'static str,
    type_fence: &'static str,
) -> Result<T, StoreError> {
    if raw_len < 0 {
        return Err(StoreError::ContinuityFence(type_fence));
    }
    bounded_value(value, raw_len, limit, resource)
}

pub(crate) fn stored_digest(
    value: Option<Vec<u8>>,
    raw_len: i64,
    field: &'static str,
) -> Result<Digest, StoreError> {
    let actual = sqlite_length(raw_len, field)?;
    if actual != DIGEST_BYTES {
        return Err(StoreError::InvalidStoredDigest { field, actual });
    }
    value
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(StoreError::InvalidStoredDigest { field, actual })
}

pub(crate) fn stored_typed_digest(
    value: Option<Vec<u8>>,
    raw_len: i64,
    field: &'static str,
    type_fence: &'static str,
) -> Result<Digest, StoreError> {
    if raw_len < 0 {
        return Err(StoreError::ContinuityFence(type_fence));
    }
    stored_digest(value, raw_len, field)
}

pub(crate) fn connection_database_bytes(conn: &Connection) -> Result<u64, StoreError> {
    let page_count: i64 = conn.query_row("PRAGMA page_count", [], |row| row.get(0))?;
    let page_size: i64 = conn.query_row("PRAGMA page_size", [], |row| row.get(0))?;
    let page_count = sqlite_length(page_count, "store.database_pages")?;
    let page_size = sqlite_length(page_size, "store.database_page_size")?;
    page_count
        .checked_mul(page_size)
        .ok_or(StoreError::StorageBudgetExceeded {
            resource: "store.database_bytes",
            limit: MAX_STORE_DATABASE_BYTES,
            actual: u64::MAX,
        })
}

pub(crate) fn enforce_connection_database_budget(conn: &Connection) -> Result<(), StoreError> {
    enforce_database_budget(connection_database_bytes(conn)?)
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

    // Preserve the established storage/API shape in this compatibility boundary.
    #[allow(clippy::should_implement_trait)]
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

pub(crate) struct StoredJournalColumns {
    pub(crate) revision: i64,
    pub(crate) base_revision: i64,
    pub(crate) event_kind: Option<String>,
    pub(crate) event_kind_len: i64,
    pub(crate) event_bytes: Option<Vec<u8>>,
    pub(crate) event_bytes_len: i64,
    pub(crate) event_digest: Option<Vec<u8>>,
    pub(crate) event_digest_len: i64,
    pub(crate) receipt_bytes: Option<Vec<u8>>,
    pub(crate) receipt_bytes_len: i64,
    pub(crate) delta_bytes: Option<Vec<u8>>,
    pub(crate) delta_bytes_len: i64,
    pub(crate) chain_digest: Option<Vec<u8>>,
    pub(crate) chain_digest_len: i64,
}

pub(crate) struct PreparedJournalCommit {
    pub(crate) revision: JournalRevision,
    pub(crate) revision_sql: SqliteRevision,
    pub(crate) base_revision_sql: SqliteRevision,
    pub(crate) event_digest: Digest,
    pub(crate) receipt_bytes: Vec<u8>,
    pub(crate) chain_digest: Digest,
    pub(crate) committed_at_ms: i64,
}

/// Selects the only authority lane allowed to append a canonical event.
/// Keeping this crate-private prevents external callers from opting a
/// `UserStimulus` out of its semantic transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JournalCommitLane {
    // Retained historical verification data; no active executor is restored.
    #[allow(dead_code)]
    JournalOnly,
    PairedSemantic,
}

pub(crate) fn decode_stored_journal_row(
    scope_digest: &Digest,
    stored: StoredJournalColumns,
) -> Result<JournalRow, StoreError> {
    let revision = JournalRevision::try_from(stored.revision)?.get();
    let base_revision = JournalRevision::try_from(stored.base_revision)?.get();
    let event_kind = bounded_typed_value(
        stored.event_kind,
        stored.event_kind_len,
        MAX_JOURNAL_EVENT_KIND_BYTES,
        "journal.event_kind",
        "journal_event_kind_type",
    )?;
    let event_bytes = bounded_typed_value(
        stored.event_bytes,
        stored.event_bytes_len,
        MAX_JOURNAL_EVENT_BYTES,
        "journal.event_bytes",
        "journal_event_bytes_type",
    )?;
    let event_digest = stored_typed_digest(
        stored.event_digest,
        stored.event_digest_len,
        "journal.event_digest",
        "journal_event_digest_type",
    )?;
    let receipt_bytes = bounded_typed_value(
        stored.receipt_bytes,
        stored.receipt_bytes_len,
        MAX_JOURNAL_RECEIPT_BYTES,
        "journal.receipt_bytes",
        "journal_receipt_bytes_type",
    )?;
    let delta_bytes = bounded_typed_value(
        stored.delta_bytes,
        stored.delta_bytes_len,
        MAX_JOURNAL_DELTA_BYTES,
        "journal.delta_bytes",
        "journal_delta_bytes_type",
    )?;
    let chain_digest = stored_typed_digest(
        stored.chain_digest,
        stored.chain_digest_len,
        "journal.chain_digest",
        "journal_chain_digest_type",
    )?;
    Ok(JournalRow {
        revision,
        scope_digest: *scope_digest,
        base_revision,
        event_kind,
        event_bytes,
        event_digest,
        receipt_bytes,
        delta_bytes,
        chain_digest,
    })
}

pub(crate) fn query_bounded_journal_row(
    conn: &Connection,
    scope_digest: &Digest,
    revision: JournalRevision,
) -> Result<Option<JournalRow>, StoreError> {
    let revision_sql = revision.to_sqlite()?.get();
    let stored = conn
        .query_row(
            "SELECT logical_revision, base_revision,
                CASE WHEN typeof(event_kind)='text' AND length(CAST(event_kind AS BLOB))<=?3 THEN event_kind END,
                CASE WHEN typeof(event_kind)='text' THEN length(CAST(event_kind AS BLOB)) ELSE -1 END,
                CASE WHEN typeof(event_bytes)='blob' AND length(event_bytes)<=?4 THEN event_bytes END,
                CASE WHEN typeof(event_bytes)='blob' THEN length(event_bytes) ELSE -1 END,
                CASE WHEN typeof(event_digest)='blob' AND length(event_digest)=32 THEN event_digest END,
                CASE WHEN typeof(event_digest)='blob' THEN length(event_digest) ELSE -1 END,
                CASE WHEN typeof(receipt_bytes)='blob' AND length(receipt_bytes)<=?5 THEN receipt_bytes END,
                CASE WHEN typeof(receipt_bytes)='blob' THEN length(receipt_bytes) ELSE -1 END,
                CASE WHEN typeof(delta_bytes)='blob' AND length(delta_bytes)<=?6 THEN delta_bytes END,
                CASE WHEN typeof(delta_bytes)='blob' THEN length(delta_bytes) ELSE -1 END,
                CASE WHEN typeof(chain_digest)='blob' AND length(chain_digest)=32 THEN chain_digest END,
                CASE WHEN typeof(chain_digest)='blob' THEN length(chain_digest) ELSE -1 END
             FROM journal WHERE scope_digest = ?1 AND logical_revision = ?2",
            params![
                blob(*scope_digest),
                revision_sql,
                MAX_JOURNAL_EVENT_KIND_BYTES,
                MAX_JOURNAL_EVENT_BYTES,
                MAX_JOURNAL_RECEIPT_BYTES,
                MAX_JOURNAL_DELTA_BYTES,
            ],
            |row| {
                Ok(StoredJournalColumns {
                    revision: row.get(0)?,
                    base_revision: row.get(1)?,
                    event_kind: row.get(2)?,
                    event_kind_len: row.get(3)?,
                    event_bytes: row.get(4)?,
                    event_bytes_len: row.get(5)?,
                    event_digest: row.get(6)?,
                    event_digest_len: row.get(7)?,
                    receipt_bytes: row.get(8)?,
                    receipt_bytes_len: row.get(9)?,
                    delta_bytes: row.get(10)?,
                    delta_bytes_len: row.get(11)?,
                    chain_digest: row.get(12)?,
                    chain_digest_len: row.get(13)?,
                })
            },
        )
        .optional()?;
    stored
        .map(|row| decode_stored_journal_row(scope_digest, row))
        .transpose()
}

// Preserve the established storage/API shape in this compatibility boundary.
#[allow(clippy::type_complexity)]
pub(crate) fn query_bounded_snapshot_row(
    conn: &Connection,
    scope_digest: &Digest,
    revision: JournalRevision,
) -> Result<Option<SnapshotRow>, StoreError> {
    let revision_sql = revision.to_sqlite()?.get();
    let stored: Option<(Option<Vec<u8>>, i64, Option<Vec<u8>>, i64)> = conn
        .query_row(
            "SELECT
                CASE WHEN typeof(state_digest)='blob' AND length(state_digest)=32 THEN state_digest END,
                CASE WHEN typeof(state_digest)='blob' THEN length(state_digest) ELSE -1 END,
                CASE WHEN typeof(state_bytes)='blob' AND length(state_bytes)<=?3 THEN state_bytes END,
                CASE WHEN typeof(state_bytes)='blob' THEN length(state_bytes) ELSE -1 END
             FROM snapshots WHERE scope_digest = ?1 AND revision = ?2",
            params![blob(*scope_digest), revision_sql, MAX_SNAPSHOT_STATE_BYTES],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    stored
        .map(|(digest, digest_len, state_bytes, state_len)| {
            Ok(SnapshotRow {
                revision: revision.get(),
                scope_digest: *scope_digest,
                state_digest: stored_typed_digest(
                    digest,
                    digest_len,
                    "snapshot.state_digest",
                    "snapshot_state_digest_type",
                )?,
                state_bytes: bounded_typed_value(
                    state_bytes,
                    state_len,
                    MAX_SNAPSHOT_STATE_BYTES,
                    "snapshot.state_bytes",
                    "snapshot_state_bytes_type",
                )?,
            })
        })
        .transpose()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoreDatabaseIdentity {
    canonical_path: PathBuf,
    platform_identity: u64,
}

pub(crate) fn store_database_identity(path: &Path) -> Result<StoreDatabaseIdentity, StoreError> {
    let canonical_path = std::fs::canonicalize(path).map_err(|source| StoreError::Io {
        context: "canonicalizing store database path",
        source,
    })?;
    let metadata = std::fs::symlink_metadata(&canonical_path).map_err(|source| StoreError::Io {
        context: "checking store database identity",
        source,
    })?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(StoreError::ContinuityFence("store_database_identity"));
    }
    #[cfg(windows)]
    let platform_identity = {
        use std::os::windows::fs::MetadataExt;
        metadata.creation_time()
    };
    #[cfg(unix)]
    let platform_identity = {
        use std::os::unix::fs::MetadataExt;
        metadata.ino()
    };
    #[cfg(not(any(windows, unix)))]
    let platform_identity = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_nanos() as u64)
        .unwrap_or(0);
    if platform_identity == 0 {
        return Err(StoreError::ContinuityFence("store_database_identity"));
    }
    Ok(StoreDatabaseIdentity {
        canonical_path,
        platform_identity,
    })
}

pub struct Store {
    conn: Option<Connection>,
    observe_witness_cache: RefCell<autonomy::ObserveWitnessCache>,
    database_identity: Option<StoreDatabaseIdentity>,
}

fn blob<const N: usize>(value: [u8; N]) -> Vec<u8> {
    value.to_vec()
}

impl Store {
    /// Explicit fixture seam: constructs the historical schema directly and
    /// never installs or removes a v9 fence. Absent from production builds.
    #[cfg(feature = "migration-test-hooks")]
    pub fn open_legacy_v8_fixture(path: &Path) -> Result<Self, StoreError> {
        let mut conn = Connection::open(path)?;
        if core_boundary_v9::preflight(&conn)? == core_boundary_v9::OpenRoute::V9 {
            return Err(StoreError::ContinuityFence("V9_FIXTURE_DOWNGRADE_DENIED"));
        }
        Self::migrate(&mut conn)?;
        Ok(Self {
            conn: Some(conn),
            observe_witness_cache: RefCell::new(autonomy::ObserveWitnessCache::default()),
            database_identity: None,
        })
    }

    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if path.is_file() {
            let byte_len = std::fs::metadata(path)
                .map_err(|source| StoreError::Io {
                    context: "reading store database metadata",
                    source,
                })?
                .len();
            enforce_database_budget(byte_len)?;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| StoreError::Io {
                context: "creating store directory",
                source,
            })?;
        }
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let route = core_boundary_v9::preflight(&conn)?;
        if route == core_boundary_v9::OpenRoute::V9 {
            if core_boundary_v9::reopen(&conn)? == core_boundary_v9::TerminalState::Applied {
                let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;
                let before = tx.total_changes();
                semantic::verify_core_semantic_history(&tx)?;
                if tx.total_changes() != before {
                    return Err(StoreError::ContinuityFence("V9_SEMANTIC_REOPEN_WROTE"));
                }
                tx.commit()?;
            }
        } else {
            conn.pragma_update(None, "journal_mode", "WAL")?;
            conn.pragma_update(None, "synchronous", "NORMAL")?;
            conn.pragma_update(None, "foreign_keys", "ON")?;
            enforce_connection_database_budget(&conn)?;
            Self::prepare_core_boundary(&mut conn, route)?;
        }
        enforce_connection_database_budget(&conn)?;
        let database_identity = store_database_identity(path)?;
        Ok(Self {
            conn: Some(conn),
            observe_witness_cache: RefCell::new(autonomy::ObserveWitnessCache::default()),
            database_identity: Some(database_identity),
        })
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        let mut conn = Connection::open_in_memory()?;
        let route = core_boundary_v9::preflight(&conn)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Self::prepare_core_boundary(&mut conn, route)?;
        enforce_connection_database_budget(&conn)?;
        Ok(Self {
            conn: Some(conn),
            observe_witness_cache: RefCell::new(autonomy::ObserveWitnessCache::default()),
            database_identity: None,
        })
    }

    fn prepare_core_boundary(
        conn: &mut Connection,
        route: core_boundary_v9::OpenRoute,
    ) -> Result<(), StoreError> {
        match route {
            core_boundary_v9::OpenRoute::Legacy(8) => {
                // Exact v8 must never enter the legacy repair/backfill prologue.
                autonomy::verify_autonomy_v8_schema_identity(conn)?;
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                semantic::migrate_schema(&tx)?;
                tx.commit()?;
            }
            core_boundary_v9::OpenRoute::Fresh | core_boundary_v9::OpenRoute::Legacy(0..=7) => {
                Self::migrate(conn)?
            }
            _ => return Err(StoreError::ContinuityFence("V9_INVALID_UPGRADE_ROUTE")),
        }
        if core_boundary_v9::upgrade(conn)? == core_boundary_v9::TerminalState::FailedClosed {
            conn.pragma_update(None, "query_only", true)?;
        }
        Ok(())
    }

    // Retained historical verification data; no active executor is restored.
    #[allow(dead_code)]
    fn enforce_core_boundary_v9_retired(
        &self,
        operation: core_boundary_v9::RetiredOperationTagV1,
    ) -> Result<(), StoreError> {
        core_boundary_v9::enforce_retired(self.conn.as_ref().ok_or(StoreError::Closed)?, operation)
    }

    fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        legacy_semantic_schema::prepare_empty_legacy_upgrade_table(&tx)?;
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
                delta_bytes BLOB NOT NULL DEFAULT X'',
                chain_digest BLOB NOT NULL,
                committed_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS applied_events (
                scope_digest BLOB NOT NULL,
                event_digest BLOB NOT NULL,
                revision INTEGER NOT NULL,
                PRIMARY KEY (scope_digest, event_digest)
            );
            CREATE INDEX IF NOT EXISTS applied_events_origin_lookup_v1
                ON applied_events(event_digest, scope_digest);
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
                base_graph_digest BLOB NOT NULL CHECK(length(base_graph_digest) = 32),
                graph_digest BLOB NOT NULL CHECK(length(graph_digest) = 32),
                formula_digest BLOB NOT NULL CHECK(length(formula_digest) = 32),
                delta_bytes BLOB NOT NULL,
                replay_state_bytes BLOB NOT NULL,
                PRIMARY KEY (scope_digest, revision)
            );
            CREATE TABLE IF NOT EXISTS context_commits (
                scope_digest BLOB NOT NULL,
                relation_scope_token BLOB NOT NULL CHECK(length(relation_scope_token) = 16),
                relation_hmac BLOB NOT NULL CHECK(length(relation_hmac) = 32),
                revision INTEGER NOT NULL,
                context_digest BLOB NOT NULL CHECK(length(context_digest) = 32),
                canonical_state_bytes BLOB NOT NULL,
                PRIMARY KEY (scope_digest, relation_scope_token, revision)
            );
            CREATE TABLE IF NOT EXISTS legacy_semantic_formula_upgrades (
                migration_id BLOB PRIMARY KEY,
                scope_digest BLOB NOT NULL,
                from_formula_digest BLOB NOT NULL,
                to_formula_digest BLOB NOT NULL,
                base_revision INTEGER NOT NULL,
                next_revision INTEGER NOT NULL,
                event_digest BLOB NOT NULL,
                receipt_digest BLOB NOT NULL,
                source_state_digest BLOB NOT NULL,
                target_state_before BLOB NOT NULL,
                source_graph_digest BLOB NOT NULL,
                prior_chain_digest BLOB NOT NULL,
                upgrade_bytes BLOB NOT NULL,
                backup_digest BLOB NOT NULL,
                UNIQUE(scope_digest, from_formula_digest, to_formula_digest)
            );
            CREATE TABLE IF NOT EXISTS field_migration_preimage_backups (
                migration_id BLOB PRIMARY KEY,
                scope_digest BLOB NOT NULL,
                source_revision INTEGER NOT NULL,
                source_state_digest BLOB NOT NULL,
                source_formula_digest BLOB NOT NULL,
                source_graph_digest BLOB NOT NULL,
                incarnation_id BLOB NOT NULL,
                manifest_digest BLOB NOT NULL,
                byte_len INTEGER NOT NULL,
                sha256 BLOB NOT NULL,
                manifest_bytes BLOB NOT NULL,
                creator_package_identity TEXT NOT NULL,
                creator_build_identity TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS semantic_migration_authority_v1 (
                migration_id BLOB PRIMARY KEY CHECK(length(migration_id) = 32),
                commitment_digest BLOB NOT NULL CHECK(length(commitment_digest) = 32),
                telemetry_digest BLOB NOT NULL CHECK(length(telemetry_digest) = 32),
                snapshot_wire_digest BLOB NOT NULL CHECK(length(snapshot_wire_digest) = 32),
                authority_receipt_digest BLOB NOT NULL CHECK(length(authority_receipt_digest) = 32),
                FOREIGN KEY (migration_id) REFERENCES legacy_semantic_formula_upgrades(migration_id)
            );
            "#,
        )?;
        Self::require_applied_events_origin_lookup_index_v1(&tx)?;
        if !Self::journal_has_logical_revision(&tx)? {
            tx.execute(
                "ALTER TABLE journal ADD COLUMN logical_revision INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
            Self::backfill_scope_local_revisions(&tx)?;
        }
        if !Self::journal_has_column(&tx, "delta_bytes")? {
            tx.execute(
                "ALTER TABLE journal ADD COLUMN delta_bytes BLOB NOT NULL DEFAULT X''",
                [],
            )?;
        }
        semantic_field_attestation::migrate_field_backup_creator_provenance(&tx)?;
        tx.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS journal_scope_logical_revision ON journal (scope_digest, logical_revision);
             CREATE UNIQUE INDEX IF NOT EXISTS legacy_semantic_upgrade_journal_identity
                 ON legacy_semantic_formula_upgrades(scope_digest, next_revision, event_digest);",
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO meta (key, value) VALUES ('schema_version', X'01')",
            [],
        )?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations(
                version INTEGER PRIMARY KEY,
                digest BLOB NOT NULL,
                completed_at_ms INTEGER NOT NULL
            );",
        )?;
        // A semantic-upgrade row is trusted by autonomy migration only after
        // the complete Store-owned event/receipt/history/snapshot/backup
        // closure has been re-derived. This also authenticates legacy rows
        // created before the relational journal-identity index existed.
        semantic_field_attestation::verify_committed_semantic_migration_rows(&tx)?;
        let autonomy_version = tx.query_row(
            "SELECT COALESCE(MAX(version),0) FROM schema_migrations",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        if autonomy_version < 0 || autonomy_version > u32::MAX as i64 {
            return Err(StoreError::AutonomyConflict(
                "invalid autonomy schema version".into(),
            ));
        }
        autonomy::migrate_autonomy(&tx, autonomy_version as u32)?;
        semantic::migrate_schema(&tx)?;
        tx.commit()?;
        Ok(())
    }

    fn require_applied_events_origin_lookup_index_v1(
        tx: &Transaction<'_>,
    ) -> Result<(), StoreError> {
        let identity: Option<(i64, String, i64)> = tx
            .query_row(
                "SELECT \"unique\",origin,partial
                 FROM pragma_index_list('applied_events') WHERE name=?1",
                params![APPLIED_EVENTS_ORIGIN_LOOKUP_INDEX_V1],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if identity
            .as_ref()
            .map(|(unique, origin, partial)| (*unique, origin.as_str(), *partial))
            != Some((0, "c", 0))
        {
            return Err(StoreError::ContinuityFence("applied_events_origin_index"));
        }

        let mut statement = tx.prepare(
            "SELECT seqno,cid,
                    CASE WHEN typeof(name)='text' AND length(CAST(name AS BLOB))<=?1
                         THEN name END,
                    CASE WHEN typeof(name)='text' THEN length(CAST(name AS BLOB)) ELSE -1 END,
                    desc,
                    CASE WHEN typeof(coll)='text' AND length(CAST(coll AS BLOB))<=?1
                         THEN coll END,
                    CASE WHEN typeof(coll)='text' THEN length(CAST(coll AS BLOB)) ELSE -1 END
             FROM pragma_index_xinfo('applied_events_origin_lookup_v1')
             WHERE key=1 ORDER BY seqno",
        )?;
        let mut rows = statement.query(params![MAX_SQLITE_SCHEMA_IDENTIFIER_BYTES])?;
        for (expected_sequence, expected_column_id, expected_name) in
            [(0_i64, 1_i64, "event_digest"), (1, 0, "scope_digest")]
        {
            let row = rows
                .next()?
                .ok_or(StoreError::ContinuityFence("applied_events_origin_index"))?;
            let sequence: i64 = row.get(0)?;
            let column_id: i64 = row.get(1)?;
            let name = bounded_typed_value(
                row.get::<_, Option<String>>(2)?,
                row.get(3)?,
                MAX_SQLITE_SCHEMA_IDENTIFIER_BYTES,
                "sqlite_schema.applied_events_origin_index.column_name",
                "applied_events_origin_index",
            )?;
            let descending: i64 = row.get(4)?;
            let collation = bounded_typed_value(
                row.get::<_, Option<String>>(5)?,
                row.get(6)?,
                MAX_SQLITE_SCHEMA_IDENTIFIER_BYTES,
                "sqlite_schema.applied_events_origin_index.collation",
                "applied_events_origin_index",
            )?;
            if sequence != expected_sequence
                || column_id != expected_column_id
                || name != expected_name
                || descending != 0
                || collation != "BINARY"
            {
                return Err(StoreError::ContinuityFence("applied_events_origin_index"));
            }
        }
        if rows.next()?.is_some() {
            return Err(StoreError::ContinuityFence("applied_events_origin_index"));
        }
        Ok(())
    }

    fn journal_has_logical_revision(tx: &Transaction<'_>) -> Result<bool, StoreError> {
        Self::journal_has_column(tx, "logical_revision")
    }

    fn journal_has_column(tx: &Transaction<'_>, wanted: &str) -> Result<bool, StoreError> {
        let mut statement = tx.prepare(
            "SELECT
                CASE WHEN typeof(name)='text' AND length(CAST(name AS BLOB))<=?1 THEN name END,
                CASE WHEN typeof(name)='text' THEN length(CAST(name AS BLOB)) ELSE -1 END
             FROM pragma_table_info('journal')",
        )?;
        let mut rows = statement.query(params![MAX_SQLITE_SCHEMA_IDENTIFIER_BYTES])?;
        while let Some(row) = rows.next()? {
            let column = bounded_typed_value(
                row.get::<_, Option<String>>(0)?,
                row.get(1)?,
                MAX_SQLITE_SCHEMA_IDENTIFIER_BYTES,
                "sqlite_schema.journal.column_name",
                "sqlite_schema_identifier_type",
            )?;
            if column == wanted {
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
                "SELECT revision,
                    CASE WHEN typeof(scope_digest)='blob' AND length(scope_digest)=32 THEN scope_digest END,
                    CASE WHEN typeof(scope_digest)='blob' THEN length(scope_digest) ELSE -1 END
                 FROM journal ORDER BY scope_digest ASC, revision ASC",
            )?;
            let mut rows = statement.query([])?;
            let mut mappings = Vec::new();
            let mut previous_scope: Option<Digest> = None;
            let mut logical_revision = 0_i64;
            while let Some(row) = rows.next()? {
                let physical_revision: i64 = row.get(0)?;
                let scope_digest = stored_typed_digest(
                    row.get(1)?,
                    row.get(2)?,
                    "journal.scope_digest",
                    "journal_scope_digest_type",
                )?;
                if previous_scope == Some(scope_digest) {
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
        let stored: Option<(Option<Vec<u8>>, i64, i64)> = conn
            .query_row(
                "SELECT CASE WHEN length(incarnation_id)=32 THEN incarnation_id END, length(incarnation_id), revision FROM active_bindings WHERE bot_token = ?1 AND persona_token = ?2",
                params![blob(*bot_token), blob(*persona_token)],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        stored
            .map(|(incarnation, incarnation_len, revision)| {
                Ok(BindingRow {
                    bot_token: *bot_token,
                    persona_token: *persona_token,
                    incarnation_id: stored_digest(
                        incarnation,
                        incarnation_len,
                        "active_bindings.incarnation_id",
                    )?,
                    revision: JournalRevision::try_from(revision)?.get(),
                })
            })
            .transpose()
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
        Ok(JournalRevision::try_from(revision)?.get())
    }

    pub fn last_chain_digest(&self, scope_digest: &Digest) -> Result<Option<Digest>, StoreError> {
        let conn = self.connection()?;
        let stored: Option<(Option<Vec<u8>>, i64)> = conn
            .query_row(
            "SELECT CASE WHEN length(chain_digest)=32 THEN chain_digest END, length(chain_digest) FROM journal WHERE scope_digest = ?1 ORDER BY logical_revision DESC LIMIT 1",
                params![blob(*scope_digest)],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        stored
            .map(|(bytes, len)| stored_digest(bytes, len, "journal.chain_digest"))
            .transpose()
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
        self.read_journal_row(scope_digest, JournalRevision::try_from(revision)?.get())
    }

    fn read_journal_row(
        &self,
        scope_digest: &Digest,
        revision: u64,
    ) -> Result<Option<JournalRow>, StoreError> {
        query_bounded_journal_row(
            self.connection()?,
            scope_digest,
            JournalRevision::new(revision),
        )
    }

    pub fn read_journal(&self, scope_digest: &Digest) -> Result<Vec<JournalRow>, StoreError> {
        let conn = self.connection()?;
        let (raw_rows, raw_bytes): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(
                length(CAST(event_kind AS BLOB)) + length(event_bytes) + length(event_digest)
                + length(receipt_bytes) + length(delta_bytes) + length(chain_digest)
             ), 0) FROM journal WHERE scope_digest = ?1",
            params![blob(*scope_digest)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        enforce_read_budget(
            "journal.scope.rows",
            "journal.scope.bytes",
            sqlite_length(raw_rows, "journal.scope.rows")?,
            sqlite_length(raw_bytes, "journal.scope.bytes")?,
            MAX_JOURNAL_ROWS_PER_SCOPE,
            MAX_JOURNAL_AGGREGATE_BYTES,
        )?;
        let mut statement = conn.prepare(
            "SELECT logical_revision, base_revision,
                CASE WHEN length(CAST(event_kind AS BLOB))<=?2 THEN event_kind END, length(CAST(event_kind AS BLOB)),
                CASE WHEN length(event_bytes)<=?3 THEN event_bytes END, length(event_bytes),
                CASE WHEN length(event_digest)=32 THEN event_digest END, length(event_digest),
                CASE WHEN length(receipt_bytes)<=?4 THEN receipt_bytes END, length(receipt_bytes),
                CASE WHEN length(delta_bytes)<=?5 THEN delta_bytes END, length(delta_bytes),
                CASE WHEN length(chain_digest)=32 THEN chain_digest END, length(chain_digest)
             FROM journal WHERE scope_digest = ?1 ORDER BY logical_revision ASC",
        )?;
        let mut query = statement.query(params![
            blob(*scope_digest),
            MAX_JOURNAL_EVENT_KIND_BYTES,
            MAX_JOURNAL_EVENT_BYTES,
            MAX_JOURNAL_RECEIPT_BYTES,
            MAX_JOURNAL_DELTA_BYTES,
        ])?;
        let mut decoded = Vec::with_capacity(usize::try_from(raw_rows).unwrap_or(0));
        while let Some(row) = query.next()? {
            decoded.push(decode_stored_journal_row(
                scope_digest,
                StoredJournalColumns {
                    revision: row.get(0)?,
                    base_revision: row.get(1)?,
                    event_kind: row.get(2)?,
                    event_kind_len: row.get(3)?,
                    event_bytes: row.get(4)?,
                    event_bytes_len: row.get(5)?,
                    event_digest: row.get(6)?,
                    event_digest_len: row.get(7)?,
                    receipt_bytes: row.get(8)?,
                    receipt_bytes_len: row.get(9)?,
                    delta_bytes: row.get(10)?,
                    delta_bytes_len: row.get(11)?,
                    chain_digest: row.get(12)?,
                    chain_digest_len: row.get(13)?,
                },
            )?);
        }
        Ok(decoded)
    }

    pub(crate) fn prepare_journal_commit_tx(
        tx: &Transaction<'_>,
        envelope: &CommitEnvelope,
        lane: JournalCommitLane,
    ) -> Result<PreparedJournalCommit, StoreError> {
        let requested_base = JournalRevision::new(envelope.receipt.base_revision);
        let requested_next = JournalRevision::new(envelope.receipt.next_revision);
        let requested_base_sql = requested_base.to_sqlite()?;
        requested_next.to_sqlite()?;
        enforce_byte_budget(
            "journal.event_kind",
            u64::try_from(envelope.event_kind.len()).unwrap_or(u64::MAX),
            MAX_JOURNAL_EVENT_KIND_BYTES,
        )?;
        enforce_byte_budget(
            "journal.event_bytes",
            u64::try_from(envelope.event_bytes.len()).unwrap_or(u64::MAX),
            MAX_JOURNAL_EVENT_BYTES,
        )?;
        enforce_byte_budget(
            "journal.delta_bytes",
            u64::try_from(envelope.delta_bytes.len()).unwrap_or(u64::MAX),
            MAX_JOURNAL_DELTA_BYTES,
        )?;

        let event = wire::decode_event(&envelope.event_bytes)
            .map_err(|error| StoreError::Sqlite(format!("event decode failed: {error}")))?;
        if wire::encode_event(&event) != envelope.event_bytes
            || wire::event_kind_name(&event) != envelope.event_kind
        {
            return Err(StoreError::ContinuityFence("journal_event_canonical"));
        }
        if matches!(event, CanonicalEvent::InteractionFactBatch(_)) {
            return Err(StoreError::AutonomyConflict(
                "alpha3 interaction facts require their dedicated authority transaction".into(),
            ));
        }
        if matches!(event, CanonicalEvent::UserStimulus(_))
            && lane != JournalCommitLane::PairedSemantic
        {
            return Err(StoreError::SemanticInvalid(
                "user_stimulus_requires_paired_semantic_commit",
            ));
        }
        if !envelope.delta_bytes.is_empty()
            || matches!(
                event,
                CanonicalEvent::TimeAdvance(_) | CanonicalEvent::SelfActionCandidate(_)
            )
        {
            return Err(StoreError::AutonomyConflict(
                "autonomy events and deltas require commit_autonomous_wake".into(),
            ));
        }
        let event_digest = wire::event_digest(&event);
        if event_digest != envelope.receipt.event_digest {
            return Err(StoreError::StaleRevision {
                expected: 0,
                actual: 0,
            });
        }
        let receipt_bytes = wire::encode_transition_receipt(&envelope.receipt);
        enforce_byte_budget(
            "journal.receipt_bytes",
            u64::try_from(receipt_bytes.len()).unwrap_or(u64::MAX),
            MAX_JOURNAL_RECEIPT_BYTES,
        )?;

        let current_sql: i64 = tx.query_row(
            "SELECT COALESCE(MAX(logical_revision), 0) FROM journal WHERE scope_digest = ?1",
            params![blob(envelope.receipt.scope_digest)],
            |row| row.get::<_, i64>(0),
        )?;
        let current = JournalRevision::try_from(current_sql)?;
        let expected_next = current.checked_next()?;
        if requested_base != current {
            return Err(StoreError::StaleRevision {
                expected: envelope.receipt.base_revision,
                actual: current.get(),
            });
        }
        if requested_next != expected_next {
            return Err(StoreError::StaleRevision {
                expected: envelope.receipt.next_revision,
                actual: expected_next.get(),
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
            return Err(StoreError::DuplicateEvent(
                JournalRevision::try_from(revision)?.get(),
            ));
        }

        let last_chain: Option<(Option<Vec<u8>>, i64)> = tx
            .query_row(
            "SELECT CASE WHEN length(chain_digest)=32 THEN chain_digest END, length(chain_digest) FROM journal WHERE scope_digest = ?1 ORDER BY logical_revision DESC LIMIT 1",
                params![blob(envelope.receipt.scope_digest)],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((bytes, len)) = last_chain {
            if stored_digest(bytes, len, "journal.chain_digest")? != envelope.chain_seed {
                return Err(StoreError::StaleRevision {
                    expected: 0,
                    actual: 0,
                });
            }
        }

        let chain_digest = if envelope.delta_bytes.is_empty() {
            ae_continuum::chain_link(&envelope.chain_seed, &envelope.event_bytes, &receipt_bytes)
        } else {
            ae_continuum::chain_link_with_delta(
                &envelope.chain_seed,
                &envelope.event_bytes,
                &receipt_bytes,
                &envelope.delta_bytes,
            )
        };
        let committed_at_ms = now_ms();
        let committed_at_ms =
            i64::try_from(committed_at_ms).map_err(|_| StoreError::RevisionOutOfRange {
                revision: committed_at_ms,
            })?;
        Ok(PreparedJournalCommit {
            revision: expected_next,
            revision_sql: expected_next.to_sqlite()?,
            base_revision_sql: requested_base_sql,
            event_digest,
            receipt_bytes,
            chain_digest,
            committed_at_ms,
        })
    }

    pub(crate) fn insert_journal_commit_tx(
        tx: &Transaction<'_>,
        envelope: &CommitEnvelope,
        prepared: &PreparedJournalCommit,
    ) -> Result<(), StoreError> {
        tx.execute(
            "INSERT INTO journal (logical_revision, scope_digest, base_revision, event_kind, event_bytes, event_digest, receipt_bytes, delta_bytes, chain_digest, committed_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                prepared.revision_sql.get(),
                blob(envelope.receipt.scope_digest),
                prepared.base_revision_sql.get(),
                envelope.event_kind.clone(),
                envelope.event_bytes.clone(),
                blob(prepared.event_digest),
                prepared.receipt_bytes.clone(),
                envelope.delta_bytes.clone(),
                blob(prepared.chain_digest),
                prepared.committed_at_ms,
            ],
        )?;
        tx.execute(
            "INSERT INTO applied_events (scope_digest, event_digest, revision) VALUES (?1, ?2, ?3)",
            params![
                blob(envelope.receipt.scope_digest),
                blob(prepared.event_digest),
                prepared.revision_sql.get(),
            ],
        )?;
        Ok(())
    }

    // Retained historical verification data; no active executor is restored.
    #[allow(dead_code)]
    pub(crate) fn journal_row_from_prepared(
        envelope: &CommitEnvelope,
        prepared: &PreparedJournalCommit,
    ) -> JournalRow {
        JournalRow {
            revision: prepared.revision.get(),
            scope_digest: envelope.receipt.scope_digest,
            base_revision: envelope.receipt.base_revision,
            event_kind: envelope.event_kind.clone(),
            event_bytes: envelope.event_bytes.clone(),
            event_digest: prepared.event_digest,
            receipt_bytes: prepared.receipt_bytes.clone(),
            delta_bytes: envelope.delta_bytes.clone(),
            chain_digest: prepared.chain_digest,
        }
    }

    // CAS commit of one journal entry. The caller supplies the chain seed
    // (genesis snapshot digest for the first entry, previous chain digest
    // afterwards); the store verifies it against its own last chain digest
    // and appends atomically. Duplicate events update zero rows and fail.

    // ------------------------------------------------------------ snapshots

    pub fn write_snapshot(
        &mut self,
        scope_digest: &Digest,
        revision: u64,
        state_digest: &Digest,
        state_bytes: &[u8],
    ) -> Result<(), StoreError> {
        let revision_sql = JournalRevision::new(revision).to_sqlite()?.get();
        enforce_byte_budget(
            "snapshot.state_bytes",
            u64::try_from(state_bytes.len()).unwrap_or(u64::MAX),
            MAX_SNAPSHOT_STATE_BYTES,
        )?;
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
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
        query_bounded_snapshot_row(conn, scope_digest, JournalRevision::new(revision))
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
        let mut store = Store {
            conn: Some(conn),
            observe_witness_cache: RefCell::new(autonomy::ObserveWitnessCache::default()),
            database_identity: None,
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
        let event = ae_contracts::CanonicalEvent::AdminAction(ae_contracts::AdminAction {
            event_id: [42; 16],
            scope: ae_contracts::ScopeRef {
                bot_token: commit.source.scope.bot_token,
                persona_token: commit.source.scope.persona_token,
                relation_token: None,
                session_token: [5; 16],
            },
            operation: "journal_test".into(),
            nonce_digest: [43; 32],
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
            event_kind: "admin_action".to_string(),
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
        let _ = std::fs::remove_dir_all(&dir);
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
        drop(store);
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
