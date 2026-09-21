use ae_store::{Store, StoreError};
use rusqlite::{params, Connection};

const ORIGIN_INDEX: &str = "applied_events_origin_lookup_v1";

fn database(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "ae-perception-origin-index-{label}-{}-{:?}.db",
        std::process::id(),
        std::thread::current().id()
    ))
}

#[test]
fn open_installs_origin_index_and_origin_lookup_uses_it() {
    let path = database("install-plan");
    let _ = std::fs::remove_file(&path);
    Store::open(&path).unwrap().close().unwrap();

    // Simulate a database created by the immediately preceding release.
    let connection = Connection::open(&path).unwrap();
    connection
        .execute(&format!("DROP INDEX {ORIGIN_INDEX}"), [])
        .unwrap();
    drop(connection);
    Store::open(&path).unwrap().close().unwrap();

    let mut connection = Connection::open(&path).unwrap();
    let columns = connection
        .prepare(&format!("PRAGMA index_info('{ORIGIN_INDEX}')"))
        .unwrap()
        .query_map([], |row| row.get::<_, String>(2))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(columns, ["event_digest", "scope_digest"]);

    let transaction = connection.transaction().unwrap();
    for ordinal in 0_u64..2_048 {
        let mut scope = [0_u8; 32];
        scope[..8].copy_from_slice(&ordinal.to_le_bytes());
        let mut event = [0_u8; 32];
        event[..8].copy_from_slice(&(ordinal / 2).to_le_bytes());
        transaction
            .execute(
                "INSERT INTO applied_events(scope_digest,event_digest,revision)
                 VALUES(?1,?2,?3)",
                params![scope.to_vec(), event.to_vec(), 1_i64],
            )
            .unwrap();
    }
    transaction.commit().unwrap();

    let detail = connection
        .prepare(
            "EXPLAIN QUERY PLAN
             SELECT scope_digest,revision FROM applied_events
             WHERE event_digest=?1 ORDER BY scope_digest LIMIT 2",
        )
        .unwrap()
        .query_map([vec![0_u8; 32]], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join(" | ");
    assert!(
        detail.contains(&format!("SEARCH applied_events USING INDEX {ORIGIN_INDEX}")),
        "unexpected origin lookup plan: {detail}"
    );
    assert!(!detail.contains("SCAN applied_events"), "{detail}");

    drop(connection);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn same_name_wrong_origin_index_fails_closed() {
    let path = database("wrong-index");
    let _ = std::fs::remove_file(&path);
    Store::open(&path).unwrap().close().unwrap();

    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(&format!(
            "DROP INDEX {ORIGIN_INDEX};
             CREATE INDEX {ORIGIN_INDEX}
             ON applied_events(scope_digest,event_digest);"
        ))
        .unwrap();
    drop(connection);

    assert!(matches!(
        Store::open(&path),
        Err(StoreError::ContinuityFence("applied_events_origin_index"))
    ));
    let _ = std::fs::remove_file(&path);
}
