use crate::{locate_vault, VaultLocateError, VaultMode};
use ae_contracts::{wire, Digest};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use thiserror::Error;

const GENERATIONS_DIRECTORY: &str = "generations";
const AUTHORITY_DATABASE: &str = "authority.sqlite";
const MIGRATION_INTENT_FILE: &str = "migration.intent";
const LOCATOR_DATABASE: &str = "continuity_locator.sqlite";
const LOCATOR_SLOT: i64 = 1;
const MAX_GENERATION_ID_BYTES: usize = 128;
static NEXT_SHADOW: AtomicU64 = AtomicU64::new(0);

/// The identity and durable continuity values that must be identical before
/// and after a promoted generation change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContinuityAuthority {
    pub incarnation_id: Digest,
    pub revision: u64,
    pub state_digest: Digest,
    pub graph_digest: Digest,
    pub history_digest: Digest,
}

/// The durable generation resolved by the migration locator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurrentContinuityGeneration {
    pub generation_id: String,
    pub authority: ContinuityAuthority,
}

/// The terminal result encoded in a migration receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContinuityMigrationDecision {
    Switched,
    Replayed,
}

/// A receipt records the observed authority before and after the operation,
/// whether it was a retry, and the durable decision taken by the locator CAS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContinuityMigrationReceipt {
    pub before: ContinuityAuthority,
    pub after: ContinuityAuthority,
    pub replay: bool,
    pub decision: ContinuityMigrationDecision,
}

/// Deterministic migration failure points.  These are intentionally part of
/// the public boundary so process supervisors can prove their recovery path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContinuityMigrationFault {
    BeforeBackup,
    AfterBackup,
    BeforeLocatorCas,
    AfterLocatorCas,
    AfterLocatorFileCreate,
    AfterFirstLocatorSchemaDdl,
    AfterSecondLocatorSchemaDdl,
    AfterLocatorCommit,
}

/// A fixed stage code for a failed durability operation.  These remain public
/// so supervisors can distinguish the failed fence without receiving an OS
/// error, path, or other runtime payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum ContinuityMigrationStage {
    #[error("CONTINUITY_MIGRATION_DURABILITY_ROOT_CREATE_FAILED")]
    EnsureGenerationsRoot,
    #[error("CONTINUITY_MIGRATION_DURABILITY_FILE_OPEN_FAILED")]
    OpenFile,
    #[error("CONTINUITY_MIGRATION_DURABILITY_FILE_FLUSH_FAILED")]
    FlushFile,
    #[error("CONTINUITY_MIGRATION_DURABILITY_DIRECTORY_OPEN_FAILED")]
    OpenDirectory,
    #[error("CONTINUITY_MIGRATION_DURABILITY_DIRECTORY_FLUSH_FAILED")]
    FlushDirectory,
}

#[derive(Debug, Error)]
pub enum ContinuityMigrationError {
    #[error("CONTINUITY_MIGRATION_SOURCE_NOT_READY")]
    SourceNotReady,
    #[error("CONTINUITY_MIGRATION_VAULT_FAILURE")]
    Vault,
    #[error("CONTINUITY_MIGRATION_INVALID_GENERATION")]
    InvalidGeneration,
    #[error("CONTINUITY_MIGRATION_SOURCE_MISSING")]
    SourceMissing,
    #[error("CONTINUITY_MIGRATION_TARGET_EXISTS")]
    TargetGenerationExists,
    #[error("CONTINUITY_MIGRATION_AUTHORITY_INVALID")]
    AuthorityMissingOrAmbiguous,
    #[error("CONTINUITY_MIGRATION_AUTHORITY_BYTES_INVALID")]
    InvalidAuthorityBytes,
    #[error("CONTINUITY_MIGRATION_AUTHORITY_LOCATOR_MISMATCH")]
    AuthorityLocatorMismatch,
    #[error("CONTINUITY_MIGRATION_REVISION_OUT_OF_RANGE")]
    RevisionOutOfRange,
    #[error("CONTINUITY_MIGRATION_DATABASE_FAILURE")]
    Sqlite,
    #[error("CONTINUITY_MIGRATION_FILESYSTEM_FAILURE")]
    Io,
    #[error("{0}")]
    StageIo(ContinuityMigrationStage),
    #[error("CONTINUITY_MIGRATION_SHADOW_INTEGRITY_FAILURE")]
    ShadowIntegrity,
    #[error("CONTINUITY_MIGRATION_CONCURRENT_LOCATOR_CHANGE")]
    ConcurrentLocatorChange,
    #[error("CONTINUITY_MIGRATION_POST_SWITCH_AUTHORITY_MISMATCH")]
    PostSwitchAuthorityMismatch,
    #[error("CONTINUITY_MIGRATION_INJECTED_FAULT")]
    InjectedFault(ContinuityMigrationFault),
    #[error("CONTINUITY_MIGRATION_LOCATOR_INVALID")]
    LocatorInvalid,
    #[error("CONTINUITY_MIGRATION_SHADOW_SEQUENCE_EXHAUSTED")]
    ShadowSequenceExhausted,
}

