use ae_continuum::CommitEnvelope;
use ae_contracts::{
    wire, CanonicalEvent, CommitStatus, Digest, InvariantResiduals, ScopeRef, TimeAdvance,
    TransitionReceipt,
};
use ae_store::{StatefulCommit, Store};
use rusqlite::{params, Connection};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT_DATABASE: AtomicUsize = AtomicUsize::new(0);

fn scope_digest() -> Digest {
    wire::persona_scope_digest(&[7; 16], &[8; 16], None)
}

fn envelope(
    event_id: u8,
    base_revision: u64,
    next_revision: u64,
    chain_seed: Digest,
    state_before: Digest,
    state_after: Digest,
) -> CommitEnvelope {
    let event = CanonicalEvent::TimeAdvance(TimeAdvance {
        event_id: [event_id; 16],
        scope: ScopeRef {
            bot_token: [7; 16],
            persona_token: [8; 16],
            relation_token: None,
            session_token: [event_id; 16],
        },
        elapsed_ms: u64::from(event_id),
    });
    let event_bytes = wire::encode_event(&event);

    CommitEnvelope {
        event_kind: "time_advance".to_owned(),
        event_bytes,
        receipt: TransitionReceipt {
            schema_version: 1,
            formula_digest: [31; 32],
            scope_digest: scope_digest(),
            event_digest: wire::event_digest(&event),
            authority_digest: [32; 32],
            base_revision,
            next_revision,
            state_before,
            state_after,
            graph_after: [33; 32],
            action_contract: None,
            active_nodes: 0,
            active_edges: 0,
            residuals: InvariantResiduals::default(),
            status: CommitStatus::Committed,
        },
        chain_seed,
        delta_bytes: Vec::new(),
    }
}

fn test_database_path(label: &str) -> PathBuf {
    let task_root = PathBuf::from(
        std::env::var_os("CODEX_TASK_TEMP").expect("CODEX_TASK_TEMP must be set for tests"),
    );
    let directory = task_root
        .join("test-databases")
        .join("ae-store-durable-state-commit");
    std::fs::create_dir_all(&directory).unwrap();
    directory.join(format!(
        "{label}-{}-{}.sqlite",
        std::process::id(),
        NEXT_DATABASE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn table_counts(connection: &Connection) -> (u64, u64, u64) {
    connection
        .query_row(
            "SELECT \
                (SELECT COUNT(*) FROM journal), \
                (SELECT COUNT(*) FROM applied_events), \
                (SELECT COUNT(*) FROM snapshots)",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? as u64,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, i64>(2)? as u64,
                ))
            },
        )
        .unwrap()
}

fn assert_unchanged(
    store: &Store,
    raw_connection: &Connection,
    scope: &Digest,
    expected_counts: (u64, u64, u64),
    expected_revision: u64,
) {
    assert_eq!(table_counts(raw_connection), expected_counts);
    assert_eq!(store.current_revision(scope).unwrap(), expected_revision);
}

#[test]
fn stateful_commit_inserts_journal_event_and_snapshot_together() {
    let mut store = Store::open_in_memory().unwrap();
    let state_bytes = vec![1, 2, 3, 4];
    let journal = envelope(1, 0, 1, [41; 32], [42; 32], [43; 32]);
    let commit = StatefulCommit {
        journal: journal.clone(),
        state_bytes: state_bytes.clone(),
    };

    let (revision, row) = store.commit_stateful_journal(&commit).unwrap();
    let applied = store
        .lookup_event(&journal.receipt.scope_digest, &journal.receipt.event_digest)
        .unwrap()
        .unwrap();
    let snapshot = store
        .read_snapshot(&journal.receipt.scope_digest, revision)
        .unwrap()
        .unwrap();

    assert_eq!(revision, 1);
    assert_eq!(row.revision, revision);
    assert_eq!(applied.revision, revision);
    assert_eq!(snapshot.revision, revision);
    assert_eq!(snapshot.scope_digest, journal.receipt.scope_digest);
    assert_eq!(snapshot.state_digest, journal.receipt.state_after);
    assert_eq!(snapshot.state_bytes, state_bytes);
    assert_eq!(store.count_journal().unwrap(), 1);
    assert_eq!(
        store
            .current_revision(&journal.receipt.scope_digest)
            .unwrap(),
        1
    );
}

