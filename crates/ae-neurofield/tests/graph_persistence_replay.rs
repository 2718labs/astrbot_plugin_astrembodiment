use ae_neurofield::{
    bind_delta_to_graph_replay_rule, graph_digest, graph_replay_rule_descriptor,
    graph_replay_rule_digest, graph_replay_rule_digest_for_descriptor, EdgeOperationV1,
    GraphReplayError, GraphReplayV1, GraphSnapshotV1, SparseGraph, StructuralDeltaV1, Synapse,
    GRAPH_REPLAY_FORMULA_V1, NEURON_SLOTS,
};

fn edge(target: u32, weight: i16) -> Synapse {
    Synapse {
        target,
        weight,
        eligibility: 20,
        stability: 30,
        last_used_epoch: 40,
        operator_id: 0,
        delay_class: 0,
        flags: 0,
    }
}

fn graph_with_entries(entries: &[(u32, Synapse)]) -> SparseGraph {
    let mut rows = vec![Vec::new(); NEURON_SLOTS];
    for (source, value) in entries {
        rows[*source as usize].push(*value);
    }
    for row in &mut rows {
        row.sort_unstable_by_key(|value| value.target);
    }

    let mut graph = SparseGraph::empty();
    graph.row_offsets.clear();
    graph.row_offsets.push(0);
    for row in rows {
        graph.edges.extend(row);
        graph.row_offsets.push(graph.edges.len() as u32);
    }
    assert!(graph.validate());
    graph
}

fn delta(
    base_revision: u64,
    base_graph: &SparseGraph,
    delta_sequence: u64,
    operations: Vec<EdgeOperationV1>,
    after_graph: &SparseGraph,
) -> StructuralDeltaV1 {
    let mut delta = StructuralDeltaV1 {
        base_revision,
        base_graph_digest: graph_digest(base_graph),
        delta_sequence,
        rule_digest: Default::default(),
        operations,
        after_graph_digest: graph_digest(after_graph),
    };
    bind_delta_to_graph_replay_rule(GRAPH_REPLAY_FORMULA_V1, &mut delta).unwrap();
    delta
}

fn sealed_history() -> (GraphReplayV1, SparseGraph, SparseGraph) {
    let genesis = SparseGraph::empty();
    let after_add = graph_with_entries(&[(2, edge(3, 10))]);
    let final_graph = graph_with_entries(&[(2, edge(3, -99))]);
    let first = delta(
        10,
        &genesis,
        1,
        vec![EdgeOperationV1::Add {
            source: 2,
            edge: edge(3, 10),
        }],
        &after_add,
    );
    let second = delta(
        11,
        &after_add,
        2,
        vec![EdgeOperationV1::Update {
            source: 2,
            edge: edge(3, -99),
        }],
        &final_graph,
    );
    let anchor = GraphSnapshotV1::from_graph(GRAPH_REPLAY_FORMULA_V1, 10, &genesis).unwrap();
    let history = GraphReplayV1::seal(anchor, vec![first, second]).unwrap();
    (history, genesis, final_graph)
}

#[test]
fn production_rule_digest_is_derived_from_the_canonical_formula_descriptor() {
    let (history, _, _) = sealed_history();
    let descriptor = graph_replay_rule_descriptor(GRAPH_REPLAY_FORMULA_V1).unwrap();
    let rule_digest = graph_replay_rule_digest(GRAPH_REPLAY_FORMULA_V1).unwrap();

    assert!(descriptor.contains(&format!("formula_version={GRAPH_REPLAY_FORMULA_V1}")));
    assert!(descriptor.contains("delta_schema=StructuralDeltaV1"));
    assert!(descriptor.contains("apply_delta=v1-cas-canonical-operations"));
    assert_eq!(
        rule_digest,
        graph_replay_rule_digest_for_descriptor(GRAPH_REPLAY_FORMULA_V1, &descriptor).unwrap()
    );
    assert_ne!(
        rule_digest, [rule_digest[0]; 32],
        "a valid graph replay rule digest must derive from its rule contract"
    );
    assert_ne!(rule_digest, [0_u8; 32]);
    assert_ne!(rule_digest, [u8::MAX; 32]);
    assert!(history
        .deltas
        .iter()
        .all(|delta| delta.rule_digest == rule_digest));
}

#[test]
fn rule_contract_rejects_formula_or_descriptor_mismatch() {
    let descriptor = graph_replay_rule_descriptor(GRAPH_REPLAY_FORMULA_V1).unwrap();

    assert_eq!(
        graph_replay_rule_descriptor(GRAPH_REPLAY_FORMULA_V1 + 1).unwrap_err(),
        GraphReplayError::UnsupportedFormulaVersion
    );

    let mut tampered_descriptor = descriptor;
    tampered_descriptor.push('!');
    assert_eq!(
        graph_replay_rule_digest_for_descriptor(GRAPH_REPLAY_FORMULA_V1, &tampered_descriptor,)
            .unwrap_err(),
        GraphReplayError::RuleDescriptorMismatch
    );
}