impl From<VaultLocateError> for ContinuityMigrationError {
    fn from(_: VaultLocateError) -> Self {
        Self::Vault
    }
}

impl From<rusqlite::Error> for ContinuityMigrationError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Sqlite
    }
}

impl From<std::io::Error> for ContinuityMigrationError {
    fn from(_: std::io::Error) -> Self {
        Self::Io
    }
}

/// Resolve the current authoritative generation.  The SQLite locator is
/// authoritative once present; an existing vault without one falls back to the
/// original guarded `current` control file.
pub fn open_current_generation(
    vault_root: impl AsRef<Path>,
) -> Result<CurrentContinuityGeneration, ContinuityMigrationError> {
    let location = locate_vault(vault_root)?;
    if location.mode != VaultMode::Ready {
        return Err(ContinuityMigrationError::SourceNotReady);
    }
    let generation_id =
        read_locator_generation(&location.root)?.unwrap_or_else(|| location.generation_id.clone());
    validate_generation_id(&generation_id)?;
    let authority = capture_authority(
        &generation_database_path(&location.root, &generation_id),
        &location.incarnation_id,
        location.revision,
    )?;
    Ok(CurrentContinuityGeneration {
        generation_id,
        authority,
    })
}

/// Make a native SQLite shadow copy and publish it only through a durable
/// compare-and-swap locator update.  The source database is never edited or
/// moved; a failed operation leaves it authoritative unless the locator has
/// already atomically selected the fully verified target.
pub fn migrate_continuity(
    vault_root: impl AsRef<Path>,
    target_generation: &str,
    fault: Option<ContinuityMigrationFault>,
) -> Result<ContinuityMigrationReceipt, ContinuityMigrationError> {
    validate_generation_id(target_generation)?;
    let location = locate_vault(vault_root)?;
    if location.mode != VaultMode::Ready {
        return Err(ContinuityMigrationError::SourceNotReady);
    }
    let root = location.root;
    let source_generation =
        read_locator_generation(&root)?.unwrap_or_else(|| location.generation_id.clone());
    validate_generation_id(&source_generation)?;

    if source_generation == target_generation {
        let source_database = generation_database_path(&root, &source_generation);
        if !source_database.is_file() {
            return Err(ContinuityMigrationError::SourceMissing);
        }
        let mut source_lock_connection = Connection::open(&source_database)?;
        source_lock_connection.busy_timeout(Duration::from_secs(5))?;
        let source_write_fence =
            source_lock_connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let fenced_source_generation =
            read_locator_generation(&root)?.unwrap_or_else(|| location.generation_id.clone());
        if fenced_source_generation != source_generation {
            return Err(ContinuityMigrationError::ConcurrentLocatorChange);
        }
        let authority = capture_authority(
            &source_database,
            &location.incarnation_id,
            location.revision,
        )?;
        record_replay_receipt(&root, &source_generation, &authority)?;
        source_write_fence.commit()?;
        return Ok(ContinuityMigrationReceipt {
            before: authority.clone(),
            after: authority,
            replay: true,
            decision: ContinuityMigrationDecision::Replayed,
        });
    }

    let source_database = generation_database_path(&root, &source_generation);
    if !source_database.is_file() {
        return Err(ContinuityMigrationError::SourceMissing);
    }
    // An IMMEDIATE transaction takes the only source-writer lease before the
    // backup snapshot is selected.  The migration does not write the source,
    // but no concurrent writer may advance its authority between verification
    // and locator promotion.
    let mut source_lock_connection = Connection::open(&source_database)?;
    source_lock_connection.busy_timeout(Duration::from_secs(5))?;
    let source_write_fence =
        source_lock_connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let fenced_source_generation =
        read_locator_generation(&root)?.unwrap_or_else(|| location.generation_id.clone());
    if fenced_source_generation != source_generation {
        return Err(ContinuityMigrationError::ConcurrentLocatorChange);
    }
    let before = capture_authority(
        &source_database,
        &location.incarnation_id,
        location.revision,
    )?;

    let generations_root = root.join(GENERATIONS_DIRECTORY);
    fs::create_dir_all(&generations_root).map_err(|_| {
        ContinuityMigrationError::StageIo(ContinuityMigrationStage::EnsureGenerationsRoot)
    })?;
    let target_directory = generations_root.join(target_generation);
    if target_directory.exists() {
        verify_migration_intent(&target_directory, target_generation, &before)?;
        let target_database = target_directory.join(AUTHORITY_DATABASE);
        let target_authority = capture_authority(
            &target_database,
            &location.incarnation_id,
            location.revision,
        )?;
        if target_authority != before {
            return Err(ContinuityMigrationError::TargetGenerationExists);
        }
        durably_publish_target(&target_directory, &generations_root)?;
        if fault == Some(ContinuityMigrationFault::BeforeLocatorCas) {
            return Err(ContinuityMigrationError::InjectedFault(
                ContinuityMigrationFault::BeforeLocatorCas,
            ));
        }
        let current = activate_target_generation(
            &root,
            &source_generation,
            target_generation,
            &before,
            fault,
        )?;
        if fault == Some(ContinuityMigrationFault::AfterLocatorCas) {
            return Err(ContinuityMigrationError::InjectedFault(
                ContinuityMigrationFault::AfterLocatorCas,
            ));
        }
        source_write_fence.commit()?;
        return Ok(ContinuityMigrationReceipt {
            before,
            after: current.authority,
            replay: false,
            decision: ContinuityMigrationDecision::Switched,
        });
    }
    if fault == Some(ContinuityMigrationFault::BeforeBackup) {
        return Err(ContinuityMigrationError::InjectedFault(
            ContinuityMigrationFault::BeforeBackup,
        ));
    }

    let shadow_directory = create_shadow_directory(&generations_root, target_generation)?;
    let result = (|| {
        let shadow_database = shadow_directory.join(AUTHORITY_DATABASE);
        sqlite_shadow_backup(&source_database, &shadow_database)?;
        sync_database(&shadow_database)?;
        let shadow_authority = capture_authority(
            &shadow_database,
            &location.incarnation_id,
            location.revision,
        )?;
        if shadow_authority != before {
            return Err(ContinuityMigrationError::PostSwitchAuthorityMismatch);
        }
        write_migration_intent(
            &shadow_directory,
            &source_generation,
            target_generation,
            &before,
        )?;
        if fault == Some(ContinuityMigrationFault::AfterBackup) {
            return Err(ContinuityMigrationError::InjectedFault(
                ContinuityMigrationFault::AfterBackup,
            ));
        }
        fs::rename(&shadow_directory, &target_directory)?;
        durably_publish_target(&target_directory, &generations_root)?;
        if fault == Some(ContinuityMigrationFault::BeforeLocatorCas) {
            return Err(ContinuityMigrationError::InjectedFault(
                ContinuityMigrationFault::BeforeLocatorCas,
            ));
        }
        let current = activate_target_generation(
            &root,
            &source_generation,
            target_generation,
            &before,
            fault,
        )?;
        if fault == Some(ContinuityMigrationFault::AfterLocatorCas) {
            return Err(ContinuityMigrationError::InjectedFault(
                ContinuityMigrationFault::AfterLocatorCas,
            ));
        }
        source_write_fence.commit()?;
        Ok(ContinuityMigrationReceipt {
            before: before.clone(),
            after: current.authority,
            replay: false,
            decision: ContinuityMigrationDecision::Switched,
        })
    })();

    if result.is_err() && shadow_directory.exists() {
        let _ = fs::remove_dir_all(&shadow_directory);
    }
    result
}

