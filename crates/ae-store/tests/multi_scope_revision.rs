use ae_continuum::CommitEnvelope;
use ae_contracts::{
    wire, CanonicalEvent, CommitStatus, InvariantResiduals, ScopeRef, TimeAdvance,
    TransitionReceipt,
};
use ae_store::Store;

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

fn envelope(
    event_id: u8,
    bot: u8,
    persona: u8,
    base_revision: u64,
    next_revision: u64,
    chain_seed: [u8; 32],
) -> CommitEnvelope {
    let event = time_advance(event_id, bot, persona);
    let event_bytes = wire::encode_event(&event);
    let scope_digest = wire::persona_scope_digest(&[bot; 16], &[persona; 16], None);

    CommitEnvelope {
        event_kind: "time_advance".to_owned(),
        event_bytes,
        receipt: TransitionReceipt {
            schema_version: 1,
            formula_digest: [41; 32],
            scope_digest,
            event_digest: wire::event_digest(&event),
            authority_digest: [42; 32],
            base_revision,
            next_revision,
            state_before: [43; 32],
            state_after: [44; 32],
            graph_after: [45; 32],
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

#[test]
fn multi_scope_revisions_are_independent_and_replayable() {
    let mut store = Store::open_in_memory().unwrap();
    let scope_a = wire::persona_scope_digest(&[1; 16], &[2; 16], None);
    let scope_b = wire::persona_scope_digest(&[3; 16], &[4; 16], None);
    let seed_a = [51; 32];
    let seed_b = [52; 32];

    let (a1_revision, a1_row) = store
        .commit_journal(&envelope(1, 1, 2, 0, 1, seed_a))
        .unwrap();
    let (b1_revision, b1_row) = store
        .commit_journal(&envelope(2, 3, 4, 0, 1, seed_b))
        .unwrap();
    let a2_result = store
        .commit_journal(&envelope(3, 1, 2, 1, 2, a1_row.chain_digest))
        .map(|(revision, _)| revision);
    let b2_result = store
        .commit_journal(&envelope(4, 3, 4, 1, 2, b1_row.chain_digest))
        .map(|(revision, _)| revision);

    assert_eq!(a1_revision, 1);
    assert_eq!(b1_revision, 1);
    let a2_revision = match a2_result {
        Ok(revision) => revision,
        Err(error) => panic!("scope A2 commit failed: {error}"),
    };
    let b2_revision = match b2_result {
        Ok(revision) => revision,
        Err(error) => panic!("scope B2 commit failed: {error}"),
    };
    assert_eq!(a2_revision, 2);
    assert_eq!(b2_revision, 2);
    assert_eq!(scope_a, a1_row.scope_digest);
    assert_eq!(scope_b, b1_row.scope_digest);

    assert_eq!(store.current_revision(&scope_a).unwrap(), 2);
    assert_eq!(store.current_revision(&scope_b).unwrap(), 2);

    let a_rows = store.read_journal(&scope_a).unwrap();
    let b_rows = store.read_journal(&scope_b).unwrap();
    assert_eq!(
        a_rows.iter().map(|row| row.revision).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(
        b_rows.iter().map(|row| row.revision).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(store.count_journal().unwrap(), 4);

    let replay_a = ae_continuum::verify_replay(seed_a, &a_rows);
    assert!(
        replay_a.ok,
        "scope A replay failed: {:?}",
        replay_a.first_error
    );
    let replay_b = ae_continuum::verify_replay(seed_b, &b_rows);
    assert!(
        replay_b.ok,
        "scope B replay failed: {:?}",
        replay_b.first_error
    );
}
