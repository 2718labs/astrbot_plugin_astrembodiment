//! Closed semantic snapshot and frozen AESEM2 writer verifier used by the
//! Store authority boundary.  This deliberately duplicates the narrow
//! predecessor algorithm instead of accepting a runtime-supplied summary.

use crate::{
    blob, bounded_typed_value, connection_database_bytes, enforce_byte_budget,
    enforce_connection_database_budget, enforce_database_budget, enforce_read_budget, now_ms,
    query_bounded_journal_row, query_bounded_snapshot_row, sqlite_length, store_database_identity,
    stored_typed_digest, JournalRevision, Store, StoreError, MAX_JOURNAL_DELTA_BYTES,
    MAX_JOURNAL_EVENT_BYTES, MAX_JOURNAL_EVENT_KIND_BYTES, MAX_SNAPSHOT_STATE_BYTES,
};
#[cfg(test)]
use crate::{SqliteRevision, MAX_STORE_DATABASE_BYTES};
use ae_attention::emotion_matrix::assemble_full_vector_load;
use ae_continuum::CommitEnvelope;
use ae_contracts::{
    legacy_reserved_zero_digest_v1, phase0_canonical_formula_digest_v1, wire, CanonicalEvent,
    CapacityTelemetryV1, CausalRef, CommitStatus, Digest, EnergyTelemetryV1, EvidenceVector,
    GenesisReceipt, GenesisStatus, InvariantResiduals, NativeTelemetryFormulaV1,
    NativeTelemetryPhaseV1, NativeTelemetryReceiptV1, PersonaSourceRef, ScopeRef, SemanticEstimate,
    SemanticVectorFormulaV2, SemanticVectorReceiptV2, TransitionReceipt, UserStimulus,
    NATIVE_TELEMETRY_RECEIPT_SCHEMA_V1,
};
use ae_fixed::Fixed;
use ae_neurofield::{
    graph_digest, initial_state_from_manifest, state_digest, NeuralField, SparseGraph, Synapse,
    EDGE_CAPACITY, NEURON_SLOTS, REGION_LAYOUT,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use sha2::{Digest as ShaDigest, Sha256};
#[cfg(feature = "migration-test-hooks")]
use std::cell::RefCell;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const AESEM2_MAGIC: &[u8] = b"AESEM2\0";
const AESEM2_SCHEMA: u16 = 2;
const AESEM3_MAGIC: &[u8] = b"AESEM3\0";
const AESEM3_SCHEMA: u16 = 3;
const TRANSITION_RECEIPT_V2_WIRE_LEN: usize = 302;
const NATIVE_TELEMETRY_RECEIPT_V1_WIRE_LEN: usize = 588;
const FIELD_WIRE_LEN: usize = 8 * (4 + NEURON_SLOTS * 8);
const GRAPH_WIRE_MIN_LEN: usize = 4 + ((NEURON_SLOTS + 1) * 4) + 4;
const GRAPH_EDGE_WIRE_LEN: usize = 16;
const GRAPH_WIRE_MAX_EDGE_BYTES: usize = match EDGE_CAPACITY.checked_mul(GRAPH_EDGE_WIRE_LEN) {
    Some(value) => value,
    None => panic!("semantic graph edge wire bound overflow"),
};
const GRAPH_WIRE_MAX_LEN: usize = match GRAPH_WIRE_MIN_LEN.checked_add(GRAPH_WIRE_MAX_EDGE_BYTES) {
    Some(value) => value,
    None => panic!("semantic graph wire bound overflow"),
};

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanonicalAesem3Error {
    MagicOrSchema,
    WireInvalid,
    ReservedNonzero,
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CanonicalAesem3Blocks<'a> {
    pub field: &'a [u8],
    pub graph: &'a [u8],
    pub telemetry: &'a [u8],
}

#[doc(hidden)]
pub struct DecodedCanonicalSemanticSnapshotV3 {
    pub field: NeuralField,
    pub graph: SparseGraph,
    pub telemetry: NativeTelemetryReceiptV1,
}
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
const FIELD_MIGRATION_BACKUP_SOURCE_DOMAIN_V1: &[u8] =
    b"astr-embodiment/field-migration-backup-source-v1";
const MAX_FIELD_MIGRATION_STAGE_DIRS: u64 = 64;
const MAX_FIELD_MIGRATION_BACKUP_IDENTITY_BYTES: u64 = u8::MAX as u64;
const FIELD_MIGRATION_BACKUP_MANIFEST_FIXED_BYTES_V2: usize = 316;
const MAX_FIELD_MIGRATION_BACKUP_MANIFEST_BYTES: u64 =
    FIELD_MIGRATION_BACKUP_MANIFEST_FIXED_BYTES_V2 as u64
        + (MAX_FIELD_MIGRATION_BACKUP_IDENTITY_BYTES * 2);
const MAX_SQLITE_SCHEMA_IDENTIFIER_BYTES: u64 = 255;
const MAX_SQLITE_DATABASE_PATH_BYTES: u64 = 1024 * 1024;
static NEXT_FIELD_MIGRATION_STAGE_NONCE: AtomicU64 = AtomicU64::new(1);

/// Internal crash seams. The type is unreachable through the default public
/// API because this module is private; only the explicit non-default
/// `migration-test-hooks` feature re-exports it for integration acceptance.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticMigrationTestFailpointV1 {
    BeforeSnapshot,
    AfterSnapshot,
    BeforeDatabaseSync,
    AfterDatabaseSync,
    BeforeManifestSync,
    AfterManifestSync,
    BeforeDirectorySync,
    AfterDirectorySync,
    BeforeAtomicPublish,
    AfterAtomicPublish,
    BeforeParentSync,
    AfterParentSync,
    BeforeTransactionBegin,
    AfterTransactionBegin,
    BeforeRowInsertion,
    AfterRowInsertion,
    AfterBackupRowInsert,
    AfterJournalInsert,
    AfterAppliedEventInsert,
    AfterSnapshotInsert,
    AfterGraphInsert,
    AfterContextInsert,
    AfterUpgradeInsert,
    AfterAuthorityInsert,
    BeforeCommit,
    AfterCommit,
}

#[cfg(feature = "migration-test-hooks")]
/// One-shot callback for the deterministic backup/CAS concurrency fixture.
#[doc(hidden)]
pub enum SemanticMigrationTestHookV1 {
    AfterBackupBeforeImmediateCas(Box<dyn FnOnce(&Path) + Send + 'static>),
}

#[cfg(any(test, feature = "migration-test-hooks"))]
thread_local! {
    static SEMANTIC_MIGRATION_FAILPOINT_V1: std::cell::Cell<Option<SemanticMigrationTestFailpointV1>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(feature = "migration-test-hooks")]
thread_local! {
    static SEMANTIC_MIGRATION_TEST_HOOK_V1: RefCell<Option<SemanticMigrationTestHookV1>> = const { RefCell::new(None) };
}

#[cfg(feature = "migration-test-hooks")]
#[doc(hidden)]
pub fn set_semantic_migration_test_hook_v1(hook: SemanticMigrationTestHookV1) {
    SEMANTIC_MIGRATION_TEST_HOOK_V1.with(|slot| slot.replace(Some(hook)));
}

#[cfg(any(test, feature = "migration-test-hooks"))]
#[doc(hidden)]
pub fn set_semantic_migration_test_failpoint_v1(point: SemanticMigrationTestFailpointV1) {
    SEMANTIC_MIGRATION_FAILPOINT_V1.with(|slot| slot.set(Some(point)));
}

#[cfg(feature = "migration-test-hooks")]
fn fire_semantic_migration_test_hook_v1(database_path: &Path) {
    let hook = SEMANTIC_MIGRATION_TEST_HOOK_V1.with(|slot| slot.borrow_mut().take());
    if let Some(SemanticMigrationTestHookV1::AfterBackupBeforeImmediateCas(hook)) = hook {
        hook(database_path);
    }
}

#[cfg(not(feature = "migration-test-hooks"))]
fn fire_semantic_migration_test_hook_v1(_database_path: &Path) {}

#[cfg(any(test, feature = "migration-test-hooks"))]
fn fire_semantic_migration_failpoint_v1(
    point: SemanticMigrationTestFailpointV1,
) -> Result<(), StoreError> {
    let fired = SEMANTIC_MIGRATION_FAILPOINT_V1.with(|slot| {
        if slot.get() == Some(point) {
            slot.set(None);
            true
        } else {
            false
        }
    });
    if fired {
        Err(StoreError::FieldMigrationBackup {
            context: "test atomic migration failpoint",
        })
    } else {
        Ok(())
    }
}

#[cfg(not(any(test, feature = "migration-test-hooks")))]
fn fire_semantic_migration_failpoint_v1(
    _point: SemanticMigrationTestFailpointV1,
) -> Result<(), StoreError> {
    Ok(())
}
pub const JOINT_MAX_LINEAR_FXP6_V1: u8 = 1;
pub const LEGACY_FIELD_FXP6_SCALE: u32 = 1_000_000;
const LEGACY_NEUTRAL_RELAXATION_MAX_RATE: Fixed = Fixed::from_raw(125_000);
const SEMANTIC_LANE_NAMESPACE_DOMAIN_V1: &[u8] = b"astr-embodiment/semantic-lane-namespace-v1";
const CONTINUITY_CONTEXT_DIGEST_DOMAIN_V1: &[u8] = b"AE-CONTEXT-PROJECTION-STATE-V1";
const CONTEXT_STATE_MAGIC_V1: &[u8; 8] = b"AECPSTV1";
const CONTEXT_STATE_SCHEMA_V1: u32 = 1;
const CONTEXT_DIMENSION_COUNT_V1: usize = 15;
const CONTEXT_MAX_TURNS_V1: usize = 32;
const CONTEXT_TURN_WIRE_LEN_V1: usize =
    16 + 32 + 8 + (CONTEXT_DIMENSION_COUNT_V1 * 8) + 1 + 1 + 8 + 1;
const CONTEXT_HEADER_WIRE_LEN_V1: usize = 8 + 4 + 32 + 4 + 4;
const CONTEXT_STATE_WIRE_MAX_LEN_V1: usize =
    CONTEXT_HEADER_WIRE_LEN_V1 + (CONTEXT_MAX_TURNS_V1 * CONTEXT_TURN_WIRE_LEN_V1);
const SEMANTIC_MIGRATION_EVENT_ID_DOMAIN_V2: &[u8] =
    b"astr-embodiment/store-semantic-migration-event-id-v2";
const SEMANTIC_MIGRATION_TURN_ID_DOMAIN_V2: &[u8] =
    b"astr-embodiment/store-semantic-migration-turn-id-v2";
const SEMANTIC_TRANSITION_COMMITMENT_DOMAIN_V1: &[u8] =
    b"astr-embodiment/store-semantic-transition-commitment-v1";
const SEMANTIC_MIGRATION_LOCAL_DOMAIN_V1: &[u8] =
    b"astr-embodiment/store-semantic-migration-local-v1";
const SEMANTIC_MIGRATION_EFFECTIVE_DOMAIN_V1: &[u8] =
    b"astr-embodiment/store-semantic-migration-effective-v1";
const SEMANTIC_MIGRATION_SNAPSHOT_DOMAIN_V1: &[u8] =
    b"astr-embodiment/store-semantic-migration-snapshot-v1";
const SEMANTIC_MIGRATION_AUTHORITY_RECEIPT_DOMAIN_V1: &[u8] =
    b"astr-embodiment/store-semantic-migration-authority-receipt-v1";
const MAX_SEMANTIC_HISTORY_ROWS: u64 = 65_536;
const MAX_SEMANTIC_HISTORY_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_GENESIS_MANIFEST_BYTES: u64 = 256 * 1024;
const MAX_GENESIS_SOURCE_JSON_BYTES: u64 = 64 * 1024;
const MAX_LEGACY_UPGRADE_BYTES: u64 = 4 * 1024;
const CONTEXT_RELATION_HMAC_KEY_V1: [u8; 32] = [
    0x6d, 0x18, 0x4a, 0xf3, 0x82, 0x97, 0x51, 0x0c, 0x34, 0xbe, 0x76, 0x29, 0xd1, 0x45, 0xa8, 0x63,
    0x9b, 0x27, 0xce, 0x40, 0x75, 0x1f, 0xe2, 0x5a, 0xb4, 0x08, 0x9d, 0x36, 0xc1, 0x7e, 0x52, 0xfa,
];

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
    pub field_domain: Option<LegacySemanticFieldDomainUpgradeV1>,
    pub migration_id: Digest,
}

/// Selector/CAS-only request for the unique finite semantic migration. It
/// deliberately cannot carry events, receipts, telemetry, or snapshot bytes.
pub struct SemanticMigrationRequestV2<'a> {
    pub scope: &'a ScopeRef,
    pub expected_source_revision: u64,
    pub expected_source_state_digest: Digest,
    pub expected_source_graph_digest: Digest,
    pub expected_source_history_root: Digest,
    pub expected_incarnation_id: Digest,
    pub expected_manifest_digest: Digest,
}

struct SemanticMigrationArtifactsV2 {
    envelope: CommitEnvelope,
    target_state_bytes: Vec<u8>,
    incarnation_id: Digest,
    manifest_digest: Digest,
    commitment_digest: Digest,
    telemetry_digest: Digest,
    snapshot_wire_digest: Digest,
    authority_receipt_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SemanticTransitionCommitmentV1 {
    scope_digest: Digest,
    event_digest: Digest,
    authority_digest: Digest,
    formula_digest: Digest,
    source_history_root: Digest,
    source_state_digest: Digest,
    base_revision: u64,
    next_revision: u64,
    state_before: Digest,
    state_after: Digest,
    graph_before: Digest,
    graph_after: Digest,
    active_nodes: u32,
    active_edges: u32,
    residuals: InvariantResiduals,
}

impl SemanticTransitionCommitmentV1 {
    fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(32 * 10 + 16 + 8 + 4 * 2 + 8 * 5);
        out.extend_from_slice(b"AE-STC1\0");
        for digest in [
            self.scope_digest,
            self.event_digest,
            self.authority_digest,
            self.formula_digest,
            self.source_history_root,
            self.source_state_digest,
        ] {
            out.extend_from_slice(&digest);
        }
        out.extend_from_slice(&self.base_revision.to_le_bytes());
        out.extend_from_slice(&self.next_revision.to_le_bytes());
        for digest in [
            self.state_before,
            self.state_after,
            self.graph_before,
            self.graph_after,
        ] {
            out.extend_from_slice(&digest);
        }
        out.extend_from_slice(&self.active_nodes.to_le_bytes());
        out.extend_from_slice(&self.active_edges.to_le_bytes());
        for value in [
            self.residuals.authority,
            self.residuals.continuity,
            self.residuals.energy,
            self.residuals.renormalization,
            self.residuals.capacity,
        ] {
            out.extend_from_slice(&value.encode());
        }
        out
    }

    fn digest(&self) -> Digest {
        wire::domain_hash(
            SEMANTIC_TRANSITION_COMMITMENT_DOMAIN_V1,
            &[&self.canonical_bytes()],
        )
    }
}

struct SemanticMigrationAuthorityReceiptV1<'a> {
    migration_id: Digest,
    commitment_digest: Digest,
    telemetry_digest: Digest,
    snapshot_wire_digest: Digest,
    authority_digest: Digest,
    upgrade_bytes: &'a [u8],
    transition_receipt_bytes: &'a [u8],
}