fn activate_target_generation(
    root: &Path,
    source_generation: &str,
    target_generation: &str,
    expected_authority: &ContinuityAuthority,
    fault: Option<ContinuityMigrationFault>,
) -> Result<CurrentContinuityGeneration, ContinuityMigrationError> {
    if !compare_and_swap_locator(
        root,
        source_generation,
        target_generation,
        expected_authority,
        fault,
    )? {
        return Err(ContinuityMigrationError::ConcurrentLocatorChange);
    }
    let current = open_current_generation(root)?;
    if current.generation_id != target_generation || current.authority != *expected_authority {
        return Err(ContinuityMigrationError::PostSwitchAuthorityMismatch);
    }
    Ok(current)
}

fn write_migration_intent(
    directory: &Path,
    source_generation: &str,
    target_generation: &str,
    authority: &ContinuityAuthority,
) -> Result<(), ContinuityMigrationError> {
    let path = directory.join(MIGRATION_INTENT_FILE);
    fs::write(
        &path,
        migration_intent(source_generation, target_generation, authority),
    )?;
    sync_file(&path)
}

/// Ensure every file accepted as the target authority and both directory
/// entries are durable before the locator can make that target authoritative.
fn durably_publish_target(
    target_directory: &Path,
    generations_root: &Path,
) -> Result<(), ContinuityMigrationError> {
    sync_file(&target_directory.join(AUTHORITY_DATABASE))?;
    sync_file(&target_directory.join(MIGRATION_INTENT_FILE))?;
    sync_directory(target_directory)?;
    sync_directory(generations_root)
}

