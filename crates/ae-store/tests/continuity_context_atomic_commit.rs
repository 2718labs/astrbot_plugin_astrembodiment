use ae_continuum::{CommitEnvelope, JournalRow};
use ae_contracts::{
    wire, CanonicalEvent, CommitStatus, Digest, InvariantResiduals, ScopeRef, TimeAdvance,
    TransitionReceipt,
};
use ae_store::{
    continuity_context_digest, ContextCommitV1, ContinuityCommitBundleV1, GraphCommitV1,
    SnapshotCommitV1, Store,
};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const FORMULA_DIGEST: Digest = [0x71; 32];
type BundleMutation = Box<dyn Fn(&mut ContinuityCommitBundleV1)>;
type FenceCase = (&'static str, BundleMutation);

#[derive(Clone)]
struct FirstCommit {
    state_digest: Digest,
    graph_digest: Digest,
    row: JournalRow,
}

fn fixture_path(name: &str) -> PathBuf {
    let root = std::env::var_os("AE_STORE_TEST_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let dir = root.join(format!(
        "ae-store-continuity-context-atomic-{name}-{}-{sequence}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("authority.sqlite")
}

fn scope() -> ScopeRef {
    ScopeRef {
        bot_token: [0x11; 16],
        persona_token: [0x22; 16],
        relation_token: Some([0x33; 16]),
        session_token: [0x44; 16],
    }
}

fn scope_digest() -> Digest {
    let scope = scope();
    wire::persona_scope_digest(
        &scope.bot_token,
        &scope.persona_token,
        scope.relation_token.as_ref(),
    )
}

fn event(revision: u64) -> CanonicalEvent {
    CanonicalEvent::TimeAdvance(TimeAdvance {
        event_id: [revision as u8; 16],
        scope: scope(),
        elapsed_ms: revision,
    })
}

fn bundle(
    revision: u64,
    state_before: Digest,
    base_graph_digest: Digest,
    chain_seed: Digest,
) -> ContinuityCommitBundleV1 {
    let event = event(revision);
    let event_bytes = wire::encode_event(&event);
    let event_digest = wire::event_digest(&event);
    let state_digest = [0x80_u8.wrapping_add(revision as u8); 32];
    let graph_digest = [0x90_u8.wrapping_add(revision as u8); 32];
    let delta_bytes = vec![0xD0, revision as u8];
    let replay_state_bytes = vec![0xE0, revision as u8];
    let canonical_state_bytes = vec![0xC0, revision as u8, 0x01];
    let relation_scope_token = scope().relation_token.unwrap();

    ContinuityCommitBundleV1 {
        envelope: CommitEnvelope {
            event_kind: "time_advance".to_string(),
            event_bytes,
            receipt: TransitionReceipt {
                schema_version: 1,
                formula_digest: FORMULA_DIGEST,
                scope_digest: scope_digest(),
                event_digest,
                authority_digest: [0x55; 32],
                base_revision: revision - 1,
                next_revision: revision,
                state_before,
                state_after: state_digest,
                graph_after: graph_digest,
                action_contract: None,
                active_nodes: 16_384,
                active_edges: 0,
                residuals: InvariantResiduals::default(),
                status: CommitStatus::Committed,
            },
            chain_seed,
            delta_bytes: delta_bytes.clone(),
        },
        snapshot: SnapshotCommitV1 {
            state_digest,
            state_bytes: vec![0xB0, revision as u8],
        },
        graph: GraphCommitV1 {
            base_graph_digest,
            graph_digest,
            formula_digest: FORMULA_DIGEST,
            delta_bytes,
            replay_state_bytes,
        },
        context: ContextCommitV1 {
            relation_scope_token,
            relation_hmac: [0xA0; 32],
            source_continuum_revision: revision,
            context_digest: continuity_context_digest(&canonical_state_bytes),
            canonical_state_bytes,
        },
    }
}

fn seed_first_commit(name: &str) -> (PathBuf, Store, FirstCommit) {
    let path = fixture_path(name);
    let mut store = Store::open(&path).unwrap();
    let first = bundle(1, [0; 32], [0; 32], [0x66; 32]);
    let state_digest = first.snapshot.state_digest;
    let graph_digest = first.graph.graph_digest;
    let committed = store.commit_continuity_bundle(&first).unwrap();
    let row = committed.row().clone();
    assert_eq!(revision_counts(&path, 1), [1, 1, 1, 1, 1]);
    (
        path,
        store,
        FirstCommit {
            state_digest,
            graph_digest,
            row,
        },
    )
}

fn next_bundle(first: &FirstCommit) -> ContinuityCommitBundleV1 {
    bundle(
        2,
        first.state_digest,
        first.graph_digest,
        first.row.chain_digest,
    )
}

fn revision_counts(path: &Path, revision: u64) -> [u64; 5] {
    let conn = Connection::open(path).unwrap();
    let scope = scope_digest();
    let queries = [
        "SELECT COUNT(*) FROM journal WHERE scope_digest = ?1 AND logical_revision = ?2",
        "SELECT COUNT(*) FROM applied_events WHERE scope_digest = ?1 AND revision = ?2",
        "SELECT COUNT(*) FROM snapshots WHERE scope_digest = ?1 AND revision = ?2",
        "SELECT COUNT(*) FROM graph_commits WHERE scope_digest = ?1 AND revision = ?2",
        "SELECT COUNT(*) FROM context_commits WHERE scope_digest = ?1 AND revision = ?2",
    ];
    let mut counts = [0; 5];
    for (index, query) in queries.iter().enumerate() {
        counts[index] = conn
            .query_row(query, params![scope.to_vec(), revision as i64], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap() as u64;
    }
    counts
}

fn assert_old_authority_survives(path: &Path, store: &Store, first: &FirstCommit) {
    assert_eq!(revision_counts(path, 1), [1, 1, 1, 1, 1]);
    assert_eq!(revision_counts(path, 2), [0, 0, 0, 0, 0]);
    assert_eq!(store.current_revision(&scope_digest()).unwrap(), 1);
    assert_eq!(
        store
            .read_snapshot(&scope_digest(), 1)
            .unwrap()
            .unwrap()
            .state_digest,
        first.state_digest
    );
    let context = store
        .read_context_commit(&scope_digest(), &scope().relation_token.unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(context.revision, 1);
    assert_eq!(
        context.context_digest,
        continuity_context_digest(&context.canonical_state_bytes)
    );
}

#[test]
fn atomic_bundle_commits_journal_snapshot_graph_context_and_receipt_together() {
    let (path, mut store, first) = seed_first_commit("success");
    let first_row = first.row.clone();

    assert_eq!(first_row.revision, 1);
    assert_eq!(store.current_revision(&scope_digest()).unwrap(), 1);
    assert_old_authority_survives(&path, &store, &first);

    let context = store
        .read_context_commit(&scope_digest(), &scope().relation_token.unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(context.relation_scope_token, [0x33; 16]);
    assert_eq!(context.relation_hmac, [0xA0; 32]);
    assert_eq!(context.revision, 1);
    assert_eq!(context.canonical_state_bytes, vec![0xC0, 1, 0x01]);

    let second = next_bundle(&first);
    let committed = store.commit_continuity_bundle(&second).unwrap();
    assert_eq!(committed.revision(), 2);
    assert_eq!(committed.row().revision, 2);
    assert_eq!(revision_counts(&path, 2), [1, 1, 1, 1, 1]);

    drop(store);
    let reopened = Store::open(&path).unwrap();
    assert_eq!(reopened.current_revision(&scope_digest()).unwrap(), 2);
    assert_eq!(
        reopened
            .read_snapshot(&scope_digest(), 2)
            .unwrap()
            .unwrap()
            .state_digest,
        second.snapshot.state_digest
    );
    let reopened_context = reopened
        .read_context_commit(&scope_digest(), &scope().relation_token.unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(reopened_context.revision, 2);
    assert_eq!(
        reopened_context.context_digest,
        continuity_context_digest(&reopened_context.canonical_state_bytes)
    );
}

#[test]
fn duplicate_bundle_returns_the_original_receipt_without_a_second_write() {
    let (path, mut store, first) = seed_first_commit("dedup");
    let replay = bundle(1, [0; 32], [0; 32], [0x66; 32]);

    let replayed = store.commit_continuity_bundle(&replay).unwrap();
    assert!(matches!(
        replayed,
        ae_store::ContinuityCommitOutcomeV1::ExistingIdentical { .. }
    ));
    assert_eq!(replayed.revision(), 1);
    assert_eq!(replayed.row(), &first.row);
    assert_eq!(revision_counts(&path, 1), [1, 1, 1, 1, 1]);
}

#[test]
fn rejected_fences_leave_all_five_domains_at_the_old_revision() {
    let cases: Vec<FenceCase> = vec![
        (
            "stale-base",
            Box::new(|bundle| bundle.envelope.receipt.base_revision = 9),
        ),
        (
            "wrong-next",
            Box::new(|bundle| bundle.envelope.receipt.next_revision = 3),
        ),
        (
            "context-revision",
            Box::new(|bundle| bundle.context.source_continuum_revision = 3),
        ),
        (
            "graph-base",
            Box::new(|bundle| bundle.graph.base_graph_digest = [0xFF; 32]),
        ),
        (
            "graph-formula",
            Box::new(|bundle| bundle.graph.formula_digest = [0xEE; 32]),
        ),
        (
            "context-digest",
            Box::new(|bundle| bundle.context.context_digest = [0xDD; 32]),
        ),
        (
            "scope",
            Box::new(|bundle| bundle.envelope.receipt.scope_digest = [0xCC; 32]),
        ),
    ];

    for (name, mutate) in cases {
        let (path, mut store, first) = seed_first_commit(name);
        let mut candidate = next_bundle(&first);
        mutate(&mut candidate);
        assert!(
            store.commit_continuity_bundle(&candidate).is_err(),
            "{name}"
        );
        assert_old_authority_survives(&path, &store, &first);
    }
}

fn assert_fault_rolls_back(name: &str, fault_sql: &str) {
    let (path, mut store, first) = seed_first_commit(name);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(fault_sql).unwrap();

    let candidate = next_bundle(&first);
    assert!(
        store.commit_continuity_bundle(&candidate).is_err(),
        "fault {name} unexpectedly committed"
    );
    assert_old_authority_survives(&path, &store, &first);

    drop(store);
    let reopened = Store::open(&path).unwrap();
    assert_eq!(
        reopened.current_revision(&scope_digest()).unwrap(),
        1,
        "{name}"
    );
    assert_eq!(revision_counts(&path, 2), [0, 0, 0, 0, 0], "{name}");
}

#[test]
fn every_precommit_fault_point_rolls_back_journal_snapshot_graph_and_context_together() {
    let faults = [
        (
            "before-journal",
            "CREATE TRIGGER fault_before_journal BEFORE INSERT ON journal BEGIN SELECT RAISE(ABORT, 'before_journal'); END;",
        ),
        (
            "after-journal",
            "CREATE TRIGGER fault_after_journal AFTER INSERT ON journal BEGIN SELECT RAISE(ABORT, 'after_journal'); END;",
        ),
        (
            "after-applied-event",
            "CREATE TRIGGER fault_after_applied AFTER INSERT ON applied_events BEGIN SELECT RAISE(ABORT, 'after_applied'); END;",
        ),
        (
            "after-snapshot",
            "CREATE TRIGGER fault_after_snapshot AFTER INSERT ON snapshots WHEN NEW.revision = 2 BEGIN SELECT RAISE(ABORT, 'after_snapshot'); END;",
        ),
        (
            "after-graph",
            "CREATE TRIGGER fault_after_graph AFTER INSERT ON graph_commits BEGIN SELECT RAISE(ABORT, 'after_graph'); END;",
        ),
        (
            "after-context",
            "CREATE TRIGGER fault_after_context AFTER INSERT ON context_commits BEGIN SELECT RAISE(ABORT, 'after_context'); END;",
        ),
        (
            "before-sqlite-commit",
            "CREATE TABLE fault_parent (id INTEGER PRIMARY KEY); CREATE TABLE fault_child (parent_id INTEGER REFERENCES fault_parent(id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER fault_before_commit AFTER INSERT ON context_commits BEGIN INSERT INTO fault_child (parent_id) VALUES (1); END;",
        ),
    ];

    for (name, sql) in faults {
        assert_fault_rolls_back(name, sql);
    }
}

#[test]
fn unknown_commit_result_is_resolved_by_reopen_and_idempotent_replay() {
    let (path, mut store, first) = seed_first_commit("unknown-result");
    let candidate = next_bundle(&first);
    assert!(store.commit_continuity_bundle(&candidate).is_ok());
    drop(store);

    let mut reopened = Store::open(&path).unwrap();
    let replayed = reopened.commit_continuity_bundle(&candidate).unwrap();
    assert!(matches!(
        replayed,
        ae_store::ContinuityCommitOutcomeV1::ExistingIdentical { .. }
    ));
    assert_eq!(replayed.revision(), 2);
    assert_eq!(
        replayed.row().decode_receipt().unwrap(),
        candidate.envelope.receipt
    );
    assert_eq!(revision_counts(&path, 2), [1, 1, 1, 1, 1]);
}

#[test]
fn migration_is_idempotent_and_context_bytes_have_no_forbidden_raw_sentinels() {
    let (path, store, _) = seed_first_commit("migration-privacy");
    drop(store);
    let reopened = Store::open(&path).unwrap();
    drop(reopened);

    let conn = Connection::open(&path).unwrap();
    let tables: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('graph_commits', 'context_commits')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tables, 2);
    let bytes: Vec<u8> = conn
        .query_row(
            "SELECT canonical_state_bytes FROM context_commits WHERE scope_digest = ?1 AND relation_scope_token = ?2 AND revision = 1",
            params![scope_digest().to_vec(), [0x33_u8; 16].to_vec()],
            |row| row.get(0),
        )
        .unwrap();
    for forbidden in [
        b"RAW_CONTENT_SENTINEL".as_slice(),
        b"platform-id".as_slice(),
        b"provider-response".as_slice(),
        b"C:\\\\".as_slice(),
    ] {
        assert!(
            !bytes
                .windows(forbidden.len())
                .any(|window| window == forbidden),
            "forbidden raw sentinel persisted"
        );
    }
}