impl SemanticMigrationAuthorityReceiptV1<'_> {
    fn digest(&self) -> Digest {
        wire::domain_hash(
            SEMANTIC_MIGRATION_AUTHORITY_RECEIPT_DOMAIN_V1,
            &[
                &self.migration_id,
                &self.commitment_digest,
                &self.telemetry_digest,
                &self.snapshot_wire_digest,
                &self.authority_digest,
                self.upgrade_bytes,
                self.transition_receipt_bytes,
            ],
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticMigrationOutcomeV1 {
    NotRequired,
    Migrated {
        from_revision: u64,
        to_revision: u64,
        backup_digest: Digest,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TransitionReceiptV2 {
    schema_version: u16,
    formula_digest: Digest,
    scope_digest: Digest,
    event_digest: Digest,
    authority_digest: Digest,
    base_revision: u64,
    next_revision: u64,
    state_before: Digest,
    state_after: Digest,
    graph_after: Digest,
    action_contract: Option<Digest>,
    active_nodes: u32,
    active_edges: u32,
    residuals: InvariantResiduals,
    status: CommitStatus,
    semantic_vector: SemanticVectorReceiptV2,
}

impl TransitionReceiptV2 {
    const SCHEMA_VERSION: u16 = 2;

    fn validate(&self) -> bool {
        self.schema_version == Self::SCHEMA_VERSION
            && self.status == CommitStatus::Committed
            && self.action_contract.is_none()
            && self.base_revision.checked_add(1) == Some(self.next_revision)
            && self.semantic_vector.validate()
            && self.semantic_vector.state_changed == (self.state_before != self.state_after)
    }
}

pub(crate) struct DecodedSemanticSnapshotV2 {
    pub(crate) field: NeuralField,
    pub(crate) graph: SparseGraph,
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], StoreError> {
        let end = self
            .position
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(StoreError::ContinuityFence("semantic_snapshot_wire"))?;
        let value = &self.bytes[self.position..end];
        self.position = end;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16, StoreError> {
        let mut value = [0_u8; 2];
        value.copy_from_slice(self.take(2)?);
        Ok(u16::from_le_bytes(value))
    }

    fn u8(&mut self) -> Result<u8, StoreError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, StoreError> {
        let mut value = [0_u8; 4];
        value.copy_from_slice(self.take(4)?);
        Ok(u32::from_le_bytes(value))
    }

    fn u64(&mut self) -> Result<u64, StoreError> {
        let mut value = [0_u8; 8];
        value.copy_from_slice(self.take(8)?);
        Ok(u64::from_le_bytes(value))
    }

    fn digest(&mut self) -> Result<Digest, StoreError> {
        let mut value = [0_u8; 32];
        value.copy_from_slice(self.take(32)?);
        Ok(value)
    }

    fn bool(&mut self) -> Result<bool, StoreError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(StoreError::ContinuityFence("semantic_snapshot_wire")),
        }
    }

    fn opt_digest(&mut self) -> Result<Option<Digest>, StoreError> {
        if self.bool()? {
            Ok(Some(self.digest()?))
        } else {
            Ok(None)
        }
    }

    fn fixed(&mut self) -> Result<Fixed, StoreError> {
        let mut value = [0_u8; 8];
        value.copy_from_slice(self.take(8)?);
        Ok(Fixed::decode(value))
    }

    fn eof(&self) -> bool {
        self.position == self.bytes.len()
    }
}

/// Parse the current AESEM3 envelope. Schema 3 has exactly four mandatory
/// blocks; the fourth is the fixed-width retired-compensation zero block.
/// This is shared by Runtime and Store so a byte string cannot cross one
/// authority boundary while being rejected by the other.
#[doc(hidden)]
pub fn decode_canonical_aesem3_blocks(
    bytes: &[u8],
) -> Result<CanonicalAesem3Blocks<'_>, CanonicalAesem3Error> {
    let mut cursor = Cursor::new(bytes);
    let magic = cursor
        .take(AESEM3_MAGIC.len())
        .map_err(|_| CanonicalAesem3Error::WireInvalid)?;
    let schema = cursor
        .u16()
        .map_err(|_| CanonicalAesem3Error::WireInvalid)?;
    if magic != AESEM3_MAGIC || schema != AESEM3_SCHEMA {
        return Err(CanonicalAesem3Error::MagicOrSchema);
    }

    let field_len = usize::try_from(
        cursor
            .u32()
            .map_err(|_| CanonicalAesem3Error::WireInvalid)?,
    )
    .map_err(|_| CanonicalAesem3Error::WireInvalid)?;
    if field_len != FIELD_WIRE_LEN {
        return Err(CanonicalAesem3Error::WireInvalid);
    }
    let field = cursor
        .take(field_len)
        .map_err(|_| CanonicalAesem3Error::WireInvalid)?;

    let graph_len = usize::try_from(
        cursor
            .u32()
            .map_err(|_| CanonicalAesem3Error::WireInvalid)?,
    )
    .map_err(|_| CanonicalAesem3Error::WireInvalid)?;
    if !(GRAPH_WIRE_MIN_LEN..=GRAPH_WIRE_MAX_LEN).contains(&graph_len) {
        return Err(CanonicalAesem3Error::WireInvalid);
    }
    let graph = cursor
        .take(graph_len)
        .map_err(|_| CanonicalAesem3Error::WireInvalid)?;

    let telemetry_len = usize::try_from(
        cursor
            .u32()
            .map_err(|_| CanonicalAesem3Error::WireInvalid)?,
    )
    .map_err(|_| CanonicalAesem3Error::WireInvalid)?;
    if telemetry_len != NATIVE_TELEMETRY_RECEIPT_V1_WIRE_LEN {
        return Err(CanonicalAesem3Error::WireInvalid);
    }
    let telemetry = cursor
        .take(telemetry_len)
        .map_err(|_| CanonicalAesem3Error::WireInvalid)?;

    let reserved_len = usize::try_from(
        cursor
            .u32()
            .map_err(|_| CanonicalAesem3Error::WireInvalid)?,
    )
    .map_err(|_| CanonicalAesem3Error::WireInvalid)?;
    if reserved_len != REGION_LAYOUT.len() * 8 {
        return Err(CanonicalAesem3Error::WireInvalid);
    }
    let reserved = cursor
        .take(reserved_len)
        .map_err(|_| CanonicalAesem3Error::WireInvalid)?;
    if !cursor.eof() {
        return Err(CanonicalAesem3Error::WireInvalid);
    }
    for raw in reserved.as_chunks::<8>().0 {
        if Fixed::decode(*raw) != Fixed::ZERO {
            return Err(CanonicalAesem3Error::ReservedNonzero);
        }
    }

    Ok(CanonicalAesem3Blocks {
        field,
        graph,
        telemetry,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ContextTurnV1 {
    event_id: [u8; 16],
    receipt_digest: Digest,
    source_revision: u64,
    dimensions: [i64; CONTEXT_DIMENSION_COUNT_V1],
    unresolved_boundary: bool,
    unresolved_repair: bool,
    repetition_increment: u64,
    delivery_outcome: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ContextProjectionStateV1 {
    relation_hmac: Digest,
    summary_revision: u32,
    turns: Vec<ContextTurnV1>,
}

struct ContextCursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> ContextCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N], StoreError> {
        let end = self
            .position
            .checked_add(N)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(StoreError::ContinuityFence("semantic_context_wire"))?;
        let value = self.bytes[self.position..end]
            .try_into()
            .map_err(|_| StoreError::ContinuityFence("semantic_context_wire"))?;
        self.position = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, StoreError> {
        Ok(self.take::<1>()?[0])
    }

    fn bool(&mut self) -> Result<bool, StoreError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(StoreError::ContinuityFence("semantic_context_wire")),
        }
    }

    fn u32(&mut self) -> Result<u32, StoreError> {
        Ok(u32::from_le_bytes(self.take()?))
    }

    fn u64(&mut self) -> Result<u64, StoreError> {
        Ok(u64::from_le_bytes(self.take()?))
    }

    fn i64(&mut self) -> Result<i64, StoreError> {
        Ok(i64::from_le_bytes(self.take()?))
    }

    fn eof(&self) -> bool {
        self.position == self.bytes.len()
    }
}

fn context_relation_hmac(relation_scope_token: [u8; 16]) -> Digest {
    let mut inner_pad = [0x36_u8; 64];
    let mut outer_pad = [0x5c_u8; 64];
    for (index, key) in CONTEXT_RELATION_HMAC_KEY_V1.iter().enumerate() {
        inner_pad[index] ^= key;
        outer_pad[index] ^= key;
    }
    let mut inner = Vec::with_capacity(80);
    inner.extend_from_slice(&inner_pad);
    inner.extend_from_slice(&relation_scope_token);
    let inner_digest: Digest = Sha256::digest(&inner).into();
    let mut outer = Vec::with_capacity(96);
    outer.extend_from_slice(&outer_pad);
    outer.extend_from_slice(&inner_digest);
    Sha256::digest(&outer).into()
}

fn context_dimensions(event: &CanonicalEvent) -> Result<[i64; 15], StoreError> {
    let CanonicalEvent::UserStimulus(stimulus) = event else {
        return Err(StoreError::ContinuityFence("semantic_history_context"));
    };
    let dimensions = &stimulus.evidence.dimensions;
    Ok([
        dimensions.positive,
        dimensions.affiliation,
        dimensions.harm,
        dimensions.boundary,
        dimensions.repair,
        dimensions.repetition,
        dimensions.new_information,
        dimensions.constraint_instability,
        dimensions.epistemic_conflict,
        dimensions.self_responsibility,
        dimensions.other_responsibility,
        dimensions.hostility,
        dimensions.publicness,
        dimensions.engagement,
        dimensions.rejection,
    ]
    .map(|value| value.raw().clamp(0, 1_000_000)))
}

fn context_receipt_digest(
    event: &CanonicalEvent,
    relation_scope_token: [u8; 16],
    source_revision: u64,
) -> Result<Digest, StoreError> {
    let CanonicalEvent::UserStimulus(stimulus) = event else {
        return Err(StoreError::ContinuityFence("semantic_history_context"));
    };
    if stimulus.event_id == [0; 16] || relation_scope_token == [0; 16] || source_revision == 0 {
        return Err(StoreError::ContinuityFence("semantic_history_context"));
    }
    let dimensions = context_dimensions(event)?;
    let mut bytes = Vec::with_capacity(177);
    bytes.extend_from_slice(b"AECRPTV1");
    bytes.extend_from_slice(&stimulus.event_id);
    bytes.extend_from_slice(&relation_scope_token);
    bytes.extend_from_slice(&source_revision.to_le_bytes());
    for dimension in dimensions {
        bytes.extend_from_slice(&dimension.to_le_bytes());
    }
    bytes.push(u8::from(stimulus.evidence.dimensions.boundary.raw() > 0));
    bytes.push(u8::from(stimulus.evidence.dimensions.repair.raw() > 0));
    bytes.extend_from_slice(&1_u64.to_le_bytes());
    bytes.push(0);
    Ok(*blake3::hash(&bytes).as_bytes())
}

fn encode_context_state(state: &ContextProjectionStateV1) -> Result<Vec<u8>, StoreError> {
    if state.relation_hmac == [0; 32]
        || state.summary_revision == 0
        || state.turns.is_empty()
        || state.turns.len() > CONTEXT_MAX_TURNS_V1
    {
        return Err(StoreError::ContinuityFence("semantic_context_canonical"));
    }
    let turns_bytes = state
        .turns
        .len()
        .checked_mul(CONTEXT_TURN_WIRE_LEN_V1)
        .ok_or(StoreError::ContinuityFence("semantic_context_wire"))?;
    let capacity = CONTEXT_HEADER_WIRE_LEN_V1
        .checked_add(turns_bytes)
        .ok_or(StoreError::ContinuityFence("semantic_context_wire"))?;
    let mut out = Vec::with_capacity(capacity);
    out.extend_from_slice(CONTEXT_STATE_MAGIC_V1);
    out.extend_from_slice(&CONTEXT_STATE_SCHEMA_V1.to_le_bytes());
    out.extend_from_slice(&state.relation_hmac);
    out.extend_from_slice(&state.summary_revision.to_le_bytes());
    out.extend_from_slice(
        &u32::try_from(state.turns.len())
            .map_err(|_| StoreError::ContinuityFence("semantic_context_wire"))?
            .to_le_bytes(),
    );
    let mut previous_revision = 0_u64;
    for turn in &state.turns {
        if turn.event_id == [0; 16]
            || turn.receipt_digest == [0; 32]
            || turn.source_revision == 0
            || turn.source_revision <= previous_revision
            || turn
                .dimensions
                .iter()
                .any(|dimension| !(0..=1_000_000).contains(dimension))
            || !(1..=4_096).contains(&turn.repetition_increment)
            || turn.delivery_outcome > 2
        {
            return Err(StoreError::ContinuityFence("semantic_context_canonical"));
        }
        previous_revision = turn.source_revision;
        out.extend_from_slice(&turn.event_id);
        out.extend_from_slice(&turn.receipt_digest);
        out.extend_from_slice(&turn.source_revision.to_le_bytes());
        for dimension in turn.dimensions {
            out.extend_from_slice(&dimension.to_le_bytes());
        }
        out.push(u8::from(turn.unresolved_boundary));
        out.push(u8::from(turn.unresolved_repair));
        out.extend_from_slice(&turn.repetition_increment.to_le_bytes());
        out.push(turn.delivery_outcome);
    }
    Ok(out)
}

fn decode_context_state(bytes: &[u8]) -> Result<ContextProjectionStateV1, StoreError> {
    if bytes.len() < CONTEXT_HEADER_WIRE_LEN_V1 {
        return Err(StoreError::ContinuityFence("semantic_context_wire"));
    }
    let mut cursor = ContextCursor::new(bytes);
    if cursor.take::<8>()? != *CONTEXT_STATE_MAGIC_V1 || cursor.u32()? != CONTEXT_STATE_SCHEMA_V1 {
        return Err(StoreError::ContinuityFence("semantic_context_wire"));
    }
    let relation_hmac = cursor.take::<32>()?;
    let summary_revision = cursor.u32()?;
    let turn_count = usize::try_from(cursor.u32()?)
        .map_err(|_| StoreError::ContinuityFence("semantic_context_wire"))?;
    if relation_hmac == [0; 32]
        || summary_revision == 0
        || turn_count == 0
        || turn_count > CONTEXT_MAX_TURNS_V1
    {
        return Err(StoreError::ContinuityFence("semantic_context_wire"));
    }
    let expected_len = CONTEXT_HEADER_WIRE_LEN_V1
        .checked_add(
            turn_count
                .checked_mul(CONTEXT_TURN_WIRE_LEN_V1)
                .ok_or(StoreError::ContinuityFence("semantic_context_wire"))?,
        )
        .ok_or(StoreError::ContinuityFence("semantic_context_wire"))?;
    if bytes.len() != expected_len {
        return Err(StoreError::ContinuityFence("semantic_context_wire"));
    }
    let mut previous_revision = 0_u64;
    let mut turns = Vec::with_capacity(turn_count);
    for _ in 0..turn_count {
        let event_id = cursor.take::<16>()?;
        let receipt_digest = cursor.take::<32>()?;
        let source_revision = cursor.u64()?;
        let mut dimensions = [0_i64; CONTEXT_DIMENSION_COUNT_V1];
        for dimension in &mut dimensions {
            *dimension = cursor.i64()?;
        }
        let unresolved_boundary = cursor.bool()?;
        let unresolved_repair = cursor.bool()?;
        let repetition_increment = cursor.u64()?;
        let delivery_outcome = cursor.u8()?;
        if event_id == [0; 16]
            || receipt_digest == [0; 32]
            || source_revision == 0
            || source_revision <= previous_revision
            || dimensions
                .iter()
                .any(|dimension| !(0..=1_000_000).contains(dimension))
            || !(1..=4_096).contains(&repetition_increment)
            || delivery_outcome > 2
        {
            return Err(StoreError::ContinuityFence("semantic_context_wire"));
        }
        previous_revision = source_revision;
        turns.push(ContextTurnV1 {
            event_id,
            receipt_digest,
            source_revision,
            dimensions,
            unresolved_boundary,
            unresolved_repair,
            repetition_increment,
            delivery_outcome,
        });
    }
    if !cursor.eof() {
        return Err(StoreError::ContinuityFence("semantic_context_wire"));
    }
    let state = ContextProjectionStateV1 {
        relation_hmac,
        summary_revision,
        turns,
    };
    if encode_context_state(&state)? != bytes {
        return Err(StoreError::ContinuityFence("semantic_context_canonical"));
    }
    Ok(state)
}

fn project_context_state(
    previous_state: Option<&[u8]>,
    event: &CanonicalEvent,
    relation_scope_token: [u8; 16],
    source_revision: u64,
) -> Result<Vec<u8>, StoreError> {
    let CanonicalEvent::UserStimulus(stimulus) = event else {
        return Err(StoreError::ContinuityFence("semantic_history_context"));
    };
    let expected_hmac = context_relation_hmac(relation_scope_token);
    let mut state = match previous_state {
        Some(bytes) => {
            let decoded = decode_context_state(bytes)?;
            if decoded.relation_hmac != expected_hmac
                || decoded
                    .turns
                    .last()
                    .is_none_or(|turn| source_revision <= turn.source_revision)
                || decoded
                    .turns
                    .iter()
                    .any(|turn| turn.event_id == stimulus.event_id)
            {
                return Err(StoreError::ContinuityFence("semantic_history_context"));
            }
            decoded
        }
        None => ContextProjectionStateV1 {
            relation_hmac: expected_hmac,
            summary_revision: 0,
            turns: Vec::new(),
        },
    };
    state.summary_revision = state
        .summary_revision
        .checked_add(1)
        .ok_or(StoreError::ContinuityFence("semantic_history_context"))?;
    state.turns.push(ContextTurnV1 {
        event_id: stimulus.event_id,
        receipt_digest: context_receipt_digest(event, relation_scope_token, source_revision)?,
        source_revision,
        dimensions: context_dimensions(event)?,
        unresolved_boundary: stimulus.evidence.dimensions.boundary.raw() > 0,
        unresolved_repair: stimulus.evidence.dimensions.repair.raw() > 0,
        repetition_increment: 1,
        delivery_outcome: 0,
    });
    if state.turns.len() > CONTEXT_MAX_TURNS_V1 {
        state.turns.remove(0);
    }
    encode_context_state(&state)
}

fn continuity_context_digest(canonical_state_bytes: &[u8]) -> Digest {
    wire::domain_hash(
        CONTINUITY_CONTEXT_DIGEST_DOMAIN_V1,
        &[canonical_state_bytes],
    )
}

fn encode_field(field: &NeuralField) -> Result<Vec<u8>, StoreError> {
    if !field.validate() {
        return Err(StoreError::ContinuityFence("semantic_field_shape"));
    }
    let mut out = Vec::with_capacity(8 * (4 + NEURON_SLOTS * 8));
    for values in [
        &field.potential,
        &field.excitation,
        &field.inhibition,
        &field.adaptation,
        &field.precision,
        &field.prediction_error,
        &field.eligibility,
        &field.metabolic_reserve,
    ] {
        out.extend_from_slice(
            &(u32::try_from(values.len())
                .map_err(|_| StoreError::ContinuityFence("semantic_field_shape"))?)
            .to_le_bytes(),
        );
        for value in values {
            out.extend_from_slice(&value.encode());
        }
    }
    Ok(out)
}

fn decode_field(bytes: &[u8]) -> Result<NeuralField, StoreError> {
    if bytes.len() != FIELD_WIRE_LEN {
        return Err(StoreError::ContinuityFence("semantic_field_wire"));
    }
    let mut cursor = Cursor::new(bytes);
    let mut vectors = Vec::with_capacity(8);
    for _ in 0..8 {
        if usize::try_from(cursor.u32()?)
            .map_err(|_| StoreError::ContinuityFence("semantic_field_shape"))?
            != NEURON_SLOTS
        {
            return Err(StoreError::ContinuityFence("semantic_field_shape"));
        }
        let mut values = Vec::with_capacity(NEURON_SLOTS);
        for _ in 0..NEURON_SLOTS {
            values.push(cursor.fixed()?);
        }
        vectors.push(values);
    }
    if !cursor.eof() {
        return Err(StoreError::ContinuityFence("semantic_field_wire"));
    }
    let mut vectors = vectors.into_iter();
    let field = NeuralField {
        potential: vectors
            .next()
            .ok_or(StoreError::ContinuityFence("semantic_field_shape"))?,
        excitation: vectors
            .next()
            .ok_or(StoreError::ContinuityFence("semantic_field_shape"))?,
        inhibition: vectors
            .next()
            .ok_or(StoreError::ContinuityFence("semantic_field_shape"))?,
        adaptation: vectors
            .next()
            .ok_or(StoreError::ContinuityFence("semantic_field_shape"))?,
        precision: vectors
            .next()
            .ok_or(StoreError::ContinuityFence("semantic_field_shape"))?,
        prediction_error: vectors
            .next()
            .ok_or(StoreError::ContinuityFence("semantic_field_shape"))?,
        eligibility: vectors
            .next()
            .ok_or(StoreError::ContinuityFence("semantic_field_shape"))?,
        metabolic_reserve: vectors
            .next()
            .ok_or(StoreError::ContinuityFence("semantic_field_shape"))?,
    };
    if !field.validate() || encode_field(&field)? != bytes {
        return Err(StoreError::ContinuityFence("semantic_field_canonical"));
    }
    Ok(field)
}

fn encode_graph(graph: &SparseGraph) -> Result<Vec<u8>, StoreError> {
    if !graph.validate() {
        return Err(StoreError::ContinuityFence("semantic_graph_shape"));
    }
    Ok(graph.canonical_bytes())
}

fn decode_graph(bytes: &[u8]) -> Result<SparseGraph, StoreError> {
    if !(GRAPH_WIRE_MIN_LEN..=GRAPH_WIRE_MAX_LEN).contains(&bytes.len()) {
        return Err(StoreError::ContinuityFence("semantic_graph_wire"));
    }
    let mut cursor = Cursor::new(bytes);
    let offsets_len = usize::try_from(cursor.u32()?)
        .map_err(|_| StoreError::ContinuityFence("semantic_graph_shape"))?;
    if offsets_len != NEURON_SLOTS + 1 {
        return Err(StoreError::ContinuityFence("semantic_graph_shape"));
    }
    let mut row_offsets = Vec::with_capacity(offsets_len);
    for _ in 0..offsets_len {
        row_offsets.push(cursor.u32()?);
    }
    let edge_len = usize::try_from(cursor.u32()?)
        .map_err(|_| StoreError::ContinuityFence("semantic_graph_shape"))?;
    if edge_len > EDGE_CAPACITY {
        return Err(StoreError::ContinuityFence("semantic_graph_shape"));
    }
    let mut edges = Vec::with_capacity(edge_len);
    for _ in 0..edge_len {
        let target = cursor.u32()?;
        let mut weight = [0_u8; 2];
        weight.copy_from_slice(cursor.take(2)?);
        let mut eligibility = [0_u8; 2];
        eligibility.copy_from_slice(cursor.take(2)?);
        let mut stability = [0_u8; 2];
        stability.copy_from_slice(cursor.take(2)?);
        let mut last_used_epoch = [0_u8; 2];
        last_used_epoch.copy_from_slice(cursor.take(2)?);
        let operator_id = cursor.take(1)?[0];
        let delay_class = cursor.take(1)?[0];
        let mut flags = [0_u8; 2];
        flags.copy_from_slice(cursor.take(2)?);
        edges.push(Synapse {
            target,
            weight: i16::from_le_bytes(weight),
            eligibility: i16::from_le_bytes(eligibility),
            stability: u16::from_le_bytes(stability),
            last_used_epoch: u16::from_le_bytes(last_used_epoch),
            operator_id,
            delay_class,
            flags: u16::from_le_bytes(flags),
        });
    }
    if !cursor.eof() {
        return Err(StoreError::ContinuityFence("semantic_graph_wire"));
    }
    let graph = SparseGraph { row_offsets, edges };
    if !graph.validate() || encode_graph(&graph)? != bytes {
        return Err(StoreError::ContinuityFence("semantic_graph_canonical"));
    }
    Ok(graph)
}

fn encode_transition_receipt_v2(receipt: &TransitionReceiptV2) -> Vec<u8> {
    let mut out = Vec::with_capacity(TRANSITION_RECEIPT_V2_WIRE_LEN);
    out.extend_from_slice(&receipt.schema_version.to_le_bytes());
    for digest in [
        receipt.formula_digest,
        receipt.scope_digest,
        receipt.event_digest,
        receipt.authority_digest,
    ] {
        out.extend_from_slice(&digest);
    }
    out.extend_from_slice(&receipt.base_revision.to_le_bytes());
    out.extend_from_slice(&receipt.next_revision.to_le_bytes());
    for digest in [
        receipt.state_before,
        receipt.state_after,
        receipt.graph_after,
    ] {
        out.extend_from_slice(&digest);
    }
    match receipt.action_contract {
        None => out.push(0),
        Some(digest) => {
            out.push(1);
            out.extend_from_slice(&digest);
        }
    }
    out.extend_from_slice(&receipt.active_nodes.to_le_bytes());
    out.extend_from_slice(&receipt.active_edges.to_le_bytes());
    for value in [
        receipt.residuals.authority,
        receipt.residuals.continuity,
        receipt.residuals.energy,
        receipt.residuals.renormalization,
        receipt.residuals.capacity,
    ] {
        out.extend_from_slice(&value.encode());
    }
    out.push(wire::commit_status_code(receipt.status));
    out.extend_from_slice(&receipt.semantic_vector.schema_version.to_le_bytes());
    out.push(match receipt.semantic_vector.formula {
        SemanticVectorFormulaV2::FullVectorRouteNeutralRelaxationV1 => 1,
    });
    out.push(receipt.semantic_vector.dimension_slot_count);
    out.push(receipt.semantic_vector.evaluated_dimension_count);
    out.push(receipt.semantic_vector.injected_dimension_count);
    out.push(receipt.semantic_vector.nonzero_evidence_dimension_count);
    out.push(receipt.semantic_vector.neutral_baseline_dimension_count);
    out.push(receipt.semantic_vector.unavailable_dimension_count);
    out.push(u8::from(receipt.semantic_vector.state_changed));
    out
}

fn decode_transition_receipt_v2(bytes: &[u8]) -> Result<TransitionReceiptV2, StoreError> {
    if bytes.len() != TRANSITION_RECEIPT_V2_WIRE_LEN {
        return Err(StoreError::ContinuityFence("semantic_aesem2_receipt"));
    }
    let mut cursor = Cursor::new(bytes);
    let schema_version = cursor.u16()?;
    let receipt = TransitionReceiptV2 {
        schema_version,
        formula_digest: cursor.digest()?,
        scope_digest: cursor.digest()?,
        event_digest: cursor.digest()?,
        authority_digest: cursor.digest()?,
        base_revision: cursor.u64()?,
        next_revision: cursor.u64()?,
        state_before: cursor.digest()?,
        state_after: cursor.digest()?,
        graph_after: cursor.digest()?,
        action_contract: cursor.opt_digest()?,
        active_nodes: cursor.u32()?,
        active_edges: cursor.u32()?,
        residuals: InvariantResiduals {
            authority: cursor.fixed()?,
            continuity: cursor.fixed()?,
            energy: cursor.fixed()?,
            renormalization: cursor.fixed()?,
            capacity: cursor.fixed()?,
        },
        status: wire::commit_status_from_code(cursor.u8()?)
            .ok_or(StoreError::ContinuityFence("semantic_aesem2_receipt"))?,
        semantic_vector: SemanticVectorReceiptV2 {
            schema_version: cursor.u16()?,
            formula: match cursor.u8()? {
                1 => SemanticVectorFormulaV2::FullVectorRouteNeutralRelaxationV1,
                _ => return Err(StoreError::ContinuityFence("semantic_aesem2_receipt")),
            },
            dimension_slot_count: cursor.u8()?,
            evaluated_dimension_count: cursor.u8()?,
            injected_dimension_count: cursor.u8()?,
            nonzero_evidence_dimension_count: cursor.u8()?,
            neutral_baseline_dimension_count: cursor.u8()?,
            unavailable_dimension_count: cursor.u8()?,
            state_changed: cursor.bool()?,
        },
    };
    if !cursor.eof() || !receipt.validate() || encode_transition_receipt_v2(&receipt) != bytes {
        return Err(StoreError::ContinuityFence("semantic_aesem2_receipt"));
    }
    Ok(receipt)
}

fn encode_native_telemetry_receipt_v1(receipt: &NativeTelemetryReceiptV1) -> Vec<u8> {
    let mut out = Vec::with_capacity(NATIVE_TELEMETRY_RECEIPT_V1_WIRE_LEN);
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.push(match receipt.formula {
        NativeTelemetryFormulaV1::Phase0NativePropagationFxp6V1 => 1,
    });
    out.push(match receipt.phase {
        NativeTelemetryPhaseV1::Prepare => 1,
    });
    for digest in [
        receipt.formula_digest,
        receipt.scope_digest,
        receipt.event_digest,
        receipt.source_digest,
    ] {
        out.extend_from_slice(&digest);
    }
    out.extend_from_slice(&receipt.base_revision.to_le_bytes());
    out.extend_from_slice(&receipt.next_revision.to_le_bytes());
    for digest in [
        receipt.state_before,
        receipt.state_after,
        receipt.graph_before,
        receipt.graph_after,
        receipt.local_digest,
        receipt.compensation_digest,
        receipt.effective_digest,
    ] {
        out.extend_from_slice(&digest);
    }
    for value in [
        receipt.energy.reserve_before,
        receipt.energy.reserve_after,
        receipt.energy.recovered,
        receipt.energy.spent,
        receipt.energy.headroom,
        receipt.energy.residual,
    ] {
        out.extend_from_slice(&value.encode());
    }
    out.extend_from_slice(&receipt.capacity.upper_saturated_nodes.to_le_bytes());
    out.extend_from_slice(&receipt.capacity.node_limit.to_le_bytes());
    out.extend_from_slice(&receipt.capacity.node_headroom.encode());
    out.extend_from_slice(&receipt.capacity.edge_used.to_le_bytes());
    out.extend_from_slice(&receipt.capacity.edge_limit.to_le_bytes());
    for value in [
        receipt.capacity.edge_headroom,
        receipt.capacity.headroom,
        receipt.capacity.residual,
        receipt.residuals.authority,
        receipt.residuals.continuity,
        receipt.residuals.energy,
        receipt.residuals.renormalization,
        receipt.residuals.capacity,
        receipt.residual_health,
        receipt.native_gate,
    ] {
        out.extend_from_slice(&value.encode());
    }
    out.extend_from_slice(&receipt.checkpoint_digest);
    out.extend_from_slice(&receipt.telemetry_digest);
    out
}

fn decode_native_telemetry_receipt_v1(
    bytes: &[u8],
) -> Result<NativeTelemetryReceiptV1, StoreError> {
    if bytes.len() != NATIVE_TELEMETRY_RECEIPT_V1_WIRE_LEN {
        return Err(StoreError::ContinuityFence("semantic_aesem3_receipt"));
    }
    let mut cursor = Cursor::new(bytes);
    if cursor.u16()? != 1 {
        return Err(StoreError::ContinuityFence("semantic_aesem3_receipt"));
    }
    let formula = match cursor.u8()? {
        1 => NativeTelemetryFormulaV1::Phase0NativePropagationFxp6V1,
        _ => return Err(StoreError::ContinuityFence("semantic_aesem3_receipt")),
    };
    let phase = match cursor.u8()? {
        1 => NativeTelemetryPhaseV1::Prepare,
        _ => return Err(StoreError::ContinuityFence("semantic_aesem3_receipt")),
    };
    let receipt = NativeTelemetryReceiptV1 {
        schema: NATIVE_TELEMETRY_RECEIPT_SCHEMA_V1.to_owned(),
        formula,
        phase,
        formula_digest: cursor.digest()?,
        scope_digest: cursor.digest()?,
        event_digest: cursor.digest()?,
        source_digest: cursor.digest()?,
        base_revision: cursor.u64()?,
        next_revision: cursor.u64()?,
        state_before: cursor.digest()?,
        state_after: cursor.digest()?,
        graph_before: cursor.digest()?,
        graph_after: cursor.digest()?,
        local_digest: cursor.digest()?,
        compensation_digest: cursor.digest()?,
        effective_digest: cursor.digest()?,
        energy: EnergyTelemetryV1 {
            reserve_before: cursor.fixed()?,
            reserve_after: cursor.fixed()?,
            recovered: cursor.fixed()?,
            spent: cursor.fixed()?,
            headroom: cursor.fixed()?,
            residual: cursor.fixed()?,
        },
        capacity: CapacityTelemetryV1 {
            upper_saturated_nodes: cursor.u32()?,
            node_limit: cursor.u32()?,
            node_headroom: cursor.fixed()?,
            edge_used: cursor.u32()?,
            edge_limit: cursor.u32()?,
            edge_headroom: cursor.fixed()?,
            headroom: cursor.fixed()?,
            residual: cursor.fixed()?,
        },
        residuals: InvariantResiduals {
            authority: cursor.fixed()?,
            continuity: cursor.fixed()?,
            energy: cursor.fixed()?,
            renormalization: cursor.fixed()?,
            capacity: cursor.fixed()?,
        },
        residual_health: cursor.fixed()?,
        native_gate: cursor.fixed()?,
        checkpoint_digest: cursor.digest()?,
        telemetry_digest: cursor.digest()?,
    };
    if !cursor.eof() || !receipt.validate() || encode_native_telemetry_receipt_v1(&receipt) != bytes
    {
        return Err(StoreError::ContinuityFence("semantic_aesem3_receipt"));
    }
    Ok(receipt)
}

fn semantic_v2_matches_legacy_receipt(
    vector: &TransitionReceiptV2,
    legacy: &TransitionReceipt,
) -> bool {
    vector.schema_version == TransitionReceiptV2::SCHEMA_VERSION
        && vector.formula_digest == legacy.formula_digest
        && vector.scope_digest == legacy.scope_digest
        && vector.event_digest == legacy.event_digest
        && vector.authority_digest == legacy.authority_digest
        && vector.base_revision == legacy.base_revision
        && vector.next_revision == legacy.next_revision
        && vector.state_before == legacy.state_before
        && vector.state_after == legacy.state_after
        && vector.graph_after == legacy.graph_after
        && vector.action_contract == legacy.action_contract
        && vector.active_nodes == legacy.active_nodes
        && vector.active_edges == legacy.active_edges
        && vector.residuals == legacy.residuals
        && vector.status == legacy.status
}

fn semantic_v3_matches_legacy_receipt(
    telemetry: &NativeTelemetryReceiptV1,
    legacy: &TransitionReceipt,
) -> bool {
    legacy.schema_version == 1
        && legacy.status == CommitStatus::Committed
        && legacy.action_contract.is_none()
        && telemetry.validate()
        && telemetry.formula_digest == legacy.formula_digest
        && telemetry.scope_digest == legacy.scope_digest
        && telemetry.event_digest == legacy.event_digest
        && telemetry.base_revision == legacy.base_revision
        && telemetry.next_revision == legacy.next_revision
        && telemetry.state_before == legacy.state_before
        && telemetry.state_after == legacy.state_after
        && telemetry.graph_after == legacy.graph_after
        && telemetry.residuals == legacy.residuals
}

fn encode_snapshot_v2(
    field: &NeuralField,
    graph: &SparseGraph,
    receipt: &TransitionReceiptV2,
) -> Result<Vec<u8>, StoreError> {
    let field = encode_field(field)?;
    let graph = encode_graph(graph)?;
    let receipt = encode_transition_receipt_v2(receipt);
    let mut out =
        Vec::with_capacity(AESEM2_MAGIC.len() + 2 + 12 + field.len() + graph.len() + receipt.len());
    out.extend_from_slice(AESEM2_MAGIC);
    out.extend_from_slice(&AESEM2_SCHEMA.to_le_bytes());
    for block in [&field, &graph, &receipt] {
        out.extend_from_slice(
            &(u32::try_from(block.len())
                .map_err(|_| StoreError::ContinuityFence("semantic_snapshot_wire"))?)
            .to_le_bytes(),
        );
        out.extend_from_slice(block);
    }
    Ok(out)
}

pub(crate) fn decode_semantic_snapshot_v2(
    bytes: &[u8],
    expected_formula_digest: &Digest,
    expected_state_digest: &Digest,
    expected_graph_digest: &Digest,
    legacy_receipt: &TransitionReceipt,
) -> Result<DecodedSemanticSnapshotV2, StoreError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take(AESEM2_MAGIC.len())? != AESEM2_MAGIC || cursor.u16()? != AESEM2_SCHEMA {
        return Err(StoreError::ContinuityFence("semantic_aesem2_magic"));
    }
    let field_len = usize::try_from(cursor.u32()?)
        .map_err(|_| StoreError::ContinuityFence("semantic_snapshot_wire"))?;
    let field = decode_field(cursor.take(field_len)?)?;
    let graph_len = usize::try_from(cursor.u32()?)
        .map_err(|_| StoreError::ContinuityFence("semantic_snapshot_wire"))?;
    let graph = decode_graph(cursor.take(graph_len)?)?;
    let receipt_len = usize::try_from(cursor.u32()?)
        .map_err(|_| StoreError::ContinuityFence("semantic_snapshot_wire"))?;
    let receipt_bytes = cursor.take(receipt_len)?;
    if !cursor.eof() {
        return Err(StoreError::ContinuityFence("semantic_snapshot_wire"));
    }
    let vector_receipt = decode_transition_receipt_v2(receipt_bytes)?;
    if encode_transition_receipt_v2(&vector_receipt) != receipt_bytes
        || !vector_receipt.validate()
        || !semantic_v2_matches_legacy_receipt(&vector_receipt, legacy_receipt)
        || vector_receipt.formula_digest != *expected_formula_digest
        || vector_receipt.state_after != *expected_state_digest
        || vector_receipt.graph_after != *expected_graph_digest
        || state_digest(&field, expected_formula_digest) != *expected_state_digest
        || graph_digest(&graph) != *expected_graph_digest
        || encode_snapshot_v2(&field, &graph, &vector_receipt)? != bytes
    {
        return Err(StoreError::ContinuityFence("semantic_aesem2_closure"));
    }
    Ok(DecodedSemanticSnapshotV2 { field, graph })
}

fn encode_snapshot_v3(
    field: &NeuralField,
    graph: &SparseGraph,
    telemetry: &NativeTelemetryReceiptV1,
) -> Result<Vec<u8>, StoreError> {
    let field = encode_field(field)?;
    let graph = encode_graph(graph)?;
    let telemetry = encode_native_telemetry_receipt_v1(telemetry);
    let mut reserved = Vec::with_capacity(REGION_LAYOUT.len() * 8);
    for _ in REGION_LAYOUT {
        reserved.extend_from_slice(&Fixed::ZERO.encode());
    }
    let mut out = Vec::with_capacity(
        AESEM3_MAGIC.len() + 2 + 16 + field.len() + graph.len() + telemetry.len() + reserved.len(),
    );
    out.extend_from_slice(AESEM3_MAGIC);
    out.extend_from_slice(&AESEM3_SCHEMA.to_le_bytes());
    for block in [&field, &graph, &telemetry, &reserved] {
        out.extend_from_slice(
            &(u32::try_from(block.len())
                .map_err(|_| StoreError::ContinuityFence("semantic_snapshot_wire"))?)
            .to_le_bytes(),
        );
        out.extend_from_slice(block);
    }
    Ok(out)
}

/// Store-local canonical AESEM3 closure verification for an incoming migration
/// transition. The Store does not accept an opaque caller-provided snapshot.
#[doc(hidden)]
pub fn decode_canonical_semantic_snapshot_v3(
    bytes: &[u8],
    expected_formula_digest: &Digest,
    expected_state_digest: &Digest,
    expected_graph_digest: &Digest,
    legacy_receipt: &TransitionReceipt,
) -> Result<DecodedCanonicalSemanticSnapshotV3, StoreError> {
    let blocks = decode_canonical_aesem3_blocks(bytes).map_err(|error| match error {
        CanonicalAesem3Error::MagicOrSchema => StoreError::ContinuityFence("semantic_aesem3_magic"),
        CanonicalAesem3Error::WireInvalid => StoreError::ContinuityFence("semantic_snapshot_wire"),
        CanonicalAesem3Error::ReservedNonzero => {
            StoreError::ContinuityFence("semantic_aesem3_reserved")
        }
    })?;
    let field = decode_field(blocks.field)?;
    let graph = decode_graph(blocks.graph)?;
    let telemetry = decode_native_telemetry_receipt_v1(blocks.telemetry)?;
    if encode_native_telemetry_receipt_v1(&telemetry) != blocks.telemetry || !telemetry.validate() {
        return Err(StoreError::ContinuityFence("semantic_aesem3_receipt"));
    }
    if !semantic_v3_matches_legacy_receipt(&telemetry, legacy_receipt)
        || telemetry.formula_digest != *expected_formula_digest
        || telemetry.state_after != *expected_state_digest
        || telemetry.graph_after != *expected_graph_digest
        || telemetry.compensation_digest != legacy_reserved_zero_digest_v1()
        || state_digest(&field, expected_formula_digest) != *expected_state_digest
        || graph_digest(&graph) != *expected_graph_digest
        || encode_snapshot_v3(&field, &graph, &telemetry)? != bytes
    {
        return Err(StoreError::ContinuityFence("semantic_aesem3_closure"));
    }
    Ok(DecodedCanonicalSemanticSnapshotV3 {
        field,
        graph,
        telemetry,
    })
}

/// Store-derived claims for one current semantic sidecar.  Unlike the legacy
/// helper above, this accepts a semantic receipt whose revision is independent
/// from the main journal receipt.  All three byte strings are decoded and
/// re-encoded here, inside the Store authority crate, before a writer may
/// insert any row.
pub(crate) struct CanonicalSemanticSidecarClaimsV1 {
    pub(crate) formula_digest: Digest,
    pub(crate) scope_digest: Digest,
    pub(crate) event_digest: Digest,
    pub(crate) authority_digest: Digest,
    pub(crate) base_revision: u64,
    pub(crate) next_revision: u64,
    pub(crate) state_before: Digest,
    pub(crate) state_after: Digest,
    pub(crate) graph_before: Digest,
    pub(crate) graph_after: Digest,
    pub(crate) source_digest: Digest,
    pub(crate) telemetry_digest: Digest,
    pub(crate) active_nodes: u32,
    pub(crate) active_edges: u32,
    pub(crate) nonzero_evidence_dimension_count: u8,
    pub(crate) residuals: InvariantResiduals,
}

pub(crate) fn derive_canonical_semantic_sidecar_v1(
    snapshot_bytes: &[u8],
    receipt_bytes: &[u8],
    telemetry_bytes: &[u8],
) -> Result<CanonicalSemanticSidecarClaimsV1, StoreError> {
    let blocks = decode_canonical_aesem3_blocks(snapshot_bytes).map_err(|error| match error {
        CanonicalAesem3Error::MagicOrSchema => StoreError::ContinuityFence("semantic_aesem3_magic"),
        CanonicalAesem3Error::WireInvalid => StoreError::ContinuityFence("semantic_snapshot_wire"),
        CanonicalAesem3Error::ReservedNonzero => {
            StoreError::ContinuityFence("semantic_aesem3_reserved")
        }
    })?;
    if blocks.telemetry != telemetry_bytes {
        return Err(StoreError::ContinuityFence("semantic_telemetry_binding"));
    }
    let receipt = decode_transition_receipt_v2(receipt_bytes)?;
    let telemetry = decode_native_telemetry_receipt_v1(telemetry_bytes)?;
    if encode_transition_receipt_v2(&receipt) != receipt_bytes
        || encode_native_telemetry_receipt_v1(&telemetry) != telemetry_bytes
        || receipt.formula_digest != telemetry.formula_digest
        || receipt.scope_digest != telemetry.scope_digest
        || receipt.event_digest != telemetry.event_digest
        || receipt.base_revision != telemetry.base_revision
        || receipt.next_revision != telemetry.next_revision
        || receipt.state_before != telemetry.state_before
        || receipt.state_after != telemetry.state_after
        || receipt.graph_after != telemetry.graph_after
        || receipt.residuals != telemetry.residuals
    {
        return Err(StoreError::ContinuityFence(
            "semantic_sidecar_receipt_binding",
        ));
    }

    let field = decode_field(blocks.field)?;
    let graph = decode_graph(blocks.graph)?;
    if state_digest(&field, &receipt.formula_digest) != receipt.state_after
        || graph_digest(&graph) != receipt.graph_after
        || u32::try_from(graph.edges.len()).ok() != Some(receipt.active_edges)
        || telemetry.capacity.node_limit != u32::try_from(NEURON_SLOTS).unwrap_or(u32::MAX)
        || telemetry.capacity.edge_limit != u32::try_from(EDGE_CAPACITY).unwrap_or(u32::MAX)
        || telemetry.capacity.edge_used != receipt.active_edges
        || encode_snapshot_v3(&field, &graph, &telemetry)? != snapshot_bytes
    {
        return Err(StoreError::ContinuityFence("semantic_aesem3_closure"));
    }

    Ok(CanonicalSemanticSidecarClaimsV1 {
        formula_digest: receipt.formula_digest,
        scope_digest: receipt.scope_digest,
        event_digest: receipt.event_digest,
        authority_digest: receipt.authority_digest,
        base_revision: receipt.base_revision,
        next_revision: receipt.next_revision,
        state_before: receipt.state_before,
        state_after: receipt.state_after,
        graph_before: telemetry.graph_before,
        graph_after: receipt.graph_after,
        source_digest: telemetry.source_digest,
        telemetry_digest: telemetry.telemetry_digest,
        active_nodes: receipt.active_nodes,
        active_edges: receipt.active_edges,
        nonzero_evidence_dimension_count: receipt.semantic_vector.nonzero_evidence_dimension_count,
        residuals: receipt.residuals,
    })
}

pub(crate) fn verify_semantic_snapshot_v3(
    bytes: &[u8],
    expected_formula_digest: &Digest,
    expected_state_digest: &Digest,
    expected_graph_digest: &Digest,
    legacy_receipt: &TransitionReceipt,
) -> Result<Vec<u8>, StoreError> {
    Ok(decode_canonical_semantic_snapshot_v3(
        bytes,
        expected_formula_digest,
        expected_state_digest,
        expected_graph_digest,
        legacy_receipt,
    )?
    .graph
    .canonical_bytes())
}

#[derive(Clone, Debug)]
pub(crate) struct LegacyReplayTransitionV1 {
    pub(crate) next_field: NeuralField,
    pub(crate) active_nodes: u32,
}

fn legacy_component_update(
    current: Fixed,
    baseline: Fixed,
    drive: Fixed,
    neutral_rate: Fixed,
) -> Result<(Fixed, Fixed), StoreError> {
    let displacement = current.saturating_sub(baseline);
    let recovery = displacement
        .checked_mul(neutral_rate)
        .ok_or(StoreError::ContinuityFence("legacy_replay_arithmetic"))?;
    Ok((
        current.saturating_add(drive).saturating_sub(recovery),
        recovery,
    ))
}

/// Frozen predecessor writer used exclusively to authenticate legacy history.
pub(crate) fn replay_legacy_aesem2_transition_v1(
    field: &NeuralField,
    baseline: &NeuralField,
    dimensions: &EvidenceVector,
    estimator_confidence: Fixed,
) -> Result<LegacyReplayTransitionV1, StoreError> {
    if !field.validate()
        || !baseline.validate()
        || !(Fixed::ZERO < estimator_confidence && estimator_confidence <= Fixed::ONE)
    {
        return Err(StoreError::ContinuityFence("legacy_replay_input"));
    }
    let full_vector_load = assemble_full_vector_load(dimensions)
        .map_err(|_| StoreError::ContinuityFence("legacy_replay_input"))?;
    if full_vector_load.evaluated_dimension_count != 15
        || full_vector_load.injected_dimension_count != 15
    {
        return Err(StoreError::ContinuityFence("legacy_replay_input"));
    }
    let mut next_field = field.clone();
    let mut active_nodes = 0_u32;
    for (region, &(start, count)) in REGION_LAYOUT.iter().enumerate() {
        let drive = full_vector_load.evidence_means[region]
            .checked_mul(estimator_confidence)
            .ok_or(StoreError::ContinuityFence("legacy_replay_arithmetic"))?;
        let neutral_rate = full_vector_load.neutral_means[region]
            .checked_mul(LEGACY_NEUTRAL_RELAXATION_MAX_RATE)
            .ok_or(StoreError::ContinuityFence("legacy_replay_arithmetic"))?;
        let end = start
            .checked_add(count)
            .filter(|end| *end <= NEURON_SLOTS)
            .ok_or(StoreError::ContinuityFence("legacy_replay_shape"))?;
        for node in start..end {
            let (potential, potential_recovery) = legacy_component_update(
                field.potential[node],
                baseline.potential[node],
                drive,
                neutral_rate,
            )?;
            let (excitation, excitation_recovery) = legacy_component_update(
                field.excitation[node],
                baseline.excitation[node],
                drive,
                neutral_rate,
            )?;
            if drive == Fixed::ZERO
                && potential_recovery == Fixed::ZERO
                && excitation_recovery == Fixed::ZERO
            {
                continue;
            }
            active_nodes = active_nodes
                .checked_add(1)
                .ok_or(StoreError::ContinuityFence("legacy_replay_arithmetic"))?;
            next_field.potential[node] = potential;
            next_field.excitation[node] = excitation;
        }
    }
    if !next_field.validate() {
        return Err(StoreError::ContinuityFence("legacy_replay_shape"));
    }
    Ok(LegacyReplayTransitionV1 {
        next_field,
        active_nodes,
    })
}

fn checked_joint_scaled_fxp6(value: i64, common_max: i64) -> Result<i64, StoreError> {
    let numerator = i128::from(value)
        .checked_mul(i128::from(LEGACY_FIELD_FXP6_SCALE))
        .ok_or(StoreError::ContinuityFence("field_transform"))?;
    let denominator = i128::from(common_max);
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    let doubled_remainder = remainder
        .checked_mul(2)
        .ok_or(StoreError::ContinuityFence("field_transform"))?;
    let rounded = if doubled_remainder > denominator
        || (doubled_remainder == denominator && quotient % 2 != 0)
    {
        quotient
            .checked_add(1)
            .ok_or(StoreError::ContinuityFence("field_transform"))?
    } else {
        quotient
    };
    let scaled =
        i64::try_from(rounded).map_err(|_| StoreError::ContinuityFence("field_transform"))?;
    if !(0..=i64::from(LEGACY_FIELD_FXP6_SCALE)).contains(&scaled) {
        return Err(StoreError::ContinuityFence("field_transform"));
    }
    Ok(scaled)
}

/// Independently recompute the frozen joint P/E transform and its aggregate
/// receipt fields.  Caller-provided metadata is never used as an input.
pub(crate) fn normalize_legacy_aesem2_field_domain_v1(
    field: &NeuralField,
) -> Result<Option<(NeuralField, LegacySemanticFieldDomainUpgradeV1)>, StoreError> {
    if !field.validate() {
        return Err(StoreError::ContinuityFence("field_shape"));
    }
    for values in [
        &field.inhibition,
        &field.adaptation,
        &field.precision,
        &field.prediction_error,
        &field.eligibility,
        &field.metabolic_reserve,
    ] {
        if values
            .iter()
            .any(|value| !(0..=i64::from(LEGACY_FIELD_FXP6_SCALE)).contains(&value.raw()))
        {
            return Err(StoreError::ContinuityFence("field_nonpe_range"));
        }
    }
    let mut common_max = 0_i64;
    let mut out_of_range_count = 0_u32;
    let mut potential_out_of_range_count = 0_u32;
    let mut excitation_out_of_range_count = 0_u32;
    let mut signal_mass_before = 0_i128;
    for (values, component_count) in [
        (&field.potential, &mut potential_out_of_range_count),
        (&field.excitation, &mut excitation_out_of_range_count),
    ] {
        for value in values {
            let raw = value.raw();
            if raw < 0 {
                return Err(StoreError::ContinuityFence("field_pe_range"));
            }
            common_max = common_max.max(raw);
            signal_mass_before = signal_mass_before
                .checked_add(i128::from(raw))
                .ok_or(StoreError::ContinuityFence("field_transform"))?;
            if raw > i64::from(LEGACY_FIELD_FXP6_SCALE) {
                *component_count = component_count
                    .checked_add(1)
                    .ok_or(StoreError::ContinuityFence("field_transform"))?;
                out_of_range_count = out_of_range_count
                    .checked_add(1)
                    .ok_or(StoreError::ContinuityFence("field_transform"))?;
            }
        }
    }
    if common_max <= i64::from(LEGACY_FIELD_FXP6_SCALE) {
        return Ok(None);
    }
    let mut normalized = field.clone();
    let mut signal_mass_after = 0_i128;
    for (source, destination) in [
        (&field.potential, &mut normalized.potential),
        (&field.excitation, &mut normalized.excitation),
    ] {
        for (before, after) in source.iter().zip(destination.iter_mut()) {
            let scaled = checked_joint_scaled_fxp6(before.raw(), common_max)?;
            *after = Fixed::from_raw(scaled);
            signal_mass_after = signal_mass_after
                .checked_add(i128::from(scaled))
                .ok_or(StoreError::ContinuityFence("field_transform"))?;
        }
    }
    if !normalized.validate()
        || normalized
            .potential
            .iter()
            .chain(normalized.excitation.iter())
            .any(|value| !(0..=i64::from(LEGACY_FIELD_FXP6_SCALE)).contains(&value.raw()))
    {
        return Err(StoreError::ContinuityFence("field_transform"));
    }
    Ok(Some((
        normalized,
        LegacySemanticFieldDomainUpgradeV1 {
            algorithm: JOINT_MAX_LINEAR_FXP6_V1,
            fxp6_scale: LEGACY_FIELD_FXP6_SCALE,
            source_common_max: common_max,
            out_of_range_count,
            potential_out_of_range_count,
            excitation_out_of_range_count,
            signal_mass_before,
            signal_mass_after,
        },
    )))
}

pub(crate) fn p_and_e_within_legacy_revision_bound(field: &NeuralField, revision: u64) -> bool {
    let Some(limit) = i128::from(revision)
        .checked_add(1)
        .and_then(|value| value.checked_mul(i128::from(LEGACY_FIELD_FXP6_SCALE)))
    else {
        return false;
    };
    field
        .potential
        .iter()
        .chain(field.excitation.iter())
        .all(|value| value.raw() >= 0 && i128::from(value.raw()) <= limit)
}

impl LegacySemanticFormulaUpgradeReceiptV1 {
    fn expected_migration_id(&self) -> Digest {
        let mut parts: Vec<&[u8]> = vec![
            &self.scope_digest,
            // The locals below are introduced separately in canonical_bytes;
            // this branch uses one owned buffer to keep references stable.
        ];
        let base = self.base_revision.to_le_bytes();
        let next = self.next_revision.to_le_bytes();
        parts.extend_from_slice(&[
            &base,
            &next,
            &self.source_state_digest,
            &self.target_state_before,
            &self.source_graph_digest,
            &self.prior_chain_digest,
            &self.from_formula_digest,
            &self.to_formula_digest,
        ]);
        match self.field_domain {
            None => wire::domain_hash(LEGACY_SEMANTIC_FORMULA_UPGRADE_ID_DOMAIN_V1, &parts),
            Some(field) => {
                let algorithm = [field.algorithm];
                let scale = field.fxp6_scale.to_le_bytes();
                let max = field.source_common_max.to_le_bytes();
                let count = field.out_of_range_count.to_le_bytes();
                let potential = field.potential_out_of_range_count.to_le_bytes();
                let excitation = field.excitation_out_of_range_count.to_le_bytes();
                let before = field.signal_mass_before.to_le_bytes();
                let after = field.signal_mass_after.to_le_bytes();
                parts.extend_from_slice(&[
                    &algorithm,
                    &scale,
                    &max,
                    &count,
                    &potential,
                    &excitation,
                    &before,
                    &after,
                ]);
                wire::domain_hash(LEGACY_SEMANTIC_FIELD_DOMAIN_UPGRADE_ID_DOMAIN_V1, &parts)
            }
        }
    }

    pub fn from_transition_receipt(
        receipt: &TransitionReceipt,
        source_state_digest: Digest,
        source_graph_digest: Digest,
        from_formula_digest: Digest,
        prior_chain_digest: Digest,
    ) -> Self {
        let mut upgrade = Self {
            scope_digest: receipt.scope_digest,
            event_digest: receipt.event_digest,
            receipt_digest: wire::receipt_digest(receipt),
            base_revision: receipt.base_revision,
            next_revision: receipt.next_revision,
            source_state_digest,
            target_state_before: receipt.state_before,
            source_graph_digest,
            prior_chain_digest,
            from_formula_digest,
            to_formula_digest: receipt.formula_digest,
            field_domain: None,
            migration_id: [0; 32],
        };
        upgrade.migration_id = upgrade.expected_migration_id();
        upgrade
    }

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

    pub fn canonical_bytes(self) -> Vec<u8> {
        let has_field_domain = self.field_domain.is_some();
        let mut out = Vec::with_capacity(
            LEGACY_SEMANTIC_FORMULA_UPGRADE_MAGIC_V1.len()
                + 2
                + 1
                + (32 * 10)
                + (8 * 2)
                + if has_field_domain { 57 } else { 0 },
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
        if let Some(field) = self.field_domain {
            out.push(field.algorithm);
            out.extend_from_slice(&field.fxp6_scale.to_le_bytes());
            out.extend_from_slice(&field.source_common_max.to_le_bytes());
            out.extend_from_slice(&field.out_of_range_count.to_le_bytes());
            out.extend_from_slice(&field.potential_out_of_range_count.to_le_bytes());
            out.extend_from_slice(&field.excitation_out_of_range_count.to_le_bytes());
            out.extend_from_slice(&field.signal_mass_before.to_le_bytes());
            out.extend_from_slice(&field.signal_mass_after.to_le_bytes());
        }
        out.extend_from_slice(&self.migration_id);
        out
    }

    fn decode(bytes: &[u8]) -> Result<Self, StoreError> {
        if !bytes.starts_with(LEGACY_SEMANTIC_FORMULA_UPGRADE_MAGIC_V1) {
            return Err(StoreError::ContinuityFence("legacy_upgrade_magic"));
        }
        let mut reader =
            wire::Reader::new(&bytes[LEGACY_SEMANTIC_FORMULA_UPGRADE_MAGIC_V1.len()..]);
        let schema = reader
            .u16()
            .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
        let kind = reader
            .u8()
            .map_err(|_| StoreError::ContinuityFence("legacy_upgrade_decode"))?;
        let formula_only = schema == LEGACY_SEMANTIC_FORMULA_UPGRADE_SCHEMA_V1
            && kind == LEGACY_SEMANTIC_FORMULA_UPGRADE_KIND_V1;
        let field_domain = schema == LEGACY_SEMANTIC_FIELD_DOMAIN_UPGRADE_SCHEMA_V1
            && kind == LEGACY_SEMANTIC_FIELD_DOMAIN_UPGRADE_KIND_V1;
        if !formula_only && !field_domain {
            return Err(StoreError::ContinuityFence("legacy_upgrade_schema"));
        }
        let map = |_| StoreError::ContinuityFence("legacy_upgrade_decode");
        let scope_digest = reader.digest().map_err(map)?;
        let event_digest = reader.digest().map_err(map)?;
        let receipt_digest = reader.digest().map_err(map)?;
        let base_revision = reader.u64().map_err(map)?;
        let next_revision = reader.u64().map_err(map)?;
        let source_state_digest = reader.digest().map_err(map)?;
        let target_state_before = reader.digest().map_err(map)?;
        let source_graph_digest = reader.digest().map_err(map)?;
        let prior_chain_digest = reader.digest().map_err(map)?;
        let from_formula_digest = reader.digest().map_err(map)?;
        let to_formula_digest = reader.digest().map_err(map)?;
        let field_domain = if field_domain {
            let algorithm = reader.u8().map_err(map)?;
            let fxp6_scale = reader.u32().map_err(map)?;
            let source_common_max = i64::from_le_bytes(reader.u64().map_err(map)?.to_le_bytes());
            let out_of_range_count = reader.u32().map_err(map)?;
            let potential_out_of_range_count = reader.u32().map_err(map)?;
            let excitation_out_of_range_count = reader.u32().map_err(map)?;
            let mut read_i128 = || -> Result<i128, StoreError> {
                let low = reader.u64().map_err(map)?;
                let high = reader.u64().map_err(map)?;
                let mut raw = [0_u8; 16];
                raw[..8].copy_from_slice(&low.to_le_bytes());
                raw[8..].copy_from_slice(&high.to_le_bytes());
                Ok(i128::from_le_bytes(raw))
            };
            let field = LegacySemanticFieldDomainUpgradeV1 {
                algorithm,
                fxp6_scale,
                source_common_max,
                out_of_range_count,
                potential_out_of_range_count,
                excitation_out_of_range_count,
                signal_mass_before: read_i128()?,
                signal_mass_after: read_i128()?,
            };
            if field.algorithm != JOINT_MAX_LINEAR_FXP6_V1
                || field.fxp6_scale != LEGACY_FIELD_FXP6_SCALE
                || field.source_common_max <= i64::from(LEGACY_FIELD_FXP6_SCALE)
                || field.out_of_range_count == 0
                || field.out_of_range_count
                    != field
                        .potential_out_of_range_count
                        .checked_add(field.excitation_out_of_range_count)
                        .ok_or(StoreError::ContinuityFence("legacy_upgrade_field_domain"))?
                || field.signal_mass_before <= 0
                || field.signal_mass_after < 0
            {
                return Err(StoreError::ContinuityFence("legacy_upgrade_field_domain"));
            }
            Some(field)
        } else {
            None
        };
        let migration_id = reader.digest().map_err(map)?;
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
        if upgrade.migration_id != upgrade.expected_migration_id()
            || upgrade.canonical_bytes() != bytes
        {
            return Err(StoreError::ContinuityFence("legacy_upgrade_closure"));
        }
        Ok(upgrade)
    }
}

fn revision_to_sqlite(revision: u64) -> Result<i64, StoreError> {
    Ok(JournalRevision::new(revision).to_sqlite()?.get())
}

fn revision_from_sqlite(revision: i64) -> Result<u64, StoreError> {
    Ok(JournalRevision::try_from(revision)?.get())
}

fn digest_from_blob(bytes: &[u8], field: &'static str) -> Result<Digest, StoreError> {
    bytes
        .try_into()
        .map_err(|_| StoreError::InvalidStoredDigest {
            field,
            actual: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        })
}

fn scope_from_event(event: &CanonicalEvent) -> Result<&ScopeRef, StoreError> {
    match event {
        CanonicalEvent::UserStimulus(value) => Ok(&value.scope),
        _ => Err(StoreError::ContinuityFence("field_upgrade_event")),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum FieldMigrationBackupCaptureMethodV1 {
    SqliteBackupApi = 1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
    source_authority_fingerprint: Digest,
    byte_len: u64,
    sha256: Digest,
}

fn backup_package_identity() -> String {
    format!("{}@{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
}

fn backup_build_identity() -> String {
    format!(
        "{}@{};target={}-{};manifest=AE-FMP2",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}

fn decode_backup_identity(bytes: &[u8], resource: &'static str) -> Result<String, StoreError> {
    let actual = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if bytes.is_empty() {
        return Err(StoreError::ContinuityFence("field_backup_creator_identity"));
    }
    if actual > MAX_FIELD_MIGRATION_BACKUP_IDENTITY_BYTES {
        return Err(StoreError::StorageBudgetExceeded {
            resource,
            limit: MAX_FIELD_MIGRATION_BACKUP_IDENTITY_BYTES,
            actual,
        });
    }
    let identity = std::str::from_utf8(bytes)
        .map_err(|_| StoreError::ContinuityFence("field_backup_creator_identity"))?;
    if identity.contains('\0') {
        return Err(StoreError::ContinuityFence("field_backup_creator_identity"));
    }
    Ok(identity.to_owned())
}

fn decode_backup_manifest_creator_identity(
    manifest: &[u8],
) -> Result<(String, String), StoreError> {
    let decoded = decode_backup_manifest(manifest)?;
    Ok((decoded.package_identity, decoded.build_identity))
}

fn field_backup_has_column(tx: &Transaction<'_>, wanted: &str) -> Result<bool, StoreError> {
    let mut statement = tx.prepare(
        "SELECT
            CASE WHEN typeof(name)='text' AND length(CAST(name AS BLOB))<=?1 THEN name END,
            CASE WHEN typeof(name)='text' THEN length(CAST(name AS BLOB)) ELSE -1 END
         FROM pragma_table_info('field_migration_preimage_backups')",
    )?;
    let mut rows = statement.query(params![MAX_SQLITE_SCHEMA_IDENTIFIER_BYTES])?;
    while let Some(row) = rows.next()? {
        let column = bounded_typed_value(
            row.get::<_, Option<String>>(0)?,
            row.get(1)?,
            MAX_SQLITE_SCHEMA_IDENTIFIER_BYTES,
            "sqlite_schema.field_backup.column_name",
            "sqlite_schema_identifier_type",
        )?;
        if column == wanted {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn migrate_field_backup_creator_provenance(
    tx: &Transaction<'_>,
) -> Result<(), StoreError> {
    for (column, sql) in [
        (
            "creator_package_identity",
            "ALTER TABLE field_migration_preimage_backups ADD COLUMN creator_package_identity TEXT NOT NULL DEFAULT ''",
        ),
        (
            "creator_build_identity",
            "ALTER TABLE field_migration_preimage_backups ADD COLUMN creator_build_identity TEXT NOT NULL DEFAULT ''",
        ),
    ] {
        if !field_backup_has_column(tx, column)? {
            tx.execute(sql, [])?;
        }
    }

    let (raw_rows, raw_bytes): (i64, i64) = tx.query_row(
        "SELECT COUNT(*), COALESCE(SUM(length(manifest_bytes)),0)
         FROM field_migration_preimage_backups",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let rows = sqlite_length(raw_rows, "semantic_migration_backups.rows")?;
    let bytes = sqlite_length(raw_bytes, "semantic_migration_backups.bytes")?;
    enforce_read_budget(
        "semantic_migration_backups.rows",
        "semantic_migration_backups.bytes",
        rows,
        bytes,
        MAX_SEMANTIC_HISTORY_ROWS,
        MAX_SEMANTIC_HISTORY_BYTES,
    )?;

    type CreatorRow = (
        Option<Vec<u8>>,
        i64,
        Option<Vec<u8>>,
        i64,
        Option<Vec<u8>>,
        i64,
        Option<Vec<u8>>,
        i64,
    );
    let mut statement = tx.prepare(
        "SELECT
            CASE WHEN typeof(migration_id)='blob' AND length(migration_id)=32 THEN migration_id END,
            CASE WHEN typeof(migration_id)='blob' THEN length(migration_id) ELSE -1 END,
            CASE WHEN typeof(manifest_bytes)='blob' AND length(manifest_bytes)<=?1 THEN manifest_bytes END,
            CASE WHEN typeof(manifest_bytes)='blob' THEN length(manifest_bytes) ELSE -1 END,
            CASE WHEN typeof(creator_package_identity)='text'
                       AND length(CAST(creator_package_identity AS BLOB))<=?2
                 THEN CAST(creator_package_identity AS BLOB) END,
            CASE WHEN typeof(creator_package_identity)='text'
                 THEN length(CAST(creator_package_identity AS BLOB)) ELSE -1 END,
            CASE WHEN typeof(creator_build_identity)='text'
                       AND length(CAST(creator_build_identity AS BLOB))<=?2
                 THEN CAST(creator_build_identity AS BLOB) END,
            CASE WHEN typeof(creator_build_identity)='text'
                 THEN length(CAST(creator_build_identity AS BLOB)) ELSE -1 END
         FROM field_migration_preimage_backups ORDER BY migration_id",
    )?;
    let mut query = statement.query(params![
        MAX_FIELD_MIGRATION_BACKUP_MANIFEST_BYTES,
        MAX_FIELD_MIGRATION_BACKUP_IDENTITY_BYTES,
    ])?;
    let mut updates = Vec::with_capacity(usize::try_from(rows).unwrap_or(0));
    while let Some(row) = query.next()? {
        let stored: CreatorRow = (
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
            row.get(6)?,
            row.get(7)?,
        );
        let migration_id = stored_typed_digest(
            stored.0,
            stored.1,
            "field_backup.migration_id",
            "field_backup_migration_id_type",
        )?;
        let manifest = bounded_typed_value(
            stored.2,
            stored.3,
            MAX_FIELD_MIGRATION_BACKUP_MANIFEST_BYTES,
            "field_backup.manifest_bytes",
            "field_backup_manifest_type",
        )?;
        let (package, build) = decode_backup_manifest_creator_identity(&manifest)?;
        let stored_package = bounded_typed_value(
            stored.4,
            stored.5,
            MAX_FIELD_MIGRATION_BACKUP_IDENTITY_BYTES,
            "field_backup.creator_package_identity_bytes",
            "field_backup_creator_identity",
        )?;
        let stored_build = bounded_typed_value(
            stored.6,
            stored.7,
            MAX_FIELD_MIGRATION_BACKUP_IDENTITY_BYTES,
            "field_backup.creator_build_identity_bytes",
            "field_backup_creator_identity",
        )?;
        if (!stored_package.is_empty() && stored_package != package.as_bytes())
            || (!stored_build.is_empty() && stored_build != build.as_bytes())
        {
            return Err(StoreError::ContinuityFence("field_backup_record"));
        }
        if stored_package.is_empty() || stored_build.is_empty() {
            updates.push((migration_id, package, build));
        }
    }
    drop(query);
    drop(statement);
    for (migration_id, package, build) in updates {
        tx.execute(
            "UPDATE field_migration_preimage_backups
             SET creator_package_identity=?2, creator_build_identity=?3
             WHERE migration_id=?1",
            params![blob(migration_id), package, build],
        )?;
    }
    Ok(())
}

fn backup_manifest(backup: &FieldMigrationPreimageBackupV1) -> Vec<u8> {
    let package = backup.package_identity.as_bytes();
    let build = backup.build_identity.as_bytes();
    let package_len = u8::try_from(package.len()).expect("bounded package identity");
    let build_len = u8::try_from(build.len()).expect("bounded build identity");
    let mut bytes = Vec::with_capacity(8 + 3 + package.len() + build.len() + (32 * 9) + 17);
    bytes.extend_from_slice(FIELD_MIGRATION_BACKUP_MANIFEST_MAGIC_V2);
    bytes.push(package_len);
    bytes.extend_from_slice(package);
    bytes.push(build_len);
    bytes.extend_from_slice(build);
    bytes.push(backup.capture_method as u8);
    bytes.extend_from_slice(&backup.migration_id);
    bytes.extend_from_slice(&backup.scope_digest);
    bytes.extend_from_slice(&backup.source_revision.to_le_bytes());
    for digest in [
        backup.source_state_digest,
        backup.source_formula_digest,
        backup.source_graph_digest,
        backup.incarnation_id,
        backup.manifest_digest,
        backup.source_authority_fingerprint,
    ] {
        bytes.extend_from_slice(&digest);
    }
    bytes.extend_from_slice(&backup.byte_len.to_le_bytes());
    bytes.extend_from_slice(&backup.sha256);
    bytes.push(1);
    bytes
}

fn backup_source_authority_fingerprint(backup: &FieldMigrationPreimageBackupV1) -> Digest {
    wire::domain_hash(
        FIELD_MIGRATION_BACKUP_SOURCE_DOMAIN_V1,
        &[
            &backup.migration_id,
            &backup.scope_digest,
            &backup.source_revision.to_le_bytes(),
            &backup.source_state_digest,
            &backup.source_formula_digest,
            &backup.source_graph_digest,
            &backup.incarnation_id,
            &backup.manifest_digest,
        ],
    )
}

fn backup_manifest_take<'a>(
    manifest: &'a [u8],
    position: &mut usize,
    count: usize,
) -> Result<&'a [u8], StoreError> {
    let end = position
        .checked_add(count)
        .ok_or(StoreError::ContinuityFence("field_backup_manifest"))?;
    let value = manifest
        .get(*position..end)
        .ok_or(StoreError::ContinuityFence("field_backup_manifest"))?;
    *position = end;
    Ok(value)
}

fn backup_manifest_digest(manifest: &[u8], position: &mut usize) -> Result<Digest, StoreError> {
    let mut value = [0_u8; 32];
    value.copy_from_slice(backup_manifest_take(manifest, position, 32)?);
    Ok(value)
}

fn decode_backup_manifest(manifest: &[u8]) -> Result<FieldMigrationPreimageBackupV1, StoreError> {
    let actual = u64::try_from(manifest.len()).unwrap_or(u64::MAX);
    if actual > MAX_FIELD_MIGRATION_BACKUP_MANIFEST_BYTES {
        return Err(StoreError::StorageBudgetExceeded {
            resource: "field_backup.manifest_bytes",
            limit: MAX_FIELD_MIGRATION_BACKUP_MANIFEST_BYTES,
            actual,
        });
    }
    if !manifest.starts_with(FIELD_MIGRATION_BACKUP_MANIFEST_MAGIC_V2) {
        return Err(StoreError::ContinuityFence("field_backup_manifest"));
    }
    let mut position = FIELD_MIGRATION_BACKUP_MANIFEST_MAGIC_V2.len();
    let package_len = usize::from(backup_manifest_take(manifest, &mut position, 1)?[0]);
    let package_identity = decode_backup_identity(
        backup_manifest_take(manifest, &mut position, package_len)?,
        "field_backup.creator_package_identity_bytes",
    )?;
    let build_len = usize::from(backup_manifest_take(manifest, &mut position, 1)?[0]);
    let build_identity = decode_backup_identity(
        backup_manifest_take(manifest, &mut position, build_len)?,
        "field_backup.creator_build_identity_bytes",
    )?;
    if backup_manifest_take(manifest, &mut position, 1)?[0]
        != FieldMigrationBackupCaptureMethodV1::SqliteBackupApi as u8
    {
        return Err(StoreError::ContinuityFence("field_backup_manifest"));
    }
    let migration_id = backup_manifest_digest(manifest, &mut position)?;
    let scope_digest = backup_manifest_digest(manifest, &mut position)?;
    let mut revision = [0_u8; 8];
    revision.copy_from_slice(backup_manifest_take(manifest, &mut position, 8)?);
    let source_revision = u64::from_le_bytes(revision);
    let source_state_digest = backup_manifest_digest(manifest, &mut position)?;
    let source_formula_digest = backup_manifest_digest(manifest, &mut position)?;
    let source_graph_digest = backup_manifest_digest(manifest, &mut position)?;
    let incarnation_id = backup_manifest_digest(manifest, &mut position)?;
    let manifest_digest = backup_manifest_digest(manifest, &mut position)?;
    let source_authority_fingerprint = backup_manifest_digest(manifest, &mut position)?;
    let mut byte_len = [0_u8; 8];
    byte_len.copy_from_slice(backup_manifest_take(manifest, &mut position, 8)?);
    let byte_len = u64::from_le_bytes(byte_len);
    let sha256 = backup_manifest_digest(manifest, &mut position)?;
    if backup_manifest_take(manifest, &mut position, 1)?[0] != 1 || position != manifest.len() {
        return Err(StoreError::ContinuityFence("field_backup_manifest"));
    }
    enforce_database_budget(byte_len)?;
    let backup = FieldMigrationPreimageBackupV1 {
        migration_id,
        scope_digest,
        source_revision,
        source_state_digest,
        source_formula_digest,
        source_graph_digest,
        incarnation_id,
        manifest_digest,
        package_identity,
        build_identity,
        capture_method: FieldMigrationBackupCaptureMethodV1::SqliteBackupApi,
        source_authority_fingerprint,
        byte_len,
        sha256,
    };
    // AE-FMP2 is self-describing rather than tied to the current binary. Its
    // canonical byte-for-byte re-encoding closes the creator provenance and
    // internal fields, while this digest independently closes the source
    // authority tuple. The database SHA is verified from its held file handle.
    if backup.source_authority_fingerprint != backup_source_authority_fingerprint(&backup)
        || backup_manifest(&backup) != manifest
    {
        return Err(StoreError::ContinuityFence("field_backup_manifest"));
    }
    Ok(backup)
}

fn sha256_open_file(file: &mut fs::File) -> Result<(u64, Digest), StoreError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|source| StoreError::Io {
            context: "seeking field migration backup",
            source,
        })?;
    let expected_len = file
        .metadata()
        .map_err(|source| StoreError::Io {
            context: "reading field migration backup metadata",
            source,
        })?
        .len();
    enforce_database_budget(expected_len)?;
    let mut hasher = Sha256::new();
    let mut byte_len = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|source| StoreError::Io {
            context: "reading field migration backup",
            source,
        })?;
        if read == 0 {
            break;
        }
        byte_len = byte_len
            .checked_add(
                u64::try_from(read)
                    .map_err(|_| StoreError::ContinuityFence("field_backup_length"))?,
            )
            .ok_or(StoreError::ContinuityFence("field_backup_length"))?;
        hasher.update(&buffer[..read]);
    }
    let mut digest = [0_u8; 32];
    digest.copy_from_slice(&hasher.finalize());
    if byte_len != expected_len {
        return Err(StoreError::ContinuityFence("field_backup_length_changed"));
    }
    Ok((byte_len, digest))
}

fn sha256_file(path: &Path) -> Result<(u64, Digest), StoreError> {
    let mut file =
        ae_platform_fs::open_regular_file_no_follow(path).map_err(|source| StoreError::Io {
            context: "opening field migration backup without following links",
            source,
        })?;
    sha256_open_file(&mut file)
}

struct FieldMigrationBackupPaths {
    root: PathBuf,
    final_dir: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceDatabaseIdentity {
    canonical_path: PathBuf,
    byte_len: u64,
    platform_file_id: (u64, u64),
}

fn source_database_identity(path: &Path) -> Result<SourceDatabaseIdentity, StoreError> {
    let canonical_path = fs::canonicalize(path).map_err(|source| StoreError::Io {
        context: "canonicalizing authority database path",
        source,
    })?;
    let metadata = fs::symlink_metadata(&canonical_path).map_err(|source| StoreError::Io {
        context: "checking authority database identity",
        source,
    })?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(StoreError::ContinuityFence("field_backup_source_path"));
    }
    #[cfg(windows)]
    let platform_file_id = {
        use std::os::windows::fs::MetadataExt;
        (metadata.creation_time(), metadata.last_write_time())
    };
    #[cfg(unix)]
    let platform_file_id = {
        use std::os::unix::fs::MetadataExt;
        (metadata.dev(), metadata.ino())
    };
    #[cfg(not(any(windows, unix)))]
    let platform_file_id = (0, 0);
    if platform_file_id == (0, 0) {
        return Err(StoreError::ContinuityFence("field_backup_source_identity"));
    }
    Ok(SourceDatabaseIdentity {
        canonical_path,
        byte_len: metadata.len(),
        platform_file_id,
    })
}

fn backup_paths(
    database_path: &Path,
    migration_id: &Digest,
    _source_revision: u64,
) -> Result<FieldMigrationBackupPaths, StoreError> {
    let parent = database_path
        .parent()
        .ok_or(StoreError::ContinuityFence("field_backup_path"))?;
    let root = parent.join(".astr-embodiment-field-migration-preimages");
    ae_platform_fs::create_dir_all_durable(&root).map_err(|source| StoreError::Io {
        context: "creating field migration backup directory",
        source,
    })?;
    let root_metadata = fs::symlink_metadata(&root).map_err(|source| StoreError::Io {
        context: "checking field migration backup root",
        source,
    })?;
    if !root_metadata.file_type().is_dir() || root_metadata.file_type().is_symlink() {
        return Err(StoreError::ContinuityFence("field_backup_path"));
    }
    let canonical_parent = fs::canonicalize(parent).map_err(|source| StoreError::Io {
        context: "canonicalizing field migration backup parent",
        source,
    })?;
    let canonical_root = fs::canonicalize(&root).map_err(|source| StoreError::Io {
        context: "canonicalizing field migration backup root",
        source,
    })?;
    if canonical_root.parent() != Some(canonical_parent.as_path()) {
        return Err(StoreError::ContinuityFence("field_backup_path"));
    }
    let final_dir = canonical_root.join(ae_contracts::hex::encode32(migration_id));
    if final_dir.parent() != Some(canonical_root.as_path()) {
        return Err(StoreError::ContinuityFence("field_backup_path"));
    }
    Ok(FieldMigrationBackupPaths {
        root: canonical_root,
        final_dir,
    })
}

fn read_backup_manifest_handle(
    file: &mut fs::File,
    exact_len: Option<usize>,
) -> Result<Vec<u8>, StoreError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|source| StoreError::Io {
            context: "seeking field migration backup manifest",
            source,
        })?;
    let actual = file
        .metadata()
        .map_err(|source| StoreError::Io {
            context: "reading field migration backup manifest metadata",
            source,
        })?
        .len();
    if actual > MAX_FIELD_MIGRATION_BACKUP_MANIFEST_BYTES {
        return Err(StoreError::StorageBudgetExceeded {
            resource: "field_backup.manifest_bytes",
            limit: MAX_FIELD_MIGRATION_BACKUP_MANIFEST_BYTES,
            actual,
        });
    }
    if let Some(exact_len) = exact_len {
        let limit = u64::try_from(exact_len).map_err(|_| StoreError::StorageBudgetExceeded {
            resource: "field_backup.manifest_bytes",
            limit: u64::MAX,
            actual: u64::MAX,
        })?;
        if actual != limit {
            return Err(StoreError::StorageBudgetExceeded {
                resource: "field_backup.manifest_bytes",
                limit,
                actual,
            });
        }
    }
    let length = usize::try_from(actual)
        .map_err(|_| StoreError::ContinuityFence("field_backup_manifest"))?;
    let mut bytes = vec![0_u8; length];
    file.read_exact(&mut bytes)
        .map_err(|source| StoreError::Io {
            context: "reading field migration backup manifest",
            source,
        })?;
    let mut trailing = [0_u8; 1];
    if file.read(&mut trailing).map_err(|source| StoreError::Io {
        context: "checking field migration backup manifest trailing bytes",
        source,
    })? != 0
    {
        return Err(StoreError::StorageBudgetExceeded {
            resource: "field_backup.manifest_bytes",
            limit: actual,
            actual: actual.saturating_add(1),
        });
    }
    Ok(bytes)
}

struct ValidatedBackupPackage {
    backup: FieldMigrationPreimageBackupV1,
    package: ae_platform_fs::ExactRegularFilePackage,
}

fn inspect_backup_directory(
    directory: &Path,
    expected: &FieldMigrationPreimageBackupV1,
) -> Result<ValidatedBackupPackage, StoreError> {
    let mut package =
        ae_platform_fs::ExactRegularFilePackage::open(directory, &["authority.sqlite", "manifest"])
            .map_err(|source| StoreError::Io {
                context: "opening exact field migration backup package",
                source,
            })?;
    let manifest_bytes = read_backup_manifest_handle(
        package
            .member_mut("manifest")
            .map_err(|source| StoreError::Io {
                context: "opening anchored field migration backup manifest",
                source,
            })?,
        None,
    )?;
    let backup = decode_backup_manifest(&manifest_bytes)?;
    if !same_backup_source_authority(&backup, expected) {
        return Err(StoreError::ContinuityFence("field_backup_manifest"));
    }
    let (byte_len, sha256) =
        sha256_open_file(package.member_mut("authority.sqlite").map_err(|source| {
            StoreError::Io {
                context: "opening anchored field migration backup database",
                source,
            }
        })?)?;
    if byte_len != backup.byte_len || sha256 != backup.sha256 {
        return Err(StoreError::ContinuityFence("field_backup_hash"));
    }
    package.revalidate().map_err(|source| StoreError::Io {
        context: "revalidating exact field migration backup package",
        source,
    })?;
    Ok(ValidatedBackupPackage { backup, package })
}

fn same_backup_source_authority(
    actual: &FieldMigrationPreimageBackupV1,
    expected: &FieldMigrationPreimageBackupV1,
) -> bool {
    actual.migration_id == expected.migration_id
        && actual.scope_digest == expected.scope_digest
        && actual.source_revision == expected.source_revision
        && actual.source_state_digest == expected.source_state_digest
        && actual.source_formula_digest == expected.source_formula_digest
        && actual.source_graph_digest == expected.source_graph_digest
        && actual.incarnation_id == expected.incarnation_id
        && actual.manifest_digest == expected.manifest_digest
        && actual.capture_method == expected.capture_method
        && actual.source_authority_fingerprint == expected.source_authority_fingerprint
}

fn validate_backup_directory(
    directory: &Path,
    backup: &FieldMigrationPreimageBackupV1,
) -> Result<FieldMigrationPreimageBackupV1, StoreError> {
    Ok(inspect_backup_directory(directory, backup)?.backup)
}

fn validate_backup_files(
    database_path: &Path,
    backup: &FieldMigrationPreimageBackupV1,
) -> Result<(), StoreError> {
    let paths = backup_paths(database_path, &backup.migration_id, backup.source_revision)?;
    let completed = validate_backup_directory(&paths.final_dir, backup)?;
    if completed != *backup {
        return Err(StoreError::ContinuityFence("field_backup_record"));
    }
    Ok(())
}

fn backup_from_row(
    tx: &Transaction<'_>,
    expected: &FieldMigrationPreimageBackupV1,
) -> Result<Option<FieldMigrationPreimageBackupV1>, StoreError> {
    type Columns = (
        Vec<u8>,
        i64,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        i64,
        Vec<u8>,
        Option<Vec<u8>>,
        i64,
        Option<Vec<u8>>,
        i64,
        Option<Vec<u8>>,
        i64,
    );
    let stored: Option<Columns> = tx
        .query_row(
            "SELECT
                CASE WHEN typeof(scope_digest)='blob' AND length(scope_digest)=32 THEN scope_digest ELSE zeroblob(0) END,
                source_revision,
                CASE WHEN typeof(source_state_digest)='blob' AND length(source_state_digest)=32 THEN source_state_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(source_formula_digest)='blob' AND length(source_formula_digest)=32 THEN source_formula_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(source_graph_digest)='blob' AND length(source_graph_digest)=32 THEN source_graph_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(incarnation_id)='blob' AND length(incarnation_id)=32 THEN incarnation_id ELSE zeroblob(0) END,
                CASE WHEN typeof(manifest_digest)='blob' AND length(manifest_digest)=32 THEN manifest_digest ELSE zeroblob(0) END,
                byte_len,
                CASE WHEN typeof(sha256)='blob' AND length(sha256)=32 THEN sha256 ELSE zeroblob(0) END,
                CASE WHEN typeof(manifest_bytes)='blob' AND length(manifest_bytes)<=?2 THEN manifest_bytes END,
                CASE WHEN typeof(manifest_bytes)='blob' THEN length(manifest_bytes) ELSE -1 END,
                CASE WHEN typeof(creator_package_identity)='text'
                           AND length(CAST(creator_package_identity AS BLOB)) BETWEEN 1 AND ?3
                     THEN CAST(creator_package_identity AS BLOB) END,
                CASE WHEN typeof(creator_package_identity)='text'
                     THEN length(CAST(creator_package_identity AS BLOB)) ELSE -1 END,
                CASE WHEN typeof(creator_build_identity)='text'
                           AND length(CAST(creator_build_identity AS BLOB)) BETWEEN 1 AND ?3
                     THEN CAST(creator_build_identity AS BLOB) END,
                CASE WHEN typeof(creator_build_identity)='text'
                     THEN length(CAST(creator_build_identity AS BLOB)) ELSE -1 END
             FROM field_migration_preimage_backups WHERE migration_id = ?1",
            params![
                blob(expected.migration_id),
                MAX_FIELD_MIGRATION_BACKUP_MANIFEST_BYTES,
                MAX_FIELD_MIGRATION_BACKUP_IDENTITY_BYTES,
            ],
            |row| {
                Ok((
                    row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?,
                    row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?,
                    row.get(10)?, row.get(11)?, row.get(12)?, row.get(13)?, row.get(14)?,
                ))
            },
        )
        .optional()?;
    let Some((
        scope,
        revision,
        state,
        formula,
        graph,
        incarnation,
        manifest,
        len,
        sha,
        manifest_bytes,
        manifest_bytes_len,
        package_identity,
        package_identity_len,
        build_identity,
        build_identity_len,
    )) = stored
    else {
        return Ok(None);
    };
    let backup = FieldMigrationPreimageBackupV1 {
        migration_id: expected.migration_id,
        scope_digest: digest_from_blob(&scope, "field_backup_scope")?,
        source_revision: revision_from_sqlite(revision)?,
        source_state_digest: digest_from_blob(&state, "field_backup_state")?,
        source_formula_digest: digest_from_blob(&formula, "field_backup_formula")?,
        source_graph_digest: digest_from_blob(&graph, "field_backup_graph")?,
        incarnation_id: digest_from_blob(&incarnation, "field_backup_incarnation")?,
        manifest_digest: digest_from_blob(&manifest, "field_backup_manifest_digest")?,
        package_identity: decode_backup_identity(
            &bounded_typed_value(
                package_identity,
                package_identity_len,
                MAX_FIELD_MIGRATION_BACKUP_IDENTITY_BYTES,
                "field_backup.creator_package_identity_bytes",
                "field_backup_creator_identity",
            )?,
            "field_backup.creator_package_identity_bytes",
        )?,
        build_identity: decode_backup_identity(
            &bounded_typed_value(
                build_identity,
                build_identity_len,
                MAX_FIELD_MIGRATION_BACKUP_IDENTITY_BYTES,
                "field_backup.creator_build_identity_bytes",
                "field_backup_creator_identity",
            )?,
            "field_backup.creator_build_identity_bytes",
        )?,
        capture_method: expected.capture_method,
        source_authority_fingerprint: expected.source_authority_fingerprint,
        byte_len: u64::try_from(len)
            .map_err(|_| StoreError::ContinuityFence("field_backup_length"))?,
        sha256: digest_from_blob(&sha, "field_backup_sha256")?,
    };
    let manifest_bytes = bounded_typed_value(
        manifest_bytes,
        manifest_bytes_len,
        MAX_FIELD_MIGRATION_BACKUP_MANIFEST_BYTES,
        "field_backup.manifest_bytes",
        "field_backup_manifest_type",
    )?;
    if backup.scope_digest != expected.scope_digest
        || backup.source_revision != expected.source_revision
        || backup.source_state_digest != expected.source_state_digest
        || backup.source_formula_digest != expected.source_formula_digest
        || backup.source_graph_digest != expected.source_graph_digest
        || backup.incarnation_id != expected.incarnation_id
        || backup.manifest_digest != expected.manifest_digest
        || manifest_bytes != backup_manifest(&backup)
    {
        return Err(StoreError::ContinuityFence("field_backup_record"));
    }
    Ok(Some(backup))
}

fn expected_backup(
    upgrade: &LegacySemanticFormulaUpgradeReceiptV1,
    incarnation_id: Digest,
    manifest_digest: Digest,
) -> FieldMigrationPreimageBackupV1 {
    let mut backup = FieldMigrationPreimageBackupV1 {
        migration_id: upgrade.migration_id,
        scope_digest: upgrade.scope_digest,
        source_revision: upgrade.base_revision,
        source_state_digest: upgrade.source_state_digest,
        source_formula_digest: upgrade.from_formula_digest,
        source_graph_digest: upgrade.source_graph_digest,
        incarnation_id,
        manifest_digest,
        package_identity: backup_package_identity(),
        build_identity: backup_build_identity(),
        capture_method: FieldMigrationBackupCaptureMethodV1::SqliteBackupApi,
        source_authority_fingerprint: [0; 32],
        byte_len: 0,
        sha256: [0; 32],
    };
    backup.source_authority_fingerprint = backup_source_authority_fingerprint(&backup);
    backup
}

fn source_data_version(conn: &Connection) -> Result<i64, StoreError> {
    let version: i64 = conn.query_row("PRAGMA data_version", [], |row| row.get(0))?;
    if version < 0 {
        return Err(StoreError::ContinuityFence("field_backup_source_changed"));
    }
    Ok(version)
}

fn stage_nonce(expected: &FieldMigrationPreimageBackupV1) -> String {
    let serial = NEXT_FIELD_MIGRATION_STAGE_NONCE.fetch_add(1, Ordering::Relaxed);
    let pid = u64::from(std::process::id());
    let digest = wire::domain_hash(
        b"astr-embodiment/field-migration-stage-nonce-v1",
        &[
            &expected.migration_id,
            &pid.to_le_bytes(),
            &serial.to_le_bytes(),
            &now_ms().to_le_bytes(),
        ],
    );
    ae_contracts::hex::encode32(&digest)
}

fn sync_directory(path: &Path) -> Result<(), StoreError> {
    ae_platform_fs::sync_directory_no_follow(path).map_err(|source| StoreError::Io {
        context: "syncing anchored field migration backup directory",
        source,
    })
}

fn publish_directory(source: &Path, destination: &Path, parent: &Path) -> Result<(), StoreError> {
    fire_semantic_migration_failpoint_v1(SemanticMigrationTestFailpointV1::BeforeAtomicPublish)?;
    ae_platform_fs::durable_rename_directory(source, destination).map_err(|source| {
        StoreError::Io {
            context: "durably publishing field migration backup directory",
            source,
        }
    })?;
    fire_semantic_migration_failpoint_v1(SemanticMigrationTestFailpointV1::AfterAtomicPublish)?;
    complete_published_directory(parent)
}

fn complete_published_directory(parent: &Path) -> Result<(), StoreError> {
    // Unix durable_rename_directory is the atomic rename only, so this is the
    // required parent fsync. On Windows MoveFileExW used WRITE_THROUGH; this
    // pass verifies the published non-reparse directory and flushed files.
    fire_semantic_migration_failpoint_v1(SemanticMigrationTestFailpointV1::BeforeParentSync)?;
    sync_directory(parent)?;
    fire_semantic_migration_failpoint_v1(SemanticMigrationTestFailpointV1::AfterParentSync)
}

fn is_owned_stage_name(name: &str, migration_hex: &str) -> bool {
    let Some(nonce) = name.strip_prefix(&format!(".stage-{migration_hex}-")) else {
        return false;
    };
    nonce.len() == 64
        && nonce
            .bytes()
            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
}

fn remove_owned_stage(directory: &Path) -> Result<(), StoreError> {
    let metadata = fs::symlink_metadata(directory).map_err(|source| StoreError::Io {
        context: "checking owned field migration stage",
        source,
    })?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Ok(());
    }
    let mut children = Vec::new();
    for entry in fs::read_dir(directory).map_err(|source| StoreError::Io {
        context: "reading owned field migration stage",
        source,
    })? {
        let entry = entry.map_err(|source| StoreError::Io {
            context: "reading owned field migration stage entry",
            source,
        })?;
        let name = entry.file_name();
        if name != "authority.sqlite" && name != "manifest" {
            return Ok(());
        }
        let file_type = entry.file_type().map_err(|source| StoreError::Io {
            context: "checking owned field migration stage entry",
            source,
        })?;
        if !file_type.is_file() || file_type.is_symlink() {
            return Ok(());
        }
        children.push(entry.path());
    }
    for child in children {
        fs::remove_file(child).map_err(|source| StoreError::Io {
            context: "removing owned field migration stage file",
            source,
        })?;
    }
    fs::remove_dir(directory).map_err(|source| StoreError::Io {
        context: "removing owned field migration stage directory",
        source,
    })
}

fn stale_final_path(
    paths: &FieldMigrationBackupPaths,
    expected: &FieldMigrationPreimageBackupV1,
) -> Result<PathBuf, StoreError> {
    let migration_hex = ae_contracts::hex::encode32(&expected.migration_id);
    if paths.final_dir.parent() != Some(paths.root.as_path())
        || paths.final_dir.file_name().and_then(|name| name.to_str()) != Some(&migration_hex)
    {
        return Err(StoreError::ContinuityFence("field_backup_path"));
    }
    Ok(paths.root.join(format!(".stale-{migration_hex}")))
}

fn remove_stale_final(
    paths: &FieldMigrationBackupPaths,
    expected: &FieldMigrationPreimageBackupV1,
) -> Result<(), StoreError> {
    let stale = stale_final_path(paths, expected)?;
    if !stale.exists() {
        return Ok(());
    }
    // The exact internal tombstone name is created only by the atomic move
    // below. remove_owned_stage refuses links and unknown children, while also
    // finishing a prior interrupted removal of the two known package files.
    remove_owned_stage(&stale)?;
    if stale.exists() {
        return Err(StoreError::ContinuityFence("field_backup_conflict"));
    }
    sync_directory(&paths.root)
}

fn quarantine_uncommitted_final(
    paths: &FieldMigrationBackupPaths,
    expected: &FieldMigrationPreimageBackupV1,
) -> Result<(), StoreError> {
    // This helper is reachable only after backup_from_row proved there is no
    // committed backup row while the caller owns BEGIN IMMEDIATE. Therefore a
    // valid-but-different final is an orphan from a failed pre-CAS attempt, not
    // committed recovery authority. Never repair a corrupt package here.
    validate_backup_directory(&paths.final_dir, expected)?;
    remove_stale_final(paths, expected)?;
    let stale = stale_final_path(paths, expected)?;
    ae_platform_fs::durable_rename_directory(&paths.final_dir, &stale).map_err(|source| {
        StoreError::Io {
            context: "quarantining stale field migration backup directory",
            source,
        }
    })?;
    // Publishing the fresh stage can now fail or the process can crash without
    // destroying the stale package: it remains atomically recoverable at the
    // owned tombstone until the fresh final is durable.
    sync_directory(&paths.root)
}

fn discard_owned_stages(
    paths: &FieldMigrationBackupPaths,
    expected: &FieldMigrationPreimageBackupV1,
) -> Result<(), StoreError> {
    let migration_hex = ae_contracts::hex::encode32(&expected.migration_id);
    let mut owned = Vec::new();
    for entry in fs::read_dir(&paths.root).map_err(|source| StoreError::Io {
        context: "reading field migration backup root",
        source,
    })? {
        let entry = entry.map_err(|source| StoreError::Io {
            context: "reading field migration backup entry",
            source,
        })?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !is_owned_stage_name(name, &migration_hex) {
            continue;
        }
        let count = u64::try_from(owned.len()).unwrap_or(u64::MAX) + 1;
        if count > MAX_FIELD_MIGRATION_STAGE_DIRS {
            return Err(StoreError::StorageBudgetExceeded {
                resource: "field_backup.stage_directories",
                limit: MAX_FIELD_MIGRATION_STAGE_DIRS,
                actual: count,
            });
        }
        owned.push(entry.path());
    }
    owned.sort();
    for directory in owned {
        // A row-less retry always captures the current whole SQLite snapshot;
        // no older stage is authoritative merely because its target-scope
        // manifest still validates.
        let _ = validate_backup_directory(&directory, expected);
        remove_owned_stage(&directory)?;
    }
    Ok(())
}

struct PreparedFieldMigrationBackupV1 {
    backup: FieldMigrationPreimageBackupV1,
    source_data_version: i64,
    paths: FieldMigrationBackupPaths,
    stage: Option<PathBuf>,
}

fn prepare_backup(
    tx: &Transaction<'_>,
    database_path: &Path,
    expected: FieldMigrationPreimageBackupV1,
) -> Result<PreparedFieldMigrationBackupV1, StoreError> {
    let data_version = source_data_version(tx)?;
    let paths = backup_paths(
        database_path,
        &expected.migration_id,
        expected.source_revision,
    )?;
    if let Some(existing) = backup_from_row(tx, &expected)? {
        validate_backup_files(database_path, &existing)?;
        return Ok(PreparedFieldMigrationBackupV1 {
            backup: existing,
            source_data_version: data_version,
            paths,
            stage: None,
        });
    }
    discard_owned_stages(&paths, &expected)?;

    let database_bytes = connection_database_bytes(tx)?;
    let manifest_bytes = u64::try_from(backup_manifest(&expected).len()).map_err(|_| {
        StoreError::StorageBudgetExceeded {
            resource: "field_backup.manifest_bytes",
            limit: u64::MAX,
            actual: u64::MAX,
        }
    })?;
    let required_space =
        database_bytes
            .checked_add(manifest_bytes)
            .ok_or(StoreError::StorageBudgetExceeded {
                resource: "field_backup.free_space",
                limit: u64::MAX,
                actual: u64::MAX,
            })?;
    let available_space =
        ae_platform_fs::available_space(&paths.root).map_err(|source| StoreError::Io {
            context: "checking field migration backup free space",
            source,
        })?;
    if available_space < required_space {
        return Err(StoreError::StorageBudgetExceeded {
            resource: "field_backup.free_space",
            limit: available_space,
            actual: required_space,
        });
    }

    let stage = paths.root.join(format!(
        ".stage-{}-{}",
        ae_contracts::hex::encode32(&expected.migration_id),
        stage_nonce(&expected)
    ));
    fs::create_dir(&stage).map_err(|source| StoreError::Io {
        context: "creating field migration backup stage",
        source,
    })?;
    let database = stage.join("authority.sqlite");
    let manifest = stage.join("manifest");
    let data_version_before = data_version;
    fire_semantic_migration_failpoint_v1(SemanticMigrationTestFailpointV1::BeforeSnapshot)?;
    tx.backup(rusqlite::DatabaseName::Main, &database, None)
        .map_err(|_| StoreError::FieldMigrationBackup {
            context: "capturing authority preimage",
        })?;
    fire_semantic_migration_failpoint_v1(SemanticMigrationTestFailpointV1::AfterSnapshot)?;
    fire_semantic_migration_failpoint_v1(SemanticMigrationTestFailpointV1::BeforeDatabaseSync)?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(&database)
        .map_err(|source| StoreError::Io {
            context: "opening field migration backup for sync",
            source,
        })?
        .sync_all()
        .map_err(|source| StoreError::Io {
            context: "syncing field migration backup",
            source,
        })?;
    fire_semantic_migration_failpoint_v1(SemanticMigrationTestFailpointV1::AfterDatabaseSync)?;
    let (byte_len, sha256) = sha256_file(&database)?;
    let backup = FieldMigrationPreimageBackupV1 {
        byte_len,
        sha256,
        ..expected
    };
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&manifest)
        .map_err(|source| StoreError::Io {
            context: "creating field migration backup manifest",
            source,
        })?;
    output
        .write_all(&backup_manifest(&backup))
        .map_err(|source| StoreError::Io {
            context: "writing field migration backup manifest",
            source,
        })?;
    fire_semantic_migration_failpoint_v1(SemanticMigrationTestFailpointV1::BeforeManifestSync)?;
    output.sync_all().map_err(|source| StoreError::Io {
        context: "syncing field migration backup manifest",
        source,
    })?;
    drop(output);
    fire_semantic_migration_failpoint_v1(SemanticMigrationTestFailpointV1::AfterManifestSync)?;
    let mut validated_stage = inspect_backup_directory(&stage, &backup)?;
    if validated_stage.backup != backup {
        return Err(StoreError::ContinuityFence("field_backup_record"));
    }
    fire_semantic_migration_failpoint_v1(SemanticMigrationTestFailpointV1::BeforeDirectorySync)?;
    validated_stage
        .package
        .sync_directory()
        .map_err(|source| StoreError::Io {
            context: "syncing exact field migration backup package",
            source,
        })?;
    fire_semantic_migration_failpoint_v1(SemanticMigrationTestFailpointV1::AfterDirectorySync)?;
    drop(validated_stage);
    if source_data_version(tx)? != data_version_before {
        remove_owned_stage(&stage)?;
        return Err(StoreError::ContinuityFence("field_backup_source_changed"));
    }
    if paths.final_dir.exists() {
        let existing = validate_backup_directory(&paths.final_dir, &backup)?;
        if existing.byte_len == backup.byte_len && existing.sha256 == backup.sha256 {
            remove_owned_stage(&stage)?;
            // A prior attempt may have completed the rename but failed at the
            // parent durability seam. The exact whole-database SHA proves this
            // is the snapshot just captured, so it is safe to reuse.
            complete_published_directory(&paths.root)?;
            return Ok(PreparedFieldMigrationBackupV1 {
                backup: existing,
                source_data_version: data_version_before,
                paths,
                stage: None,
            });
        }
        // Keep the fresh stage until BEGIN IMMEDIATE proves the existing final
        // is uncommitted. Only that writer lock makes replacement safe.
        return Ok(PreparedFieldMigrationBackupV1 {
            backup,
            source_data_version: data_version_before,
            paths,
            stage: Some(stage),
        });
    }
    publish_directory(&stage, &paths.final_dir, &paths.root)?;
    Ok(PreparedFieldMigrationBackupV1 {
        backup,
        source_data_version: data_version_before,
        paths,
        stage: None,
    })
}