fn verify_migration_intent(
    directory: &Path,
    target_generation: &str,
    authority: &ContinuityAuthority,
) -> Result<(), ContinuityMigrationError> {
    let intent = fs::read_to_string(directory.join(MIGRATION_INTENT_FILE))?;
    let Some(source_line_end) = intent.find('\n') else {
        return Err(ContinuityMigrationError::TargetGenerationExists);
    };
    let Some(intent_source) = intent[..source_line_end].strip_prefix("source_generation=") else {
        return Err(ContinuityMigrationError::TargetGenerationExists);
    };
    validate_generation_id(intent_source)
        .map_err(|_| ContinuityMigrationError::TargetGenerationExists)?;
    let expected_tail = format!(
        "target_generation={target_generation}\nincarnation_id={}\nrevision={}\nstate_digest={}\ngraph_digest={}\nhistory_digest={}\n",
        digest_hex(&authority.incarnation_id),
        authority.revision,
        digest_hex(&authority.state_digest),
        digest_hex(&authority.graph_digest),
        digest_hex(&authority.history_digest),
    );
    if intent[source_line_end + 1..] != expected_tail {
        return Err(ContinuityMigrationError::TargetGenerationExists);
    }
    Ok(())
}

fn migration_intent(
    source_generation: &str,
    target_generation: &str,
    authority: &ContinuityAuthority,
) -> String {
    format!(
        "source_generation={source_generation}\ntarget_generation={target_generation}\nincarnation_id={}\nrevision={}\nstate_digest={}\ngraph_digest={}\nhistory_digest={}\n",
        digest_hex(&authority.incarnation_id),
        authority.revision,
        digest_hex(&authority.state_digest),
        digest_hex(&authority.graph_digest),
        digest_hex(&authority.history_digest),
    )
}

fn digest_hex(value: &Digest) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn generation_database_path(root: &Path, generation_id: &str) -> PathBuf {
    root.join(GENERATIONS_DIRECTORY)
        .join(generation_id)
        .join(AUTHORITY_DATABASE)
}

fn validate_generation_id(value: &str) -> Result<(), ContinuityMigrationError> {
    if value.is_empty()
        || value.len() > MAX_GENERATION_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(ContinuityMigrationError::InvalidGeneration);
    }
    Ok(())
}

fn create_shadow_directory(
    generations_root: &Path,
    target_generation: &str,
) -> Result<PathBuf, ContinuityMigrationError> {
    loop {
        let sequence = next_shadow_sequence(&NEXT_SHADOW)?;
        let directory = generations_root.join(format!(
            ".shadow-{target_generation}-{}-{sequence}",
            std::process::id()
        ));
        match fs::create_dir(&directory) {
            Ok(()) => {
                sync_directory(generations_root)?;
                return Ok(directory);
            }
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(ContinuityMigrationError::Io),
        }
    }
}

fn next_shadow_sequence(counter: &AtomicU64) -> Result<u64, ContinuityMigrationError> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .map_err(|_| ContinuityMigrationError::ShadowSequenceExhausted)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LocatorSchemaState {
    Empty,
    Valid,
}

struct LocatorSchemaObject {
    kind: String,
    name: String,
    table: String,
    sql: Option<String>,
}

type LocatorColumnSpec = (
    i64,
    &'static str,
    &'static str,
    i64,
    Option<&'static str>,
    i64,
);

fn read_locator_generation(root: &Path) -> Result<Option<String>, ContinuityMigrationError> {
    let path = root.join(LOCATOR_DATABASE);
    if !path.exists() {
        return Ok(None);
    }
    let connection = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| ContinuityMigrationError::LocatorInvalid)?;
    if classify_locator_schema(&connection)? == LocatorSchemaState::Empty {
        return Ok(None);
    }
    let generation: Option<String> = connection
        .query_row(
            "SELECT generation_id FROM continuity_generation_locator WHERE slot = ?1",
            params![LOCATOR_SLOT],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| ContinuityMigrationError::LocatorInvalid)?;
    let generation = generation.ok_or(ContinuityMigrationError::LocatorInvalid)?;
    validate_generation_id(&generation).map_err(|_| ContinuityMigrationError::LocatorInvalid)?;
    Ok(Some(generation))
}

fn classify_locator_schema(
    connection: &Connection,
) -> Result<LocatorSchemaState, ContinuityMigrationError> {
    let schema_version: i64 = connection
        .pragma_query_value(None, "schema_version", |row| row.get(0))
        .map_err(|_| ContinuityMigrationError::LocatorInvalid)?;
    let objects = read_locator_schema_objects(connection)?;
    if objects.is_empty() && schema_version == 0 {
        return Ok(LocatorSchemaState::Empty);
    }

    let locator_sql = objects
        .iter()
        .find(|object| {
            object.kind == "table"
                && object.name == "continuity_generation_locator"
                && object.table == "continuity_generation_locator"
        })
        .and_then(|object| object.sql.as_deref());
    let receipts_sql = objects
        .iter()
        .find(|object| {
            object.kind == "table"
                && object.name == "continuity_migration_receipts"
                && object.table == "continuity_migration_receipts"
        })
        .and_then(|object| object.sql.as_deref());
    let has_exact_internal_index = objects.iter().any(|object| {
        object.kind == "index"
            && object.name == "sqlite_autoindex_continuity_migration_receipts_1"
            && object.table == "continuity_migration_receipts"
            && object.sql.is_none()
    });
    if objects.len() != 3
        || !has_exact_internal_index
        || !matches_locator_table_sql(locator_sql)
        || !matches_receipts_table_sql(receipts_sql)
    {
        return Err(ContinuityMigrationError::LocatorInvalid);
    }

    validate_locator_table_columns(connection)?;
    validate_receipts_table_columns(connection)?;
    validate_locator_indexes(connection)?;
    Ok(LocatorSchemaState::Valid)
}

