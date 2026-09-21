use ae_neurofield::{
    apply_delta, bind_delta_to_graph_replay_rule, graph_digest, DeltaError, EdgeOperationV1,
    GraphReplayError, GraphReplayV1, GraphSnapshotV1, SparseGraph, StructuralDeltaV1, Synapse,
    EDGE_CAPACITY, GRAPH_REPLAY_FORMULA_V1, MAX_OPERATIONS_PER_DELTA_V1, MAX_REPLAY_DELTAS_V1,
    NEURON_SLOTS,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

fn edge(target: u32) -> Synapse {
    Synapse {
        target,
        weight: 10,
        eligibility: 20,
        stability: 30,
        last_used_epoch: 40,
        operator_id: 0,
        delay_class: 0,
        flags: 0,
    }
}

fn empty_snapshot() -> GraphSnapshotV1 {
    GraphSnapshotV1::from_graph(GRAPH_REPLAY_FORMULA_V1, 0, &SparseGraph::empty()).unwrap()
}

fn delta(base: &SparseGraph, operations: Vec<EdgeOperationV1>) -> StructuralDeltaV1 {
    let mut delta = StructuralDeltaV1 {
        base_revision: 7,
        base_graph_digest: graph_digest(base),
        delta_sequence: 1,
        rule_digest: [0; 32],
        operations,
        after_graph_digest: graph_digest(base),
    };
    bind_delta_to_graph_replay_rule(GRAPH_REPLAY_FORMULA_V1, &mut delta).unwrap();
    delta
}

fn assert_json_rejected<T: DeserializeOwned>(value: Value) {
    assert!(serde_json::from_value::<T>(value).is_err());
}

#[test]
fn graph_replay_resource_limit_and_persisted_schema_are_closed() {
    assert_eq!(MAX_REPLAY_DELTAS_V1, 64);
    assert_eq!(MAX_REPLAY_DELTAS_V1 * MAX_OPERATIONS_PER_DELTA_V1, 262_144);

    let anchor = empty_snapshot();
    let empty_delta = delta(&SparseGraph::empty(), Vec::new());
    assert_eq!(
        GraphReplayV1::seal(
            anchor.clone(),
            vec![empty_delta.clone(); MAX_REPLAY_DELTAS_V1 + 1],
        )
        .unwrap_err(),
        GraphReplayError::TooManyDeltas
    );

    let binary_golden = include_bytes!("vectors/graph-replay-v1.bin");
    assert_eq!(
        binary_golden.as_slice(),
        SparseGraph::empty().canonical_bytes()
    );
    let replay = GraphReplayV1::seal(anchor.clone(), Vec::new()).unwrap();

    let replay_value = serde_json::to_value(&replay).unwrap();
    let mut unknown_replay = replay_value.clone();
    unknown_replay
        .as_object_mut()
        .unwrap()
        .insert("future".into(), json!(true));
    assert_json_rejected::<GraphReplayV1>(unknown_replay);

    for snapshot_field in ["anchor", "authoritative"] {
        let mut unknown_snapshot = replay_value.clone();
        unknown_snapshot[snapshot_field]
            .as_object_mut()
            .unwrap()
            .insert("future".into(), json!(true));
        assert_json_rejected::<GraphReplayV1>(unknown_snapshot);
    }

    let mut unknown_standalone_snapshot = serde_json::to_value(&anchor).unwrap();
    unknown_standalone_snapshot
        .as_object_mut()
        .unwrap()
        .insert("future".into(), json!(true));
    assert_json_rejected::<GraphSnapshotV1>(unknown_standalone_snapshot);

    let mut oversized_json = replay_value;
    oversized_json["deltas"] = Value::Array(vec![
        serde_json::to_value(empty_delta).unwrap();
        MAX_REPLAY_DELTAS_V1 + 1
    ]);
    assert_json_rejected::<GraphReplayV1>(oversized_json);
}

fn assert_delta_rejected_without_mutation(
    base: &SparseGraph,
    value: &StructuralDeltaV1,
    expected_digest: [u8; 32],
    expected: DeltaError,
) {
    let before = base.canonical_bytes();
    assert_eq!(
        apply_delta(7, &expected_digest, 7, base, value).unwrap_err(),
        expected
    );
    assert_eq!(base.canonical_bytes(), before);
}

fn graph_at_capacity() -> SparseGraph {
    let mut graph = SparseGraph::empty();
    graph.row_offsets.clear();
    graph.row_offsets.push(0);
    for _source in 0..NEURON_SLOTS {
        for target in 0..32 {
            graph.edges.push(edge(target));
        }
        graph.row_offsets.push(graph.edges.len() as u32);
    }
    assert_eq!(graph.edges.len(), EDGE_CAPACITY);
    assert!(graph.validate());
    graph
}

#[test]
fn structural_delta_resource_limit_cardinality_and_persisted_schema_are_closed() {
    assert_eq!(MAX_OPERATIONS_PER_DELTA_V1, 4_096);

    let invalid_graph = SparseGraph::default();
    let oversized = StructuralDeltaV1 {
        base_revision: 7,
        base_graph_digest: [0; 32],
        delta_sequence: 1,
        rule_digest: [7; 32],
        operations: vec![
            EdgeOperationV1::Add {
                source: 0,
                edge: edge(1),
            };
            MAX_OPERATIONS_PER_DELTA_V1 + 1
        ],
        after_graph_digest: [0; 32],
    };
    assert_delta_rejected_without_mutation(
        &invalid_graph,
        &oversized,
        [0; 32],
        DeltaError::TooManyOperations,
    );

    let empty = SparseGraph::empty();
    let underflow = delta(
        &empty,
        vec![EdgeOperationV1::Remove {
            source: 0,
            target: 1,
        }],
    );
    assert_delta_rejected_without_mutation(
        &empty,
        &underflow,
        graph_digest(&empty),
        DeltaError::EdgeCardinalityUnderflow,
    );

    let capacity = graph_at_capacity();
    let over_capacity = delta(
        &capacity,
        vec![EdgeOperationV1::Add {
            source: 0,
            edge: edge(32),
        }],
    );
    assert_delta_rejected_without_mutation(
        &capacity,
        &over_capacity,
        graph_digest(&capacity),
        DeltaError::EdgeCapacityExceeded,
    );

    let operations = [
        EdgeOperationV1::Add {
            source: 0,
            edge: edge(1),
        },
        EdgeOperationV1::Update {
            source: 0,
            edge: edge(1),
        },
        EdgeOperationV1::Remove {
            source: 0,
            target: 1,
        },
    ];
    for operation in operations {
        let mut unknown_operation = serde_json::to_value(&operation).unwrap();
        unknown_operation
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("future".into(), json!(true));
        assert_json_rejected::<EdgeOperationV1>(unknown_operation);
    }

    let add = EdgeOperationV1::Add {
        source: 0,
        edge: edge(1),
    };
    let mut wrong_variant_name = serde_json::to_value(&add).unwrap();
    let payload = wrong_variant_name
        .as_object_mut()
        .unwrap()
        .remove("Add")
        .unwrap();
    wrong_variant_name
        .as_object_mut()
        .unwrap()
        .insert("add".into(), payload);
    assert_json_rejected::<EdgeOperationV1>(wrong_variant_name);
    assert_json_rejected::<EdgeOperationV1>(json!({
        "kind": "Add",
        "source": 0,
        "edge": edge(1),
    }));

    let mut unknown_synapse = serde_json::to_value(&add).unwrap();
    unknown_synapse["Add"]["edge"]
        .as_object_mut()
        .unwrap()
        .insert("future".into(), json!(true));
    assert_json_rejected::<EdgeOperationV1>(unknown_synapse);

    let valid_delta = delta(&empty, vec![add.clone()]);
    let delta_value = serde_json::to_value(&valid_delta).unwrap();
    let mut unknown_delta = delta_value.clone();
    unknown_delta
        .as_object_mut()
        .unwrap()
        .insert("future".into(), json!(true));
    assert_json_rejected::<StructuralDeltaV1>(unknown_delta);

    let mut oversized_json = delta_value;
    oversized_json["operations"] = Value::Array(vec![
        serde_json::to_value(&add).unwrap();
        MAX_OPERATIONS_PER_DELTA_V1 + 1
    ]);
    assert_json_rejected::<StructuralDeltaV1>(oversized_json);
}