fn finalize_prepared_backup(
    tx: &Transaction<'_>,
    database_path: &Path,
    mut prepared: PreparedFieldMigrationBackupV1,
) -> Result<FieldMigrationPreimageBackupV1, StoreError> {
    if let Some(existing) = backup_from_row(tx, &prepared.backup)? {
        validate_backup_files(database_path, &existing)?;
        if let Some(stage) = prepared.stage.take() {
            remove_owned_stage(&stage)?;
        }
        remove_stale_final(&prepared.paths, &prepared.backup)?;
        return Ok(existing);
    }
    let Some(stage) = prepared.stage.take() else {
        let existing = validate_backup_directory(&prepared.paths.final_dir, &prepared.backup)?;
        if existing.byte_len != prepared.backup.byte_len
            || existing.sha256 != prepared.backup.sha256
        {
            return Err(StoreError::ContinuityFence("field_backup_hash"));
        }
        complete_published_directory(&prepared.paths.root)?;
        remove_stale_final(&prepared.paths, &prepared.backup)?;
        return Ok(existing);
    };
    if prepared.paths.final_dir.exists() {
        let existing = validate_backup_directory(&prepared.paths.final_dir, &prepared.backup)?;
        if existing.byte_len == prepared.backup.byte_len
            && existing.sha256 == prepared.backup.sha256
        {
            remove_owned_stage(&stage)?;
            // A previous attempt may have stopped after the atomic rename but
            // before syncing the parent. Complete that seam before returning.
            complete_published_directory(&prepared.paths.root)?;
            return Ok(existing);
        }
        // The caller owns BEGIN IMMEDIATE and backup_from_row proved that no
        // committed package owns this path. A different whole-database SHA is
        // therefore a stale orphan from a failed pre-CAS attempt.
        quarantine_uncommitted_final(&prepared.paths, &prepared.backup)?;
    }
    publish_directory(&stage, &prepared.paths.final_dir, &prepared.paths.root)?;
    validate_backup_files(database_path, &prepared.backup)?;
    remove_stale_final(&prepared.paths, &prepared.backup)?;
    Ok(prepared.backup)
}