fn read_locator_schema_objects(
    connection: &Connection,
) -> Result<Vec<LocatorSchemaObject>, ContinuityMigrationError> {
    let mut statement = connection
        .prepare("SELECT type, name, tbl_name, sql FROM sqlite_schema ORDER BY type, name")
        .map_err(|_| ContinuityMigrationError::LocatorInvalid)?;
    let rows = statement
        .query_map([], |row| {
            Ok(LocatorSchemaObject {
                kind: row.get(0)?,
                name: row.get(1)?,
                table: row.get(2)?,
                sql: row.get(3)?,
            })
        })
        .map_err(|_| ContinuityMigrationError::LocatorInvalid)?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|_| ContinuityMigrationError::LocatorInvalid)
}

fn matches_locator_table_sql(sql: Option<&str>) -> bool {
    matches_table_sql(
        sql,
        "createtablecontinuity_generation_locator(slotintegerprimarykeycheck(slot=1),generation_idtextnotnull)",
    )
}

fn matches_receipts_table_sql(sql: Option<&str>) -> bool {
    matches_table_sql(
        sql,
        "createtablecontinuity_migration_receipts(source_generationtextnotnull,target_generationtextnotnull,before_incarnation_idblobnotnull,before_revisionintegernotnull,before_state_digestblobnotnull,before_graph_digestblobnotnull,before_history_digestblobnotnull,after_incarnation_idblobnotnull,after_revisionintegernotnull,after_state_digestblobnotnull,after_graph_digestblobnotnull,after_history_digestblobnotnull,replayintegernotnull,decisiontextnotnull,primarykey(source_generation,target_generation))",
    )
}

fn matches_table_sql(sql: Option<&str>, expected: &str) -> bool {
    let Some(sql) = sql else {
        return false;
    };
    let normalized = normalize_schema_sql(sql);
    normalized == expected
        || normalized == expected.replacen("createtable", "createtableifnotexists", 1)
}

fn normalize_schema_sql(sql: &str) -> String {
    let mut normalized = sql
        .chars()
        .filter(|character| !character.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if normalized.ends_with(';') {
        normalized.pop();
    }
    normalized
}

fn validate_locator_table_columns(connection: &Connection) -> Result<(), ContinuityMigrationError> {
    validate_table_columns(
        connection,
        "PRAGMA table_info(continuity_generation_locator)",
        &[
            (0, "slot", "INTEGER", 0, None, 1),
            (1, "generation_id", "TEXT", 1, None, 0),
        ],
    )
}

fn validate_receipts_table_columns(
    connection: &Connection,
) -> Result<(), ContinuityMigrationError> {
    validate_table_columns(
        connection,
        "PRAGMA table_info(continuity_migration_receipts)",
        &[
            (0, "source_generation", "TEXT", 1, None, 1),
            (1, "target_generation", "TEXT", 1, None, 2),
            (2, "before_incarnation_id", "BLOB", 1, None, 0),
            (3, "before_revision", "INTEGER", 1, None, 0),
            (4, "before_state_digest", "BLOB", 1, None, 0),
            (5, "before_graph_digest", "BLOB", 1, None, 0),
            (6, "before_history_digest", "BLOB", 1, None, 0),
            (7, "after_incarnation_id", "BLOB", 1, None, 0),
            (8, "after_revision", "INTEGER", 1, None, 0),
            (9, "after_state_digest", "BLOB", 1, None, 0),
            (10, "after_graph_digest", "BLOB", 1, None, 0),
            (11, "after_history_digest", "BLOB", 1, None, 0),
            (12, "replay", "INTEGER", 1, None, 0),
            (13, "decision", "TEXT", 1, None, 0),
        ],
    )
}

fn validate_table_columns(
    connection: &Connection,
    pragma: &str,
    expected: &[LocatorColumnSpec],
) -> Result<(), ContinuityMigrationError> {
    let mut statement = connection
        .prepare(pragma)
        .map_err(|_| ContinuityMigrationError::LocatorInvalid)?;
    let columns = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(|_| ContinuityMigrationError::LocatorInvalid)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ContinuityMigrationError::LocatorInvalid)?;
    if columns.len() != expected.len()
        || columns.iter().zip(expected).any(|(actual, expected)| {
            actual.0 != expected.0
                || actual.1 != expected.1
                || actual.2 != expected.2
                || actual.3 != expected.3
                || actual.4.as_deref() != expected.4
                || actual.5 != expected.5
        })
    {
        return Err(ContinuityMigrationError::LocatorInvalid);
    }
    Ok(())
}

