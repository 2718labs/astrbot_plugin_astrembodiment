use ae_continuum::CommitEnvelope;
use ae_contracts::{
    wire, CanonicalEvent, CommitStatus, InvariantResiduals, ScopeRef, TimeAdvance,
    TransitionReceipt,
};
use ae_store::{Store, StoreError};
use rusqlite::{params, Connection, Transaction};
use std::path::Path;

fn event(id: u8, bot: u8, persona: u8) -> CanonicalEvent {
    CanonicalEvent::TimeAdvance(TimeAdvance {
        event_id: [id; 16],
        scope: ScopeRef {
            bot_token: [bot; 16],
            persona_token: [persona; 16],
            relation_token: None,
            session_token: [id; 16],
        },
        elapsed_ms: u64::from(id),
    })
}

fn legacy_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE meta (key TEXT PRIMARY KEY, value BLOB NOT NULL);
        CREATE TABLE journal (
            revision INTEGER PRIMARY KEY AUTOINCREMENT,
            scope_digest BLOB NOT NULL, base_revision INTEGER NOT NULL,
            event_kind TEXT NOT NULL, event_bytes BLOB NOT NULL,
            event_digest BLOB NOT NULL, receipt_bytes BLOB NOT NULL,
            chain_digest BLOB NOT NULL, committed_at_ms INTEGER NOT NULL
        );
        CREATE TABLE applied_events (
            scope_digest BLOB NOT NULL, event_digest BLOB NOT NULL,
            revision INTEGER NOT NULL, PRIMARY KEY (scope_digest, event_digest)
        );
        CREATE TABLE snapshots (
            revision INTEGER NOT NULL, scope_digest BLOB NOT NULL,
            state_digest BLOB NOT NULL, state_bytes BLOB NOT NULL,
            PRIMARY KEY (revision, scope_digest)
        );
        CREATE TABLE active_bindings (
            bot_token BLOB NOT NULL, persona_token BLOB NOT NULL,
            incarnation_id BLOB NOT NULL, revision INTEGER NOT NULL,
            PRIMARY KEY (bot_token, persona_token)
        );
        INSERT INTO meta (key, value) VALUES ('schema_version', X'01');
        "#,
    )
}

fn legacy_row(
    tx: &Transaction<'_>,
    scope: &[u8; 32],
    ev: &CanonicalEvent,
    base: u64,
    next: u64,
    seed: &[u8; 32],
    marker: u8,
) -> rusqlite::Result<[u8; 32]> {
    let event_bytes = wire::encode_event(ev);
    let event_digest = wire::event_digest(ev);
    let receipt = TransitionReceipt {
        schema_version: 1,
        formula_digest: [marker; 32],
        scope_digest: *scope,
        event_digest,
        authority_digest: [marker.wrapping_add(10); 32],
        base_revision: base,
        next_revision: next,
        state_before: [marker.wrapping_add(20); 32],
        state_after: [marker.wrapping_add(21); 32],
        graph_after: [marker.wrapping_add(22); 32],
        action_contract: None,
        active_nodes: 0,
        active_edges: 0,
        residuals: InvariantResiduals::default(),
        status: CommitStatus::Committed,
    };
    let receipt_bytes = wire::encode_transition_receipt(&receipt);
    let chain = ae_continuum::chain_link(seed, &event_bytes, &receipt_bytes);
    tx.execute(
        "INSERT INTO journal (scope_digest, base_revision, event_kind, event_bytes, event_digest, receipt_bytes, chain_digest, committed_at_ms) VALUES (?1,?2,'time_advance',?3,?4,?5,?6,1)",
        params![scope.to_vec(), base as i64, event_bytes, event_digest.to_vec(), receipt_bytes, chain.to_vec()],
    )?;
    let physical = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO applied_events (scope_digest, event_digest, revision) VALUES (?1,?2,?3)",
        params![scope.to_vec(), event_digest.to_vec(), physical],
    )?;
    Ok(chain)
}

type RawJournalRow = (i64, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>);

fn raw_rows(path: &Path) -> Vec<RawJournalRow> {
    let conn = Connection::open(path).unwrap();
    let mut stmt = conn.prepare("SELECT revision,event_bytes,event_digest,receipt_bytes,chain_digest FROM journal ORDER BY revision").unwrap();
    stmt.query_map([], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
        ))
    })
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap()
}

fn envelope(
    ev: &CanonicalEvent,
    scope: [u8; 32],
    base: u64,
    next: u64,
    seed: [u8; 32],
    marker: u8,
) -> CommitEnvelope {
    let event_bytes = wire::encode_event(ev);
    let event_digest = wire::event_digest(ev);
    CommitEnvelope {
        event_kind: "time_advance".to_owned(),
        event_bytes,
        receipt: TransitionReceipt {
            schema_version: 1,
            formula_digest: [marker; 32],
            scope_digest: scope,
            event_digest,
            authority_digest: [marker.wrapping_add(1); 32],
            base_revision: base,
            next_revision: next,
            state_before: [marker.wrapping_add(2); 32],
            state_after: [marker.wrapping_add(3); 32],
            graph_after: [marker.wrapping_add(4); 32],
            action_contract: None,
            active_nodes: 0,
            active_edges: 0,
            residuals: InvariantResiduals::default(),
            status: CommitStatus::Committed,
        },
        chain_seed: seed,
        delta_bytes: Vec::new(),
    }
}