#[test]
fn stateful_commit_rolls_back_when_each_insert_statement_is_aborted() {
    let path = test_database_path("statement-abort");
    let mut store = Store::open(&path).unwrap();
    let raw_connection = Connection::open(&path).unwrap();
    raw_connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let scope = scope_digest();
    let mut chain_seed = [51; 32];

    for (index, (table, trigger)) in [
        ("journal", "abort_journal_insert"),
        ("applied_events", "abort_applied_event_insert"),
        ("snapshots", "abort_snapshot_insert"),
    ]
    .iter()
    .enumerate()
    {
        let current_revision = store.current_revision(&scope).unwrap();
        let journal = envelope(
            (index + 1) as u8,
            current_revision,
            current_revision + 1,
            chain_seed,
            [61 + index as u8; 32],
            [71 + index as u8; 32],
        );
        let commit = StatefulCommit {
            journal,
            state_bytes: vec![81 + index as u8],
        };
        let before_counts = table_counts(&raw_connection);

        raw_connection
            .execute_batch(&format!(
                "CREATE TRIGGER {trigger} BEFORE INSERT ON {table} BEGIN SELECT RAISE(ABORT, 'forced insert abort'); END;"
            ))
            .unwrap();
        assert!(store.commit_stateful_journal(&commit).is_err());
        assert_unchanged(
            &store,
            &raw_connection,
            &scope,
            before_counts,
            current_revision,
        );

        raw_connection
            .execute_batch(&format!("DROP TRIGGER {trigger};"))
            .unwrap();
        let (revision, row) = store.commit_stateful_journal(&commit).unwrap();
        assert_eq!(revision, current_revision + 1);
        assert_eq!(
            table_counts(&raw_connection),
            (
                before_counts.0 + 1,
                before_counts.1 + 1,
                before_counts.2 + 1
            )
        );
        chain_seed = row.chain_digest;
    }

    let current_revision = store.current_revision(&scope).unwrap();
    let empty_bytes = StatefulCommit {
        journal: envelope(
            41,
            current_revision,
            current_revision + 1,
            chain_seed,
            [91; 32],
            [92; 32],
        ),
        state_bytes: Vec::new(),
    };
    let before_empty = table_counts(&raw_connection);
    assert!(store.commit_stateful_journal(&empty_bytes).is_err());
    assert_unchanged(
        &store,
        &raw_connection,
        &scope,
        before_empty,
        current_revision,
    );

    let conflict = StatefulCommit {
        journal: envelope(
            42,
            current_revision,
            current_revision + 1,
            chain_seed,
            [93; 32],
            [94; 32],
        ),
        state_bytes: vec![95],
    };
    raw_connection
        .execute(
            "INSERT INTO snapshots (revision, scope_digest, state_digest, state_bytes) VALUES (?1, ?2, ?3, ?4)",
            params![
                (current_revision + 1) as i64,
                scope.to_vec(),
                vec![96u8; 32],
                vec![97u8]
            ],
        )
        .unwrap();
    let before_conflict = table_counts(&raw_connection);
    assert!(store.commit_stateful_journal(&conflict).is_err());
    assert_unchanged(
        &store,
        &raw_connection,
        &scope,
        before_conflict,
        current_revision,
    );
}

#[test]
fn latest_snapshot_survives_a_later_noop_journal_revision() {
    let mut store = Store::open_in_memory().unwrap();
    let first = StatefulCommit {
        journal: envelope(1, 0, 1, [101; 32], [102; 32], [103; 32]),
        state_bytes: vec![104, 105],
    };
    let (first_revision, first_row) = store.commit_stateful_journal(&first).unwrap();
    let noop = envelope(
        2,
        1,
        2,
        first_row.chain_digest,
        first.journal.receipt.state_after,
        first.journal.receipt.state_after,
    );
    let (noop_revision, _) = store.commit_journal(&noop).unwrap();

    assert_eq!(first_revision, 1);
    assert_eq!(noop_revision, 2);
    assert!(store
        .read_snapshot(&first.journal.receipt.scope_digest, noop_revision)
        .unwrap()
        .is_none());
    let latest = store
        .read_latest_snapshot(&first.journal.receipt.scope_digest, noop_revision)
        .unwrap()
        .unwrap();
    assert_eq!(latest.revision, first_revision);
    assert_eq!(latest.state_digest, first.journal.receipt.state_after);
    assert_eq!(latest.state_bytes, first.state_bytes);
    assert!(store
        .read_latest_snapshot(&first.journal.receipt.scope_digest, 0)
        .unwrap()
        .is_none());
}