fn validate_locator_indexes(connection: &Connection) -> Result<(), ContinuityMigrationError> {
    let mut locator_statement = connection
        .prepare("PRAGMA index_list(continuity_generation_locator)")
        .map_err(|_| ContinuityMigrationError::LocatorInvalid)?;
    let locator_indexes = locator_statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|_| ContinuityMigrationError::LocatorInvalid)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ContinuityMigrationError::LocatorInvalid)?;
    if !locator_indexes.is_empty() {
        return Err(ContinuityMigrationError::LocatorInvalid);
    }

    let mut receipts_statement = connection
        .prepare("PRAGMA index_list(continuity_migration_receipts)")
        .map_err(|_| ContinuityMigrationError::LocatorInvalid)?;
    let receipts_indexes = receipts_statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|_| ContinuityMigrationError::LocatorInvalid)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ContinuityMigrationError::LocatorInvalid)?;
    if receipts_indexes.as_slice()
        != [(
            0,
            "sqlite_autoindex_continuity_migration_receipts_1".to_owned(),
            1,
            "pk".to_owned(),
            0,
        )]
    {
        return Err(ContinuityMigrationError::LocatorInvalid);
    }

    let mut index_info_statement = connection
        .prepare("PRAGMA index_info(sqlite_autoindex_continuity_migration_receipts_1)")
        .map_err(|_| ContinuityMigrationError::LocatorInvalid)?;
    let index_columns = index_info_statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|_| ContinuityMigrationError::LocatorInvalid)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ContinuityMigrationError::LocatorInvalid)?;
    if index_columns.as_slice()
        != [
            (0, 0, "source_generation".to_owned()),
            (1, 1, "target_generation".to_owned()),
        ]
    {
        return Err(ContinuityMigrationError::LocatorInvalid);
    }
    Ok(())
}

fn compare_and_swap_locator(
    root: &Path,
    expected_generation: &str,
    target_generation: &str,
    authority: &ContinuityAuthority,
    fault: Option<ContinuityMigrationFault>,
) -> Result<bool, ContinuityMigrationError> {
    let path = root.join(LOCATOR_DATABASE);
    let locator_preexisted = path.exists();
    let mut connection = Connection::open(&path)?;
    configure_locator_connection(&connection)?;
    if !locator_preexisted && fault == Some(ContinuityMigrationFault::AfterLocatorFileCreate) {
        return Err(ContinuityMigrationError::InjectedFault(
            ContinuityMigrationFault::AfterLocatorFileCreate,
        ));
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    classify_locator_schema(&transaction)?;
    prepare_locator_schema(&transaction, fault)?;
    transaction.execute(
        "INSERT OR IGNORE INTO continuity_generation_locator (slot, generation_id) VALUES (?1, ?2)",
        params![LOCATOR_SLOT, expected_generation],
    )?;
    let changed = transaction.execute(
        "UPDATE continuity_generation_locator SET generation_id = ?2 WHERE slot = ?1 AND generation_id = ?3",
        params![LOCATOR_SLOT, target_generation, expected_generation],
    )?;
    if changed == 1 {
        insert_receipt(
            &transaction,
            expected_generation,
            target_generation,
            authority,
            false,
            "switched",
        )?;
    }
    transaction.commit()?;
    if fault == Some(ContinuityMigrationFault::AfterLocatorCommit) {
        return Err(ContinuityMigrationError::InjectedFault(
            ContinuityMigrationFault::AfterLocatorCommit,
        ));
    }
    // `synchronous=FULL` makes the committed SQLite locator durable.  Do not
    // perform another fallible filesystem flush after commit: callers must
    // never mistake a selected generation for an unselected one and remove it.
    Ok(changed == 1)
}

fn record_replay_receipt(
    root: &Path,
    generation: &str,
    authority: &ContinuityAuthority,
) -> Result<(), ContinuityMigrationError> {
    let path = root.join(LOCATOR_DATABASE);
    let mut connection = Connection::open(&path)?;
    configure_locator_connection(&connection)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    classify_locator_schema(&transaction)?;
    prepare_locator_schema(&transaction, None)?;
    transaction.execute(
        "INSERT OR IGNORE INTO continuity_generation_locator (slot, generation_id) VALUES (?1, ?2)",
        params![LOCATOR_SLOT, generation],
    )?;
    let selected: String = transaction.query_row(
        "SELECT generation_id FROM continuity_generation_locator WHERE slot = ?1",
        params![LOCATOR_SLOT],
        |row| row.get(0),
    )?;
    if selected != generation {
        return Err(ContinuityMigrationError::ConcurrentLocatorChange);
    }
    insert_receipt(
        &transaction,
        generation,
        generation,
        authority,
        true,
        "replayed",
    )?;
    transaction.commit()?;
    Ok(())
}

fn configure_locator_connection(connection: &Connection) -> Result<(), ContinuityMigrationError> {
    connection.pragma_update(None, "journal_mode", "DELETE")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    Ok(())
}

fn prepare_locator_schema(
    transaction: &rusqlite::Transaction<'_>,
    fault: Option<ContinuityMigrationFault>,
) -> Result<(), ContinuityMigrationError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS continuity_generation_locator (
             slot INTEGER PRIMARY KEY CHECK (slot = 1),
             generation_id TEXT NOT NULL
          );",
    )?;
    if fault == Some(ContinuityMigrationFault::AfterFirstLocatorSchemaDdl) {
        return Err(ContinuityMigrationError::InjectedFault(
            ContinuityMigrationFault::AfterFirstLocatorSchemaDdl,
        ));
    }
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS continuity_migration_receipts (
             source_generation TEXT NOT NULL,
             target_generation TEXT NOT NULL,
             before_incarnation_id BLOB NOT NULL,
             before_revision INTEGER NOT NULL,
             before_state_digest BLOB NOT NULL,
             before_graph_digest BLOB NOT NULL,
             before_history_digest BLOB NOT NULL,
             after_incarnation_id BLOB NOT NULL,
             after_revision INTEGER NOT NULL,
             after_state_digest BLOB NOT NULL,
             after_graph_digest BLOB NOT NULL,
             after_history_digest BLOB NOT NULL,
             replay INTEGER NOT NULL,
             decision TEXT NOT NULL,
             PRIMARY KEY (source_generation, target_generation)
         );",
    )?;
    if fault == Some(ContinuityMigrationFault::AfterSecondLocatorSchemaDdl) {
        return Err(ContinuityMigrationError::InjectedFault(
            ContinuityMigrationFault::AfterSecondLocatorSchemaDdl,
        ));
    }
    if classify_locator_schema(transaction)? != LocatorSchemaState::Valid {
        return Err(ContinuityMigrationError::LocatorInvalid);
    }
    Ok(())
}

