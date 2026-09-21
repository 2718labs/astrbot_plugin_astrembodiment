use ae_store::{Store, StoreError, AUTONOMY_DB_VERSION};
use rusqlite::Connection;

#[test]
fn public_open_installs_v9_and_reopen_preserves_logical_database() {
    let root = std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .expect("task-local CARGO_TARGET_DIR");
    let path = root.join(format!("core-boundary-v9-{}.sqlite3", std::process::id()));
    assert!(!path.exists(), "use a fresh fixture filename");
    let store = Store::open(&path).unwrap();
    assert_eq!(AUTONOMY_DB_VERSION, 9);
    // Retired executors are absent; the bounded classifier owns rejection.
    assert_eq!(
        ae_contracts::classify_retired_operation_v1("apply_event"),
        "UNSUPPORTED_CORE_BOUNDARY"
    );
    drop(store);
    let read = Connection::open(&path).unwrap();
    let snapshot = |conn: &Connection| -> Vec<(String, Vec<u8>)> {
        let mut stmt = conn.prepare("SELECT 'control',CAST(quote(schema_digest)||quote(receipt_root)||authoritative_now_utc_ms AS BLOB) FROM core_boundary_control_v1 UNION ALL SELECT 'migration',CAST(version||quote(digest)||completed_at_ms AS BLOB) FROM schema_migrations ORDER BY 1,2").unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let before = snapshot(&read);
    let reopened = Store::open(&path).unwrap();
    assert_eq!(snapshot(&read), before);
    assert_eq!(reopened.count_journal().unwrap(), 0);
    drop(reopened);
    drop(read);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn sqlite_revision_boundaries_preserve_typed_errors() {
    let root = std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .expect("task-local CARGO_TARGET_DIR");
    let path = root.join(format!("typed-revision-{}.sqlite3", std::process::id()));
    assert!(!path.exists());
    let store = Store::open(&path).unwrap();
    let conn = Connection::open(&path).unwrap();
    let scope = [114_u8; 32];
    // Synthetic corruption fixture, never a user database.
    conn.execute(
        "INSERT INTO journal (logical_revision,scope_digest,base_revision,event_kind,event_bytes,event_digest,receipt_bytes,chain_digest,committed_at_ms) VALUES (-1,?1,0,'fixture',X'',zeroblob(32),X'',zeroblob(32),0)",
        rusqlite::params![scope.to_vec()],
    )
    .unwrap();
    assert!(matches!(
        store.current_revision(&scope),
        Err(StoreError::InvalidStoredRevision { revision: -1 })
    ));
    for revision in [u64::try_from(i64::MAX).unwrap() + 1, u64::MAX] {
        assert!(matches!(
            store.read_snapshot(&scope, revision),
            Err(StoreError::RevisionOutOfRange { revision: actual }) if actual == revision
        ));
    }
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM journal", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(store);
    drop(conn);
    std::fs::remove_file(path).unwrap();
}
