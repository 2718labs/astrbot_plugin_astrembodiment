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
    wire, Digest, GenesisManifest, GenesisReceipt, GenesisStatus, PersonaSourceRef,
};
use ae_genesis::r7::{
    verify_authority_closure_v1, BootstrapActivationReceiptV1, CustodyDispositionReceiptV1,
    GenesisIdentityPolicyV1, IndependentSolReviewReceiptV1, KeyCeremonyReceiptV1,
    PolicyAttestationV1, ReleaseTrustRootV1, RootRegistrySnapshotV1, UserDelegationReceiptV1,
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
    #[error("stateful journal commit requires non-empty state bytes")]
    EmptyStateBytes,
    #[error("revision {revision} exceeds SQLite INTEGER range")]
    RevisionOutOfRange { revision: u64 },
    #[error("stored revision is negative: {revision}")]
    InvalidStoredRevision { revision: i64 },
    #[error("no committed genesis for this scope")]
    GenesisNotFound,
    #[error("snapshot not found")]
    SnapshotNotFound,
    #[error("store is closed")]
    Closed,
    #[error("r7 policy material is invalid: {0}")]
    R7PolicyInvalid(String),
    #[error("r7 policy sequence gap: expected {expected}, found {actual}")]
    R7PolicySequenceGap { expected: u64, actual: u64 },
    #[error("r7 policy sequence conflict")]
    R7PolicySequenceConflict,
    #[error("r7 policy sequence is stale: stored {stored}, found {actual}")]
    R7PolicySequenceStale { stored: u64, actual: u64 },
    #[error("r7 policy sequence overflow")]
    R7PolicySequenceOverflow,
    #[error("r7 policy G0 binding is not the committed incarnation")]
    R7PolicyG0BindingMismatch,
    #[error("r7 policy validation context is required")]
    R7PolicyValidationContextRequired,
    #[error("r7 policy registry predecessor is invalid")]
    R7PolicyRegistryPredecessor,
    #[error("r7 policy registry epoch is not the immediate successor")]
    R7PolicyRegistryEpochGap,
    #[error("r7 policy registry revocation set rolled back")]
    R7PolicyRevocationRollback,
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

    // Kept for alpha's public compatibility ABI; `FromStr` is also provided
    // below for generic callers.
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

impl std::str::FromStr for LeaseStatus {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        LeaseStatus::from_str(value).ok_or(())
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct R7PolicyBindingKeyV1 {
    pub bot_token: [u8; 16],
    pub persona_token: [u8; 16],
    pub committed_g0_incarnation_id: Digest,
    pub identity_scope_id: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct R7PublicPolicyBundleV1 {
    pub delegation: UserDelegationReceiptV1,
    pub ceremony: KeyCeremonyReceiptV1,
    pub root_custody: CustodyDispositionReceiptV1,
    pub policy_custody: CustodyDispositionReceiptV1,
    pub reviewer_custody: CustodyDispositionReceiptV1,
    pub policy: GenesisIdentityPolicyV1,
    pub root: ReleaseTrustRootV1,
    pub registry: RootRegistrySnapshotV1,
    pub review: IndependentSolReviewReceiptV1,
    pub attestation: PolicyAttestationV1,
    pub activation: BootstrapActivationReceiptV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct R7PolicyValidationContextV1 {
    pub native_source_identity_digest: Digest,
    pub plugin_source_identity_digest: Digest,
    pub control_evidence_set_digest: Digest,
    pub g0_binding_contract_digest: Digest,
    pub g0_only_fallback_contract_digest: Digest,
    pub committed_g0_incarnation_id: Digest,
    pub committed_g0_manifest_digest: Digest,
    pub committed_g0_seed_code_digest: Digest,
    pub committed_g0_persona_source_digest: Digest,
    pub committed_g0_genesis_receipt_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct R7PolicyBindingRowV1 {
    pub key: R7PolicyBindingKeyV1,
    pub highest_accepted_sequence: u64,
    pub policy_body_digest: Digest,
    pub policy_attestation_digest: Digest,
    pub attested_registry_epoch: u64,
    pub attested_registry_snapshot_digest: Digest,
    pub policy_bytes: Vec<u8>,
    pub root_bytes: Vec<u8>,
    pub registry_bytes: Vec<u8>,
    pub review_bytes: Vec<u8>,
    pub attestation_bytes: Vec<u8>,
    pub activation_bytes: Vec<u8>,
    pub delegation_bytes: Vec<u8>,
    pub ceremony_bytes: Vec<u8>,
    pub root_custody_bytes: Vec<u8>,
    pub policy_custody_bytes: Vec<u8>,
    pub reviewer_custody_bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum R7PolicyCommitOutcomeV1 {
    Inserted,
    Replay,
    Successor,
}

type R7EncodedBundleRowV1 = (
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
);
type R7StoredPolicyRowV1 = (
    i64,
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
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
);

/// A journal transition and the opaque semantic state it commits.
#[derive(Clone, Debug)]
pub struct StatefulCommit {
    pub journal: CommitEnvelope,
    pub state_bytes: Vec<u8>,
}

struct ValidatedJournalCommit {
    revision: u64,
    revision_sqlite: i64,
    base_revision_sqlite: i64,
    event_digest: Digest,
    receipt_bytes: Vec<u8>,
    chain_digest: Digest,
}

pub struct Store {
    conn: Option<Connection>,
}

fn blob<const N: usize>(value: [u8; N]) -> Vec<u8> {
    value.to_vec()
}

fn revision_to_sqlite(revision: u64) -> Result<i64, StoreError> {
    i64::try_from(revision).map_err(|_| StoreError::RevisionOutOfRange { revision })
}

fn revision_from_sqlite(revision: i64) -> Result<u64, StoreError> {
    u64::try_from(revision).map_err(|_| StoreError::InvalidStoredRevision { revision })
}

fn snapshot_upper_bound_to_sqlite(revision: u64) -> i64 {
    i64::try_from(revision).unwrap_or(i64::MAX)
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
            CREATE TABLE IF NOT EXISTS r7_policy_bindings_v1 (
                bot_token BLOB NOT NULL,
                persona_token BLOB NOT NULL,
                committed_g0_incarnation_id BLOB NOT NULL,
                identity_scope_id INTEGER NOT NULL,
                highest_accepted_sequence INTEGER NOT NULL,
                policy_body_digest BLOB NOT NULL,
                policy_attestation_digest BLOB NOT NULL,
                attested_registry_epoch INTEGER NOT NULL,
                attested_registry_snapshot_digest BLOB NOT NULL,
                policy_bytes BLOB NOT NULL,
                root_bytes BLOB NOT NULL,
                registry_bytes BLOB NOT NULL,
                review_bytes BLOB NOT NULL,
                attestation_bytes BLOB NOT NULL,
                activation_bytes BLOB NOT NULL,
                delegation_bytes BLOB NOT NULL,
                ceremony_bytes BLOB NOT NULL,
                root_custody_bytes BLOB NOT NULL,
                policy_custody_bytes BLOB NOT NULL,
                reviewer_custody_bytes BLOB NOT NULL,
                PRIMARY KEY (bot_token, persona_token, committed_g0_incarnation_id, identity_scope_id)
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
        for (name, ddl) in [
            (
                "delegation_bytes",
                "ALTER TABLE r7_policy_bindings_v1 ADD COLUMN delegation_bytes BLOB NOT NULL DEFAULT X''",
            ),
            (
                "ceremony_bytes",
                "ALTER TABLE r7_policy_bindings_v1 ADD COLUMN ceremony_bytes BLOB NOT NULL DEFAULT X''",
            ),
            (
                "root_custody_bytes",
                "ALTER TABLE r7_policy_bindings_v1 ADD COLUMN root_custody_bytes BLOB NOT NULL DEFAULT X''",
            ),
            (
                "policy_custody_bytes",
                "ALTER TABLE r7_policy_bindings_v1 ADD COLUMN policy_custody_bytes BLOB NOT NULL DEFAULT X''",
            ),
            (
                "reviewer_custody_bytes",
                "ALTER TABLE r7_policy_bindings_v1 ADD COLUMN reviewer_custody_bytes BLOB NOT NULL DEFAULT X''",
            ),
        ] {
            if !Self::r7_policy_column_exists(&tx, name)? {
                tx.execute(ddl, [])?;
            }
        }
        tx.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS journal_scope_logical_revision ON journal (scope_digest, logical_revision);
             CREATE INDEX IF NOT EXISTS snapshots_scope_revision ON snapshots (scope_digest, revision DESC);",
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

    fn r7_policy_column_exists(tx: &Transaction<'_>, name: &str) -> Result<bool, StoreError> {
        let mut statement = tx.prepare("PRAGMA table_info(r7_policy_bindings_v1)")?;
        let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
        for column in columns {
            if column? == name {
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
        let stored = conn
            .query_row(
                "SELECT incarnation_id, revision FROM active_bindings WHERE bot_token = ?1 AND persona_token = ?2",
                params![blob(*bot_token), blob(*persona_token)],
                |row| {
                    let bytes: Vec<u8> = row.get(0)?;
                    let mut incarnation = [0u8; 32];
                    incarnation.copy_from_slice(&bytes);
                    Ok((incarnation, row.get::<_, i64>(1)?))
                },
            )
            .optional()?;
        let Some((incarnation_id, revision)) = stored else {
            return Ok(None);
        };
        Ok(Some(BindingRow {
            bot_token: *bot_token,
            persona_token: *persona_token,
            incarnation_id,
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
        let revision_sqlite = revision_to_sqlite(revision)?;
        let stored = conn
            .query_row(
                "SELECT base_revision, event_kind, event_bytes, event_digest, receipt_bytes, chain_digest FROM journal WHERE scope_digest = ?1 AND logical_revision = ?2",
                params![blob(*scope_digest), revision_sqlite],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                ))
            },
        )
            .optional()?;
        let Some((
            base_revision,
            event_kind,
            event_bytes,
            event_digest_bytes,
            receipt_bytes,
            chain_bytes,
        )) = stored
        else {
            return Ok(None);
        };
        let mut event_digest = [0u8; 32];
        event_digest.copy_from_slice(&event_digest_bytes);
        let mut chain_digest = [0u8; 32];
        chain_digest.copy_from_slice(&chain_bytes);
        Ok(Some(JournalRow {
            revision,
            scope_digest: *scope_digest,
            base_revision: revision_from_sqlite(base_revision)?,
            event_kind,
            event_bytes,
            event_digest,
            receipt_bytes,
            chain_digest,
        }))
    }

    pub fn read_journal(&self, scope_digest: &Digest) -> Result<Vec<JournalRow>, StoreError> {
        let conn = self.connection()?;
        let mut statement = conn.prepare(
            "SELECT logical_revision, base_revision, event_kind, event_bytes, event_digest, receipt_bytes, chain_digest FROM journal WHERE scope_digest = ?1 ORDER BY logical_revision ASC",
        )?;
        let stored_rows = statement
            .query_map(params![blob(*scope_digest)], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                    row.get::<_, Vec<u8>>(6)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut rows = Vec::with_capacity(stored_rows.len());
        for (
            revision,
            base_revision,
            event_kind,
            event_bytes,
            event_digest_bytes,
            receipt_bytes,
            chain_bytes,
        ) in stored_rows
        {
            let mut event_digest = [0u8; 32];
            event_digest.copy_from_slice(&event_digest_bytes);
            let mut chain_digest = [0u8; 32];
            chain_digest.copy_from_slice(&chain_bytes);
            rows.push(JournalRow {
                revision: revision_from_sqlite(revision)?,
                scope_digest: *scope_digest,
                base_revision: revision_from_sqlite(base_revision)?,
                event_kind,
                event_bytes,
                event_digest,
                receipt_bytes,
                chain_digest,
            });
        }
        Ok(rows)
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

        let validated = Self::validate_journal_commit(&tx, envelope)?;
        Self::insert_journal_row(&tx, envelope, &validated)?;
        Self::insert_applied_event(&tx, envelope, &validated)?;
        tx.commit()?;
        Ok((validated.revision, Self::journal_row(envelope, validated)))
    }

    /// CAS commit one journal entry and the semantic snapshot it certifies in
    /// one transaction. A failed write leaves no journal or event residue.
    pub fn commit_stateful_journal(
        &mut self,
        commit: &StatefulCommit,
    ) -> Result<(u64, JournalRow), StoreError> {
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        if commit.state_bytes.is_empty() {
            return Err(StoreError::EmptyStateBytes);
        }
        let validated = Self::validate_journal_commit(&tx, &commit.journal)?;
        Self::insert_journal_row(&tx, &commit.journal, &validated)?;
        Self::insert_applied_event(&tx, &commit.journal, &validated)?;
        tx.execute(
            "INSERT INTO snapshots (revision, scope_digest, state_digest, state_bytes) VALUES (?1, ?2, ?3, ?4)",
            params![
                validated.revision_sqlite,
                blob(commit.journal.receipt.scope_digest),
                blob(commit.journal.receipt.state_after),
                commit.state_bytes.as_slice(),
            ],
        )?;
        tx.commit()?;
        Ok((
            validated.revision,
            Self::journal_row(&commit.journal, validated),
        ))
    }

    fn validate_journal_commit(
        tx: &Transaction<'_>,
        envelope: &CommitEnvelope,
    ) -> Result<ValidatedJournalCommit, StoreError> {
        let base_revision_sqlite = revision_to_sqlite(envelope.receipt.base_revision)?;
        let _next_revision_sqlite = revision_to_sqlite(envelope.receipt.next_revision)?;
        let current_sqlite = tx.query_row(
            "SELECT COALESCE(MAX(logical_revision), 0) FROM journal WHERE scope_digest = ?1",
            params![blob(envelope.receipt.scope_digest)],
            |row| row.get::<_, i64>(0),
        )?;
        let current = revision_from_sqlite(current_sqlite)?;
        let next_revision = current
            .checked_add(1)
            .ok_or(StoreError::RevisionOutOfRange { revision: u64::MAX })?;
        let revision_sqlite = revision_to_sqlite(next_revision)?;
        if envelope.receipt.base_revision != current {
            return Err(StoreError::StaleRevision {
                expected: envelope.receipt.base_revision,
                actual: current,
            });
        }
        if envelope.receipt.next_revision != next_revision {
            return Err(StoreError::StaleRevision {
                expected: envelope.receipt.next_revision,
                actual: next_revision,
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
        Ok(ValidatedJournalCommit {
            revision: next_revision,
            revision_sqlite,
            base_revision_sqlite,
            event_digest,
            receipt_bytes,
            chain_digest,
        })
    }

    fn insert_journal_row(
        tx: &Transaction<'_>,
        envelope: &CommitEnvelope,
        validated: &ValidatedJournalCommit,
    ) -> Result<(), StoreError> {
        tx.execute(
            "INSERT INTO journal (logical_revision, scope_digest, base_revision, event_kind, event_bytes, event_digest, receipt_bytes, chain_digest, committed_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                validated.revision_sqlite,
                blob(envelope.receipt.scope_digest),
                validated.base_revision_sqlite,
                envelope.event_kind.clone(),
                envelope.event_bytes.clone(),
                blob(validated.event_digest),
                validated.receipt_bytes.clone(),
                blob(validated.chain_digest),
                now_ms() as i64,
            ],
        )?;
        Ok(())
    }

    fn insert_applied_event(
        tx: &Transaction<'_>,
        envelope: &CommitEnvelope,
        validated: &ValidatedJournalCommit,
    ) -> Result<(), StoreError> {
        tx.execute(
            "INSERT INTO applied_events (scope_digest, event_digest, revision) VALUES (?1, ?2, ?3)",
            params![
                blob(envelope.receipt.scope_digest),
                blob(validated.event_digest),
                validated.revision_sqlite,
            ],
        )?;
        Ok(())
    }

    fn journal_row(envelope: &CommitEnvelope, validated: ValidatedJournalCommit) -> JournalRow {
        JournalRow {
            revision: validated.revision,
            scope_digest: envelope.receipt.scope_digest,
            base_revision: envelope.receipt.base_revision,
            event_kind: envelope.event_kind.clone(),
            event_bytes: envelope.event_bytes.clone(),
            event_digest: validated.event_digest,
            receipt_bytes: validated.receipt_bytes,
            chain_digest: validated.chain_digest,
        }
    }

    // ------------------------------------------------------------ snapshots

    pub fn write_snapshot(
        &mut self,
        scope_digest: &Digest,
        revision: u64,
        state_digest: &Digest,
        state_bytes: &[u8],
    ) -> Result<(), StoreError> {
        let revision_sqlite = revision_to_sqlite(revision)?;
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        conn.execute(
            "INSERT OR REPLACE INTO snapshots (revision, scope_digest, state_digest, state_bytes) VALUES (?1, ?2, ?3, ?4)",
            params![
                revision_sqlite,
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
        let revision_sqlite = revision_to_sqlite(revision)?;
        let conn = self.connection()?;
        conn.query_row(
            "SELECT state_digest, state_bytes FROM snapshots WHERE scope_digest = ?1 AND revision = ?2",
            params![blob(*scope_digest), revision_sqlite],
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

    pub fn read_latest_snapshot(
        &self,
        scope_digest: &Digest,
        at_or_before_revision: u64,
    ) -> Result<Option<SnapshotRow>, StoreError> {
        let conn = self.connection()?;
        let stored = conn
            .query_row(
                "SELECT revision, state_digest, state_bytes FROM snapshots WHERE scope_digest = ?1 AND revision <= ?2 ORDER BY revision DESC LIMIT 1",
                params![
                    blob(*scope_digest),
                    snapshot_upper_bound_to_sqlite(at_or_before_revision)
                ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            },
        )
            .optional()?;
        let Some((revision, state_digest_bytes, state_bytes)) = stored else {
            return Ok(None);
        };
        let mut state_digest = [0u8; 32];
        state_digest.copy_from_slice(&state_digest_bytes);
        Ok(Some(SnapshotRow {
            revision: revision_from_sqlite(revision)?,
            scope_digest: *scope_digest,
            state_digest,
            state_bytes,
        }))
    }

    fn validate_r7_successor_guard(
        stored_sequence: u64,
        candidate_sequence: u64,
        stored_registry_epoch: u64,
        stored_registry_snapshot_digest: Digest,
        stored_registry: &RootRegistrySnapshotV1,
        candidate_registry: &RootRegistrySnapshotV1,
    ) -> Result<(), StoreError> {
        if stored_sequence == u64::MAX {
            return Err(StoreError::R7PolicySequenceOverflow);
        }
        if candidate_sequence != stored_sequence + 1 {
            return Err(StoreError::R7PolicySequenceGap {
                expected: stored_sequence.saturating_add(1),
                actual: candidate_sequence,
            });
        }
        let expected_epoch = stored_registry_epoch
            .checked_add(1)
            .ok_or(StoreError::R7PolicySequenceOverflow)?;
        if candidate_registry.registry_epoch != expected_epoch {
            return Err(StoreError::R7PolicyRegistryEpochGap);
        }
        if candidate_registry.previous_snapshot_digest != Some(stored_registry_snapshot_digest) {
            return Err(StoreError::R7PolicyRegistryPredecessor);
        }
        if stored_registry
            .revocations
            .iter()
            .any(|revocation| !candidate_registry.revocations.contains(revocation))
        {
            return Err(StoreError::R7PolicyRevocationRollback);
        }
        Ok(())
    }

    fn validate_r7_context_fields(
        bundle: &R7PublicPolicyBundleV1,
        context: &R7PolicyValidationContextV1,
    ) -> Result<(), StoreError> {
        if bundle.policy.g0_incarnation_id != context.committed_g0_incarnation_id
            || bundle.policy.g0_manifest_digest != context.committed_g0_manifest_digest
            || bundle.policy.g0_seed_code_digest != context.committed_g0_seed_code_digest
            || bundle.policy.g0_persona_source_digest != context.committed_g0_persona_source_digest
            || bundle.policy.g0_genesis_receipt_digest
                != context.committed_g0_genesis_receipt_digest
            || bundle.review.message.policy_body_digest != bundle.policy.policy_body_digest
            || bundle.attestation.message.policy_body_digest != bundle.policy.policy_body_digest
            || bundle.activation.message.policy_body_digest != bundle.policy.policy_body_digest
            || bundle.activation.message.policy_spec_normalized_sha256
                != bundle.review.message.policy_spec_normalized_sha256
            || bundle.review.message.native_source_identity_digest
                != context.native_source_identity_digest
            || bundle.review.message.plugin_source_identity_digest
                != context.plugin_source_identity_digest
            || bundle.review.message.control_evidence_set_digest
                != context.control_evidence_set_digest
            || bundle.activation.message.native_source_identity_digest
                != context.native_source_identity_digest
            || bundle.activation.message.plugin_source_identity_digest
                != context.plugin_source_identity_digest
            || bundle.activation.message.control_evidence_set_digest
                != context.control_evidence_set_digest
            || bundle.activation.message.g0_binding_contract_digest
                != context.g0_binding_contract_digest
            || bundle.activation.message.g0_only_fallback_contract_digest
                != context.g0_only_fallback_contract_digest
        {
            return Err(StoreError::R7PolicyInvalid(
                "public authority context mismatch".to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_r7_bundle(
        bundle: &R7PublicPolicyBundleV1,
        context: &R7PolicyValidationContextV1,
    ) -> Result<(), StoreError> {
        let canonical_policy = GenesisIdentityPolicyV1::decode(&bundle.policy.encode())
            .map_err(|error| StoreError::R7PolicyInvalid(error.to_string()))?;
        if canonical_policy != bundle.policy {
            return Err(StoreError::R7PolicyInvalid(
                "public authority DTO is not canonical".to_owned(),
            ));
        }
        verify_authority_closure_v1(
            &bundle.delegation,
            &bundle.ceremony,
            &bundle.root_custody,
            &bundle.policy_custody,
            &bundle.reviewer_custody,
            &bundle.root,
            &bundle.registry,
            &bundle.review,
            &bundle.attestation,
            &bundle.activation,
        )
        .map_err(|error| StoreError::R7PolicyInvalid(error.to_string()))?;

        Self::validate_r7_context_fields(bundle, context)?;
        Ok(())
    }

    pub fn lookup_r7_policy_binding(
        &self,
        key: &R7PolicyBindingKeyV1,
    ) -> Result<Option<R7PolicyBindingRowV1>, StoreError> {
        let conn = self.connection()?;
        conn.query_row(
            "SELECT highest_accepted_sequence, policy_body_digest, policy_attestation_digest, attested_registry_epoch, attested_registry_snapshot_digest, policy_bytes, root_bytes, registry_bytes, review_bytes, attestation_bytes, activation_bytes, delegation_bytes, ceremony_bytes, root_custody_bytes, policy_custody_bytes, reviewer_custody_bytes FROM r7_policy_bindings_v1 WHERE bot_token = ?1 AND persona_token = ?2 AND committed_g0_incarnation_id = ?3 AND identity_scope_id = ?4",
            params![blob(key.bot_token), blob(key.persona_token), blob(key.committed_g0_incarnation_id), i64::from(key.identity_scope_id)],
            |row| {
                let digest = |index: usize| -> rusqlite::Result<Digest> {
                    let bytes: Vec<u8> = row.get(index)?;
                    bytes.try_into().map_err(|_| rusqlite::Error::InvalidQuery)
                };
                Ok(R7PolicyBindingRowV1 {
                    key: key.clone(),
                    highest_accepted_sequence: revision_from_sqlite(row.get(0)?).map_err(|_| rusqlite::Error::InvalidQuery)?,
                    policy_body_digest: digest(1)?,
                    policy_attestation_digest: digest(2)?,
                    attested_registry_epoch: revision_from_sqlite(row.get(3)?).map_err(|_| rusqlite::Error::InvalidQuery)?,
                    attested_registry_snapshot_digest: digest(4)?,
                    policy_bytes: row.get(5)?,
                    root_bytes: row.get(6)?,
                    registry_bytes: row.get(7)?,
                    review_bytes: row.get(8)?,
                    attestation_bytes: row.get(9)?,
                    activation_bytes: row.get(10)?,
                    delegation_bytes: row.get(11)?,
                    ceremony_bytes: row.get(12)?,
                    root_custody_bytes: row.get(13)?,
                    policy_custody_bytes: row.get(14)?,
                    reviewer_custody_bytes: row.get(15)?,
                })
            },
        )
        .optional()
        .map_err(StoreError::from)
    }

    fn decode_r7_bundle(row: &R7EncodedBundleRowV1) -> Result<R7PublicPolicyBundleV1, StoreError> {
        let error =
            |error: ae_genesis::r7::PolicyErrorV1| StoreError::R7PolicyInvalid(error.to_string());
        Ok(R7PublicPolicyBundleV1 {
            delegation: UserDelegationReceiptV1::decode(&row.6).map_err(error)?,
            ceremony: KeyCeremonyReceiptV1::decode(&row.7).map_err(error)?,
            root_custody: CustodyDispositionReceiptV1::decode(&row.8).map_err(error)?,
            policy_custody: CustodyDispositionReceiptV1::decode(&row.9).map_err(error)?,
            reviewer_custody: CustodyDispositionReceiptV1::decode(&row.10).map_err(error)?,
            policy: GenesisIdentityPolicyV1::decode(&row.0).map_err(error)?,
            root: ReleaseTrustRootV1::decode(&row.1).map_err(error)?,
            registry: RootRegistrySnapshotV1::decode(&row.2).map_err(error)?,
            review: IndependentSolReviewReceiptV1::decode(&row.3).map_err(error)?,
            attestation: PolicyAttestationV1::decode(&row.4).map_err(error)?,
            activation: BootstrapActivationReceiptV1::decode(&row.5).map_err(error)?,
        })
    }

    /// Persist one fully verified public policy chain under the exact native
    /// Bot/Persona/G0-incarnation/scope key.  The transaction is a monotonic
    /// sequence CAS: no gap, replay conflict, stale row, or overflow can write.
    pub fn compare_and_commit_r7_policy(
        &mut self,
        key: &R7PolicyBindingKeyV1,
        bundle: &R7PublicPolicyBundleV1,
    ) -> Result<R7PolicyCommitOutcomeV1, StoreError> {
        let _ = (key, bundle);
        Err(StoreError::R7PolicyValidationContextRequired)
    }

    pub fn compare_and_commit_r7_policy_with_context(
        &mut self,
        key: &R7PolicyBindingKeyV1,
        bundle: &R7PublicPolicyBundleV1,
        context: &R7PolicyValidationContextV1,
    ) -> Result<R7PolicyCommitOutcomeV1, StoreError> {
        if key.identity_scope_id != 1 {
            return Err(StoreError::R7PolicyInvalid(
                "unsupported identity scope".to_owned(),
            ));
        }
        if context.committed_g0_incarnation_id != key.committed_g0_incarnation_id {
            return Err(StoreError::R7PolicyG0BindingMismatch);
        }
        Self::validate_r7_bundle(bundle, context)?;
        let candidate_sequence = bundle.policy.incarnation_sequence;
        if candidate_sequence == 0 {
            return Err(StoreError::R7PolicyInvalid(
                "zero policy sequence".to_owned(),
            ));
        }
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let bound_incarnation: Option<Vec<u8>> = tx
            .query_row(
                "SELECT incarnation_id FROM active_bindings WHERE bot_token = ?1 AND persona_token = ?2",
                params![blob(key.bot_token), blob(key.persona_token)],
                |row| row.get(0),
            )
            .optional()?;
        if !bound_incarnation
            .as_ref()
            .is_some_and(|value| value.as_slice() == key.committed_g0_incarnation_id.as_slice())
        {
            return Err(StoreError::R7PolicyG0BindingMismatch);
        }
        let existing: Option<R7StoredPolicyRowV1> = tx
            .query_row(
                "SELECT highest_accepted_sequence, policy_body_digest, policy_attestation_digest, attested_registry_epoch, attested_registry_snapshot_digest, policy_bytes, root_bytes, registry_bytes, review_bytes, attestation_bytes, activation_bytes, delegation_bytes, ceremony_bytes, root_custody_bytes, policy_custody_bytes, reviewer_custody_bytes FROM r7_policy_bindings_v1 WHERE bot_token = ?1 AND persona_token = ?2 AND committed_g0_incarnation_id = ?3 AND identity_scope_id = ?4",
                params![blob(key.bot_token), blob(key.persona_token), blob(key.committed_g0_incarnation_id), i64::from(key.identity_scope_id)],
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
                        row.get(10)?,
                        row.get(11)?,
                        row.get(12)?,
                        row.get(13)?,
                        row.get(14)?,
                        row.get(15)?,
                    ))
                },
            )
            .optional()?;
        let policy_bytes = bundle.policy.encode();
        let root_bytes = bundle.root.encode();
        let registry_bytes = bundle.registry.encode();
        let review_bytes = bundle.review.encode();
        let attestation_bytes = bundle.attestation.encode();
        let activation_bytes = bundle.activation.encode();
        let delegation_bytes = bundle.delegation.encode();
        let ceremony_bytes = bundle.ceremony.encode();
        let root_custody_bytes = bundle.root_custody.encode();
        let policy_custody_bytes = bundle.policy_custody.encode();
        let reviewer_custody_bytes = bundle.reviewer_custody.encode();
        let outcome = match existing {
            None => {
                if candidate_sequence != 1 {
                    return Err(StoreError::R7PolicySequenceGap {
                        expected: 1,
                        actual: candidate_sequence,
                    });
                }
                tx.execute(
                    "INSERT INTO r7_policy_bindings_v1 (bot_token, persona_token, committed_g0_incarnation_id, identity_scope_id, highest_accepted_sequence, policy_body_digest, policy_attestation_digest, attested_registry_epoch, attested_registry_snapshot_digest, policy_bytes, root_bytes, registry_bytes, review_bytes, attestation_bytes, activation_bytes, delegation_bytes, ceremony_bytes, root_custody_bytes, policy_custody_bytes, reviewer_custody_bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
                    params![blob(key.bot_token), blob(key.persona_token), blob(key.committed_g0_incarnation_id), i64::from(key.identity_scope_id), revision_to_sqlite(candidate_sequence)?, blob(bundle.policy.policy_body_digest), blob(bundle.attestation.policy_attestation_digest), revision_to_sqlite(bundle.registry.registry_epoch)?, blob(bundle.registry.registry_snapshot_digest), policy_bytes, root_bytes, registry_bytes, review_bytes, attestation_bytes, activation_bytes, delegation_bytes, ceremony_bytes, root_custody_bytes, policy_custody_bytes, reviewer_custody_bytes],
                )?;
                R7PolicyCommitOutcomeV1::Inserted
            }
            Some((
                stored_sequence,
                stored_body,
                stored_attestation,
                stored_epoch,
                stored_registry,
                stored_policy,
                stored_root,
                stored_snapshot,
                stored_review,
                stored_attestation_bytes,
                stored_activation,
                stored_delegation,
                stored_ceremony,
                stored_root_custody,
                stored_policy_custody,
                stored_reviewer_custody,
            )) => {
                let stored_sequence = revision_from_sqlite(stored_sequence)?;
                let stored_epoch = revision_from_sqlite(stored_epoch)?;
                if candidate_sequence == stored_sequence {
                    if stored_body != bundle.policy.policy_body_digest.as_slice()
                        || stored_attestation
                            != bundle.attestation.policy_attestation_digest.as_slice()
                        || stored_epoch != bundle.registry.registry_epoch
                        || stored_registry != bundle.registry.registry_snapshot_digest.as_slice()
                        || stored_policy != policy_bytes
                        || stored_root != root_bytes
                        || stored_snapshot != registry_bytes
                        || stored_review != review_bytes
                        || stored_attestation_bytes != attestation_bytes
                        || stored_activation != activation_bytes
                        || stored_delegation != delegation_bytes
                        || stored_ceremony != ceremony_bytes
                        || stored_root_custody != root_custody_bytes
                        || stored_policy_custody != policy_custody_bytes
                        || stored_reviewer_custody != reviewer_custody_bytes
                    {
                        return Err(StoreError::R7PolicySequenceConflict);
                    }
                    let current = Self::decode_r7_bundle(&(
                        stored_policy,
                        stored_root,
                        stored_snapshot,
                        stored_review,
                        stored_attestation_bytes,
                        stored_activation,
                        stored_delegation,
                        stored_ceremony,
                        stored_root_custody,
                        stored_policy_custody,
                        stored_reviewer_custody,
                    ))?;
                    Self::validate_r7_bundle(&current, context)?;
                    R7PolicyCommitOutcomeV1::Replay
                } else if candidate_sequence < stored_sequence {
                    return Err(StoreError::R7PolicySequenceStale {
                        stored: stored_sequence,
                        actual: candidate_sequence,
                    });
                } else if stored_sequence == u64::MAX {
                    return Err(StoreError::R7PolicySequenceOverflow);
                } else if candidate_sequence != stored_sequence + 1 {
                    return Err(StoreError::R7PolicySequenceGap {
                        expected: stored_sequence + 1,
                        actual: candidate_sequence,
                    });
                } else {
                    let current = Self::decode_r7_bundle(&(
                        stored_policy,
                        stored_root,
                        stored_snapshot,
                        stored_review,
                        stored_attestation_bytes,
                        stored_activation,
                        stored_delegation,
                        stored_ceremony,
                        stored_root_custody,
                        stored_policy_custody,
                        stored_reviewer_custody,
                    ))?;
                    Self::validate_r7_bundle(&current, context)?;
                    Self::validate_r7_successor_guard(
                        stored_sequence,
                        candidate_sequence,
                        stored_epoch,
                        current.registry.registry_snapshot_digest,
                        &current.registry,
                        &bundle.registry,
                    )?;
                    let updated = tx.execute(
                        "UPDATE r7_policy_bindings_v1 SET highest_accepted_sequence = ?5, policy_body_digest = ?6, policy_attestation_digest = ?7, attested_registry_epoch = ?8, attested_registry_snapshot_digest = ?9, policy_bytes = ?10, root_bytes = ?11, registry_bytes = ?12, review_bytes = ?13, attestation_bytes = ?14, activation_bytes = ?15, delegation_bytes = ?16, ceremony_bytes = ?17, root_custody_bytes = ?18, policy_custody_bytes = ?19, reviewer_custody_bytes = ?20 WHERE bot_token = ?1 AND persona_token = ?2 AND committed_g0_incarnation_id = ?3 AND identity_scope_id = ?4 AND highest_accepted_sequence = ?21",
                        params![blob(key.bot_token), blob(key.persona_token), blob(key.committed_g0_incarnation_id), i64::from(key.identity_scope_id), revision_to_sqlite(candidate_sequence)?, blob(bundle.policy.policy_body_digest), blob(bundle.attestation.policy_attestation_digest), revision_to_sqlite(bundle.registry.registry_epoch)?, blob(bundle.registry.registry_snapshot_digest), policy_bytes, root_bytes, registry_bytes, review_bytes, attestation_bytes, activation_bytes, delegation_bytes, ceremony_bytes, root_custody_bytes, policy_custody_bytes, reviewer_custody_bytes, revision_to_sqlite(stored_sequence)?],
                    )?;
                    if updated != 1 {
                        return Err(StoreError::R7PolicySequenceConflict);
                    }
                    R7PolicyCommitOutcomeV1::Successor
                }
            }
        };
        tx.commit()?;
        Ok(outcome)
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
                revision_to_sqlite(base_revision).unwrap(),
                vec![event_digest[0]; 32],
                blob(event_digest),
                vec![event_digest[0].wrapping_add(1); 32],
                vec![event_digest[0].wrapping_add(2); 32],
            ],
        )
        .unwrap();
        let physical_revision = u64::try_from(conn.last_insert_rowid()).unwrap();
        conn.execute(
            "INSERT INTO applied_events (scope_digest, event_digest, revision) VALUES (?1, ?2, ?3)",
            params![
                blob(scope_digest),
                blob(event_digest),
                revision_to_sqlite(physical_revision).unwrap(),
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

    fn gv3_hex(value: &str) -> Vec<u8> {
        assert_eq!(value.len() % 2, 0);
        value
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let high = (pair[0] as char).to_digit(16).unwrap();
                let low = (pair[1] as char).to_digit(16).unwrap();
                ((high << 4) | low) as u8
            })
            .collect()
    }

    fn gv3_digest(value: &str) -> Digest {
        gv3_hex(value).try_into().unwrap()
    }

    fn gv3_byte_digest(value: u8) -> Digest {
        let mut digest = [0; 32];
        digest[0] = value;
        digest
    }

    fn gv3_signature(value: &str) -> [u8; 64] {
        gv3_hex(value).try_into().unwrap()
    }

    fn gv3_token(out: &mut Vec<u8>, value: &str) {
        out.extend_from_slice(&(value.len() as u16).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }

    fn gv3_policy_with_attested_vector_digest() -> GenesisIdentityPolicyV1 {
        let mut policy = GenesisIdentityPolicyV1::new(
            gv3_byte_digest(1),
            gv3_byte_digest(2),
            gv3_byte_digest(3),
            gv3_byte_digest(4),
            gv3_byte_digest(5),
        )
        .unwrap();
        // GV3-01 uses D(20) as a synthetic signed-body input.  The test
        // deliberately retains that attested digest while mutating the
        // public policy DTO below; canonical policy decoding must reject it.
        policy.policy_body_digest = gv3_byte_digest(20);
        policy
    }

    fn gv3_public_policy_bundle() -> R7PublicPolicyBundleV1 {
        use ae_genesis::r7::{
            domain_hash_sha256, BootstrapActivationMessageV1, IndependentSolReviewMessageV1,
            KeyCeremonyReceiptV1, PolicyAttestationMessageV1, RegistryGrantV1,
            UserDelegationReceiptV1, ATTESTATION_RECORD_DOMAIN_V1,
            BOOTSTRAP_ACTIVATION_RECORD_DOMAIN_V1, CEREMONY_RECEIPT_DOMAIN_V1,
            CUSTODY_RECEIPT_DOMAIN_V1, DELEGATION_RECEIPT_DOMAIN_V1, REGISTRY_BODY_DOMAIN_V1,
            RELEASE_TRUST_ROOT_DOMAIN_V1, REVIEW_RECEIPT_RECORD_DOMAIN_V1,
        };

        let root_key =
            gv3_digest("D75A980182B10AB7D54BFED3C964073A0EE172F3DAA62325AF021A68F707511A");
        let root_fingerprint =
            gv3_digest("A6119B0D03A43C07A6464B05ECC019F19CFCB1FA8D659FED14A722BFC75086FD");
        let reviewer_key =
            gv3_digest("3D4017C3E843895A92B70AA74D1B7EBC9C982CCF2EC4968CC0CD55F12AF4660C");
        let reviewer_fingerprint =
            gv3_digest("AEEA0411DB59CCF998D802A60C11D34D97C895E0A977F59C75771915374A0E56");
        let delegation_digest =
            gv3_digest("91BDDB2ACB94473149BBAD56846E5D2E3BC5F5254C849B2825E6000A2569E269");
        let ceremony_digest =
            gv3_digest("1D11C1AF65F6D1BB288BF91451E112ECCC8B04FDD269B99484320A79AE9FE911");
        let normalized_digest = gv3_byte_digest(19);
        let signed_policy_digest = gv3_byte_digest(20);
        let native_source_digest = gv3_byte_digest(21);
        let plugin_source_digest = gv3_byte_digest(22);
        let control_evidence_digest = gv3_byte_digest(23);
        let g0_binding_digest = gv3_byte_digest(24);
        let g0_fallback_digest = gv3_byte_digest(25);

        let mut delegation_body = Vec::new();
        delegation_body.extend_from_slice(b"UDR1");
        delegation_body.extend_from_slice(&1u16.to_le_bytes());
        gv3_token(&mut delegation_body, "astr_embodiment");
        gv3_token(&mut delegation_body, "1.0.0-rc1");
        gv3_token(&mut delegation_body, "test_delegator");
        delegation_body.push(1);
        gv3_token(&mut delegation_body, "00000000-0000-0000-0000-000000000001");
        gv3_token(&mut delegation_body, "host_user_message_1");
        delegation_body.extend_from_slice(&gv3_byte_digest(11));
        delegation_body.extend_from_slice(&gv3_byte_digest(12));
        gv3_token(&mut delegation_body, "independent_sol_policy_authority");
        delegation_body.extend_from_slice(&0x0000_001Fu32.to_le_bytes());
        delegation_body.extend_from_slice(&gv3_byte_digest(13));
        delegation_body.extend_from_slice(&gv3_byte_digest(14));
        delegation_body.extend_from_slice(&gv3_byte_digest(15));
        delegation_body.extend_from_slice(&gv3_byte_digest(16));
        delegation_body.extend_from_slice(&1u64.to_le_bytes());
        delegation_body.push(1);
        let delegation_receipt_digest =
            domain_hash_sha256(DELEGATION_RECEIPT_DOMAIN_V1, &delegation_body);
        assert_eq!(delegation_receipt_digest, delegation_digest);
        delegation_body.extend_from_slice(&delegation_receipt_digest);
        let delegation = UserDelegationReceiptV1::decode(&delegation_body).unwrap();

        let mut root_custody_body = Vec::new();
        root_custody_body.extend_from_slice(b"CDR1");
        root_custody_body.extend_from_slice(&1u16.to_le_bytes());
        gv3_token(&mut root_custody_body, "test_root_policy_custody");
        gv3_token(&mut root_custody_body, "test_root_policy_signer");
        gv3_token(&mut root_custody_body, "ae_rc1_identity_policy_signer_v1");
        root_custody_body.extend_from_slice(&1u32.to_le_bytes());
        root_custody_body.extend_from_slice(&root_fingerprint);
        root_custody_body.extend_from_slice(&[1, 1, 0, 0]);
        let root_custody_digest = domain_hash_sha256(CUSTODY_RECEIPT_DOMAIN_V1, &root_custody_body);
        assert_eq!(
            root_custody_digest,
            gv3_digest("F152DA92296A45197DCA80543161660BC30251E4D9F6B3702AAC207A8671E9D2")
        );
        root_custody_body.extend_from_slice(&root_custody_digest);
        let root_custody = CustodyDispositionReceiptV1::decode(&root_custody_body).unwrap();

        let mut reviewer_custody_body = Vec::new();
        reviewer_custody_body.extend_from_slice(b"CDR1");
        reviewer_custody_body.extend_from_slice(&1u16.to_le_bytes());
        gv3_token(&mut reviewer_custody_body, "test_reviewer_custody");
        gv3_token(&mut reviewer_custody_body, "test_reviewer_signer");
        gv3_token(&mut reviewer_custody_body, "test_reviewer_key");
        reviewer_custody_body.extend_from_slice(&1u32.to_le_bytes());
        reviewer_custody_body.extend_from_slice(&reviewer_fingerprint);
        reviewer_custody_body.extend_from_slice(&[1, 1, 0, 0]);
        let reviewer_custody_digest =
            domain_hash_sha256(CUSTODY_RECEIPT_DOMAIN_V1, &reviewer_custody_body);
        assert_eq!(
            reviewer_custody_digest,
            gv3_digest("34C0209E7E44F73FC8EAFBCEB74CCAB29451D8A892E43C7C95160FC06B4BD6BD")
        );
        reviewer_custody_body.extend_from_slice(&reviewer_custody_digest);
        let reviewer_custody = CustodyDispositionReceiptV1::decode(&reviewer_custody_body).unwrap();
        let policy_custody = root_custody.clone();

        let mut ceremony_body = Vec::new();
        ceremony_body.extend_from_slice(b"KCR1");
        ceremony_body.extend_from_slice(&1u16.to_le_bytes());
        ceremony_body.extend_from_slice(&delegation_receipt_digest);
        ceremony_body.extend_from_slice(&1u64.to_le_bytes());
        ceremony_body.push(1);
        gv3_token(&mut ceremony_body, "ae_rc1_identity_policy_signer_v1");
        ceremony_body.extend_from_slice(&1u32.to_le_bytes());
        ceremony_body.extend_from_slice(&root_key);
        ceremony_body.extend_from_slice(&root_fingerprint);
        gv3_token(&mut ceremony_body, "ae_rc1_identity_policy_signer_v1");
        ceremony_body.extend_from_slice(&1u32.to_le_bytes());
        ceremony_body.extend_from_slice(&root_key);
        ceremony_body.extend_from_slice(&root_fingerprint);
        gv3_token(&mut ceremony_body, "test_reviewer_key");
        ceremony_body.extend_from_slice(&1u32.to_le_bytes());
        ceremony_body.extend_from_slice(&reviewer_key);
        ceremony_body.extend_from_slice(&reviewer_fingerprint);
        ceremony_body.push(1);
        gv3_token(&mut ceremony_body, "delegated_bootstrap_operator");
        gv3_token(&mut ceremony_body, "test_ceremony_operator");
        ceremony_body.extend_from_slice(&gv3_byte_digest(17));
        ceremony_body.extend_from_slice(&gv3_byte_digest(18));
        ceremony_body.extend_from_slice(&root_custody_digest);
        ceremony_body.extend_from_slice(&root_custody_digest);
        ceremony_body.extend_from_slice(&reviewer_custody_digest);
        ceremony_body.extend_from_slice(&[0, 0, 1, 0, 0, 1, 0, 0, 1]);
        let ceremony_receipt_digest =
            domain_hash_sha256(CEREMONY_RECEIPT_DOMAIN_V1, &ceremony_body);
        assert_eq!(ceremony_receipt_digest, ceremony_digest);
        ceremony_body.extend_from_slice(&ceremony_receipt_digest);
        ceremony_body.extend_from_slice(&gv3_signature(
            "265C7D092EBE9F37B99DFC67CF03322E39ECC1290A4A25B95FC55D1AEBDC91EF3C18687794517075AF0F6D46A423835A63823CABEF08BA9C90A961FC8CA9C20D",
        ));
        ceremony_body.extend_from_slice(&gv3_signature(
            "F6B0977ECA32224ACABFECD080052E647FA52D51CD498AC8C4CF70488354B58171E879B7C556B8F957C4AB5D1FD3B8FBB13CEB5F134D240539C4B07DA0A16C07",
        ));
        ceremony_body.extend_from_slice(&gv3_signature(
            "1C5C91A32C8E9839AF0BED9B3527BFA48F96AF46B454CF9EAEB1F31021C55441214DD91E560E59934299EF672358521F110CB236E17F3CC56E430111198E910B",
        ));
        let ceremony = KeyCeremonyReceiptV1::decode(&ceremony_body).unwrap();

        let mut root_body = Vec::new();
        root_body.extend_from_slice(b"RTR1");
        root_body.extend_from_slice(&1u16.to_le_bytes());
        root_body.push(1);
        gv3_token(&mut root_body, "ae_rc1_identity_policy_signer_v1");
        root_body.extend_from_slice(&1u32.to_le_bytes());
        root_body.extend_from_slice(&root_key);
        root_body.extend_from_slice(&root_fingerprint);
        root_body.extend_from_slice(&delegation_digest);
        root_body.extend_from_slice(&ceremony_digest);
        root_body.extend_from_slice(&1u64.to_le_bytes());
        let root_digest = domain_hash_sha256(RELEASE_TRUST_ROOT_DOMAIN_V1, &root_body);
        assert_eq!(
            root_digest,
            gv3_digest("99BCD5E5667E5C47B0FDC536B1A0A607DAD2412660F52D9B24BEE21DC5073A6E")
        );
        root_body.extend_from_slice(&root_digest);
        let root = ReleaseTrustRootV1::decode(&root_body).unwrap();

        let role1 = RegistryGrantV1 {
            subject_ref: "ae_rc1_product_identity_authority".to_owned(),
            grant_ref: "ae_rc1_identity_policy_approval_v1".to_owned(),
            grant_role_id: 1,
            key_id: "ae_rc1_identity_policy_signer_v1".to_owned(),
            key_version: 1,
            public_key: root_key,
            public_key_fingerprint: root_fingerprint,
        };
        let role2 = RegistryGrantV1 {
            subject_ref: "test_independent_sol_reviewer".to_owned(),
            grant_ref: "test_review_grant".to_owned(),
            grant_role_id: 2,
            key_id: "test_reviewer_key".to_owned(),
            key_version: 1,
            public_key: reviewer_key,
            public_key_fingerprint: reviewer_fingerprint,
        };
        let mut registry_body = Vec::new();
        registry_body.extend_from_slice(b"RGS1");
        registry_body.extend_from_slice(&1u16.to_le_bytes());
        gv3_token(&mut registry_body, "ae_rc1_identity_policy_signer_v1");
        registry_body.extend_from_slice(&1u32.to_le_bytes());
        registry_body.extend_from_slice(&1u64.to_le_bytes());
        registry_body.extend_from_slice(&root.release_trust_root_digest);
        registry_body.extend_from_slice(&delegation_digest);
        registry_body.extend_from_slice(&ceremony_digest);
        registry_body.extend_from_slice(&1u64.to_le_bytes());
        gv3_token(&mut registry_body, "product_constitution_authority");
        registry_body.push(0);
        registry_body.extend_from_slice(&2u16.to_le_bytes());
        registry_body.extend_from_slice(&role2.encode());
        registry_body.extend_from_slice(&role1.encode());
        registry_body.extend_from_slice(&0u16.to_le_bytes());
        let registry_digest = domain_hash_sha256(REGISTRY_BODY_DOMAIN_V1, &registry_body);
        assert_eq!(
            registry_digest,
            gv3_digest("62FB535AC07826534218B069E655B7EE4E97D9F4677B249417E919B7652C64CD")
        );
        registry_body.extend_from_slice(&registry_digest);
        registry_body.extend_from_slice(&gv3_signature(
            "4DD577E257544E23B1D11B93CC47EF1FBFE998397874E62CE76BBD6A98F46B20644DCD8D70CE059C0863E3B4CC4F7F222438AA0C1D668C79FBF846728B59E80E",
        ));
        let registry = RootRegistrySnapshotV1::decode(&registry_body).unwrap();
        registry.verify_with_root(&root).unwrap();

        let review_message = IndependentSolReviewMessageV1 {
            reviewer_authority_ref: "test_independent_sol_reviewer".to_owned(),
            reviewer_grant_ref: "test_review_grant".to_owned(),
            reviewer_key_id: "test_reviewer_key".to_owned(),
            reviewer_key_version: 1,
            approval: 1,
            policy_spec_normalized_sha256: normalized_digest,
            policy_body_digest: signed_policy_digest,
            registry_snapshot_digest: registry.registry_snapshot_digest,
            native_source_identity_digest: native_source_digest,
            plugin_source_identity_digest: plugin_source_digest,
            control_evidence_set_digest: control_evidence_digest,
            delegation_receipt_digest: delegation_digest,
            key_ceremony_receipt_digest: ceremony_digest,
            release_trust_root_digest: root.release_trust_root_digest,
            root_public_key_fingerprint: root_fingerprint,
            reviewer_public_key_fingerprint: reviewer_fingerprint,
            approval_origin: 1,
            approval_actor: 1,
        };
        let review_message_bytes = review_message.encode();
        let mut review_outer = Vec::new();
        review_outer.extend_from_slice(b"IRR1");
        review_outer.extend_from_slice(&(review_message_bytes.len() as u16).to_le_bytes());
        review_outer.extend_from_slice(&review_message_bytes);
        review_outer.extend_from_slice(&gv3_signature(
            "D5B0FCD0D91B63F1B8836A564385006AA94C2AF7DAC86177087E7351809752F3C07A6ECB242BAFA273E84DE6D2EC169F01B2108C9739B2E83E22F7ADD02C7F0D",
        ));
        let review_digest = domain_hash_sha256(REVIEW_RECEIPT_RECORD_DOMAIN_V1, &review_outer);
        assert_eq!(
            review_digest,
            gv3_digest("D57A5512AA6A007B6B74CE2CA6329FC16A133597653602F50E2AB0502776F580")
        );
        review_outer.extend_from_slice(&review_digest);
        let review = IndependentSolReviewReceiptV1::decode(&review_outer).unwrap();
        review.verify(&reviewer_key).unwrap();

        let attestation_message = PolicyAttestationMessageV1 {
            scheme_id: 1,
            role_id: 1,
            registry_epoch: 1,
            policy_body_digest: signed_policy_digest,
            review_receipt_digest: review.review_receipt_digest,
            registry_snapshot_digest: registry.registry_snapshot_digest,
            release_trust_root_digest: root.release_trust_root_digest,
            delegation_receipt_digest: delegation_digest,
            key_ceremony_receipt_digest: ceremony_digest,
            policy_public_key_fingerprint: root_fingerprint,
            policy_spec_normalized_sha256: normalized_digest,
            policy_owner_ref: "ae_rc1_product_identity_authority".to_owned(),
            authorization_grant_ref: "ae_rc1_identity_policy_approval_v1".to_owned(),
            attestation_key_id: "ae_rc1_identity_policy_signer_v1".to_owned(),
            attestation_key_version: 1,
        };
        let attestation_message_bytes = attestation_message.encode();
        let mut attestation_outer = Vec::new();
        attestation_outer.extend_from_slice(b"PAT1");
        attestation_outer
            .extend_from_slice(&(attestation_message_bytes.len() as u16).to_le_bytes());
        attestation_outer.extend_from_slice(&attestation_message_bytes);
        attestation_outer.extend_from_slice(&gv3_signature(
            "539998D100ACD62A828329F25D45D1346388BC92F29732D9B8092EDCF4D4D41F2C4218B9D0193ED916BCA26568E59A7A567A8934923DCD3C5A99C7EE74FF8104",
        ));
        let attestation_digest =
            domain_hash_sha256(ATTESTATION_RECORD_DOMAIN_V1, &attestation_outer);
        assert_eq!(
            attestation_digest,
            gv3_digest("1D92CB8652D52375B94EFBF02321030C263662EF5C9719DAB7AEB4B46F13D172")
        );
        attestation_outer.extend_from_slice(&attestation_digest);
        let attestation = PolicyAttestationV1::decode(&attestation_outer).unwrap();
        attestation.verify(&root_key).unwrap();

        let activation_message = BootstrapActivationMessageV1 {
            approval_origin: 1,
            approval_actor: 1,
            user_direct_fingerprint_approval: 0,
            delegated_fingerprint_approval: 1,
            release_disposition: 1,
            delegation_receipt_digest: delegation_digest,
            key_ceremony_receipt_digest: ceremony_digest,
            release_trust_root_digest: root.release_trust_root_digest,
            root_public_key_fingerprint: root_fingerprint,
            registry_snapshot_digest: registry.registry_snapshot_digest,
            registry_epoch: 1,
            policy_spec_normalized_sha256: normalized_digest,
            policy_body_digest: signed_policy_digest,
            review_receipt_digest: review.review_receipt_digest,
            policy_attestation_digest: attestation.policy_attestation_digest,
            native_source_identity_digest: native_source_digest,
            plugin_source_identity_digest: plugin_source_digest,
            control_evidence_set_digest: control_evidence_digest,
            g0_binding_contract_digest: g0_binding_digest,
            g0_only_fallback_contract_digest: g0_fallback_digest,
            reviewer_authority_ref: "test_independent_sol_reviewer".to_owned(),
            reviewer_grant_ref: "test_review_grant".to_owned(),
            reviewer_key_id: "test_reviewer_key".to_owned(),
            reviewer_key_version: 1,
            reviewer_public_key_fingerprint: reviewer_fingerprint,
            activation_sequence: 1,
        };
        let activation_message_bytes = activation_message.encode();
        let mut activation_outer = Vec::new();
        activation_outer.extend_from_slice(b"BAR1");
        activation_outer.extend_from_slice(&(activation_message_bytes.len() as u16).to_le_bytes());
        activation_outer.extend_from_slice(&activation_message_bytes);
        activation_outer.extend_from_slice(&gv3_signature(
            "2C82A6E3A6B30A3C360819802159704EEB89B874EB4494960DBBBC86C717D53A84570B8B10CAB4095DB2BEB080B383D5A589708FC9EE0BDED82ABF6A3C9CE903",
        ));
        let activation_digest =
            domain_hash_sha256(BOOTSTRAP_ACTIVATION_RECORD_DOMAIN_V1, &activation_outer);
        assert_eq!(
            activation_digest,
            gv3_digest("B5E3F602FA36DC35DE059E3515ACCA0E04FF97A8B9EE67F8DEC096FC117C6BE3")
        );
        activation_outer.extend_from_slice(&activation_digest);
        let activation = BootstrapActivationReceiptV1::decode(&activation_outer).unwrap();
        activation.verify(&reviewer_key).unwrap();

        R7PublicPolicyBundleV1 {
            delegation,
            ceremony,
            root_custody,
            policy_custody,
            reviewer_custody,
            policy: gv3_policy_with_attested_vector_digest(),
            root,
            registry,
            review,
            attestation,
            activation,
        }
    }

    fn gv3_validation_context() -> R7PolicyValidationContextV1 {
        R7PolicyValidationContextV1 {
            native_source_identity_digest: gv3_byte_digest(21),
            plugin_source_identity_digest: gv3_byte_digest(22),
            control_evidence_set_digest: gv3_byte_digest(23),
            g0_binding_contract_digest: gv3_byte_digest(24),
            g0_only_fallback_contract_digest: gv3_byte_digest(25),
            committed_g0_incarnation_id: gv3_byte_digest(3),
            committed_g0_manifest_digest: gv3_byte_digest(1),
            committed_g0_seed_code_digest: gv3_byte_digest(2),
            committed_g0_persona_source_digest: gv3_byte_digest(4),
            committed_g0_genesis_receipt_digest: gv3_byte_digest(5),
        }
    }

    #[test]
    fn r7_store_rejects_mutated_policy_with_stale_attested_digest() {
        let mut bundle = gv3_public_policy_bundle();
        let before = bundle.policy.encode();
        bundle.policy.operational_commitments[0] = "mutated_public_policy_term".to_owned();
        let after = bundle.policy.encode();
        assert_ne!(before, after);
        assert!(GenesisIdentityPolicyV1::decode(&after).is_err());
        assert!(
            Store::validate_r7_bundle(&bundle, &gv3_validation_context()).is_err(),
            "Store accepted a public policy DTO whose bytes no longer match the attested canonical body"
        );
    }

    #[test]
    fn r7_authority_closure_requires_delegation_ceremony_custody_and_bar_identity() {
        let bundle = gv3_public_policy_bundle();
        let closure = ae_genesis::r7::verify_authority_closure_v1(
            &bundle.delegation,
            &bundle.ceremony,
            &bundle.root_custody,
            &bundle.policy_custody,
            &bundle.reviewer_custody,
            &bundle.root,
            &bundle.registry,
            &bundle.review,
            &bundle.attestation,
            &bundle.activation,
        )
        .expect("GV3 authority closure");
        assert_eq!(closure.role1_grant.grant_role_id, 1);
        assert_eq!(closure.role2_grant.grant_role_id, 2);

        let mut bar = bundle.activation.clone();
        bar.message.reviewer_key_id = "wrong_reviewer_key".to_owned();
        assert!(ae_genesis::r7::verify_authority_closure_v1(
            &bundle.delegation,
            &bundle.ceremony,
            &bundle.root_custody,
            &bundle.policy_custody,
            &bundle.reviewer_custody,
            &bundle.root,
            &bundle.registry,
            &bundle.review,
            &bundle.attestation,
            &bar,
        )
        .is_err());
    }

    #[test]
    fn r7_context_cas_requires_registry_predecessor_epoch_and_revocations() {
        use ae_genesis::r7::RegistryRevocationV1;

        let bundle = gv3_public_policy_bundle();
        let mut stored_registry = bundle.registry.clone();
        stored_registry.revocations.push(RegistryRevocationV1 {
            key_id: "old_revoked_key".to_owned(),
            key_version: 1,
            revocation_epoch: 1,
            status: 1,
        });
        let mut successor_registry = bundle.registry.clone();
        successor_registry.registry_epoch = 2;
        successor_registry.previous_snapshot_digest =
            Some(stored_registry.registry_snapshot_digest);

        assert!(Store::validate_r7_successor_guard(
            1,
            2,
            1,
            stored_registry.registry_snapshot_digest,
            &stored_registry,
            &successor_registry,
        )
        .is_err());

        successor_registry.revocations = stored_registry.revocations.clone();
        assert!(Store::validate_r7_successor_guard(
            1,
            2,
            1,
            stored_registry.registry_snapshot_digest,
            &stored_registry,
            &successor_registry,
        )
        .is_ok());

        successor_registry.registry_epoch = 3;
        assert!(matches!(
            Store::validate_r7_successor_guard(
                1,
                2,
                1,
                stored_registry.registry_snapshot_digest,
                &stored_registry,
                &successor_registry,
            ),
            Err(StoreError::R7PolicyRegistryEpochGap)
        ));
    }

    #[test]
    fn r7_context_rejects_source_g0_and_fallback_mutations() {
        let bundle = gv3_public_policy_bundle();
        let context = gv3_validation_context();
        assert!(Store::validate_r7_context_fields(&bundle, &context).is_ok());

        let mut source_mismatch = context.clone();
        source_mismatch.native_source_identity_digest[0] ^= 1;
        assert!(Store::validate_r7_context_fields(&bundle, &source_mismatch).is_err());

        let mut g0_mismatch = context.clone();
        g0_mismatch.committed_g0_manifest_digest[0] ^= 1;
        assert!(Store::validate_r7_context_fields(&bundle, &g0_mismatch).is_err());

        let mut fallback_mismatch = context;
        fallback_mismatch.g0_only_fallback_contract_digest[0] ^= 1;
        assert!(Store::validate_r7_context_fields(&bundle, &fallback_mismatch).is_err());
    }

    #[test]
    fn r7_old_schema_migration_adds_public_evidence_columns() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE r7_policy_bindings_v1 (
                bot_token BLOB NOT NULL,
                persona_token BLOB NOT NULL,
                committed_g0_incarnation_id BLOB NOT NULL,
                identity_scope_id INTEGER NOT NULL,
                highest_accepted_sequence INTEGER NOT NULL,
                policy_body_digest BLOB NOT NULL,
                policy_attestation_digest BLOB NOT NULL,
                attested_registry_epoch INTEGER NOT NULL,
                attested_registry_snapshot_digest BLOB NOT NULL,
                policy_bytes BLOB NOT NULL,
                root_bytes BLOB NOT NULL,
                registry_bytes BLOB NOT NULL,
                review_bytes BLOB NOT NULL,
                attestation_bytes BLOB NOT NULL,
                activation_bytes BLOB NOT NULL,
                PRIMARY KEY (bot_token, persona_token, committed_g0_incarnation_id, identity_scope_id)
            );
            "#,
        )
        .unwrap();

        Store::migrate(&mut conn).unwrap();
        let mut statement = conn
            .prepare("PRAGMA table_info(r7_policy_bindings_v1)")
            .unwrap();
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for expected in [
            "delegation_bytes",
            "ceremony_bytes",
            "root_custody_bytes",
            "policy_custody_bytes",
            "reviewer_custody_bytes",
        ] {
            assert!(columns.iter().any(|column| column == expected));
        }
    }
}
