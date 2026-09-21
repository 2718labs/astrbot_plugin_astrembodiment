//! Narrow compatibility for the pre-backup 1774023 / ee10648 schema.
use crate::StoreError;
use rusqlite::{OptionalExtension, Transaction};

const OLD_SCHEMA: &str = "CREATE TABLE legacy_semantic_formula_upgrades (
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
)";
const UNKNOWN: &str = "unrecognized legacy semantic upgrade schema; database preserved";
const HISTORY: &str =
    "legacy semantic upgrade history requires verified backup migration; database preserved";

fn reject(reason: &str) -> StoreError {
    StoreError::Sqlite(reason.to_owned())
}

// No literals occur in OLD_SCHEMA. Preserve every constraint/token while
// accepting only whitespace and ASCII keyword case differences.
fn normalized(sql: &str) -> String {
    sql.chars()
        .filter(|c| !c.is_ascii_whitespace())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn table_exists(tx: &Transaction<'_>, table: &str) -> Result<bool, StoreError> {
    let kind: Option<String> = tx
        .query_row(
            "SELECT type FROM sqlite_schema WHERE name=?1",
            [table],
            |r| r.get(0),
        )
        .optional()?;
    match kind.as_deref() {
        None => Ok(false),
        Some("table") => Ok(true),
        _ => Err(reject(UNKNOWN)),
    }
}

fn has_column(tx: &Transaction<'_>, table: &str, column: &str) -> Result<bool, StoreError> {
    Ok(tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name=?2)",
        [table, column],
        |r| r.get(0),
    )?)
}

fn reject_upgrade_deltas(tx: &Transaction<'_>, table: &str) -> Result<(), StoreError> {
    if !table_exists(tx, table)? {
        return Ok(());
    }
    if !has_column(tx, table, "delta_bytes")? {
        // The original journal had no delta column; graph_commits owned it.
        return if table == "journal" {
            Ok(())
        } else {
            Err(reject(UNKNOWN))
        };
    }
    // Same row/byte ceilings as semantic history attestation. Read only type,
    // length and magic prefix, never materialize arbitrary historical blobs.
    let mut statement = tx.prepare(&format!("SELECT typeof(delta_bytes), length(delta_bytes), CASE WHEN length(delta_bytes)>=8 THEN substr(delta_bytes,1,8)=X'41452D4C53553100' ELSE 0 END FROM {table} LIMIT 65537"))?;
    let mut rows = statement.query([])?;
    let mut count = 0u64;
    let mut bytes = 0u64;
    while let Some(row) = rows.next()? {
        count += 1;
        let kind: String = row.get(0)?;
        let length: Option<i64> = row.get(1)?;
        let is_upgrade: Option<bool> = row.get(2)?;
        if kind != "blob" || is_upgrade != Some(false) {
            return Err(reject(HISTORY));
        }
        bytes = bytes.saturating_add(
            length
                .and_then(|n| u64::try_from(n).ok())
                .ok_or_else(|| reject(HISTORY))?,
        );
        if count > 65_536 || bytes > 4 * 1024 * 1024 * 1024 {
            return Err(reject(HISTORY));
        }
    }
    Ok(())
}

pub(crate) fn prepare_empty_legacy_upgrade_table(tx: &Transaction<'_>) -> Result<(), StoreError> {
    let table = "legacy_semantic_formula_upgrades";
    if !table_exists(tx, table)? || has_column(tx, table, "backup_digest")? {
        return Ok(());
    }
    let sql: String = tx.query_row(
        "SELECT sql FROM sqlite_schema WHERE name=?1",
        [table],
        |r| r.get(0),
    )?;
    let extra_objects: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE tbl_name=?1 AND type<>'table' AND sql IS NOT NULL)", [table], |r| r.get(0))?;
    if normalized(&sql) != normalized(OLD_SCHEMA) || extra_objects {
        return Err(reject(UNKNOWN));
    }
    for related in [
        table,
        "field_migration_preimage_backups",
        "semantic_migration_authority_v1",
    ] {
        if table_exists(tx, related)? {
            let nonempty: bool = tx.query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM {related})"),
                [],
                |r| r.get(0),
            )?;
            if nonempty {
                return Err(reject(HISTORY));
            }
        }
    }
    reject_upgrade_deltas(tx, "journal")?;
    reject_upgrade_deltas(tx, "graph_commits")?;
    // All statements remain in Store::migrate's transaction. The unchanged
    // current CREATE statement restores its complete PK/UNIQUE contract.
    tx.execute_batch("DROP TABLE legacy_semantic_formula_upgrades")?;
    Ok(())
}
