use ae_contracts::{
    wire, CanonicalEvent, CommitStatus, Digest, InvariantResiduals, ScopeRef, TimeAdvance,
    TransitionReceipt,
};
use ae_store::Store;
use rusqlite::{params, Connection, Transaction};

fn time_advance(event_id: u8, bot: u8, persona: u8) -> CanonicalEvent {
    CanonicalEvent::TimeAdvance(TimeAdvance {
        event_id: [event_id; 16],
        scope: ScopeRef {
            bot_token: [bot; 16],
            persona_token: [persona; 16],
            relation_token: None,
            session_token: [event_id; 16],
        },
        elapsed_ms: u64::from(event_id),
    })
}

#[derive(Clone, Copy)]
struct LegacyFixture {
    scope_a: Digest,
    scope_b: Digest,
    seed_a: Digest,
    seed_b: Digest,
    a1_event_digest: Digest,
    b1_event_digest: Digest,
    a2_event_digest: Digest,
    b2_event_digest: Digest,
    a1_revision: u64,
    b1_revision: u64,
    a2_revision: u64,
    b2_revision: u64,
}

fn create_legacy_journal_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS meta (
            key TEXT PRIMARY KEY,
            value BLOB NOT NULL
        );
        CREATE TABLE IF NOT EXISTS journal (
            revision INTEGER PRIMARY KEY AUTOINCREMENT,
            scope_digest BLOB NOT NULL,
            base_revision INTEGER NOT NULL,
            event_kind TEXT NOT NULL,
            event_bytes BLOB NOT NULL,
            event_digest BLOB NOT NULL,
            receipt_bytes BLOB NOT NULL,
            chain_digest BLOB NOT NULL,
            committed_at_ms INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS applied_events (
            scope_digest BLOB NOT NULL,
            event_digest BLOB NOT NULL,
            revision INTEGER NOT NULL,
            PRIMARY KEY (scope_digest, event_digest)
        );
        CREATE TABLE IF NOT EXISTS snapshots (
            revision INTEGER NOT NULL,
            scope_digest BLOB NOT NULL,
            state_digest BLOB NOT NULL,
            state_bytes BLOB NOT NULL,
            PRIMARY KEY (revision, scope_digest)
        );
        CREATE TABLE IF NOT EXISTS active_bindings (
            bot_token BLOB NOT NULL,
            persona_token BLOB NOT NULL,
            incarnation_id BLOB NOT NULL,
            revision INTEGER NOT NULL,
            PRIMARY KEY (bot_token, persona_token)
        );
        "#,
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO meta (key, value) VALUES ('schema_version', X'01')",
        [],
    )?;
    Ok(())
}

fn write_legacy_journal_row(
    tx: &Transaction<'_>,
    scope_digest: &Digest,
    event: &CanonicalEvent,
    base_revision: u64,
    next_revision: u64,
    chain_seed: &Digest,
    formula_seed: u8,
    authority_seed: u8,
    state_seed: u8,
) -> rusqlite::Result<(Digest, Digest, u64)> {
    let event_bytes = wire::encode_event(event);
    let event_digest = wire::event_digest(event);
    let receipt = TransitionReceipt {
        schema_version: 1,
        formula_digest: [formula_seed; 32],
        scope_digest: *scope_digest,
        event_digest,
        authority_digest: [authority_seed; 32],
        base_revision,
        next_revision,
        state_before: [state_seed; 32],
        state_after: [state_seed.saturating_add(1); 32],
        graph_after: [state_seed.saturating_add(2); 32],
        action_contract: None,
        active_nodes: 0,
        active_edges: 0,
        residuals: InvariantResiduals::default(),
        status: CommitStatus::Committed,
    };
    let receipt_bytes = wire::encode_transition_receipt(&receipt);
    let chain_digest = ae_continuum::chain_link(chain_seed, &event_bytes, &receipt_bytes);

    tx.execute(
        "INSERT INTO journal (scope_digest, base_revision, event_kind, event_bytes, event_digest, receipt_bytes, chain_digest, committed_at_ms) VALUES (?1, ?2, 'time_advance', ?3, ?4, ?5, ?6, ?7)",
        params![
            scope_digest.to_vec(),
            base_revision as i64,
            event_bytes,
            event_digest.to_vec(),
            receipt_bytes.clone(),
            chain_digest.to_vec(),
            1_i64
        ],
    )?;
    let revision = tx.last_insert_rowid() as u64;

    tx.execute(
        "INSERT INTO applied_events (scope_digest, event_digest, revision) VALUES (?1, ?2, ?3)",
        params![
            scope_digest.to_vec(),
            event_digest.to_vec(),
            revision as i64
        ],
    )?;
    Ok((event_digest, chain_digest, revision))
}