#[test]
fn legacy_scope_revision_migration_reopen_preserves_chains_and_replay_guards() {
    let path = std::env::temp_dir().join(format!("ae-store-reopen-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let scope_a = wire::persona_scope_digest(&[1; 16], &[2; 16], None);
    let scope_b = wire::persona_scope_digest(&[3; 16], &[4; 16], None);
    let seed_a = [11; 32];
    let seed_b = [12; 32];
    let conn = Connection::open(&path).unwrap();
    legacy_tables(&conn).unwrap();
    let tx = conn.unchecked_transaction().unwrap();
    let a1_chain = legacy_row(&tx, &scope_a, &event(1, 1, 2), 0, 1, &seed_a, 41).unwrap();
    let b1_chain = legacy_row(&tx, &scope_b, &event(2, 3, 4), 0, 1, &seed_b, 42).unwrap();
    let a2_chain = legacy_row(&tx, &scope_a, &event(3, 1, 2), 1, 2, &a1_chain, 43).unwrap();
    let b2_chain = legacy_row(&tx, &scope_b, &event(4, 3, 4), 1, 2, &b1_chain, 44).unwrap();
    tx.commit().unwrap();
    let before = raw_rows(&path);

    let store = Store::open(&path).unwrap();
    assert_eq!(store.current_revision(&scope_a).unwrap(), 2);
    assert_eq!(store.current_revision(&scope_b).unwrap(), 2);
    assert_eq!(
        store
            .read_journal(&scope_a)
            .unwrap()
            .iter()
            .map(|r| r.revision)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(
        store
            .read_journal(&scope_b)
            .unwrap()
            .iter()
            .map(|r| r.revision)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    drop(store);
    let marker = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT value FROM meta WHERE key='schema_version'",
            [],
            |r| r.get::<_, Vec<u8>>(0),
        )
        .unwrap();
    assert_eq!(marker, vec![1]);

    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.current_revision(&scope_a).unwrap(), 2);
    assert_eq!(store.current_revision(&scope_b).unwrap(), 2);
    let a3 = envelope(&event(5, 1, 2), scope_a, 2, 3, a2_chain, 51);
    let b3 = envelope(&event(6, 3, 4), scope_b, 2, 3, b2_chain, 52);
    let (_, _) = store.commit_journal(&a3).unwrap();
    let (_, b3_row) = store.commit_journal(&b3).unwrap();
    assert_eq!(store.current_revision(&scope_a).unwrap(), 3);
    assert_eq!(store.current_revision(&scope_b).unwrap(), 3);
    assert!(matches!(
        store.commit_journal(&a3).unwrap_err(),
        StoreError::StaleRevision {
            expected: 2,
            actual: 3
        }
    ));
    let mut duplicate = a3.clone();
    duplicate.receipt.base_revision = 3;
    duplicate.receipt.next_revision = 4;
    assert!(matches!(
        store.commit_journal(&duplicate).unwrap_err(),
        StoreError::DuplicateEvent(3)
    ));
    assert_eq!(store.current_revision(&scope_a).unwrap(), 3);

    let mut isolated = a3.clone();
    isolated.receipt.scope_digest = scope_b;
    isolated.receipt.base_revision = 3;
    isolated.receipt.next_revision = 4;
    isolated.chain_seed = b3_row.chain_digest;
    let (b4_revision, _) = store.commit_journal(&isolated).unwrap();
    assert_eq!(b4_revision, 4);
    assert_eq!(
        store
            .lookup_event(&scope_a, &a3.receipt.event_digest)
            .unwrap()
            .unwrap()
            .revision,
        3
    );
    assert_eq!(
        store
            .lookup_event(&scope_b, &a3.receipt.event_digest)
            .unwrap()
            .unwrap()
            .revision,
        4
    );
    let stale_replay = {
        let mut x = isolated.clone();
        x.receipt.base_revision = 3;
        x.receipt.next_revision = 4;
        x
    };
    assert!(matches!(
        store.commit_journal(&stale_replay).unwrap_err(),
        StoreError::StaleRevision {
            expected: 3,
            actual: 4
        }
    ));
    let a_rows = store.read_journal(&scope_a).unwrap();
    let b_rows = store.read_journal(&scope_b).unwrap();
    assert_eq!(
        a_rows.iter().map(|r| r.revision).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(
        b_rows.iter().map(|r| r.revision).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert!(ae_continuum::verify_replay(seed_a, &a_rows).ok);
    assert!(ae_continuum::verify_replay(seed_b, &b_rows).ok);
    drop(store);
    let after = raw_rows(&path);
    assert_eq!(&after[..4], &before[..4]);
    assert_eq!(after.len(), 7);
    let reopened = Store::open(&path).unwrap();
    assert_eq!(reopened.current_revision(&scope_a).unwrap(), 3);
    assert_eq!(reopened.current_revision(&scope_b).unwrap(), 4);
    assert_eq!(reopened.read_journal(&scope_a).unwrap().len(), 3);
    assert_eq!(reopened.read_journal(&scope_b).unwrap().len(), 4);
}