#[test]
fn production_bound_deltas_replay_under_the_same_rule_contract() {
    let (history, _, expected) = sealed_history();
    let production_digest = graph_replay_rule_digest(GRAPH_REPLAY_FORMULA_V1).unwrap();

    assert!(history
        .deltas
        .iter()
        .all(|delta| delta.rule_digest == production_digest));
    assert_eq!(
        history.reopen().unwrap().1.canonical_bytes(),
        expected.canonical_bytes()
    );
}

#[test]
fn close_reopen_replays_to_the_authoritative_non_genesis_graph() {
    let (history, genesis, expected) = sealed_history();
    let closed = history.clone();
    drop(history);

    let (revision, reopened) = closed.reopen().unwrap();
    assert_eq!(revision, 12);
    assert_eq!(reopened.canonical_bytes(), expected.canonical_bytes());
    assert_ne!(reopened.canonical_bytes(), genesis.canonical_bytes());
    assert_eq!(graph_digest(&reopened), closed.authoritative.graph_digest);
}

#[test]
fn replay_rejects_deletion_or_tampering_of_every_persisted_delta() {
    let (history, _, _) = sealed_history();

    for index in 0..history.deltas.len() {
        let mut removed = history.clone();
        removed.deltas.remove(index);
        assert!(
            removed.reopen().is_err(),
            "deleting delta {index} must fail closed"
        );

        let mut tampered = history.clone();
        tampered.deltas[index].after_graph_digest[0] ^= 0x80;
        assert_eq!(
            tampered.reopen().unwrap_err(),
            GraphReplayError::AfterDigestMismatch,
            "tampering delta {index} must be detected"
        );
    }
}

#[test]
fn replay_rejects_rule_digest_only_tampering() {
    let (history, _, _) = sealed_history();
    let mut tampered = history.clone();
    tampered.deltas[0].rule_digest[0] ^= 0x80;

    assert_eq!(tampered.anchor, history.anchor);
    assert_eq!(tampered.authoritative, history.authoritative);
    assert_eq!(
        tampered.deltas[0].base_revision,
        history.deltas[0].base_revision
    );
    assert_eq!(
        tampered.deltas[0].base_graph_digest,
        history.deltas[0].base_graph_digest
    );
    assert_eq!(
        tampered.deltas[0].delta_sequence,
        history.deltas[0].delta_sequence
    );
    assert_eq!(tampered.deltas[0].operations, history.deltas[0].operations);
    assert_eq!(
        tampered.deltas[0].after_graph_digest,
        history.deltas[0].after_graph_digest
    );
    assert_eq!(tampered.deltas[1..], history.deltas[1..]);
    assert!(
        matches!(tampered.reopen(), Err(GraphReplayError::DeltaRejected)),
        "changing only a persisted rule digest must fail closed"
    );
}

#[test]
fn replay_rejects_formula_revision_digest_and_noncanonical_snapshot_mismatches() {
    let (history, _, _) = sealed_history();

    let mut wrong_formula = history.anchor.clone();
    wrong_formula.formula_version = GRAPH_REPLAY_FORMULA_V1 + 1;
    assert_eq!(
        wrong_formula.restore().unwrap_err(),
        GraphReplayError::UnsupportedFormulaVersion
    );

    let mut wrong_digest = history.anchor.clone();
    wrong_digest.graph_digest[0] ^= 0x01;
    assert_eq!(
        wrong_digest.restore().unwrap_err(),
        GraphReplayError::SnapshotDigestMismatch
    );

    let mut noncanonical = history.anchor.clone();
    noncanonical.canonical_bytes.push(0);
    assert_eq!(
        noncanonical.restore().unwrap_err(),
        GraphReplayError::CanonicalEncodingMismatch
    );

    let mut discontinuous = history.clone();
    discontinuous.deltas[1].base_revision = 99;
    assert_eq!(
        discontinuous.reopen().unwrap_err(),
        GraphReplayError::RevisionDiscontinuity
    );
}

#[test]
fn replay_fails_closed_on_revision_overflow() {
    let base = SparseGraph::empty();
    let after = graph_with_entries(&[(2, edge(3, 10))]);
    let overflowing_delta = delta(
        u64::MAX,
        &base,
        1,
        vec![EdgeOperationV1::Add {
            source: 2,
            edge: edge(3, 10),
        }],
        &after,
    );
    let anchor = GraphSnapshotV1::from_graph(GRAPH_REPLAY_FORMULA_V1, u64::MAX, &base).unwrap();

    assert_eq!(
        GraphReplayV1::seal(anchor, vec![overflowing_delta]).unwrap_err(),
        GraphReplayError::RevisionDiscontinuity
    );
}

#[test]
fn fixed_binary_vector_is_a_stable_v1_canonical_snapshot_payload() {
    let vector = include_bytes!("vectors/graph-replay-v1.bin");
    let empty = SparseGraph::empty();
    assert_eq!(vector.as_slice(), empty.canonical_bytes().as_slice());

    let snapshot = GraphSnapshotV1 {
        formula_version: GRAPH_REPLAY_FORMULA_V1,
        revision: 0,
        graph_digest: graph_digest(&empty),
        canonical_bytes: vector.to_vec(),
    };
    let restored = snapshot.restore().unwrap();
    assert_eq!(restored.canonical_bytes(), vector.as_slice());
    assert_eq!(graph_digest(&restored), snapshot.graph_digest);
}