fn create_legacy_scope_fixture(path: &std::path::Path) -> rusqlite::Result<LegacyFixture> {
    let mut conn = Connection::open(path)?;
    create_legacy_journal_tables(&conn)?;

    let scope_a = wire::persona_scope_digest(&[1; 16], &[2; 16], None);
    let scope_b = wire::persona_scope_digest(&[3; 16], &[4; 16], None);
    let seed_a = [11; 32];
    let seed_b = [12; 32];

    let tx = conn.transaction()?;
    let (a1_event_digest, a1_chain, a1_revision) = write_legacy_journal_row(
        &tx,
        &scope_a,
        &time_advance(1, 1, 2),
        0,
        1,
        &seed_a,
        41,
        51,
        61,
    )?;
    let (b1_event_digest, b1_chain, b1_revision) = write_legacy_journal_row(
        &tx,
        &scope_b,
        &time_advance(2, 3, 4),
        0,
        1,
        &seed_b,
        42,
        52,
        62,
    )?;
    let (a2_event_digest, _a2_chain, a2_revision) = write_legacy_journal_row(
        &tx,
        &scope_a,
        &time_advance(3, 1, 2),
        1,
        2,
        &a1_chain,
        43,
        53,
        63,
    )?;
    let (b2_event_digest, _b2_chain, b2_revision) = write_legacy_journal_row(
        &tx,
        &scope_b,
        &time_advance(4, 3, 4),
        1,
        2,
        &b1_chain,
        44,
        54,
        64,
    )?;
    tx.commit()?;

    Ok(LegacyFixture {
        scope_a,
        scope_b,
        seed_a,
        seed_b,
        a1_event_digest,
        b1_event_digest,
        a2_event_digest,
        b2_event_digest,
        a1_revision,
        b1_revision,
        a2_revision,
        b2_revision,
    })
}

#[test]
fn legacy_scope_revision_migration_does_not_expose_global_rowids() {
    let path = std::env::temp_dir().join(format!(
        "ae-store-legacy-scope-migration-{}.db",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let fixture = create_legacy_scope_fixture(&path).unwrap();
    let store = Store::open(&path).unwrap();

    assert_eq!(store.count_journal().unwrap(), 4);
    assert_eq!(fixture.a1_revision, 1);
    assert_eq!(fixture.b1_revision, 2);
    assert_eq!(fixture.a2_revision, 3);
    assert_eq!(fixture.b2_revision, 4);

    let scope_a_journal = store.read_journal(&fixture.scope_a).unwrap();
    let scope_b_journal = store.read_journal(&fixture.scope_b).unwrap();
    assert_eq!(scope_a_journal.len(), 2);
    assert_eq!(scope_b_journal.len(), 2);

    let scope_a_receipt_a =
        wire::decode_transition_receipt(&scope_a_journal[0].receipt_bytes).unwrap();
    let scope_a_receipt_b =
        wire::decode_transition_receipt(&scope_a_journal[1].receipt_bytes).unwrap();
    let scope_b_receipt_a =
        wire::decode_transition_receipt(&scope_b_journal[0].receipt_bytes).unwrap();
    let scope_b_receipt_b =
        wire::decode_transition_receipt(&scope_b_journal[1].receipt_bytes).unwrap();

    assert_eq!(scope_a_receipt_a.base_revision, 0);
    assert_eq!(scope_a_receipt_a.next_revision, 1);
    assert_eq!(scope_a_receipt_b.base_revision, 1);
    assert_eq!(scope_a_receipt_b.next_revision, 2);
    assert_eq!(scope_b_receipt_a.base_revision, 0);
    assert_eq!(scope_b_receipt_a.next_revision, 1);
    assert_eq!(scope_b_receipt_b.base_revision, 1);
    assert_eq!(scope_b_receipt_b.next_revision, 2);

    assert_eq!(
        scope_a_journal[0].chain_digest,
        ae_continuum::chain_link(
            &fixture.seed_a,
            &scope_a_journal[0].event_bytes,
            &wire::encode_transition_receipt(&scope_a_receipt_a),
        )
    );
    assert_eq!(
        scope_a_journal[1].chain_digest,
        ae_continuum::chain_link(
            &scope_a_journal[0].chain_digest,
            &scope_a_journal[1].event_bytes,
            &wire::encode_transition_receipt(&scope_a_receipt_b),
        )
    );
    assert_eq!(
        scope_b_journal[0].chain_digest,
        ae_continuum::chain_link(
            &fixture.seed_b,
            &scope_b_journal[0].event_bytes,
            &wire::encode_transition_receipt(&scope_b_receipt_a),
        )
    );
    assert_eq!(
        scope_b_journal[1].chain_digest,
        ae_continuum::chain_link(
            &scope_b_journal[0].chain_digest,
            &scope_b_journal[1].event_bytes,
            &wire::encode_transition_receipt(&scope_b_receipt_b),
        )
    );

    let a1 = store
        .lookup_event(&fixture.scope_a, &fixture.a1_event_digest)
        .unwrap()
        .unwrap();
    let b1 = store
        .lookup_event(&fixture.scope_b, &fixture.b1_event_digest)
        .unwrap()
        .unwrap();
    let a2 = store
        .lookup_event(&fixture.scope_a, &fixture.a2_event_digest)
        .unwrap()
        .unwrap();
    let b2 = store
        .lookup_event(&fixture.scope_b, &fixture.b2_event_digest)
        .unwrap()
        .unwrap();
    assert_eq!(a1.revision, 1);
    assert_eq!(b1.revision, 1);
    assert_eq!(a2.revision, 2);
    assert_eq!(b2.revision, 2);

    assert_eq!(
        scope_a_journal
            .iter()
            .map(|row| row.revision)
            .collect::<Vec<_>>(),
        vec![1_u64, 2_u64]
    );
    assert_eq!(
        scope_b_journal
            .iter()
            .map(|row| row.revision)
            .collect::<Vec<_>>(),
        vec![1_u64, 2_u64]
    );
    assert_eq!(store.current_revision(&fixture.scope_a).unwrap(), 2);
    assert_eq!(store.current_revision(&fixture.scope_b).unwrap(), 2);
}