fn insert_receipt(
    transaction: &rusqlite::Transaction<'_>,
    source_generation: &str,
    target_generation: &str,
    authority: &ContinuityAuthority,
    replay: bool,
    decision: &str,
) -> Result<(), ContinuityMigrationError> {
    let revision = i64::try_from(authority.revision)
        .map_err(|_| ContinuityMigrationError::RevisionOutOfRange)?;
    transaction.execute(
        "INSERT OR REPLACE INTO continuity_migration_receipts (
            source_generation, target_generation,
            before_incarnation_id, before_revision, before_state_digest, before_graph_digest, before_history_digest,
            after_incarnation_id, after_revision, after_state_digest, after_graph_digest, after_history_digest,
            replay, decision
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            source_generation,
            target_generation,
            authority.incarnation_id.to_vec(),
            revision,
            authority.state_digest.to_vec(),
            authority.graph_digest.to_vec(),
            authority.history_digest.to_vec(),
            authority.incarnation_id.to_vec(),
            revision,
            authority.state_digest.to_vec(),
            authority.graph_digest.to_vec(),
            authority.history_digest.to_vec(),
            i64::from(replay),
            decision,
        ],
    )?;
    Ok(())
}

fn sqlite_shadow_backup(
    source_database: &Path,
    shadow_database: &Path,
) -> Result<(), ContinuityMigrationError> {
    let source = Connection::open_with_flags(source_database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let destination = sqlite_string_literal(shadow_database)?;
    source.execute_batch(&format!("VACUUM INTO {destination};"))?;
    Ok(())
}

fn sqlite_string_literal(path: &Path) -> Result<String, ContinuityMigrationError> {
    let value = path
        .to_str()
        .ok_or(ContinuityMigrationError::InvalidGeneration)?;
    if value.contains('\0') {
        return Err(ContinuityMigrationError::InvalidGeneration);
    }
    Ok(format!("'{}'", value.replace('\'', "''")))
}

fn sync_database(path: &Path) -> Result<(), ContinuityMigrationError> {
    let connection = Connection::open(path)?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(ContinuityMigrationError::ShadowIntegrity);
    }
    drop(connection);
    sync_file(path)
}

