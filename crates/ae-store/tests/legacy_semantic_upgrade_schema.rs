use ae_store::Store;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

// Original CREATE statement from 1774023 (also ee10648), whitespace only adjusted.
const HISTORICAL: &str = include_str!("fixtures/semantic_upgrades_1774023.sql");
static NEXT: AtomicU64 = AtomicU64::new(0);

fn fixture(sql: &str) -> PathBuf {
    let root = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target"));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join(format!(
        "backup-digest-regression-{}-{}.sqlite3",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    assert!(!path.exists());
    Connection::open(&path).unwrap().execute_batch(sql).unwrap();
    path
}

fn catalog(conn: &Connection) -> Vec<(String, String, Option<String>)> {
    conn.prepare("SELECT type,name,sql FROM sqlite_schema ORDER BY type,name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn rejected_unchanged(sql: &str, expected: &str) {
    let path = fixture(sql);
    let conn = Connection::open(&path).unwrap();
    let before = catalog(&conn);
    let rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM legacy_semantic_formula_upgrades",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let data_before: Vec<String> = conn
        .prepare(
            "SELECT hex(upgrade_bytes) FROM legacy_semantic_formula_upgrades ORDER BY migration_id",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let error = Store::open(&path)
        .err()
        .expect("must reject unsafe migration")
        .to_string();
    assert!(error.contains(expected), "{error}");
    assert_eq!(catalog(&conn), before);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM legacy_semantic_formula_upgrades",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        rows
    );
    let data_after: Vec<String> = conn
        .prepare(
            "SELECT hex(upgrade_bytes) FROM legacy_semantic_formula_upgrades ORDER BY migration_id",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(data_after, data_before);
}

#[test]
fn historical_empty_public_open_and_reopen() {
    let path = fixture(HISTORICAL);
    drop(Store::open(&path).expect("empty historical schema should migrate"));
    let conn = Connection::open(&path).unwrap();
    let columns: i64 = conn.query_row("SELECT COUNT(*) FROM pragma_table_info('legacy_semantic_formula_upgrades') WHERE name='backup_digest' AND type='BLOB' AND [notnull]=1", [], |r| r.get(0)).unwrap();
    assert_eq!(columns, 1);
    let before = catalog(&conn);
    drop(Store::open(&path).expect("migrated store should reopen"));
    assert_eq!(before, catalog(&conn));
}

#[test]
fn fresh_public_open_and_reopen() {
    let path = fixture("");
    drop(Store::open(&path).unwrap());
    drop(Store::open(&path).unwrap());
}

#[test]
fn nonempty_historical_schema_is_preserved() {
    rejected_unchanged(&format!("{HISTORICAL} INSERT INTO legacy_semantic_formula_upgrades VALUES (zeroblob(32),zeroblob(32),zeroblob(32),0,1,zeroblob(32),zeroblob(32),zeroblob(32),zeroblob(32),zeroblob(32),zeroblob(32),zeroblob(32),X'010203');"), "verified backup migration");
}

#[test]
fn unknown_missing_column_schema_is_preserved() {
    rejected_unchanged(
        &HISTORICAL.replace("UNIQUE (scope_digest, next_revision),", ""),
        "unrecognized legacy semantic upgrade schema",
    );
}

#[test]
fn extra_trigger_is_preserved_and_rejected() {
    rejected_unchanged(&format!("{HISTORICAL} CREATE TRIGGER extra_upgrade_trigger AFTER INSERT ON legacy_semantic_formula_upgrades BEGIN SELECT 1; END;"), "unrecognized legacy semantic upgrade schema");
}

#[test]
fn orphan_related_records_are_preserved_and_rejected() {
    for table in [
        "field_migration_preimage_backups",
        "semantic_migration_authority_v1",
    ] {
        rejected_unchanged(&format!("{HISTORICAL} CREATE TABLE {table}(sentinel BLOB); INSERT INTO {table} VALUES(X'1234');"), "verified backup migration");
    }
}

#[test]
fn orphan_upgrade_delta_is_preserved_and_rejected() {
    for table in ["journal", "graph_commits"] {
        rejected_unchanged(&format!("{HISTORICAL} CREATE TABLE {table}(delta_bytes BLOB); INSERT INTO {table} VALUES(X'41452D4C535531000102');"), "verified backup migration");
    }
}

#[test]
fn downstream_failure_rolls_back_replaced_table_and_preserves_rows() {
    // Compatibility can safely DROP this exact empty table, but the existing
    // migration later fails while inspecting the malformed ordinary journal.
    let path = fixture(&format!(
        "{HISTORICAL} CREATE TABLE journal(sentinel BLOB); INSERT INTO journal VALUES(X'1234');"
    ));
    let conn = Connection::open(&path).unwrap();
    let before = catalog(&conn);
    let error = Store::open(&path)
        .err()
        .expect("downstream schema must fail")
        .to_string();
    assert!(error.contains("no such column"), "{error}");
    assert_eq!(catalog(&conn), before);
    assert_eq!(
        conn.query_row("SELECT hex(sentinel) FROM journal", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "1234"
    );
}

#[test]
fn ordinary_legacy_journal_survives_public_open_and_reopen() {
    use ae_contracts::{
        wire, AdminAction, CanonicalEvent, CommitStatus, InvariantResiduals, ScopeRef,
        TransitionReceipt,
    };
    // Real pre-delta journal shape and canonical ordinary admin event. A
    // populated journal is not itself evidence of a semantic formula upgrade.
    for with_delta_column in [false, true] {
        let path = fixture(&format!(
            "{HISTORICAL} CREATE TABLE journal (
        revision INTEGER PRIMARY KEY AUTOINCREMENT,
        scope_digest BLOB NOT NULL, base_revision INTEGER NOT NULL,
        event_kind TEXT NOT NULL, event_bytes BLOB NOT NULL,
        event_digest BLOB NOT NULL, receipt_bytes BLOB NOT NULL,
        chain_digest BLOB NOT NULL, committed_at_ms INTEGER NOT NULL);"
        ));
        let conn = Connection::open(&path).unwrap();
        if with_delta_column {
            conn.execute_batch(
                "ALTER TABLE journal ADD COLUMN delta_bytes BLOB NOT NULL DEFAULT X'';",
            )
            .unwrap();
        }
        let event = CanonicalEvent::AdminAction(AdminAction {
            event_id: [1; 16],
            scope: ScopeRef {
                bot_token: [2; 16],
                persona_token: [3; 16],
                relation_token: None,
                session_token: [1; 16],
            },
            operation: "journal_test".into(),
            nonce_digest: [4; 32],
        });
        let event_bytes = wire::encode_event(&event);
        let scope = wire::persona_scope_digest(&[2; 16], &[3; 16], None);
        let receipt = TransitionReceipt {
            schema_version: 1,
            formula_digest: [5; 32],
            scope_digest: scope,
            event_digest: wire::event_digest(&event),
            authority_digest: ae_authority::authority_projection_digest(&event),
            base_revision: 0,
            next_revision: 1,
            state_before: [6; 32],
            state_after: [7; 32],
            graph_after: [8; 32],
            action_contract: None,
            active_nodes: 0,
            active_edges: 0,
            residuals: InvariantResiduals::default(),
            status: CommitStatus::Committed,
        };
        let receipt_bytes = wire::encode_transition_receipt(&receipt);
        let chain = ae_continuum::chain_link(&[0; 32], &event_bytes, &receipt_bytes);
        conn.execute("INSERT INTO journal(scope_digest,base_revision,event_kind,event_bytes,event_digest,receipt_bytes,chain_digest,committed_at_ms) VALUES(?1,0,'admin_action',?2,?3,?4,?5,1)", rusqlite::params![scope.to_vec(), event_bytes, receipt.event_digest.to_vec(), receipt_bytes, chain.to_vec()]).unwrap();
        let preserved = || {
            conn.query_row(
                "SELECT event_bytes,receipt_bytes,chain_digest FROM journal",
                [],
                |r| {
                    Ok((
                        r.get::<_, Vec<u8>>(0)?,
                        r.get::<_, Vec<u8>>(1)?,
                        r.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .unwrap()
        };
        let before = preserved();
        drop(Store::open(&path).expect("ordinary history must not block schema compatibility"));
        assert_eq!(preserved(), before);
        drop(Store::open(&path).expect("ordinary historical database must reopen"));
        assert_eq!(preserved(), before);
    }
}
