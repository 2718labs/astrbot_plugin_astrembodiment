use ae_store::{Store, StoreError, AUTONOMY_DB_VERSION};
use rusqlite::Connection;

#[test]
fn public_open_installs_v9_and_reopen_preserves_logical_database() {
    let root = std::env::var_os("CARGO_TARGET_DIR").map(std::path::PathBuf::from)
        .expect("task-local CARGO_TARGET_DIR");
    let path = root.join(format!("core-boundary-v9-{}.sqlite3", std::process::id()));
    assert!(!path.exists(), "use a fresh fixture filename");
    let store = Store::open(&path).unwrap();
    assert_eq!(AUTONOMY_DB_VERSION, 9);
    assert!(matches!(store.list_autonomy_scopes(),
        Err(StoreError::ContinuityFence("UNSUPPORTED_CORE_BOUNDARY"))));
    drop(store);
    let read = Connection::open(&path).unwrap();
    let snapshot = |conn: &Connection| -> Vec<(String, Vec<u8>)> {
        let mut stmt = conn.prepare("SELECT 'control',CAST(quote(schema_digest)||quote(receipt_root)||authoritative_now_utc_ms AS BLOB) FROM core_boundary_control_v1 UNION ALL SELECT 'migration',CAST(version||quote(digest)||completed_at_ms AS BLOB) FROM schema_migrations ORDER BY 1,2").unwrap();
        stmt.query_map([],|r|Ok((r.get(0)?,r.get(1)?))).unwrap().collect::<Result<_,_>>().unwrap()
    };
    let before = snapshot(&read);
    let reopened = Store::open(&path).unwrap();
    assert_eq!(snapshot(&read), before);
    assert_eq!(reopened.count_journal().unwrap(), 0);
    drop(reopened);
    drop(read);
    std::fs::remove_file(path).unwrap();
}