fn sync_file(path: &Path) -> Result<(), ContinuityMigrationError> {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|_| ContinuityMigrationError::StageIo(ContinuityMigrationStage::OpenFile))?;
    file.sync_all()
        .map_err(|_| ContinuityMigrationError::StageIo(ContinuityMigrationStage::FlushFile))?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), ContinuityMigrationError> {
    #[cfg(windows)]
    let directory = {
        use std::os::windows::fs::OpenOptionsExt;

        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(0x0200_0000)
            .open(path)
    };
    #[cfg(not(windows))]
    let directory = fs::File::open(path);
    let directory = directory
        .map_err(|_| ContinuityMigrationError::StageIo(ContinuityMigrationStage::OpenDirectory))?;
    directory
        .sync_all()
        .map_err(|_| ContinuityMigrationError::StageIo(ContinuityMigrationStage::FlushDirectory))
}

fn capture_authority(
    database: &Path,
    expected_incarnation: &Digest,
    expected_revision: u64,
) -> Result<ContinuityAuthority, ContinuityMigrationError> {
    if !database.is_file() {
        return Err(ContinuityMigrationError::SourceMissing);
    }
    let connection = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let (bot_token, persona_token, incarnation_id, revision) = read_single_binding(&connection)?;
    if incarnation_id != *expected_incarnation || revision != expected_revision {
        return Err(ContinuityMigrationError::AuthorityLocatorMismatch);
    }
    let scope = wire::persona_scope_digest(&bot_token, &persona_token, None);
    let revision_sql =
        i64::try_from(revision).map_err(|_| ContinuityMigrationError::RevisionOutOfRange)?;
    let graph_digest = read_single_digest(
        &connection,
        "SELECT graph_digest FROM incarnations WHERE incarnation_id = ?1",
        params![incarnation_id.to_vec()],
    )?;
    let state_digest = read_single_digest(
        &connection,
        "SELECT state_digest FROM snapshots WHERE scope_digest = ?1 AND revision = ?2",
        params![scope.to_vec(), revision_sql],
    )?;
    let history_digest = if revision == 0 {
        wire::domain_hash(
            b"astr-embodiment/continuity-empty-history-v1",
            &[&incarnation_id, &revision.to_le_bytes()],
        )
    } else {
        read_single_digest(
            &connection,
            "SELECT chain_digest FROM journal WHERE scope_digest = ?1 AND logical_revision = ?2",
            params![scope.to_vec(), revision_sql],
        )?
    };
    Ok(ContinuityAuthority {
        incarnation_id,
        revision,
        state_digest,
        graph_digest,
        history_digest,
    })
}

fn read_single_binding(
    connection: &Connection,
) -> Result<([u8; 16], [u8; 16], Digest, u64), ContinuityMigrationError> {
    let mut statement = connection.prepare(
        "SELECT bot_token, persona_token, incarnation_id, revision FROM active_bindings ORDER BY bot_token ASC, persona_token ASC",
    )?;
    let mut rows = statement.query([])?;
    let Some(row) = rows.next()? else {
        return Err(ContinuityMigrationError::AuthorityMissingOrAmbiguous);
    };
    let bot_token = id_from_blob(row.get(0)?)?;
    let persona_token = id_from_blob(row.get(1)?)?;
    let incarnation_id = digest_from_blob(row.get(2)?)?;
    let revision_value: i64 = row.get(3)?;
    let revision =
        u64::try_from(revision_value).map_err(|_| ContinuityMigrationError::RevisionOutOfRange)?;
    if rows.next()?.is_some() {
        return Err(ContinuityMigrationError::AuthorityMissingOrAmbiguous);
    }
    Ok((bot_token, persona_token, incarnation_id, revision))
}

fn read_single_digest<P>(
    connection: &Connection,
    sql: &str,
    parameters: P,
) -> Result<Digest, ContinuityMigrationError>
where
    P: rusqlite::Params,
{
    let mut statement = connection.prepare(sql)?;
    let mut rows = statement.query(parameters)?;
    let Some(row) = rows.next()? else {
        return Err(ContinuityMigrationError::AuthorityMissingOrAmbiguous);
    };
    let digest = digest_from_blob(row.get(0)?)?;
    if rows.next()?.is_some() {
        return Err(ContinuityMigrationError::AuthorityMissingOrAmbiguous);
    }
    Ok(digest)
}

fn id_from_blob(value: Vec<u8>) -> Result<[u8; 16], ContinuityMigrationError> {
    value
        .try_into()
        .map_err(|_| ContinuityMigrationError::InvalidAuthorityBytes)
}

fn digest_from_blob(value: Vec<u8>) -> Result<Digest, ContinuityMigrationError> {
    value
        .try_into()
        .map_err(|_| ContinuityMigrationError::InvalidAuthorityBytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadow_sequence_refuses_to_wrap() {
        let counter = AtomicU64::new(u64::MAX);

        let error = next_shadow_sequence(&counter).unwrap_err();

        assert!(matches!(
            error,
            ContinuityMigrationError::ShadowSequenceExhausted
        ));
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }
}