fn database_path(conn: &Connection) -> Result<PathBuf, StoreError> {
    let stored: Option<(Option<String>, i64)> = conn
        .query_row(
            "SELECT
                CASE WHEN typeof(file)='text' AND length(CAST(file AS BLOB))<=?1 THEN file END,
                CASE WHEN typeof(file)='text' THEN length(CAST(file AS BLOB)) ELSE -1 END
             FROM pragma_database_list WHERE name='main'",
            params![MAX_SQLITE_DATABASE_PATH_BYTES],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let (file, file_len) = stored.ok_or(StoreError::ContinuityFence("field_backup_path"))?;
    let file = bounded_typed_value(
        file,
        file_len,
        MAX_SQLITE_DATABASE_PATH_BYTES,
        "sqlite_database.main.path",
        "field_backup_path",
    )?;
    if file.is_empty() {
        return Err(StoreError::ContinuityFence("field_backup_path"));
    }
    Ok(PathBuf::from(file))
}

pub(crate) struct ActiveSemanticIdentityV1 {
    pub(crate) incarnation_id: Digest,
    pub(crate) manifest_digest: Digest,
    pub(crate) formula_digest: Digest,
    pub(crate) initial_snapshot_digest: Digest,
    pub(crate) development_seed_digest: Digest,
    pub(crate) baseline_field: NeuralField,
    pub(crate) baseline_graph: SparseGraph,
}

fn encode_legacy_runtime_genesis_state_v1(
    field: &NeuralField,
    graph: &SparseGraph,
) -> Result<Vec<u8>, StoreError> {
    let mut bytes = encode_field(field)?;
    bytes.extend_from_slice(
        &u32::try_from(graph.row_offsets.len())
            .map_err(|_| StoreError::ContinuityFence("semantic_genesis_root"))?
            .to_le_bytes(),
    );
    for offset in &graph.row_offsets {
        bytes.extend_from_slice(&offset.to_le_bytes());
    }
    bytes.extend_from_slice(
        &u32::try_from(graph.edges.len())
            .map_err(|_| StoreError::ContinuityFence("semantic_genesis_root"))?
            .to_le_bytes(),
    );
    Ok(bytes)
}

struct AttestedLegacyHistoryV1 {
    latest_snapshot: DecodedSemanticSnapshotV2,
    context_state_bytes: Vec<u8>,
    relation_scope_token: [u8; 16],
}

fn semantic_table_budget(
    tx: &Transaction<'_>,
    scope_digest: &Digest,
    sql: &str,
    row_resource: &'static str,
    byte_resource: &'static str,
) -> Result<u64, StoreError> {
    let (raw_rows, raw_bytes): (i64, i64) =
        tx.query_row(sql, params![blob(*scope_digest)], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
    let rows = sqlite_length(raw_rows, row_resource)?;
    let bytes = sqlite_length(raw_bytes, byte_resource)?;
    enforce_read_budget(
        row_resource,
        byte_resource,
        rows,
        bytes,
        MAX_SEMANTIC_HISTORY_ROWS,
        MAX_SEMANTIC_HISTORY_BYTES,
    )?;
    Ok(bytes)
}

fn enforce_semantic_history_budget(
    tx: &Transaction<'_>,
    scope_digest: &Digest,
    through_revision: JournalRevision,
) -> Result<(), StoreError> {
    enforce_connection_database_budget(tx)?;
    enforce_byte_budget(
        "semantic.history.rows",
        through_revision.get(),
        MAX_SEMANTIC_HISTORY_ROWS,
    )?;
    let mut aggregate = 0_u64;
    for bytes in [
        semantic_table_budget(
            tx,
            scope_digest,
            "SELECT COUNT(*), COALESCE(SUM(length(CAST(event_kind AS BLOB))+length(event_bytes)+length(event_digest)+length(receipt_bytes)+length(delta_bytes)+length(chain_digest)),0) FROM journal WHERE scope_digest=?1",
            "semantic.journal.rows",
            "semantic.journal.bytes",
        )?,
        semantic_table_budget(
            tx,
            scope_digest,
            "SELECT COUNT(*), COALESCE(SUM(length(event_digest)+8),0) FROM applied_events WHERE scope_digest=?1",
            "semantic.applied_events.rows",
            "semantic.applied_events.bytes",
        )?,
        semantic_table_budget(
            tx,
            scope_digest,
            "SELECT COUNT(*), COALESCE(SUM(length(state_digest)+length(state_bytes)),0) FROM snapshots WHERE scope_digest=?1",
            "semantic.snapshots.rows",
            "semantic.snapshots.bytes",
        )?,
        semantic_table_budget(
            tx,
            scope_digest,
            "SELECT COUNT(*), COALESCE(SUM(length(base_graph_digest)+length(graph_digest)+length(formula_digest)+length(delta_bytes)+length(replay_state_bytes)),0) FROM graph_commits WHERE scope_digest=?1",
            "semantic.graph.rows",
            "semantic.graph.bytes",
        )?,
        semantic_table_budget(
            tx,
            scope_digest,
            "SELECT COUNT(*), COALESCE(SUM(length(relation_scope_token)+length(relation_hmac)+length(context_digest)+length(canonical_state_bytes)),0) FROM context_commits WHERE scope_digest=?1",
            "semantic.context.rows",
            "semantic.context.bytes",
        )?,
    ] {
        aggregate = aggregate
            .checked_add(bytes)
            .ok_or(StoreError::StorageBudgetExceeded {
                resource: "semantic.history.bytes",
                limit: MAX_SEMANTIC_HISTORY_BYTES,
                actual: u64::MAX,
            })?;
    }
    enforce_byte_budget(
        "semantic.history.bytes",
        aggregate,
        MAX_SEMANTIC_HISTORY_BYTES,
    )
}

fn attest_semantic_authority_prefix_sets(
    tx: &Transaction<'_>,
    scope_digest: &Digest,
    relation_scope_token: [u8; 16],
    through_revision: JournalRevision,
) -> Result<(), StoreError> {
    let through_sql = through_revision.to_sqlite()?.get();
    let expected_len = through_revision.get();

    {
        let mut statement = tx.prepare(
            "SELECT logical_revision FROM journal WHERE scope_digest=?1 AND logical_revision<=?2 ORDER BY logical_revision,revision",
        )?;
        let mut rows = statement.query(params![blob(*scope_digest), through_sql])?;
        let mut expected = 1_u64;
        while let Some(row) = rows.next()? {
            let revision = JournalRevision::try_from(row.get::<_, i64>(0)?)?.get();
            if expected > expected_len || revision != expected {
                return Err(StoreError::ContinuityFence("semantic_history_journal_set"));
            }
            expected = expected
                .checked_add(1)
                .ok_or(StoreError::RevisionOutOfRange { revision: expected })?;
        }
        if expected != expected_len.saturating_add(1) {
            return Err(StoreError::ContinuityFence("semantic_history_journal_set"));
        }
    }

    for (sql, fence) in [
        (
            "SELECT revision FROM snapshots WHERE scope_digest=?1 AND revision<=?2 ORDER BY revision",
            "semantic_history_snapshot_set",
        ),
        (
            "SELECT revision FROM graph_commits WHERE scope_digest=?1 AND revision<=?2 ORDER BY revision",
            "semantic_history_graph_set",
        ),
    ] {
        let mut statement = tx.prepare(sql)?;
        let mut rows = statement.query(params![blob(*scope_digest), through_sql])?;
        let mut expected = 1_u64;
        while let Some(row) = rows.next()? {
            let revision = JournalRevision::try_from(row.get::<_, i64>(0)?)?.get();
            if expected > expected_len || revision != expected {
                return Err(StoreError::ContinuityFence(fence));
            }
            expected = expected
                .checked_add(1)
                .ok_or(StoreError::RevisionOutOfRange { revision: expected })?;
        }
        if expected != expected_len.saturating_add(1) {
            return Err(StoreError::ContinuityFence(fence));
        }
    }

    {
        let mut statement = tx.prepare(
            "SELECT revision,
                CASE WHEN typeof(relation_scope_token)='blob' AND length(relation_scope_token)=16
                     THEN relation_scope_token ELSE zeroblob(0) END
             FROM context_commits WHERE scope_digest=?1 AND revision<=?2
             ORDER BY revision,relation_scope_token",
        )?;
        let mut rows = statement.query(params![blob(*scope_digest), through_sql])?;
        let mut expected = 1_u64;
        while let Some(row) = rows.next()? {
            let revision = JournalRevision::try_from(row.get::<_, i64>(0)?)?.get();
            let relation: Vec<u8> = row.get(1)?;
            if expected > expected_len
                || revision != expected
                || relation.as_slice() != relation_scope_token
            {
                return Err(StoreError::ContinuityFence("semantic_history_context_set"));
            }
            expected = expected
                .checked_add(1)
                .ok_or(StoreError::RevisionOutOfRange { revision: expected })?;
        }
        if expected != expected_len.saturating_add(1) {
            return Err(StoreError::ContinuityFence("semantic_history_context_set"));
        }
    }

    let applied_without_journal: i64 = tx.query_row(
        "SELECT COUNT(*) FROM applied_events AS ae
         LEFT JOIN journal AS j
           ON j.scope_digest=ae.scope_digest
          AND j.event_digest=ae.event_digest
          AND j.logical_revision=ae.revision
         WHERE ae.scope_digest=?1
           AND ae.revision<=?2
           AND (j.revision IS NULL OR ae.revision<1)",
        params![blob(*scope_digest), through_sql],
        |row| row.get(0),
    )?;
    let journal_without_applied: i64 = tx.query_row(
        "SELECT COUNT(*) FROM journal AS j
         LEFT JOIN applied_events AS ae
           ON ae.scope_digest=j.scope_digest
          AND ae.event_digest=j.event_digest
          AND ae.revision=j.logical_revision
         WHERE j.scope_digest=?1 AND j.logical_revision<=?2 AND ae.event_digest IS NULL",
        params![blob(*scope_digest), through_sql],
        |row| row.get(0),
    )?;
    let applied_rows: i64 = tx.query_row(
        "SELECT COUNT(*) FROM applied_events WHERE scope_digest=?1 AND revision<=?2",
        params![blob(*scope_digest), through_sql],
        |row| row.get(0),
    )?;
    if applied_without_journal != 0
        || journal_without_applied != 0
        || sqlite_length(applied_rows, "semantic.applied_events.rows")? != expected_len
    {
        return Err(StoreError::ContinuityFence(
            "semantic_history_applied_events_set",
        ));
    }
    Ok(())
}

fn attest_exact_semantic_authority_sets(
    tx: &Transaction<'_>,
    scope_digest: &Digest,
    relation_scope_token: [u8; 16],
    through_revision: JournalRevision,
) -> Result<(), StoreError> {
    attest_semantic_authority_prefix_sets(
        tx,
        scope_digest,
        relation_scope_token,
        through_revision,
    )?;
    let through_sql = through_revision.to_sqlite()?.get();
    for (sql, fence) in [
        (
            "SELECT COUNT(*) FROM journal WHERE scope_digest=?1 AND logical_revision>?2",
            "semantic_history_journal_set",
        ),
        (
            "SELECT COUNT(*) FROM applied_events WHERE scope_digest=?1 AND revision>?2",
            "semantic_history_applied_events_set",
        ),
        (
            "SELECT COUNT(*) FROM snapshots WHERE scope_digest=?1 AND revision>?2",
            "semantic_history_snapshot_set",
        ),
        (
            "SELECT COUNT(*) FROM graph_commits WHERE scope_digest=?1 AND revision>?2",
            "semantic_history_graph_set",
        ),
        (
            "SELECT COUNT(*) FROM context_commits WHERE scope_digest=?1 AND revision>?2",
            "semantic_history_context_set",
        ),
    ] {
        let extras: i64 = tx.query_row(sql, params![blob(*scope_digest), through_sql], |row| {
            row.get(0)
        })?;
        if extras != 0 {
            return Err(StoreError::ContinuityFence(fence));
        }
    }
    Ok(())
}

/// Single admission point for revisions after a committed migration boundary.
/// The migration row authenticates only the exact prefix through `boundary`;
/// later revisions stay closed until Task 7 adds replayable Store-owned
/// dynamics authority ahead of the structural closure checks below.
// Preserve the established storage/API shape in this compatibility boundary.
#[allow(clippy::too_many_arguments)]
fn attest_post_migration_semantic_suffix(
    tx: &Transaction<'_>,
    event_scope: &ScopeRef,
    scope_digest: &Digest,
    relation_scope_token: [u8; 16],
    boundary: JournalRevision,
    formula_digest: Digest,
    mut prior_state_digest: Digest,
    mut prior_graph_digest: Digest,
    mut prior_chain_digest: Digest,
    mut prior_context_bytes: Vec<u8>,
) -> Result<(), StoreError> {
    let current_sql: i64 = tx.query_row(
        "SELECT COALESCE(MAX(logical_revision),0) FROM journal WHERE scope_digest=?1",
        params![blob(*scope_digest)],
        |row| row.get(0),
    )?;
    let current = JournalRevision::try_from(current_sql)?;
    if current < boundary {
        return Err(StoreError::ContinuityFence("semantic_suffix_revision"));
    }
    // Until Task 7 persists a Store-owned dynamics commitment for ordinary
    // semantic events, AESEM3 + receipt + sidecars are only mutually
    // consistent caller-supplied data.  Keep the replay/closure code below as
    // the single admission point for that future authority, but never treat
    // those self-sealed artifacts as sufficient evidence today.
    if current > boundary {
        return Err(StoreError::SemanticSuffixAuthorityUnavailable {
            boundary_revision: boundary.get(),
            current_revision: current.get(),
        });
    }
    enforce_semantic_history_budget(tx, scope_digest, current)?;
    let suffix_len = current
        .get()
        .checked_sub(boundary.get())
        .ok_or(StoreError::ContinuityFence("semantic_suffix_revision"))?;
    let boundary_sql = boundary.to_sqlite()?.get();

    for revision_value in boundary.get().saturating_add(1)..=current.get() {
        let revision = JournalRevision::new(revision_value);
        let revision_sql = revision.to_sqlite()?.get();
        let row = query_bounded_journal_row(tx, scope_digest, revision)?
            .ok_or(StoreError::ContinuityFence("semantic_suffix_journal"))?;
        let base = JournalRevision::new(row.base_revision);
        if base.checked_next()? != revision {
            return Err(StoreError::ContinuityFence("semantic_suffix_revision"));
        }
        let event = wire::decode_event(&row.event_bytes)
            .map_err(|_| StoreError::ContinuityFence("semantic_suffix_event"))?;
        let CanonicalEvent::UserStimulus(stimulus) = &event else {
            return Err(StoreError::ContinuityFence("semantic_suffix_event"));
        };
        if wire::encode_event(&event) != row.event_bytes
            || wire::event_kind_name(&event) != row.event_kind
            || wire::event_digest(&event) != row.event_digest
            || stimulus.scope != *event_scope
            || stimulus.causal.base_revision != base.get()
            || stimulus.evidence.schema_version != 1
        {
            return Err(StoreError::ContinuityFence("semantic_suffix_event"));
        }
        let receipt = wire::decode_transition_receipt(&row.receipt_bytes)
            .map_err(|_| StoreError::ContinuityFence("semantic_suffix_receipt"))?;
        if wire::encode_transition_receipt(&receipt) != row.receipt_bytes
            || receipt.schema_version != 1
            || receipt.status != CommitStatus::Committed
            || receipt.scope_digest != *scope_digest
            || receipt.event_digest != row.event_digest
            || receipt.authority_digest != ae_authority::authority_projection_digest(&event)
            || receipt.formula_digest != formula_digest
            || receipt.base_revision != base.get()
            || receipt.next_revision != revision.get()
            || receipt.state_before != prior_state_digest
        {
            return Err(StoreError::ContinuityFence("semantic_suffix_receipt"));
        }
        let expected_chain = if row.delta_bytes.is_empty() {
            ae_continuum::chain_link(&prior_chain_digest, &row.event_bytes, &row.receipt_bytes)
        } else {
            ae_continuum::chain_link_with_delta(
                &prior_chain_digest,
                &row.event_bytes,
                &row.receipt_bytes,
                &row.delta_bytes,
            )
        };
        if row.chain_digest != expected_chain {
            return Err(StoreError::ContinuityFence("semantic_suffix_chain"));
        }

        let snapshot = query_bounded_snapshot_row(tx, scope_digest, revision)?
            .ok_or(StoreError::ContinuityFence("semantic_suffix_snapshot"))?;
        if snapshot.state_digest != receipt.state_after {
            return Err(StoreError::ContinuityFence("semantic_suffix_snapshot"));
        }
        let decoded = decode_canonical_semantic_snapshot_v3(
            &snapshot.state_bytes,
            &formula_digest,
            &snapshot.state_digest,
            &receipt.graph_after,
            &receipt,
        )?;
        if receipt.active_nodes != decoded.field.active_node_count()
            || usize::try_from(receipt.active_edges).ok() != Some(decoded.graph.edges.len())
        {
            return Err(StoreError::ContinuityFence("semantic_suffix_snapshot"));
        }

        type GraphColumns = (
            Option<Vec<u8>>,
            i64,
            Option<Vec<u8>>,
            i64,
            Option<Vec<u8>>,
            i64,
            Option<Vec<u8>>,
            i64,
            Option<Vec<u8>>,
            i64,
        );
        let graph: GraphColumns = tx
            .query_row(
                "SELECT
                    CASE WHEN typeof(base_graph_digest)='blob' AND length(base_graph_digest)=32 THEN base_graph_digest END,
                    CASE WHEN typeof(base_graph_digest)='blob' THEN length(base_graph_digest) ELSE -1 END,
                    CASE WHEN typeof(graph_digest)='blob' AND length(graph_digest)=32 THEN graph_digest END,
                    CASE WHEN typeof(graph_digest)='blob' THEN length(graph_digest) ELSE -1 END,
                    CASE WHEN typeof(formula_digest)='blob' AND length(formula_digest)=32 THEN formula_digest END,
                    CASE WHEN typeof(formula_digest)='blob' THEN length(formula_digest) ELSE -1 END,
                    CASE WHEN typeof(delta_bytes)='blob' AND length(delta_bytes)<=?3 THEN delta_bytes END,
                    CASE WHEN typeof(delta_bytes)='blob' THEN length(delta_bytes) ELSE -1 END,
                    CASE WHEN typeof(replay_state_bytes)='blob' AND length(replay_state_bytes)<=?4 THEN replay_state_bytes END,
                    CASE WHEN typeof(replay_state_bytes)='blob' THEN length(replay_state_bytes) ELSE -1 END
                 FROM graph_commits WHERE scope_digest=?1 AND revision=?2",
                params![
                    blob(*scope_digest),
                    revision_sql,
                    MAX_JOURNAL_DELTA_BYTES,
                    u64::try_from(GRAPH_WIRE_MAX_LEN).unwrap_or(u64::MAX),
                ],
                |row| {
                    Ok((
                        row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?,
                        row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?,
                    ))
                },
            )
            .optional()?
            .ok_or(StoreError::ContinuityFence("semantic_suffix_graph"))?;
        let graph_delta = bounded_typed_value(
            graph.6,
            graph.7,
            MAX_JOURNAL_DELTA_BYTES,
            "graph_commits.delta_bytes",
            "graph_delta_bytes_type",
        )?;
        let graph_replay = bounded_typed_value(
            graph.8,
            graph.9,
            u64::try_from(GRAPH_WIRE_MAX_LEN).unwrap_or(u64::MAX),
            "graph_commits.replay_state_bytes",
            "graph_replay_state_bytes_type",
        )?;
        if stored_typed_digest(
            graph.0,
            graph.1,
            "graph_commits.base_graph_digest",
            "graph_base_digest_type",
        )? != prior_graph_digest
            || stored_typed_digest(
                graph.2,
                graph.3,
                "graph_commits.graph_digest",
                "graph_digest_type",
            )? != receipt.graph_after
            || stored_typed_digest(
                graph.4,
                graph.5,
                "graph_commits.formula_digest",
                "graph_formula_digest_type",
            )? != formula_digest
            || graph_delta != row.delta_bytes
            || graph_replay != decoded.graph.canonical_bytes()
            || graph_digest(&decoded.graph) != receipt.graph_after
        {
            return Err(StoreError::ContinuityFence("semantic_suffix_graph"));
        }

        type ContextColumns = (Option<Vec<u8>>, i64, Vec<u8>, Vec<u8>, Option<Vec<u8>>, i64);
        let context: ContextColumns = tx
            .query_row(
                "SELECT
                    CASE WHEN typeof(relation_scope_token)='blob' AND length(relation_scope_token)=16 THEN relation_scope_token END,
                    CASE WHEN typeof(relation_scope_token)='blob' THEN length(relation_scope_token) ELSE -1 END,
                    CASE WHEN typeof(relation_hmac)='blob' AND length(relation_hmac)=32 THEN relation_hmac ELSE zeroblob(0) END,
                    CASE WHEN typeof(context_digest)='blob' AND length(context_digest)=32 THEN context_digest ELSE zeroblob(0) END,
                    CASE WHEN typeof(canonical_state_bytes)='blob' AND length(canonical_state_bytes)<=?4 THEN canonical_state_bytes END,
                    CASE WHEN typeof(canonical_state_bytes)='blob' THEN length(canonical_state_bytes) ELSE -1 END
                 FROM context_commits WHERE scope_digest=?1 AND relation_scope_token=?2 AND revision=?3",
                params![
                    blob(*scope_digest),
                    blob(relation_scope_token),
                    revision_sql,
                    u64::try_from(CONTEXT_STATE_WIRE_MAX_LEN_V1).unwrap_or(u64::MAX),
                ],
                |row| {
                    Ok((
                        row.get(0)?, row.get(1)?, row.get(2)?,
                        row.get(3)?, row.get(4)?, row.get(5)?,
                    ))
                },
            )
            .optional()?
            .ok_or(StoreError::ContinuityFence("semantic_suffix_context"))?;
        let context_relation = bounded_typed_value(
            context.0,
            context.1,
            16,
            "context_commits.relation_scope_token",
            "context_relation_scope_token_type",
        )?;
        let context_bytes = bounded_typed_value(
            context.4,
            context.5,
            u64::try_from(CONTEXT_STATE_WIRE_MAX_LEN_V1).unwrap_or(u64::MAX),
            "context_commits.canonical_state_bytes",
            "context_state_bytes_type",
        )?;
        let expected_context = project_context_state(
            Some(&prior_context_bytes),
            &event,
            relation_scope_token,
            revision.get(),
        )?;
        if context_relation.as_slice() != relation_scope_token
            || digest_from_blob(&context.2, "context_commits.relation_hmac")?
                != context_relation_hmac(relation_scope_token)
            || digest_from_blob(&context.3, "context_commits.context_digest")?
                != continuity_context_digest(&context_bytes)
            || context_bytes != expected_context
        {
            return Err(StoreError::ContinuityFence("semantic_suffix_context"));
        }

        let applied: Option<i64> = tx
            .query_row(
                "SELECT revision FROM applied_events WHERE scope_digest=?1 AND event_digest=?2",
                params![blob(*scope_digest), blob(row.event_digest)],
                |row| row.get(0),
            )
            .optional()?;
        if applied
            .map(JournalRevision::try_from)
            .transpose()?
            .is_none_or(|applied| applied != revision)
        {
            return Err(StoreError::ContinuityFence(
                "semantic_suffix_applied_events_set",
            ));
        }

        prior_state_digest = receipt.state_after;
        prior_graph_digest = receipt.graph_after;
        prior_chain_digest = row.chain_digest;
        prior_context_bytes = expected_context;
    }

    for (sql, fence) in [
        (
            "SELECT COUNT(*) FROM applied_events WHERE scope_digest=?1 AND revision>?2",
            "semantic_suffix_applied_events_set",
        ),
        (
            "SELECT COUNT(*) FROM snapshots WHERE scope_digest=?1 AND revision>?2",
            "semantic_suffix_snapshot_set",
        ),
        (
            "SELECT COUNT(*) FROM graph_commits WHERE scope_digest=?1 AND revision>?2",
            "semantic_suffix_graph_set",
        ),
        (
            "SELECT COUNT(*) FROM context_commits WHERE scope_digest=?1 AND revision>?2",
            "semantic_suffix_context_set",
        ),
    ] {
        let rows: i64 = tx.query_row(sql, params![blob(*scope_digest), boundary_sql], |row| {
            row.get(0)
        })?;
        if sqlite_length(rows, "semantic_suffix.rows")? != suffix_len {
            return Err(StoreError::ContinuityFence(fence));
        }
    }
    Ok(())
}

// Preserve the established storage/API shape in this compatibility boundary.
#[allow(clippy::type_complexity)]
pub(crate) fn active_identity(
    tx: &Connection,
    scope: &ScopeRef,
    incarnation_id: Digest,
    manifest_digest: Digest,
) -> Result<ActiveSemanticIdentityV1, StoreError> {
    let binding: Option<(Vec<u8>, Vec<u8>, Vec<u8>, i64)> = tx
        .query_row(
            "SELECT
                CASE WHEN typeof(bot_token)='blob' AND length(bot_token)=16 THEN bot_token ELSE zeroblob(0) END,
                CASE WHEN typeof(persona_token)='blob' AND length(persona_token)=16 THEN persona_token ELSE zeroblob(0) END,
                CASE WHEN typeof(incarnation_id)='blob' AND length(incarnation_id)=32 THEN incarnation_id ELSE zeroblob(0) END,
                revision
             FROM active_bindings WHERE bot_token=?1 AND persona_token=?2",
            params![blob(scope.bot_token), blob(scope.persona_token)],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((binding_bot, binding_persona, binding_incarnation, binding_revision)) = binding
    else {
        return Err(StoreError::ContinuityFence("semantic_identity"));
    };
    let checked_binding_revision = JournalRevision::try_from(binding_revision)?;
    if binding_bot.as_slice() != scope.bot_token
        || binding_persona.as_slice() != scope.persona_token
        || digest_from_blob(&binding_incarnation, "semantic_identity")? != incarnation_id
        || checked_binding_revision.get() != 1
    {
        return Err(StoreError::ContinuityFence("semantic_identity"));
    }

    struct IncarnationColumns {
        seed: Vec<u8>,
        manifest: Vec<u8>,
        formula: Vec<u8>,
        parent_is_null: i64,
        nonce: Vec<u8>,
        status_is_active: i64,
        initial_snapshot: Vec<u8>,
        graph: Vec<u8>,
        development_seed: Vec<u8>,
        source: Vec<u8>,
        compiler_protocol: Vec<u8>,
        compiler_model: Vec<u8>,
        equilibrium_residual: i64,
        energy_residual: i64,
        capacity_residual: i64,
        sample_fit_residual: i64,
    }
    let incarnation: Option<IncarnationColumns> = tx
        .query_row(
            "SELECT
                CASE WHEN typeof(seed_code_digest)='blob' AND length(seed_code_digest)=32 THEN seed_code_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(manifest_digest)='blob' AND length(manifest_digest)=32 THEN manifest_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(formula_digest)='blob' AND length(formula_digest)=32 THEN formula_digest ELSE zeroblob(0) END,
                parent_incarnation_id IS NULL,
                CASE WHEN typeof(nonce_digest)='blob' AND length(nonce_digest)=32 THEN nonce_digest ELSE zeroblob(0) END,
                typeof(status)='text' AND status='active',
                CASE WHEN typeof(initial_snapshot_digest)='blob' AND length(initial_snapshot_digest)=32 THEN initial_snapshot_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(graph_digest)='blob' AND length(graph_digest)=32 THEN graph_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(development_seed_digest)='blob' AND length(development_seed_digest)=32 THEN development_seed_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(persona_source_digest)='blob' AND length(persona_source_digest)=32 THEN persona_source_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(compiler_protocol_digest)='blob' AND length(compiler_protocol_digest)=32 THEN compiler_protocol_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(compiler_model_digest)='blob' AND length(compiler_model_digest)=32 THEN compiler_model_digest ELSE zeroblob(0) END,
                equilibrium_residual, energy_residual, capacity_residual, sample_fit_residual
             FROM incarnations WHERE incarnation_id=?1",
            params![blob(incarnation_id)],
            |row| {
                Ok(IncarnationColumns {
                    seed: row.get(0)?, manifest: row.get(1)?, formula: row.get(2)?,
                    parent_is_null: row.get(3)?, nonce: row.get(4)?, status_is_active: row.get(5)?,
                    initial_snapshot: row.get(6)?, graph: row.get(7)?,
                    development_seed: row.get(8)?, source: row.get(9)?,
                    compiler_protocol: row.get(10)?, compiler_model: row.get(11)?,
                    equilibrium_residual: row.get(12)?, energy_residual: row.get(13)?,
                    capacity_residual: row.get(14)?, sample_fit_residual: row.get(15)?,
                })
            },
        )
        .optional()?;
    let Some(incarnation) = incarnation else {
        return Err(StoreError::ContinuityFence("semantic_identity"));
    };
    if incarnation.parent_is_null != 1 || incarnation.status_is_active != 1 {
        return Err(StoreError::ContinuityFence("semantic_identity"));
    }
    let stored_seed = digest_from_blob(&incarnation.seed, "semantic_seed")?;
    let stored_manifest = digest_from_blob(&incarnation.manifest, "semantic_identity")?;
    let formula_digest = digest_from_blob(&incarnation.formula, "semantic_identity")?;
    let nonce_digest = digest_from_blob(&incarnation.nonce, "semantic_nonce")?;
    let initial_snapshot_digest =
        digest_from_blob(&incarnation.initial_snapshot, "semantic_initial_snapshot")?;
    let initial_graph_digest = digest_from_blob(&incarnation.graph, "semantic_initial_graph")?;
    let development_seed =
        digest_from_blob(&incarnation.development_seed, "semantic_development_seed")?;
    let persona_source_digest = digest_from_blob(&incarnation.source, "semantic_source")?;
    let incarnation_protocol =
        digest_from_blob(&incarnation.compiler_protocol, "semantic_compiler")?;
    let incarnation_model = digest_from_blob(&incarnation.compiler_model, "semantic_compiler")?;

    type ManifestColumns = (
        Vec<u8>,
        Option<Vec<u8>>,
        i64,
        Option<String>,
        i64,
        Vec<u8>,
        Vec<u8>,
    );
    let manifest_row: Option<ManifestColumns> = tx
        .query_row(
            "SELECT
                CASE WHEN typeof(seed_code_digest)='blob' AND length(seed_code_digest)=32 THEN seed_code_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(canonical_bytes)='blob' AND length(canonical_bytes)<=?2 THEN canonical_bytes END,
                CASE WHEN typeof(canonical_bytes)='blob' THEN length(canonical_bytes) ELSE -1 END,
                CASE WHEN typeof(source_json)='text' AND length(CAST(source_json AS BLOB))<=?3 THEN source_json END,
                CASE WHEN typeof(source_json)='text' THEN length(CAST(source_json AS BLOB)) ELSE -1 END,
                CASE WHEN typeof(compiler_protocol_digest)='blob' AND length(compiler_protocol_digest)=32 THEN compiler_protocol_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(compiler_model_digest)='blob' AND length(compiler_model_digest)=32 THEN compiler_model_digest ELSE zeroblob(0) END
             FROM genesis_manifests WHERE manifest_digest=?1",
            params![
                blob(manifest_digest),
                MAX_GENESIS_MANIFEST_BYTES,
                MAX_GENESIS_SOURCE_JSON_BYTES,
            ],
            |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?))
            },
        )
        .optional()?;
    let Some((
        manifest_seed,
        manifest_bytes,
        manifest_bytes_len,
        source_json,
        source_json_len,
        manifest_protocol,
        manifest_model,
    )) = manifest_row
    else {
        return Err(StoreError::ContinuityFence("semantic_identity"));
    };
    let manifest_bytes = bounded_typed_value(
        manifest_bytes,
        manifest_bytes_len,
        MAX_GENESIS_MANIFEST_BYTES,
        "genesis_manifests.canonical_bytes",
        "semantic_manifest_wire",
    )?;
    let source_json = bounded_typed_value(
        source_json,
        source_json_len,
        MAX_GENESIS_SOURCE_JSON_BYTES,
        "genesis_manifests.source_json",
        "semantic_source",
    )?;
    if u64::try_from(source_json.len()).unwrap_or(u64::MAX)
        != sqlite_length(source_json_len, "genesis_manifests.source_json")?
    {
        return Err(StoreError::ContinuityFence("semantic_source"));
    }
    let source: PersonaSourceRef = serde_json::from_str(&source_json)
        .map_err(|_| StoreError::ContinuityFence("semantic_source"))?;
    let canonical_source = serde_json::to_string(&source)
        .map_err(|_| StoreError::ContinuityFence("semantic_source"))?;
    let manifest_seed = digest_from_blob(&manifest_seed, "semantic_seed")?;
    let manifest_protocol = digest_from_blob(&manifest_protocol, "semantic_compiler")?;
    let manifest_model = digest_from_blob(&manifest_model, "semantic_compiler")?;
    let expected_seed = ae_genesis::derive_seed_code_digest(&manifest_digest);
    if stored_manifest != manifest_digest
        || stored_seed != expected_seed
        || manifest_seed != expected_seed
        || incarnation_protocol != manifest_protocol
        || incarnation_model != manifest_model
        || persona_source_digest != source.source_digest
        || source.scope.bot_token != scope.bot_token
        || source.scope.persona_token != scope.persona_token
        || canonical_source != source_json
        || ae_genesis::derive_development_seed(&expected_seed, &incarnation_id, &formula_digest)
            != development_seed
    {
        return Err(StoreError::ContinuityFence("semantic_identity"));
    }
    let manifest = wire::decode_manifest_body(&manifest_bytes)
        .map_err(|_| StoreError::ContinuityFence("semantic_manifest_wire"))?;
    if manifest.schema_version != 1
        || wire::encode_manifest_body(&manifest) != manifest_bytes
        || wire::manifest_body_digest(&manifest) != stored_manifest
    {
        return Err(StoreError::ContinuityFence("semantic_manifest_closure"));
    }
    let (baseline_field, baseline_graph) =
        initial_state_from_manifest(&manifest, &formula_digest, &development_seed);
    if !baseline_field.validate()
        || !baseline_graph.validate()
        || state_digest(&baseline_field, &formula_digest) != initial_snapshot_digest
        || graph_digest(&baseline_graph) != initial_graph_digest
    {
        return Err(StoreError::ContinuityFence("semantic_genesis_closure"));
    }

    let scope_key = ae_genesis::genesis_scope_key(
        &scope.bot_token,
        &scope.persona_token,
        &source.source_digest,
        &formula_digest,
    );
    let lease: Option<(i64, i64, Vec<u8>, Vec<u8>, Vec<u8>)> = tx
        .query_row(
            "SELECT lease_epoch,typeof(status)='text' AND status='committed',
                CASE WHEN typeof(nonce_digest)='blob' AND length(nonce_digest)=32 THEN nonce_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(manifest_digest)='blob' AND length(manifest_digest)=32 THEN manifest_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(incarnation_id)='blob' AND length(incarnation_id)=32 THEN incarnation_id ELSE zeroblob(0) END
             FROM genesis_leases WHERE scope_key=?1",
            params![blob(scope_key)],
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
    let Some((lease_epoch, lease_is_committed, lease_nonce, lease_manifest, lease_incarnation)) =
        lease
    else {
        return Err(StoreError::ContinuityFence("semantic_genesis_lease"));
    };
    if JournalRevision::try_from(lease_epoch)?.get() == 0
        || lease_is_committed != 1
        || digest_from_blob(&lease_nonce, "semantic_genesis_lease")? != nonce_digest
        || digest_from_blob(&lease_manifest, "semantic_genesis_lease")? != manifest_digest
        || digest_from_blob(&lease_incarnation, "semantic_genesis_lease")? != incarnation_id
    {
        return Err(StoreError::ContinuityFence("semantic_genesis_lease"));
    }

    let root_scope = wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
    let root = query_bounded_snapshot_row(tx, &root_scope, JournalRevision::new(0))?
        .ok_or(StoreError::ContinuityFence("semantic_genesis_root"))?;
    let canonical_root = encode_field(&baseline_field)?;
    let legacy_runtime_root =
        encode_legacy_runtime_genesis_state_v1(&baseline_field, &baseline_graph)?;
    if root.state_digest != initial_snapshot_digest
        || (root.state_bytes != canonical_root && root.state_bytes != legacy_runtime_root)
    {
        return Err(StoreError::ContinuityFence("semantic_genesis_root"));
    }

    let genesis_receipt = GenesisReceipt {
        schema_version: 1,
        seed_code_digest: expected_seed,
        manifest_digest,
        incarnation_id,
        formula_digest,
        persona_source_digest,
        compiler_protocol_digest: incarnation_protocol,
        compiler_model_digest: incarnation_model,
        development_seed_digest: development_seed,
        initial_snapshot_digest,
        graph_digest: initial_graph_digest,
        equilibrium_residual: Fixed::from_raw(incarnation.equilibrium_residual),
        energy_residual: Fixed::from_raw(incarnation.energy_residual),
        capacity_residual: Fixed::from_raw(incarnation.capacity_residual),
        sample_fit_residual: Fixed::from_raw(incarnation.sample_fit_residual),
        status: GenesisStatus::Committed,
    };
    if genesis_receipt.equilibrium_residual != Fixed::ZERO
        || genesis_receipt.energy_residual != Fixed::ZERO
        || genesis_receipt.capacity_residual != Fixed::ZERO
        || genesis_receipt.sample_fit_residual != Fixed::ZERO
        || wire::decode_genesis_receipt(&wire::encode_genesis_receipt(&genesis_receipt))
            .ok()
            .as_ref()
            != Some(&genesis_receipt)
    {
        return Err(StoreError::ContinuityFence("semantic_genesis_receipt"));
    }
    Ok(ActiveSemanticIdentityV1 {
        incarnation_id,
        manifest_digest,
        formula_digest,
        initial_snapshot_digest,
        development_seed_digest: development_seed,
        baseline_field,
        baseline_graph,
    })
}

fn attest_legacy_history(
    tx: &Transaction<'_>,
    event_scope: &ScopeRef,
    upgrade: &LegacySemanticFormulaUpgradeReceiptV1,
    incarnation_id: Digest,
    manifest_digest: Digest,
) -> Result<AttestedLegacyHistoryV1, StoreError> {
    if upgrade.base_revision == 0 {
        return Err(StoreError::ContinuityFence("semantic_history_revision"));
    }
    let through_revision = JournalRevision::new(upgrade.base_revision);
    through_revision.to_sqlite()?;
    enforce_semantic_history_budget(tx, &upgrade.scope_digest, through_revision)?;
    let identity = active_identity(tx, event_scope, incarnation_id, manifest_digest)?;
    if identity.formula_digest != upgrade.from_formula_digest {
        return Err(StoreError::ContinuityFence("semantic_identity"));
    }
    let root_scope =
        wire::persona_scope_digest(&event_scope.bot_token, &event_scope.persona_token, None);
    let binding = wire::domain_hash(
        SEMANTIC_LANE_NAMESPACE_DOMAIN_V1,
        &[&root_scope, &incarnation_id, &identity.formula_digest],
    );
    let mut expected_relation = [0_u8; 16];
    expected_relation.copy_from_slice(&binding[..16]);
    let mut expected_session = [0_u8; 16];
    expected_session.copy_from_slice(&binding[16..]);
    if event_scope.relation_token != Some(expected_relation)
        || event_scope.session_token != expected_session
        || wire::persona_scope_digest(
            &event_scope.bot_token,
            &event_scope.persona_token,
            event_scope.relation_token.as_ref(),
        ) != upgrade.scope_digest
    {
        return Err(StoreError::ContinuityFence("semantic_identity"));
    }

    let mut replay_field = identity.baseline_field.clone();
    let replay_graph = identity.baseline_graph;
    let replay_graph_digest = graph_digest(&replay_graph);
    let mut chain_seed = identity.initial_snapshot_digest;
    let mut replay_context_state: Option<Vec<u8>> = None;
    let mut latest_snapshot = None;
    for revision in 1..=upgrade.base_revision {
        let revision = JournalRevision::new(revision);
        let revision_sql = revision.to_sqlite()?.get();
        let Some(row) = query_bounded_journal_row(tx, &upgrade.scope_digest, revision)? else {
            return Err(StoreError::ContinuityFence("semantic_history_journal"));
        };
        let base_revision = JournalRevision::new(row.base_revision);
        if base_revision.checked_next()? != revision || !row.delta_bytes.is_empty() {
            return Err(StoreError::ContinuityFence("semantic_history_revision"));
        }
        let event = wire::decode_event(&row.event_bytes)
            .map_err(|_| StoreError::ContinuityFence("semantic_history_event"))?;
        if wire::encode_event(&event) != row.event_bytes
            || wire::event_digest(&event) != row.event_digest
            || wire::event_kind_name(&event) != row.event_kind
        {
            return Err(StoreError::ContinuityFence("semantic_history_event"));
        }
        let CanonicalEvent::UserStimulus(stimulus) = &event else {
            return Err(StoreError::ContinuityFence("semantic_history_event"));
        };
        if stimulus.scope != *event_scope
            || stimulus.causal.base_revision != base_revision.get()
            || stimulus.evidence.schema_version != 1
        {
            return Err(StoreError::ContinuityFence("semantic_history_event"));
        }
        let receipt = wire::decode_transition_receipt(&row.receipt_bytes)
            .map_err(|_| StoreError::ContinuityFence("semantic_history_receipt"))?;
        if wire::encode_transition_receipt(&receipt) != row.receipt_bytes
            || receipt.schema_version != 1
            || receipt.status != CommitStatus::Committed
            || receipt.action_contract.is_some()
            || receipt.scope_digest != upgrade.scope_digest
            || receipt.event_digest != row.event_digest
            || receipt.formula_digest != identity.formula_digest
            || receipt.base_revision != base_revision.get()
            || receipt.next_revision != revision.get()
            || receipt.authority_digest != ae_authority::authority_projection_digest(&event)
            || row.chain_digest
                != ae_continuum::chain_link(&chain_seed, &row.event_bytes, &row.receipt_bytes)
        {
            return Err(StoreError::ContinuityFence("semantic_history_receipt"));
        }
        let Some(snapshot_row) = query_bounded_snapshot_row(tx, &upgrade.scope_digest, revision)?
        else {
            return Err(StoreError::ContinuityFence("semantic_history_snapshot"));
        };
        let snapshot = decode_semantic_snapshot_v2(
            &snapshot_row.state_bytes,
            &identity.formula_digest,
            &snapshot_row.state_digest,
            &receipt.graph_after,
            &receipt,
        )?;
        if !p_and_e_within_legacy_revision_bound(&snapshot.field, revision.get()) {
            return Err(StoreError::ContinuityFence("semantic_history_range"));
        }
        type GraphColumns = (
            Option<Vec<u8>>,
            i64,
            Option<Vec<u8>>,
            i64,
            Option<Vec<u8>>,
            i64,
            Option<Vec<u8>>,
            i64,
            Option<Vec<u8>>,
            i64,
        );
        let graph: Option<GraphColumns> = tx
            .query_row(
                "SELECT
                    CASE WHEN typeof(base_graph_digest)='blob' AND length(base_graph_digest)=32 THEN base_graph_digest END,
                    CASE WHEN typeof(base_graph_digest)='blob' THEN length(base_graph_digest) ELSE -1 END,
                    CASE WHEN typeof(graph_digest)='blob' AND length(graph_digest)=32 THEN graph_digest END,
                    CASE WHEN typeof(graph_digest)='blob' THEN length(graph_digest) ELSE -1 END,
                    CASE WHEN typeof(formula_digest)='blob' AND length(formula_digest)=32 THEN formula_digest END,
                    CASE WHEN typeof(formula_digest)='blob' THEN length(formula_digest) ELSE -1 END,
                    CASE WHEN typeof(delta_bytes)='blob' AND length(delta_bytes)<=?3 THEN delta_bytes END,
                    CASE WHEN typeof(delta_bytes)='blob' THEN length(delta_bytes) ELSE -1 END,
                    CASE WHEN typeof(replay_state_bytes)='blob' AND length(replay_state_bytes)<=?4 THEN replay_state_bytes END,
                    CASE WHEN typeof(replay_state_bytes)='blob' THEN length(replay_state_bytes) ELSE -1 END
                 FROM graph_commits WHERE scope_digest = ?1 AND revision = ?2",
                params![
                    blob(upgrade.scope_digest),
                    revision_sql,
                    MAX_LEGACY_UPGRADE_BYTES,
                    u64::try_from(GRAPH_WIRE_MAX_LEN).unwrap_or(u64::MAX),
                ],
                |row| {
                    Ok((
                        row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?,
                        row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            graph_base,
            graph_base_len,
            graph_after,
            graph_after_len,
            graph_formula,
            graph_formula_len,
            graph_delta,
            graph_delta_len,
            graph_replay,
            graph_replay_len,
        )) = graph
        else {
            return Err(StoreError::ContinuityFence("semantic_history_graph"));
        };
        let graph_delta = bounded_typed_value(
            graph_delta,
            graph_delta_len,
            MAX_LEGACY_UPGRADE_BYTES,
            "graph_commits.delta_bytes",
            "graph_delta_bytes_type",
        )?;
        let graph_replay = bounded_typed_value(
            graph_replay,
            graph_replay_len,
            u64::try_from(GRAPH_WIRE_MAX_LEN).unwrap_or(u64::MAX),
            "graph_commits.replay_state_bytes",
            "graph_replay_state_bytes_type",
        )?;
        if stored_typed_digest(
            graph_base,
            graph_base_len,
            "graph_commits.base_graph_digest",
            "graph_base_digest_type",
        )? != replay_graph_digest
            || stored_typed_digest(
                graph_after,
                graph_after_len,
                "graph_commits.graph_digest",
                "graph_digest_type",
            )? != graph_digest(&snapshot.graph)
            || stored_typed_digest(
                graph_formula,
                graph_formula_len,
                "graph_commits.formula_digest",
                "graph_formula_digest_type",
            )? != identity.formula_digest
            || !graph_delta.is_empty()
            || graph_replay != snapshot.graph.canonical_bytes()
        {
            return Err(StoreError::ContinuityFence("semantic_history_graph"));
        }
        type ContextColumns = (
            Option<Vec<u8>>,
            i64,
            Option<Vec<u8>>,
            i64,
            Option<Vec<u8>>,
            i64,
            Option<Vec<u8>>,
            i64,
        );
        let context: Option<ContextColumns> = tx
            .query_row(
                "SELECT
                    CASE WHEN typeof(relation_scope_token)='blob' AND length(relation_scope_token)<=16 THEN relation_scope_token END,
                    CASE WHEN typeof(relation_scope_token)='blob' THEN length(relation_scope_token) ELSE -1 END,
                    CASE WHEN typeof(relation_hmac)='blob' AND length(relation_hmac)=32 THEN relation_hmac END,
                    CASE WHEN typeof(relation_hmac)='blob' THEN length(relation_hmac) ELSE -1 END,
                    CASE WHEN typeof(context_digest)='blob' AND length(context_digest)=32 THEN context_digest END,
                    CASE WHEN typeof(context_digest)='blob' THEN length(context_digest) ELSE -1 END,
                    CASE WHEN typeof(canonical_state_bytes)='blob' AND length(canonical_state_bytes)<=?4 THEN canonical_state_bytes END,
                    CASE WHEN typeof(canonical_state_bytes)='blob' THEN length(canonical_state_bytes) ELSE -1 END
                 FROM context_commits WHERE scope_digest = ?1 AND relation_scope_token = ?2 AND revision = ?3",
                params![
                    blob(upgrade.scope_digest),
                    blob(expected_relation),
                    revision_sql,
                    u64::try_from(CONTEXT_STATE_WIRE_MAX_LEN_V1).unwrap_or(u64::MAX),
                ],
                |row| {
                    Ok((
                        row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?,
                        row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            context_relation,
            context_relation_len,
            context_hmac,
            context_hmac_len,
            context_digest,
            context_digest_len,
            context_bytes,
            context_bytes_len,
        )) = context
        else {
            return Err(StoreError::ContinuityFence("semantic_history_context"));
        };
        let context_relation = bounded_typed_value(
            context_relation,
            context_relation_len,
            16,
            "context_commits.relation_scope_token",
            "context_relation_scope_token_type",
        )?;
        let context_bytes = bounded_typed_value(
            context_bytes,
            context_bytes_len,
            u64::try_from(CONTEXT_STATE_WIRE_MAX_LEN_V1).unwrap_or(u64::MAX),
            "context_commits.canonical_state_bytes",
            "context_state_bytes_type",
        )?;
        let expected_context = project_context_state(
            replay_context_state.as_deref(),
            &event,
            expected_relation,
            revision.get(),
        )?;
        if context_relation.as_slice() != expected_relation
            || stored_typed_digest(
                context_hmac,
                context_hmac_len,
                "context_commits.relation_hmac",
                "context_relation_hmac_type",
            )? != context_relation_hmac(expected_relation)
            || stored_typed_digest(
                context_digest,
                context_digest_len,
                "context_commits.context_digest",
                "context_digest_type",
            )? != continuity_context_digest(&context_bytes)
            || decode_context_state(&context_bytes).is_err()
            || context_bytes != expected_context
        {
            return Err(StoreError::ContinuityFence("semantic_history_context"));
        }
        let replay = replay_legacy_aesem2_transition_v1(
            &replay_field,
            &identity.baseline_field,
            &stimulus.evidence.dimensions,
            stimulus.evidence.estimator_confidence,
        )?;
        if state_digest(&replay_field, &identity.formula_digest) != receipt.state_before
            || state_digest(&replay.next_field, &identity.formula_digest) != receipt.state_after
            || state_digest(&snapshot.field, &identity.formula_digest) != receipt.state_after
            || graph_digest(&snapshot.graph) != replay_graph_digest
            || receipt.graph_after != replay_graph_digest
            || receipt.active_nodes != replay.active_nodes
            || receipt.active_edges != 0
            || receipt.residuals != InvariantResiduals::default()
        {
            return Err(StoreError::ContinuityFence("semantic_history_replay"));
        }
        replay_field = replay.next_field;
        chain_seed = row.chain_digest;
        replay_context_state = Some(expected_context);
        latest_snapshot = Some(snapshot);
    }
    let latest_snapshot =
        latest_snapshot.ok_or(StoreError::ContinuityFence("semantic_history_snapshot"))?;
    let context_state_bytes =
        replay_context_state.ok_or(StoreError::ContinuityFence("semantic_history_context"))?;
    if state_digest(&replay_field, &identity.formula_digest) != upgrade.source_state_digest
        || state_digest(&latest_snapshot.field, &identity.formula_digest)
            != upgrade.source_state_digest
        || graph_digest(&latest_snapshot.graph) != upgrade.source_graph_digest
        || replay_graph_digest != upgrade.source_graph_digest
        || chain_seed != upgrade.prior_chain_digest
    {
        return Err(StoreError::ContinuityFence("semantic_history_source"));
    }
    Ok(AttestedLegacyHistoryV1 {
        latest_snapshot,
        context_state_bytes,
        relation_scope_token: expected_relation,
    })
}

fn bounded_headroom(used: usize, limit: usize) -> Result<Fixed, StoreError> {
    if limit == 0 || used > limit {
        return Err(StoreError::ContinuityFence("semantic_migration_capacity"));
    }
    let numerator = i128::try_from(used)
        .ok()
        .and_then(|value| value.checked_mul(i128::from(Fixed::ONE.raw())))
        .ok_or(StoreError::ContinuityFence("semantic_migration_capacity"))?;
    let denominator = i128::try_from(limit)
        .map_err(|_| StoreError::ContinuityFence("semantic_migration_capacity"))?;
    let ratio = numerator
        .checked_add(denominator / 2)
        .ok_or(StoreError::ContinuityFence("semantic_migration_capacity"))?
        / denominator;
    let raw = i128::from(Fixed::ONE.raw())
        .checked_sub(ratio)
        .and_then(|value| i64::try_from(value).ok())
        .ok_or(StoreError::ContinuityFence("semantic_migration_capacity"))?;
    Ok(Fixed::from_raw(raw))
}

fn derive_migration_artifacts_v2(
    tx: &Transaction<'_>,
    request: &SemanticMigrationRequestV2<'_>,
) -> Result<SemanticMigrationArtifactsV2, StoreError> {
    let base = JournalRevision::new(request.expected_source_revision);
    let next = base.checked_next()?;
    base.to_sqlite()?;
    next.to_sqlite()?;
    let scope_digest = wire::persona_scope_digest(
        &request.scope.bot_token,
        &request.scope.persona_token,
        request.scope.relation_token.as_ref(),
    );
    let identity = active_identity(
        tx,
        request.scope,
        request.expected_incarnation_id,
        request.expected_manifest_digest,
    )?;
    let target_formula = phase0_canonical_formula_digest_v1(&identity.formula_digest);
    let provisional = LegacySemanticFormulaUpgradeReceiptV1 {
        scope_digest,
        event_digest: [0; 32],
        receipt_digest: [0; 32],
        base_revision: base.get(),
        next_revision: next.get(),
        source_state_digest: request.expected_source_state_digest,
        target_state_before: [0; 32],
        source_graph_digest: request.expected_source_graph_digest,
        prior_chain_digest: request.expected_source_history_root,
        from_formula_digest: identity.formula_digest,
        to_formula_digest: target_formula,
        field_domain: Some(LegacySemanticFieldDomainUpgradeV1 {
            algorithm: JOINT_MAX_LINEAR_FXP6_V1,
            fxp6_scale: LEGACY_FIELD_FXP6_SCALE,
            source_common_max: 0,
            out_of_range_count: 0,
            potential_out_of_range_count: 0,
            excitation_out_of_range_count: 0,
            signal_mass_before: 0,
            signal_mass_after: 0,
        }),
        migration_id: [0; 32],
    };
    let history = attest_legacy_history(
        tx,
        request.scope,
        &provisional,
        request.expected_incarnation_id,
        request.expected_manifest_digest,
    )?;
    let Some((normalized, field_domain)) =
        normalize_legacy_aesem2_field_domain_v1(&history.latest_snapshot.field)?
    else {
        return Err(StoreError::ContinuityFence("field_upgrade_not_needed"));
    };
    let target_state = state_digest(&normalized, &target_formula);
    let graph_after = graph_digest(&history.latest_snapshot.graph);
    if graph_after != request.expected_source_graph_digest {
        return Err(StoreError::ContinuityFence("field_upgrade_graph"));
    }

    let migration_material = LegacySemanticFormulaUpgradeReceiptV1 {
        target_state_before: target_state,
        field_domain: Some(field_domain),
        ..provisional
    };
    let migration_id = migration_material.expected_migration_id();
    let event_id_digest =
        wire::domain_hash(SEMANTIC_MIGRATION_EVENT_ID_DOMAIN_V2, &[&migration_id]);
    let turn_id_digest = wire::domain_hash(SEMANTIC_MIGRATION_TURN_ID_DOMAIN_V2, &[&migration_id]);
    let mut event_id = [0; 16];
    event_id.copy_from_slice(&event_id_digest[..16]);
    let mut turn_id = [0; 16];
    turn_id.copy_from_slice(&turn_id_digest[..16]);
    let event = CanonicalEvent::UserStimulus(UserStimulus {
        event_id,
        scope: request.scope.clone(),
        causal: CausalRef {
            turn_id,
            action_id: None,
            delivery_id: None,
            claim_id: None,
            base_revision: base.get(),
        },
        observed_at_ms: 0,
        evidence: SemanticEstimate {
            schema_version: 1,
            dimensions: EvidenceVector::default(),
            estimator_confidence: Fixed::ZERO,
            estimator_digest: [0; 32],
        },
    });
    let event_digest = wire::event_digest(&event);
    let authority_digest = ae_authority::authority_projection_digest(&event);
    let receipt = TransitionReceipt {
        schema_version: 1,
        formula_digest: target_formula,
        scope_digest,
        event_digest,
        authority_digest,
        base_revision: base.get(),
        next_revision: next.get(),
        state_before: target_state,
        state_after: target_state,
        graph_after,
        action_contract: None,
        active_nodes: normalized.active_node_count(),
        active_edges: u32::try_from(history.latest_snapshot.graph.edges.len())
            .map_err(|_| StoreError::ContinuityFence("semantic_migration_capacity"))?,
        residuals: InvariantResiduals::default(),
        status: CommitStatus::Committed,
    };
    let upgrade = LegacySemanticFormulaUpgradeReceiptV1::from_transition_receipt_with_field_domain(
        &receipt,
        request.expected_source_state_digest,
        request.expected_source_graph_digest,
        identity.formula_digest,
        request.expected_source_history_root,
        field_domain,
    );
    if upgrade.migration_id != migration_id {
        return Err(StoreError::ContinuityFence("semantic_migration_identity"));
    }
    let commitment = SemanticTransitionCommitmentV1 {
        scope_digest,
        event_digest,
        authority_digest,
        formula_digest: target_formula,
        source_history_root: request.expected_source_history_root,
        source_state_digest: request.expected_source_state_digest,
        base_revision: base.get(),
        next_revision: next.get(),
        state_before: target_state,
        state_after: target_state,
        graph_before: graph_after,
        graph_after,
        active_nodes: receipt.active_nodes,
        active_edges: receipt.active_edges,
        residuals: receipt.residuals.clone(),
    };
    let commitment_digest = commitment.digest();
    let local_digest = wire::domain_hash(
        SEMANTIC_MIGRATION_LOCAL_DOMAIN_V1,
        &[&commitment_digest, &authority_digest],
    );
    let effective_digest = wire::domain_hash(
        SEMANTIC_MIGRATION_EFFECTIVE_DOMAIN_V1,
        &[&local_digest, &target_state, &graph_after],
    );
    let reserve = normalized
        .metabolic_reserve
        .iter()
        .copied()
        .min()
        .ok_or(StoreError::ContinuityFence("semantic_migration_energy"))?;
    let upper_saturated_nodes = (0..NEURON_SLOTS)
        .filter(|index| {
            normalized.potential[*index] == Fixed::ONE
                || normalized.excitation[*index] == Fixed::ONE
        })
        .count();
    let edge_used = history.latest_snapshot.graph.edges.len();
    let node_headroom = bounded_headroom(upper_saturated_nodes, NEURON_SLOTS)?;
    let edge_headroom = bounded_headroom(edge_used, EDGE_CAPACITY)?;
    let telemetry = NativeTelemetryReceiptV1 {
        schema: NATIVE_TELEMETRY_RECEIPT_SCHEMA_V1.to_owned(),
        formula: NativeTelemetryFormulaV1::Phase0NativePropagationFxp6V1,
        formula_digest: target_formula,
        scope_digest,
        event_digest,
        source_digest: commitment_digest,
        base_revision: base.get(),
        next_revision: next.get(),
        phase: NativeTelemetryPhaseV1::Prepare,
        state_before: target_state,
        state_after: target_state,
        graph_before: graph_after,
        graph_after,
        local_digest,
        compensation_digest: legacy_reserved_zero_digest_v1(),
        effective_digest,
        energy: EnergyTelemetryV1 {
            reserve_before: reserve,
            reserve_after: reserve,
            recovered: Fixed::ZERO,
            spent: Fixed::ZERO,
            headroom: reserve,
            residual: Fixed::ZERO,
        },
        capacity: CapacityTelemetryV1 {
            upper_saturated_nodes: u32::try_from(upper_saturated_nodes)
                .map_err(|_| StoreError::ContinuityFence("semantic_migration_capacity"))?,
            node_limit: u32::try_from(NEURON_SLOTS)
                .map_err(|_| StoreError::ContinuityFence("semantic_migration_capacity"))?,
            node_headroom,
            edge_used: u32::try_from(edge_used)
                .map_err(|_| StoreError::ContinuityFence("semantic_migration_capacity"))?,
            edge_limit: u32::try_from(EDGE_CAPACITY)
                .map_err(|_| StoreError::ContinuityFence("semantic_migration_capacity"))?,
            edge_headroom,
            headroom: node_headroom.min(edge_headroom),
            residual: Fixed::ZERO,
        },
        residuals: InvariantResiduals::default(),
        residual_health: Fixed::ONE,
        native_gate: reserve.min(node_headroom.min(edge_headroom)),
        checkpoint_digest: [0; 32],
        telemetry_digest: [0; 32],
    }
    .seal();
    if !telemetry.validate() || telemetry.source_digest != commitment_digest {
        return Err(StoreError::ContinuityFence("semantic_migration_telemetry"));
    }
    let target_state_bytes =
        encode_snapshot_v3(&normalized, &history.latest_snapshot.graph, &telemetry)?;
    let snapshot_wire_digest = wire::domain_hash(
        SEMANTIC_MIGRATION_SNAPSHOT_DOMAIN_V1,
        &[&target_state_bytes],
    );
    let upgrade_bytes = upgrade.canonical_bytes();
    let receipt_bytes = wire::encode_transition_receipt(&receipt);
    let authority_receipt_digest = SemanticMigrationAuthorityReceiptV1 {
        migration_id,
        commitment_digest,
        telemetry_digest: telemetry.telemetry_digest,
        snapshot_wire_digest,
        authority_digest,
        upgrade_bytes: &upgrade_bytes,
        transition_receipt_bytes: &receipt_bytes,
    }
    .digest();
    Ok(SemanticMigrationArtifactsV2 {
        envelope: CommitEnvelope {
            event_kind: wire::event_kind_name(&event).to_owned(),
            event_bytes: wire::encode_event(&event),
            receipt,
            chain_seed: request.expected_source_history_root,
            delta_bytes: upgrade_bytes,
        },
        target_state_bytes,
        incarnation_id: request.expected_incarnation_id,
        manifest_digest: request.expected_manifest_digest,
        commitment_digest,
        telemetry_digest: telemetry.telemetry_digest,
        snapshot_wire_digest,
        authority_receipt_digest,
    })
}

fn existing_migration(
    tx: &Transaction<'_>,
    database_path: &Path,
    request: &SemanticMigrationArtifactsV2,
    upgrade: &LegacySemanticFormulaUpgradeReceiptV1,
    target_graph_bytes: &[u8],
    target_context_bytes: &[u8],
    relation_scope_token: [u8; 16],
) -> Result<Option<SemanticMigrationOutcomeV1>, StoreError> {
    struct ExistingUpgradeColumns {
        scope: Vec<u8>,
        from_formula: Vec<u8>,
        to_formula: Vec<u8>,
        base_revision: i64,
        next_revision: i64,
        event: Vec<u8>,
        receipt: Vec<u8>,
        source_state: Vec<u8>,
        target_state: Vec<u8>,
        source_graph: Vec<u8>,
        prior_chain: Vec<u8>,
        upgrade_bytes: Option<Vec<u8>>,
        upgrade_bytes_len: i64,
        backup_digest: Option<Vec<u8>>,
        backup_digest_len: i64,
    }
    let stored: Option<ExistingUpgradeColumns> = tx
        .query_row(
            "SELECT
                CASE WHEN typeof(scope_digest)='blob' AND length(scope_digest)=32 THEN scope_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(from_formula_digest)='blob' AND length(from_formula_digest)=32 THEN from_formula_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(to_formula_digest)='blob' AND length(to_formula_digest)=32 THEN to_formula_digest ELSE zeroblob(0) END,
                base_revision,next_revision,
                CASE WHEN typeof(event_digest)='blob' AND length(event_digest)=32 THEN event_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(receipt_digest)='blob' AND length(receipt_digest)=32 THEN receipt_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(source_state_digest)='blob' AND length(source_state_digest)=32 THEN source_state_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(target_state_before)='blob' AND length(target_state_before)=32 THEN target_state_before ELSE zeroblob(0) END,
                CASE WHEN typeof(source_graph_digest)='blob' AND length(source_graph_digest)=32 THEN source_graph_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(prior_chain_digest)='blob' AND length(prior_chain_digest)=32 THEN prior_chain_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(upgrade_bytes)='blob' AND length(upgrade_bytes)<=?2 THEN upgrade_bytes END,
                CASE WHEN typeof(upgrade_bytes)='blob' THEN length(upgrade_bytes) ELSE -1 END,
                CASE WHEN typeof(backup_digest)='blob' AND length(backup_digest)=32 THEN backup_digest END,
                CASE WHEN typeof(backup_digest)='blob' THEN length(backup_digest) ELSE -1 END
             FROM legacy_semantic_formula_upgrades WHERE migration_id=?1",
            params![blob(upgrade.migration_id), MAX_LEGACY_UPGRADE_BYTES],
            |row| {
                Ok(ExistingUpgradeColumns {
                    scope: row.get(0)?, from_formula: row.get(1)?, to_formula: row.get(2)?,
                    base_revision: row.get(3)?, next_revision: row.get(4)?, event: row.get(5)?,
                    receipt: row.get(6)?, source_state: row.get(7)?, target_state: row.get(8)?,
                    source_graph: row.get(9)?, prior_chain: row.get(10)?, upgrade_bytes: row.get(11)?,
                    upgrade_bytes_len: row.get(12)?, backup_digest: row.get(13)?,
                    backup_digest_len: row.get(14)?,
                })
            },
        )
        .optional()?;
    let Some(stored) = stored else {
        return Ok(None);
    };
    attest_semantic_authority_prefix_sets(
        tx,
        &upgrade.scope_digest,
        relation_scope_token,
        JournalRevision::new(upgrade.next_revision),
    )?;
    let migration_event = wire::decode_event(&request.envelope.event_bytes)
        .map_err(|_| StoreError::ContinuityFence("semantic_migration_journal_link"))?;
    let migration_scope = scope_from_event(&migration_event)?;
    let upgrade_bytes = bounded_typed_value(
        stored.upgrade_bytes,
        stored.upgrade_bytes_len,
        MAX_LEGACY_UPGRADE_BYTES,
        "legacy_upgrade.upgrade_bytes",
        "legacy_upgrade_bytes_type",
    )?;
    let stored_upgrade = LegacySemanticFormulaUpgradeReceiptV1::decode(&upgrade_bytes)?;
    if stored_upgrade != *upgrade
        || digest_from_blob(&stored.scope, "legacy_upgrade.scope_digest")? != upgrade.scope_digest
        || digest_from_blob(&stored.from_formula, "legacy_upgrade.from_formula_digest")?
            != upgrade.from_formula_digest
        || digest_from_blob(&stored.to_formula, "legacy_upgrade.to_formula_digest")?
            != upgrade.to_formula_digest
        || JournalRevision::try_from(stored.base_revision)?.get() != upgrade.base_revision
        || JournalRevision::try_from(stored.next_revision)?.get() != upgrade.next_revision
        || digest_from_blob(&stored.event, "legacy_upgrade.event_digest")? != upgrade.event_digest
        || digest_from_blob(&stored.receipt, "legacy_upgrade.receipt_digest")?
            != upgrade.receipt_digest
        || digest_from_blob(&stored.source_state, "legacy_upgrade.source_state_digest")?
            != upgrade.source_state_digest
        || digest_from_blob(&stored.target_state, "legacy_upgrade.target_state_before")?
            != upgrade.target_state_before
        || digest_from_blob(&stored.source_graph, "legacy_upgrade.source_graph_digest")?
            != upgrade.source_graph_digest
        || digest_from_blob(&stored.prior_chain, "legacy_upgrade.prior_chain_digest")?
            != upgrade.prior_chain_digest
        || upgrade_bytes != request.envelope.delta_bytes
    {
        return Err(StoreError::ContinuityFence("legacy_upgrade_duplicate"));
    }
    let receipt_bytes = wire::encode_transition_receipt(&request.envelope.receipt);
    let row = query_bounded_journal_row(
        tx,
        &upgrade.scope_digest,
        JournalRevision::new(upgrade.next_revision),
    )?;
    let expected_chain = ae_continuum::chain_link_with_delta(
        &request.envelope.chain_seed,
        &request.envelope.event_bytes,
        &receipt_bytes,
        &request.envelope.delta_bytes,
    );
    if !row.is_some_and(|row| {
        row.event_kind == request.envelope.event_kind
            && row.event_bytes == request.envelope.event_bytes
            && row.receipt_bytes == receipt_bytes
            && row.delta_bytes == request.envelope.delta_bytes
            && row.chain_digest == expected_chain
            && row.event_digest == request.envelope.receipt.event_digest
            && row.base_revision == upgrade.base_revision
            && row.revision == upgrade.next_revision
    }) {
        return Err(StoreError::ContinuityFence("legacy_upgrade_duplicate"));
    }
    let snapshot = query_bounded_snapshot_row(
        tx,
        &upgrade.scope_digest,
        JournalRevision::new(upgrade.next_revision),
    )?;
    if !snapshot.is_some_and(|snapshot| {
        snapshot.state_digest == request.envelope.receipt.state_after
            && snapshot.state_bytes == request.target_state_bytes
    }) {
        return Err(StoreError::ContinuityFence("legacy_upgrade_duplicate"));
    }
    verify_semantic_snapshot_v3(
        &request.target_state_bytes,
        &upgrade.to_formula_digest,
        &request.envelope.receipt.state_after,
        &upgrade.source_graph_digest,
        &request.envelope.receipt,
    )?;
    type ExistingGraphColumns = (
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Option<Vec<u8>>,
        i64,
        Option<Vec<u8>>,
        i64,
    );
    let graph: Option<ExistingGraphColumns> = tx
        .query_row(
            "SELECT
                CASE WHEN typeof(base_graph_digest)='blob' AND length(base_graph_digest)=32 THEN base_graph_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(graph_digest)='blob' AND length(graph_digest)=32 THEN graph_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(formula_digest)='blob' AND length(formula_digest)=32 THEN formula_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(delta_bytes)='blob' AND length(delta_bytes)<=?3 THEN delta_bytes END,
                CASE WHEN typeof(delta_bytes)='blob' THEN length(delta_bytes) ELSE -1 END,
                CASE WHEN typeof(replay_state_bytes)='blob' AND length(replay_state_bytes)<=?4 THEN replay_state_bytes END,
                CASE WHEN typeof(replay_state_bytes)='blob' THEN length(replay_state_bytes) ELSE -1 END
             FROM graph_commits WHERE scope_digest = ?1 AND revision = ?2",
            params![
                blob(upgrade.scope_digest),
                revision_to_sqlite(upgrade.next_revision)?,
                MAX_LEGACY_UPGRADE_BYTES,
                u64::try_from(GRAPH_WIRE_MAX_LEN).unwrap_or(u64::MAX),
            ],
            |row| {
                Ok((
                    row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?,
                    row.get(4)?, row.get(5)?, row.get(6)?,
                ))
            },
        )
        .optional()?;
    let graph = graph
        .map(
            |(base, after, formula, delta, delta_len, replay, replay_len)| {
                Ok::<_, StoreError>((
                    digest_from_blob(&base, "graph_commits.base_graph_digest")?,
                    digest_from_blob(&after, "graph_commits.graph_digest")?,
                    digest_from_blob(&formula, "graph_commits.formula_digest")?,
                    bounded_typed_value(
                        delta,
                        delta_len,
                        MAX_LEGACY_UPGRADE_BYTES,
                        "graph_commits.delta_bytes",
                        "graph_delta_bytes_type",
                    )?,
                    bounded_typed_value(
                        replay,
                        replay_len,
                        u64::try_from(GRAPH_WIRE_MAX_LEN).unwrap_or(u64::MAX),
                        "graph_commits.replay_state_bytes",
                        "graph_replay_state_bytes_type",
                    )?,
                ))
            },
        )
        .transpose()?;
    if graph
        != Some((
            upgrade.source_graph_digest,
            upgrade.source_graph_digest,
            upgrade.to_formula_digest,
            request.envelope.delta_bytes.clone(),
            target_graph_bytes.to_vec(),
        ))
    {
        return Err(StoreError::ContinuityFence("legacy_upgrade_duplicate"));
    }
    type ExistingContextColumns = (Option<Vec<u8>>, i64, Vec<u8>, Vec<u8>, Option<Vec<u8>>, i64);
    let context: Option<ExistingContextColumns> = tx
        .query_row(
            "SELECT
                CASE WHEN typeof(relation_scope_token)='blob' AND length(relation_scope_token)<=16 THEN relation_scope_token END,
                CASE WHEN typeof(relation_scope_token)='blob' THEN length(relation_scope_token) ELSE -1 END,
                CASE WHEN typeof(relation_hmac)='blob' AND length(relation_hmac)=32 THEN relation_hmac ELSE zeroblob(0) END,
                CASE WHEN typeof(context_digest)='blob' AND length(context_digest)=32 THEN context_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(canonical_state_bytes)='blob' AND length(canonical_state_bytes)<=?4 THEN canonical_state_bytes END,
                CASE WHEN typeof(canonical_state_bytes)='blob' THEN length(canonical_state_bytes) ELSE -1 END
             FROM context_commits WHERE scope_digest = ?1 AND relation_scope_token = ?2 AND revision = ?3",
            params![
                blob(upgrade.scope_digest),
                blob(relation_scope_token),
                revision_to_sqlite(upgrade.next_revision)?,
                u64::try_from(CONTEXT_STATE_WIRE_MAX_LEN_V1).unwrap_or(u64::MAX),
            ],
            |row| {
                Ok((
                    row.get(0)?, row.get(1)?, row.get(2)?,
                    row.get(3)?, row.get(4)?, row.get(5)?,
                ))
            },
        )
        .optional()?;
    let context = context
        .map(|(relation, relation_len, hmac, digest, bytes, bytes_len)| {
            Ok::<_, StoreError>((
                bounded_typed_value(
                    relation,
                    relation_len,
                    16,
                    "context_commits.relation_scope_token",
                    "context_relation_scope_token_type",
                )?,
                digest_from_blob(&hmac, "context_commits.relation_hmac")?,
                digest_from_blob(&digest, "context_commits.context_digest")?,
                bounded_typed_value(
                    bytes,
                    bytes_len,
                    u64::try_from(CONTEXT_STATE_WIRE_MAX_LEN_V1).unwrap_or(u64::MAX),
                    "context_commits.canonical_state_bytes",
                    "context_state_bytes_type",
                )?,
            ))
        })
        .transpose()?;
    if context
        != Some((
            relation_scope_token.to_vec(),
            context_relation_hmac(relation_scope_token),
            continuity_context_digest(target_context_bytes),
            target_context_bytes.to_vec(),
        ))
    {
        return Err(StoreError::ContinuityFence("legacy_upgrade_duplicate"));
    }
    type AuthorityColumns = (
        Option<Vec<u8>>,
        i64,
        Option<Vec<u8>>,
        i64,
        Option<Vec<u8>>,
        i64,
        Option<Vec<u8>>,
        i64,
    );
    let authority: Option<AuthorityColumns> = tx
        .query_row(
            "SELECT
                CASE WHEN typeof(commitment_digest)='blob' AND length(commitment_digest)=32 THEN commitment_digest END,
                CASE WHEN typeof(commitment_digest)='blob' THEN length(commitment_digest) ELSE -1 END,
                CASE WHEN typeof(telemetry_digest)='blob' AND length(telemetry_digest)=32 THEN telemetry_digest END,
                CASE WHEN typeof(telemetry_digest)='blob' THEN length(telemetry_digest) ELSE -1 END,
                CASE WHEN typeof(snapshot_wire_digest)='blob' AND length(snapshot_wire_digest)=32 THEN snapshot_wire_digest END,
                CASE WHEN typeof(snapshot_wire_digest)='blob' THEN length(snapshot_wire_digest) ELSE -1 END,
                CASE WHEN typeof(authority_receipt_digest)='blob' AND length(authority_receipt_digest)=32 THEN authority_receipt_digest END,
                CASE WHEN typeof(authority_receipt_digest)='blob' THEN length(authority_receipt_digest) ELSE -1 END
             FROM semantic_migration_authority_v1 WHERE migration_id=?1",
            params![blob(upgrade.migration_id)],
            |row| Ok((
                row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?,
                row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?,
            )),
        )
        .optional()?;
    let authority = authority
        .map(|stored| {
            Ok::<_, StoreError>((
                stored_typed_digest(
                    stored.0,
                    stored.1,
                    "semantic_migration_authority.commitment_digest",
                    "semantic_migration_authority_digest_type",
                )?,
                stored_typed_digest(
                    stored.2,
                    stored.3,
                    "semantic_migration_authority.telemetry_digest",
                    "semantic_migration_authority_digest_type",
                )?,
                stored_typed_digest(
                    stored.4,
                    stored.5,
                    "semantic_migration_authority.snapshot_wire_digest",
                    "semantic_migration_authority_digest_type",
                )?,
                stored_typed_digest(
                    stored.6,
                    stored.7,
                    "semantic_migration_authority.authority_receipt_digest",
                    "semantic_migration_authority_digest_type",
                )?,
            ))
        })
        .transpose()?;
    if authority
        != Some((
            request.commitment_digest,
            request.telemetry_digest,
            request.snapshot_wire_digest,
            request.authority_receipt_digest,
        ))
    {
        return Err(StoreError::ContinuityFence("semantic_migration_authority"));
    }
    let expected = expected_backup(upgrade, request.incarnation_id, request.manifest_digest);
    let backup = backup_from_row(tx, &expected)?
        .ok_or(StoreError::ContinuityFence("field_backup_record"))?;
    validate_backup_files(database_path, &backup)?;
    let stored_digest = stored_typed_digest(
        stored.backup_digest,
        stored.backup_digest_len,
        "legacy_upgrade.backup_digest",
        "legacy_upgrade_backup_digest_type",
    )?;
    if stored_digest != backup.sha256 {
        return Err(StoreError::ContinuityFence("field_backup_record"));
    }
    attest_post_migration_semantic_suffix(
        tx,
        migration_scope,
        &upgrade.scope_digest,
        relation_scope_token,
        JournalRevision::new(upgrade.next_revision),
        upgrade.to_formula_digest,
        request.envelope.receipt.state_after,
        request.envelope.receipt.graph_after,
        expected_chain,
        target_context_bytes.to_vec(),
    )?;
    Ok(Some(SemanticMigrationOutcomeV1::Migrated {
        from_revision: upgrade.base_revision,
        to_revision: upgrade.next_revision,
        backup_digest: backup.sha256,
    }))
}

pub(crate) fn verify_committed_semantic_migration_rows(
    tx: &Transaction<'_>,
) -> Result<(), StoreError> {
    let (raw_upgrades, raw_upgrade_bytes): (i64, i64) = tx.query_row(
        "SELECT COUNT(*), COALESCE(SUM(\
            length(migration_id)+length(scope_digest)+length(from_formula_digest)+\
            length(to_formula_digest)+length(event_digest)+length(receipt_digest)+\
            length(source_state_digest)+length(target_state_before)+\
            length(source_graph_digest)+length(prior_chain_digest)+\
            length(upgrade_bytes)+length(backup_digest)),0)\
         FROM legacy_semantic_formula_upgrades",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let upgrades = sqlite_length(raw_upgrades, "semantic_migrations.rows")?;
    let upgrade_bytes = sqlite_length(raw_upgrade_bytes, "semantic_migrations.bytes")?;
    enforce_read_budget(
        "semantic_migrations.rows",
        "semantic_migrations.bytes",
        upgrades,
        upgrade_bytes,
        MAX_SEMANTIC_HISTORY_ROWS,
        MAX_SEMANTIC_HISTORY_BYTES,
    )?;

    let raw_backups: i64 = tx.query_row(
        "SELECT COUNT(*) FROM field_migration_preimage_backups",
        [],
        |row| row.get(0),
    )?;
    let raw_authorities: i64 = tx.query_row(
        "SELECT COUNT(*) FROM semantic_migration_authority_v1",
        [],
        |row| row.get(0),
    )?;
    let backups = sqlite_length(raw_backups, "semantic_migration_backups.rows")?;
    let authorities = sqlite_length(raw_authorities, "semantic_migration_authorities.rows")?;
    if backups != upgrades || authorities != upgrades {
        return Err(StoreError::ContinuityFence(
            "semantic_migration_authority_set",
        ));
    }
    if upgrades == 0 {
        return Ok(());
    }

    let mut statement = tx.prepare(
        "SELECT CASE WHEN typeof(migration_id)='blob' AND length(migration_id)=32
                     THEN migration_id ELSE zeroblob(0) END \
         FROM legacy_semantic_formula_upgrades ORDER BY migration_id",
    )?;
    let mut rows = statement.query([])?;
    let mut migration_ids = Vec::with_capacity(usize::try_from(upgrades).unwrap_or(0));
    while let Some(row) = rows.next()? {
        let bytes: Vec<u8> = row.get(0)?;
        migration_ids.push(digest_from_blob(&bytes, "legacy_upgrade.migration_id")?);
    }
    drop(rows);
    drop(statement);
    if u64::try_from(migration_ids.len()).unwrap_or(u64::MAX) != upgrades {
        return Err(StoreError::ContinuityFence(
            "semantic_migration_authority_set",
        ));
    }

    let database_path = database_path(tx)?;
    for migration_id in migration_ids {
        type SelectorColumns = (
            Option<Vec<u8>>,
            i64,
            Option<Vec<u8>>,
            i64,
            Option<Vec<u8>>,
            i64,
        );
        let stored: SelectorColumns = tx.query_row(
            "SELECT
                CASE WHEN typeof(u.upgrade_bytes)='blob' AND length(u.upgrade_bytes)<=?2 THEN u.upgrade_bytes END,
                CASE WHEN typeof(u.upgrade_bytes)='blob' THEN length(u.upgrade_bytes) ELSE -1 END,
                CASE WHEN typeof(b.incarnation_id)='blob' AND length(b.incarnation_id)=32 THEN b.incarnation_id END,
                CASE WHEN typeof(b.incarnation_id)='blob' THEN length(b.incarnation_id) ELSE -1 END,
                CASE WHEN typeof(b.manifest_digest)='blob' AND length(b.manifest_digest)=32 THEN b.manifest_digest END,
                CASE WHEN typeof(b.manifest_digest)='blob' THEN length(b.manifest_digest) ELSE -1 END
             FROM legacy_semantic_formula_upgrades AS u
             LEFT JOIN field_migration_preimage_backups AS b
               ON b.migration_id=u.migration_id
             WHERE u.migration_id=?1",
            params![blob(migration_id), MAX_LEGACY_UPGRADE_BYTES],
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
        )?;
        let upgrade_bytes = bounded_typed_value(
            stored.0,
            stored.1,
            MAX_LEGACY_UPGRADE_BYTES,
            "legacy_upgrade.upgrade_bytes",
            "legacy_upgrade_bytes_type",
        )?;
        let incarnation_id = stored_typed_digest(
            stored.2,
            stored.3,
            "field_backup.incarnation_id",
            "field_backup_incarnation_id_type",
        )?;
        let manifest_digest = stored_typed_digest(
            stored.4,
            stored.5,
            "field_backup.manifest_digest",
            "field_backup_manifest_digest_type",
        )?;
        let upgrade = LegacySemanticFormulaUpgradeReceiptV1::decode(&upgrade_bytes)?;
        if upgrade.migration_id != migration_id || upgrade.canonical_bytes() != upgrade_bytes {
            return Err(StoreError::ContinuityFence("semantic_migration_authority"));
        }

        let journal = query_bounded_journal_row(
            tx,
            &upgrade.scope_digest,
            JournalRevision::new(upgrade.next_revision),
        )?
        .ok_or(StoreError::ContinuityFence(
            "semantic_migration_journal_link",
        ))?;
        if journal.base_revision != upgrade.base_revision
            || journal.event_digest != upgrade.event_digest
            || journal.delta_bytes != upgrade_bytes
        {
            return Err(StoreError::ContinuityFence(
                "semantic_migration_journal_link",
            ));
        }
        let event = wire::decode_event(&journal.event_bytes)
            .map_err(|_| StoreError::ContinuityFence("semantic_migration_journal_link"))?;
        let event_scope = scope_from_event(&event)?;
        let selector = SemanticMigrationRequestV2 {
            scope: event_scope,
            expected_source_revision: upgrade.base_revision,
            expected_source_state_digest: upgrade.source_state_digest,
            expected_source_graph_digest: upgrade.source_graph_digest,
            expected_source_history_root: upgrade.prior_chain_digest,
            expected_incarnation_id: incarnation_id,
            expected_manifest_digest: manifest_digest,
        };
        let artifacts = derive_migration_artifacts_v2(tx, &selector)?;
        if LegacySemanticFormulaUpgradeReceiptV1::decode(&artifacts.envelope.delta_bytes)?
            != upgrade
        {
            return Err(StoreError::ContinuityFence("semantic_migration_authority"));
        }
        let derived_event = wire::decode_event(&artifacts.envelope.event_bytes)
            .map_err(|_| StoreError::ContinuityFence("semantic_migration_journal_link"))?;
        let derived_scope = scope_from_event(&derived_event)?;
        let history =
            attest_legacy_history(tx, derived_scope, &upgrade, incarnation_id, manifest_digest)?;
        let target_graph_bytes = history.latest_snapshot.graph.canonical_bytes();
        let target_context_bytes = project_context_state(
            Some(&history.context_state_bytes),
            &derived_event,
            history.relation_scope_token,
            upgrade.next_revision,
        )?;
        if existing_migration(
            tx,
            &database_path,
            &artifacts,
            &upgrade,
            &target_graph_bytes,
            &target_context_bytes,
            history.relation_scope_token,
        )?
        .is_none()
        {
            return Err(StoreError::ContinuityFence("semantic_migration_authority"));
        }
    }
    Ok(())
}

fn reattest_committed_migration_on_fresh_store(
    database_path: &Path,
    request: &SemanticMigrationArtifactsV2,
    upgrade: &LegacySemanticFormulaUpgradeReceiptV1,
    target_graph_bytes: &[u8],
    target_context_bytes: &[u8],
    relation_scope_token: [u8; 16],
) -> Result<SemanticMigrationOutcomeV1, StoreError> {
    let event = wire::decode_event(&request.envelope.event_bytes)
        .map_err(|_| StoreError::ContinuityFence("field_upgrade_event"))?;
    let event_scope = scope_from_event(&event)?;
    let mut fresh = Store::open(database_path)?;
    let conn = fresh.conn.as_mut().ok_or(StoreError::Closed)?;
    enforce_connection_database_budget(conn)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let current_sql: i64 = tx.query_row(
        "SELECT COALESCE(MAX(logical_revision), 0) FROM journal WHERE scope_digest=?1",
        params![blob(upgrade.scope_digest)],
        |row| row.get(0),
    )?;
    let current = JournalRevision::try_from(current_sql)?;
    if current.get() < upgrade.next_revision {
        return Err(StoreError::StaleRevision {
            expected: upgrade.next_revision,
            actual: current.get(),
        });
    }
    let history = attest_legacy_history(
        &tx,
        event_scope,
        upgrade,
        request.incarnation_id,
        request.manifest_digest,
    )?;
    if history.relation_scope_token != relation_scope_token
        || history.latest_snapshot.graph.canonical_bytes() != target_graph_bytes
    {
        return Err(StoreError::ContinuityFence("semantic_migration_reopen"));
    }
    let projected_context = project_context_state(
        Some(&history.context_state_bytes),
        &event,
        relation_scope_token,
        upgrade.next_revision,
    )?;
    if projected_context != target_context_bytes {
        return Err(StoreError::ContinuityFence("semantic_migration_reopen"));
    }
    let outcome = existing_migration(
        &tx,
        database_path,
        request,
        upgrade,
        target_graph_bytes,
        target_context_bytes,
        relation_scope_token,
    )?
    .ok_or(StoreError::ContinuityFence("semantic_migration_reopen"))?;
    tx.commit()?;
    fresh.close()?;
    Ok(outcome)
}

impl Store {
    /// Select and compare-and-swap the unique authenticated predecessor. All
    /// migration events, receipts, telemetry and AESEM3 bytes are Store-owned.
    pub fn migrate_legacy_semantic_snapshot_v2(
        &mut self,
        request: SemanticMigrationRequestV2<'_>,
    ) -> Result<SemanticMigrationOutcomeV1, StoreError> {
        let artifacts = {
            let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
            enforce_connection_database_budget(conn)?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;
            let artifacts = derive_migration_artifacts_v2(&tx, &request)?;
            tx.commit()?;
            artifacts
        };
        self.migrate_derived_legacy_semantic_snapshot_v2(artifacts)
    }

    /// Copy/verify/commit the sole authenticated AESEM2 finite-domain
    /// predecessor. Every authority check and the immutable preimage backup
    /// complete before the first SQLite write in this transaction.
    fn migrate_derived_legacy_semantic_snapshot_v2(
        &mut self,
        request: SemanticMigrationArtifactsV2,
    ) -> Result<SemanticMigrationOutcomeV1, StoreError> {
        enforce_byte_budget(
            "journal.event_kind",
            u64::try_from(request.envelope.event_kind.len()).unwrap_or(u64::MAX),
            MAX_JOURNAL_EVENT_KIND_BYTES,
        )?;
        enforce_byte_budget(
            "journal.event_bytes",
            u64::try_from(request.envelope.event_bytes.len()).unwrap_or(u64::MAX),
            MAX_JOURNAL_EVENT_BYTES,
        )?;
        enforce_byte_budget(
            "legacy_upgrade.upgrade_bytes",
            u64::try_from(request.envelope.delta_bytes.len()).unwrap_or(u64::MAX),
            MAX_LEGACY_UPGRADE_BYTES.min(MAX_JOURNAL_DELTA_BYTES),
        )?;
        enforce_byte_budget(
            "snapshot.state_bytes",
            u64::try_from(request.target_state_bytes.len()).unwrap_or(u64::MAX),
            MAX_SNAPSHOT_STATE_BYTES,
        )?;
        let envelope_base = JournalRevision::new(request.envelope.receipt.base_revision);
        let envelope_next = JournalRevision::new(request.envelope.receipt.next_revision);
        envelope_base.to_sqlite()?;
        envelope_next.to_sqlite()?;
        let upgrade = LegacySemanticFormulaUpgradeReceiptV1::decode(&request.envelope.delta_bytes)?;
        if upgrade.field_domain.is_none() {
            return Err(StoreError::ContinuityFence("field_upgrade_required"));
        }
        let event = wire::decode_event(&request.envelope.event_bytes)
            .map_err(|_| StoreError::ContinuityFence("field_upgrade_event"))?;
        let event_scope = scope_from_event(&event)?;
        let stable_scope = wire::persona_scope_digest(
            &event_scope.bot_token,
            &event_scope.persona_token,
            event_scope.relation_token.as_ref(),
        );
        let causal_base = match &event {
            CanonicalEvent::UserStimulus(value) => value.causal.base_revision,
            _ => unreachable!(),
        };
        if stable_scope != request.envelope.receipt.scope_digest
            || wire::encode_event(&event) != request.envelope.event_bytes
            || wire::event_digest(&event) != request.envelope.receipt.event_digest
            || wire::event_kind_name(&event) != request.envelope.event_kind
            || causal_base != request.envelope.receipt.base_revision
            || request.envelope.receipt.schema_version != 1
            || request.envelope.receipt.status != CommitStatus::Committed
            || request.envelope.receipt.action_contract.is_some()
            || request.envelope.receipt.state_before != request.envelope.receipt.state_after
            || envelope_base.checked_next()? != envelope_next
            || request.envelope.receipt.authority_digest
                != ae_authority::authority_projection_digest(&event)
            || upgrade.canonical_bytes() != request.envelope.delta_bytes
        {
            return Err(StoreError::ContinuityFence("field_upgrade_envelope"));
        }

        let database_path = {
            let conn = self.conn.as_ref().ok_or(StoreError::Closed)?;
            database_path(conn)?
        };
        if self.database_identity.as_ref() != Some(&store_database_identity(&database_path)?) {
            return Err(StoreError::ContinuityFence("field_backup_source_path"));
        }
        let source_database_identity_before = source_database_identity(&database_path)?;
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        enforce_connection_database_budget(conn)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;

        let current_sql: i64 = tx.query_row(
            "SELECT COALESCE(MAX(logical_revision), 0) FROM journal WHERE scope_digest = ?1",
            params![blob(request.envelope.receipt.scope_digest)],
            |row| row.get(0),
        )?;
        let current = JournalRevision::try_from(current_sql)?;
        let upgrade_base = JournalRevision::new(upgrade.base_revision);
        let next = upgrade_base.checked_next()?;
        if request.envelope.receipt.base_revision != upgrade.base_revision
            || envelope_next != next
            || upgrade.next_revision != next.get()
            || upgrade.scope_digest != request.envelope.receipt.scope_digest
            || upgrade.event_digest != request.envelope.receipt.event_digest
            || upgrade.receipt_digest != wire::receipt_digest(&request.envelope.receipt)
            || upgrade.prior_chain_digest != request.envelope.chain_seed
        {
            return Err(StoreError::ContinuityFence("field_upgrade_revision"));
        }
        let history = attest_legacy_history(
            &tx,
            event_scope,
            &upgrade,
            request.incarnation_id,
            request.manifest_digest,
        )?;
        if upgrade.to_formula_digest
            != phase0_canonical_formula_digest_v1(&upgrade.from_formula_digest)
            || request.envelope.receipt.formula_digest != upgrade.to_formula_digest
        {
            return Err(StoreError::ContinuityFence("semantic_identity"));
        }
        let Some((normalized, field_domain)) =
            normalize_legacy_aesem2_field_domain_v1(&history.latest_snapshot.field)?
        else {
            return Err(StoreError::ContinuityFence("field_upgrade_not_needed"));
        };
        let target_state_digest = state_digest(&normalized, &upgrade.to_formula_digest);
        let expected_upgrade =
            LegacySemanticFormulaUpgradeReceiptV1::from_transition_receipt_with_field_domain(
                &request.envelope.receipt,
                upgrade.source_state_digest,
                upgrade.source_graph_digest,
                upgrade.from_formula_digest,
                upgrade.prior_chain_digest,
                field_domain,
            );
        if upgrade != expected_upgrade
            || request.envelope.receipt.state_before != target_state_digest
            || request.envelope.receipt.graph_after != upgrade.source_graph_digest
            || request.envelope.receipt.active_nodes != normalized.active_node_count()
            || usize::try_from(request.envelope.receipt.active_edges).ok()
                != Some(history.latest_snapshot.graph.edges.len())
            || request.envelope.receipt.residuals != InvariantResiduals::default()
        {
            return Err(StoreError::ContinuityFence("field_upgrade_target"));
        }
        let target_graph_bytes = verify_semantic_snapshot_v3(
            &request.target_state_bytes,
            &upgrade.to_formula_digest,
            &target_state_digest,
            &upgrade.source_graph_digest,
            &request.envelope.receipt,
        )?;
        if target_graph_bytes != history.latest_snapshot.graph.canonical_bytes() {
            return Err(StoreError::ContinuityFence("field_upgrade_graph"));
        }
        let target_context_bytes = project_context_state(
            Some(&history.context_state_bytes),
            &event,
            history.relation_scope_token,
            next.get(),
        )?;

        if let Some(outcome) = existing_migration(
            &tx,
            &database_path,
            &request,
            &upgrade,
            &target_graph_bytes,
            &target_context_bytes,
            history.relation_scope_token,
        )? {
            if current < next {
                return Err(StoreError::ContinuityFence("legacy_upgrade_duplicate"));
            }
            tx.commit()?;
            return Ok(outcome);
        }
        attest_exact_semantic_authority_sets(
            &tx,
            &upgrade.scope_digest,
            history.relation_scope_token,
            upgrade_base,
        )?;
        if current != upgrade_base {
            return Err(StoreError::ContinuityFence("field_upgrade_revision"));
        }

        let prepared_backup = prepare_backup(
            &tx,
            &database_path,
            expected_backup(&upgrade, request.incarnation_id, request.manifest_digest),
        )?;
        let source_data_version_before = prepared_backup.source_data_version;
        tx.commit()?;

        fire_semantic_migration_test_hook_v1(&database_path);
        fire_semantic_migration_failpoint_v1(
            SemanticMigrationTestFailpointV1::BeforeTransactionBegin,
        )?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        fire_semantic_migration_failpoint_v1(
            SemanticMigrationTestFailpointV1::AfterTransactionBegin,
        )?;
        let cas_current_sql: i64 = tx.query_row(
            "SELECT COALESCE(MAX(logical_revision), 0) FROM journal WHERE scope_digest = ?1",
            params![blob(request.envelope.receipt.scope_digest)],
            |row| row.get(0),
        )?;
        let cas_current = JournalRevision::try_from(cas_current_sql)?;
        if cas_current != upgrade_base {
            return Err(StoreError::StaleRevision {
                expected: upgrade_base.get(),
                actual: cas_current.get(),
            });
        }
        if source_data_version(&tx)? != source_data_version_before
            || source_database_identity(&database_path)? != source_database_identity_before
        {
            return Err(StoreError::ContinuityFence("field_backup_source_changed"));
        }
        let cas_history = attest_legacy_history(
            &tx,
            event_scope,
            &upgrade,
            request.incarnation_id,
            request.manifest_digest,
        )?;
        attest_exact_semantic_authority_sets(
            &tx,
            &upgrade.scope_digest,
            cas_history.relation_scope_token,
            upgrade_base,
        )?;
        if state_digest(
            &cas_history.latest_snapshot.field,
            &upgrade.from_formula_digest,
        ) != state_digest(&history.latest_snapshot.field, &upgrade.from_formula_digest)
            || cas_history.latest_snapshot.graph.canonical_bytes()
                != history.latest_snapshot.graph.canonical_bytes()
            || cas_history.context_state_bytes != history.context_state_bytes
            || cas_history.relation_scope_token != history.relation_scope_token
            || prepared_backup.backup.source_authority_fingerprint
                != backup_source_authority_fingerprint(&prepared_backup.backup)
        {
            return Err(StoreError::ContinuityFence("field_backup_source_changed"));
        }
        // Only now, under BEGIN IMMEDIATE and after re-attesting the exact
        // source snapshot, may a row-less stale final be replaced or the fresh
        // whole-database package become recovery authority.
        let backup = finalize_prepared_backup(&tx, &database_path, prepared_backup)?;
        let base_sql = upgrade_base.to_sqlite()?.get();
        let next_sql = next.to_sqlite()?.get();
        let receipt_bytes = wire::encode_transition_receipt(&request.envelope.receipt);
        let chain_digest = ae_continuum::chain_link_with_delta(
            &request.envelope.chain_seed,
            &request.envelope.event_bytes,
            &receipt_bytes,
            &request.envelope.delta_bytes,
        );

        fire_semantic_migration_failpoint_v1(SemanticMigrationTestFailpointV1::BeforeRowInsertion)?;
        tx.execute(
            "INSERT INTO field_migration_preimage_backups (migration_id, scope_digest, source_revision, source_state_digest, source_formula_digest, source_graph_digest, incarnation_id, manifest_digest, byte_len, sha256, manifest_bytes, creator_package_identity, creator_build_identity) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                blob(backup.migration_id), blob(backup.scope_digest), base_sql,
                blob(backup.source_state_digest), blob(backup.source_formula_digest),
                blob(backup.source_graph_digest), blob(backup.incarnation_id),
                blob(backup.manifest_digest),
                i64::try_from(backup.byte_len).map_err(|_| StoreError::ContinuityFence("field_backup_length"))?,
                blob(backup.sha256), backup_manifest(&backup),
                &backup.package_identity, &backup.build_identity,
            ],
        )?;
        fire_semantic_migration_failpoint_v1(SemanticMigrationTestFailpointV1::AfterRowInsertion)?;
        fire_semantic_migration_failpoint_v1(
            SemanticMigrationTestFailpointV1::AfterBackupRowInsert,
        )?;
        let journal_inserted = tx.execute(
            "INSERT INTO journal (logical_revision, scope_digest, base_revision, event_kind, event_bytes, event_digest, receipt_bytes, delta_bytes, chain_digest, committed_at_ms)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10
             WHERE ?3 = (SELECT COALESCE(MAX(logical_revision), 0) FROM journal WHERE scope_digest=?2)",
            params![
                next_sql, blob(upgrade.scope_digest), base_sql,
                request.envelope.event_kind.clone(), request.envelope.event_bytes.clone(),
                blob(upgrade.event_digest), receipt_bytes, request.envelope.delta_bytes.clone(),
                blob(chain_digest),
                i64::try_from(now_ms()).map_err(|_| StoreError::RevisionOutOfRange { revision: now_ms() })?,
            ],
        )?;
        if journal_inserted == 0 {
            let actual_sql: i64 = tx.query_row(
                "SELECT COALESCE(MAX(logical_revision), 0) FROM journal WHERE scope_digest=?1",
                params![blob(upgrade.scope_digest)],
                |row| row.get(0),
            )?;
            return Err(StoreError::StaleRevision {
                expected: upgrade.base_revision,
                actual: JournalRevision::try_from(actual_sql)?.get(),
            });
        }
        fire_semantic_migration_failpoint_v1(SemanticMigrationTestFailpointV1::AfterJournalInsert)?;
        tx.execute(
            "INSERT INTO applied_events (scope_digest, event_digest, revision) VALUES (?1, ?2, ?3)",
            params![
                blob(upgrade.scope_digest),
                blob(upgrade.event_digest),
                next_sql
            ],
        )?;
        fire_semantic_migration_failpoint_v1(
            SemanticMigrationTestFailpointV1::AfterAppliedEventInsert,
        )?;
        tx.execute(
            "INSERT INTO snapshots (revision, scope_digest, state_digest, state_bytes) VALUES (?1, ?2, ?3, ?4)",
            params![next_sql, blob(upgrade.scope_digest), blob(target_state_digest), &request.target_state_bytes],
        )?;
        fire_semantic_migration_failpoint_v1(
            SemanticMigrationTestFailpointV1::AfterSnapshotInsert,
        )?;
        tx.execute(
            "INSERT INTO graph_commits (scope_digest, revision, base_graph_digest, graph_digest, formula_digest, delta_bytes, replay_state_bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                blob(upgrade.scope_digest),
                next_sql,
                blob(upgrade.source_graph_digest),
                blob(upgrade.source_graph_digest),
                blob(upgrade.to_formula_digest),
                request.envelope.delta_bytes.clone(),
                &target_graph_bytes,
            ],
        )?;
        fire_semantic_migration_failpoint_v1(SemanticMigrationTestFailpointV1::AfterGraphInsert)?;
        tx.execute(
            "INSERT INTO context_commits (scope_digest, relation_scope_token, relation_hmac, revision, context_digest, canonical_state_bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                blob(upgrade.scope_digest),
                blob(history.relation_scope_token),
                blob(context_relation_hmac(history.relation_scope_token)),
                next_sql,
                blob(continuity_context_digest(&target_context_bytes)),
                &target_context_bytes,
            ],
        )?;
        fire_semantic_migration_failpoint_v1(SemanticMigrationTestFailpointV1::AfterContextInsert)?;
        tx.execute(
            "INSERT INTO legacy_semantic_formula_upgrades (migration_id, scope_digest, from_formula_digest, to_formula_digest, base_revision, next_revision, event_digest, receipt_digest, source_state_digest, target_state_before, source_graph_digest, prior_chain_digest, upgrade_bytes, backup_digest) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                blob(upgrade.migration_id), blob(upgrade.scope_digest),
                blob(upgrade.from_formula_digest), blob(upgrade.to_formula_digest),
                base_sql, next_sql, blob(upgrade.event_digest), blob(upgrade.receipt_digest),
                blob(upgrade.source_state_digest), blob(upgrade.target_state_before),
                blob(upgrade.source_graph_digest), blob(upgrade.prior_chain_digest),
                request.envelope.delta_bytes.clone(), blob(backup.sha256),
            ],
        )?;
        fire_semantic_migration_failpoint_v1(SemanticMigrationTestFailpointV1::AfterUpgradeInsert)?;
        tx.execute(
            "INSERT INTO semantic_migration_authority_v1 (migration_id, commitment_digest, telemetry_digest, snapshot_wire_digest, authority_receipt_digest) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                blob(upgrade.migration_id),
                blob(request.commitment_digest),
                blob(request.telemetry_digest),
                blob(request.snapshot_wire_digest),
                blob(request.authority_receipt_digest),
            ],
        )?;
        fire_semantic_migration_failpoint_v1(
            SemanticMigrationTestFailpointV1::AfterAuthorityInsert,
        )?;
        fire_semantic_migration_failpoint_v1(SemanticMigrationTestFailpointV1::BeforeCommit)?;
        tx.commit()?;
        fire_semantic_migration_failpoint_v1(SemanticMigrationTestFailpointV1::AfterCommit)?;
        reattest_committed_migration_on_fresh_store(
            &database_path,
            &request,
            &upgrade,
            &target_graph_bytes,
            &target_context_bytes,
            history.relation_scope_token,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_primitives_reject_unrepresentable_revisions_and_oversized_rows() {
        assert!(matches!(
            JournalRevision::try_from(-1_i64),
            Err(StoreError::InvalidStoredRevision { revision: -1 })
        ));
        assert!(matches!(
            SqliteRevision::try_from(u64::MAX),
            Err(StoreError::RevisionOutOfRange { revision: u64::MAX })
        ));
        let sqlite_max = JournalRevision::try_from(i64::MAX).expect("SQLite max is non-negative");
        assert!(matches!(
            sqlite_max.checked_next(),
            Err(StoreError::RevisionOutOfRange { .. })
        ));

        let scope = [0x51; 32];
        let mut store = Store::open_in_memory().expect("store");
        assert!(matches!(
            store.read_snapshot(&scope, u64::MAX),
            Err(StoreError::RevisionOutOfRange { revision: u64::MAX })
        ));
        let conn = store.conn.as_mut().expect("connection");
        conn.execute(
            "INSERT INTO journal (logical_revision, scope_digest, base_revision, event_kind, event_bytes, event_digest, receipt_bytes, delta_bytes, chain_digest, committed_at_ms) VALUES (-1, ?1, 0, 'test', X'', zeroblob(32), X'', X'', zeroblob(32), 0)",
            params![blob(scope)],
        )
        .expect("negative fixture");
        assert!(matches!(
            store.current_revision(&scope),
            Err(StoreError::InvalidStoredRevision { revision: -1 })
        ));

        let conn = store.conn.as_mut().expect("connection");
        conn.execute("DELETE FROM journal", [])
            .expect("clear fixture");
        conn.execute(
            "INSERT INTO journal (logical_revision, scope_digest, base_revision, event_kind, event_bytes, event_digest, receipt_bytes, delta_bytes, chain_digest, committed_at_ms) VALUES (1, ?1, 0, 'test', X'', zeroblob(31), X'', X'', zeroblob(32), 0)",
            params![blob(scope)],
        )
        .expect("digest fixture");
        assert!(matches!(
            store.read_journal(&scope),
            Err(StoreError::InvalidStoredDigest {
                field: "journal.event_digest",
                actual: 31,
            })
        ));

        let conn = store.conn.as_mut().expect("connection");
        conn.execute(
            "UPDATE journal SET event_digest=zeroblob(32), event_bytes=zeroblob(?1)",
            params![MAX_JOURNAL_EVENT_BYTES + 1],
        )
        .expect("oversized row fixture");
        assert!(matches!(
            store.read_journal(&scope),
            Err(StoreError::StorageBudgetExceeded {
                resource: "journal.event_bytes",
                ..
            })
        ));

        assert!(matches!(
            enforce_read_budget(
                "semantic.history.rows",
                "semantic.history.bytes",
                MAX_SEMANTIC_HISTORY_ROWS + 1,
                0,
                MAX_SEMANTIC_HISTORY_ROWS,
                MAX_SEMANTIC_HISTORY_BYTES,
            ),
            Err(StoreError::StorageBudgetExceeded {
                resource: "semantic.history.rows",
                ..
            })
        ));
        assert!(matches!(
            enforce_read_budget(
                "semantic.history.rows",
                "semantic.history.bytes",
                0,
                MAX_SEMANTIC_HISTORY_BYTES + 1,
                MAX_SEMANTIC_HISTORY_ROWS,
                MAX_SEMANTIC_HISTORY_BYTES,
            ),
            Err(StoreError::StorageBudgetExceeded {
                resource: "semantic.history.bytes",
                ..
            })
        ));
        assert!(matches!(
            enforce_database_budget(MAX_STORE_DATABASE_BYTES + 1),
            Err(StoreError::StorageBudgetExceeded {
                resource: "store.database_bytes",
                ..
            })
        ));

        let manifest_path = std::env::temp_dir().join(format!(
            "ae-store-manifest-bound-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::write(&manifest_path, [0_u8; 5]).expect("manifest fixture");
        let mut manifest_file = ae_platform_fs::open_regular_file_no_follow(&manifest_path)
            .expect("open manifest fixture");
        assert!(matches!(
            read_backup_manifest_handle(&mut manifest_file, Some(4)),
            Err(StoreError::StorageBudgetExceeded {
                resource: "field_backup.manifest_bytes",
                limit: 4,
                actual: 5,
            })
        ));
        std::fs::remove_file(manifest_path).expect("remove manifest fixture");
    }

    #[test]
    fn graph_wire_accepts_full_canonical_edge_width_and_rejects_oversize_ingress() {
        let edge_count = EDGE_CAPACITY
            .checked_mul(3)
            .and_then(|value| value.checked_div(4))
            .and_then(|value| value.checked_add(1))
            .expect("bounded edge fixture");
        let mut row_offsets = Vec::with_capacity(NEURON_SLOTS + 1);
        let mut edges = Vec::with_capacity(edge_count);
        row_offsets.push(0);
        for _source in 0..NEURON_SLOTS {
            let remaining = edge_count - edges.len();
            let row_len = remaining.min(NEURON_SLOTS);
            for target in 0..row_len {
                edges.push(Synapse {
                    target: u32::try_from(target).expect("target fits u32"),
                    ..Synapse::default()
                });
            }
            row_offsets.push(u32::try_from(edges.len()).expect("edge count fits u32"));
        }
        assert_eq!(edges.len(), edge_count);
        let graph = SparseGraph { row_offsets, edges };
        let canonical = encode_graph(&graph).expect("canonical graph fixture");
        assert!(canonical.len() > GRAPH_WIRE_MIN_LEN + EDGE_CAPACITY * 12);
        let decoded = decode_graph(&canonical).expect("full-width graph");
        assert_eq!(encode_graph(&decoded).expect("re-encode graph"), canonical);

        let oversize = vec![0_u8; GRAPH_WIRE_MIN_LEN + EDGE_CAPACITY * 16 + 1];
        assert!(decode_graph(&oversize).is_err());
    }

    fn test_prepare_and_finalize_backup(
        conn: &mut Connection,
        database_path: &Path,
        expected: FieldMigrationPreimageBackupV1,
    ) -> Result<(FieldMigrationPreimageBackupV1, i64), StoreError> {
        let prepared = {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;
            let prepared = prepare_backup(&tx, database_path, expected)?;
            tx.commit()?;
            prepared
        };
        let source_data_version = prepared.source_data_version;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let backup = finalize_prepared_backup(&tx, database_path, prepared)?;
        tx.commit()?;
        Ok((backup, source_data_version))
    }

    #[test]
    fn backup_publication_failpoints_are_recoverable_or_committed() {
        let publication_points = [
            SemanticMigrationTestFailpointV1::BeforeSnapshot,
            SemanticMigrationTestFailpointV1::AfterSnapshot,
            SemanticMigrationTestFailpointV1::BeforeDatabaseSync,
            SemanticMigrationTestFailpointV1::AfterDatabaseSync,
            SemanticMigrationTestFailpointV1::BeforeManifestSync,
            SemanticMigrationTestFailpointV1::AfterManifestSync,
            SemanticMigrationTestFailpointV1::BeforeDirectorySync,
            SemanticMigrationTestFailpointV1::AfterDirectorySync,
            SemanticMigrationTestFailpointV1::BeforeAtomicPublish,
            SemanticMigrationTestFailpointV1::AfterAtomicPublish,
            SemanticMigrationTestFailpointV1::BeforeParentSync,
            SemanticMigrationTestFailpointV1::AfterParentSync,
        ];
        for (index, point) in publication_points.into_iter().enumerate() {
            let root = std::env::temp_dir().join(format!(
                "ae-store-backup-failpoint-{}-{}-{index}",
                std::process::id(),
                now_ms()
            ));
            fs::create_dir_all(&root).expect("fixture root");
            let database_path = root.join("authority.sqlite");
            let mut conn = Connection::open(&database_path).expect("fixture database");
            conn.execute_batch(
                "CREATE TABLE field_migration_preimage_backups (
                    migration_id BLOB PRIMARY KEY, scope_digest BLOB NOT NULL,
                    source_revision INTEGER NOT NULL, source_state_digest BLOB NOT NULL,
                    source_formula_digest BLOB NOT NULL, source_graph_digest BLOB NOT NULL,
                    incarnation_id BLOB NOT NULL, manifest_digest BLOB NOT NULL,
                    byte_len INTEGER NOT NULL, sha256 BLOB NOT NULL, manifest_bytes BLOB NOT NULL,
                    creator_package_identity TEXT NOT NULL,
                    creator_build_identity TEXT NOT NULL
                ); CREATE TABLE source(value INTEGER NOT NULL); INSERT INTO source VALUES (7);",
            )
            .expect("fixture schema");
            let mut expected = FieldMigrationPreimageBackupV1 {
                migration_id: [0x10 + u8::try_from(index).unwrap(); 32],
                scope_digest: [0x21; 32],
                source_revision: 2,
                source_state_digest: [0x31; 32],
                source_formula_digest: [0x41; 32],
                source_graph_digest: [0x51; 32],
                incarnation_id: [0x61; 32],
                manifest_digest: [0x71; 32],
                package_identity: backup_package_identity(),
                build_identity: backup_build_identity(),
                capture_method: FieldMigrationBackupCaptureMethodV1::SqliteBackupApi,
                source_authority_fingerprint: [0; 32],
                byte_len: 0,
                sha256: [0; 32],
            };
            expected.source_authority_fingerprint = backup_source_authority_fingerprint(&expected);
            let paths = backup_paths(
                &database_path,
                &expected.migration_id,
                expected.source_revision,
            )
            .expect("backup paths");

            set_semantic_migration_test_failpoint_v1(point);
            assert!(
                test_prepare_and_finalize_backup(&mut conn, &database_path, expected.clone())
                    .is_err()
            );
            drop(conn);

            if point == SemanticMigrationTestFailpointV1::AfterAtomicPublish {
                let mut recovery =
                    Connection::open(&database_path).expect("reopen recovery source");
                set_semantic_migration_test_failpoint_v1(
                    SemanticMigrationTestFailpointV1::BeforeParentSync,
                );
                assert!(
                    test_prepare_and_finalize_backup(
                        &mut recovery,
                        &database_path,
                        expected.clone(),
                    )
                    .is_err(),
                    "recovery of a published final must complete parent sync before returning"
                );
                drop(recovery);
            }

            let source = Connection::open(&database_path).expect("reopen unchanged source");
            assert_eq!(
                source
                    .query_row("SELECT value FROM source", [], |row| row.get::<_, i64>(0))
                    .expect("source value"),
                7,
                "{point:?} changed source authority"
            );
            drop(source);

            let mut reopened = Connection::open(&database_path).expect("reopen source");
            let (backup, _) =
                test_prepare_and_finalize_backup(&mut reopened, &database_path, expected)
                    .expect("recover backup");
            validate_backup_files(&database_path, &backup).expect("valid final package");
            drop(reopened);
            assert!(
                paths.final_dir.is_dir(),
                "{point:?} did not recover final package"
            );
            let remaining = fs::read_dir(&paths.root)
                .expect("backup root")
                .map(|entry| entry.expect("backup entry").path())
                .collect::<Vec<_>>();
            assert_eq!(
                remaining,
                vec![paths.final_dir],
                "{point:?} left stage debris"
            );
            fs::remove_dir_all(root).expect("remove fixture root");
        }

        let database_path = std::env::temp_dir().join(format!(
            "ae-store-after-sql-commit-{}-{}.sqlite",
            std::process::id(),
            now_ms()
        ));
        let mut conn = Connection::open(&database_path).expect("commit fixture");
        conn.execute("CREATE TABLE committed(value INTEGER NOT NULL)", [])
            .expect("commit schema");
        set_semantic_migration_test_failpoint_v1(SemanticMigrationTestFailpointV1::AfterCommit);
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("commit transaction");
        tx.execute("INSERT INTO committed VALUES (1)", [])
            .expect("commit row");
        tx.commit().expect("sql commit");
        assert!(fire_semantic_migration_failpoint_v1(
            SemanticMigrationTestFailpointV1::AfterCommit
        )
        .is_err());
        drop(conn);
        let reopened = Connection::open(&database_path).expect("reopen committed database");
        assert_eq!(
            reopened
                .query_row("SELECT COUNT(*) FROM committed", [], |row| row
                    .get::<_, i64>(0))
                .expect("committed row count"),
            1
        );
        drop(reopened);
        fs::remove_file(database_path).expect("remove commit fixture");
    }

    #[test]
    fn source_data_version_cas_detects_external_mutation() {
        let database_path = std::env::temp_dir().join(format!(
            "ae-store-source-cas-{}-{}.sqlite",
            std::process::id(),
            now_ms()
        ));
        let mut source = Connection::open(&database_path).expect("source database");
        source
            .execute_batch(
                "CREATE TABLE authority(value INTEGER NOT NULL); INSERT INTO authority VALUES (1);",
            )
            .expect("source fixture");
        let snapshot = source
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .expect("read snapshot");
        let data_version = source_data_version(&snapshot).expect("source data version");
        assert_eq!(
            snapshot
                .query_row("SELECT value FROM authority", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        snapshot.commit().expect("close source snapshot");

        Connection::open(&database_path)
            .expect("external writer")
            .execute("UPDATE authority SET value=2", [])
            .expect("external mutation");
        let cas = source
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("CAS transaction");
        assert_ne!(source_data_version(&cas).unwrap(), data_version);
        drop(cas);
        drop(source);
        fs::remove_file(database_path).expect("remove CAS fixture");
    }
}
