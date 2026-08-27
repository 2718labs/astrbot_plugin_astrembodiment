use ae_neurofield::{
    apply_delta, graph_digest, DeltaError, EdgeOperationV1, SparseGraph, StructuralDeltaV1,
    Synapse, EDGE_CAPACITY, NEURON_SLOTS,
};

fn digest(byte: u8) -> [u8; 32] {
    [byte; 32]
}

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

fn graph_with_edges(entries: &[(u32, Synapse)]) -> SparseGraph {
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
    operations: Vec<EdgeOperationV1>,
    after_graph: &SparseGraph,
) -> StructuralDeltaV1 {
    StructuralDeltaV1 {
        base_revision,
        base_graph_digest: graph_digest(base_graph),
        delta_sequence: 1,
        rule_digest: digest(7),
        operations,
        after_graph_digest: graph_digest(after_graph),
    }
}

fn assert_rejected_preserves_graph(
    base: &SparseGraph,
    value: &StructuralDeltaV1,
    expected: DeltaError,
) {
    let before = base.canonical_bytes();
    assert_eq!(
        apply_delta(7, &graph_digest(base), 7, base, value).unwrap_err(),
        expected
    );
    assert_eq!(base.canonical_bytes(), before);
}

#[test]
fn canonical_add_update_remove_is_a_digest_checked_compare_and_swap() {
    let base = SparseGraph::empty();
    let after_add = graph_with_edges(&[(2, edge(3))]);
    let add = delta(
        7,
        &base,
        vec![EdgeOperationV1::Add {
            source: 2,
            edge: edge(3),
        }],
        &after_add,
    );
    let added = apply_delta(7, &graph_digest(&base), 7, &base, &add).unwrap();
    assert_eq!(added.canonical_bytes(), after_add.canonical_bytes());
    assert_eq!(graph_digest(&added), add.after_graph_digest);

    let mut updated_edge = edge(3);
    updated_edge.weight = -99;
    let after_update = graph_with_edges(&[(2, updated_edge)]);
    let update = delta(
        8,
        &added,
        vec![EdgeOperationV1::Update {
            source: 2,
            edge: updated_edge,
        }],
        &after_update,
    );
    let updated = apply_delta(8, &graph_digest(&added), 8, &added, &update).unwrap();
    assert_eq!(updated.canonical_bytes(), after_update.canonical_bytes());

    let after_remove = SparseGraph::empty();
    let remove = delta(
        9,
        &updated,
        vec![EdgeOperationV1::Remove {
            source: 2,
            target: 3,
        }],
        &after_remove,
    );
    let removed = apply_delta(9, &graph_digest(&updated), 9, &updated, &remove).unwrap();
    assert_eq!(removed.canonical_bytes(), after_remove.canonical_bytes());
}

#[test]
fn stale_revision_and_base_digest_return_fixed_errors_without_mutation() {
    let base = SparseGraph::empty();
    let after = graph_with_edges(&[(0, edge(1))]);
    let value = delta(
        7,
        &base,
        vec![EdgeOperationV1::Add {
            source: 0,
            edge: edge(1),
        }],
        &after,
    );
    let before = base.canonical_bytes();
    assert_eq!(
        apply_delta(7, &graph_digest(&base), 8, &base, &value).unwrap_err(),
        DeltaError::StaleRevision
    );
    assert_eq!(base.canonical_bytes(), before);

    let mut stale_digest = value.clone();
    stale_digest.base_graph_digest = digest(99);
    assert_eq!(
        apply_delta(7, &digest(99), 7, &base, &stale_digest).unwrap_err(),
        DeltaError::StaleGraphDigest
    );
    assert_eq!(base.canonical_bytes(), before);
}

#[test]
fn invalid_operations_are_rejected_before_any_caller_graph_mutation() {
    let base = SparseGraph::empty();
    let after = graph_with_edges(&[(0, edge(1))]);

    let unsorted = delta(
        7,
        &base,
        vec![
            EdgeOperationV1::Add {
                source: 1,
                edge: edge(2),
            },
            EdgeOperationV1::Add {
                source: 0,
                edge: edge(1),
            },
        ],
        &after,
    );
    assert_rejected_preserves_graph(&base, &unsorted, DeltaError::OperationsNotCanonical);

    let duplicate = delta(
        7,
        &base,
        vec![
            EdgeOperationV1::Add {
                source: 0,
                edge: edge(1),
            },
            EdgeOperationV1::Update {
                source: 0,
                edge: edge(1),
            },
        ],
        &after,
    );
    assert_rejected_preserves_graph(&base, &duplicate, DeltaError::DuplicateEdgeOperation);

    let source_out_of_bounds = delta(
        7,
        &base,
        vec![EdgeOperationV1::Add {
            source: NEURON_SLOTS as u32,
            edge: edge(1),
        }],
        &after,
    );
    assert_rejected_preserves_graph(&base, &source_out_of_bounds, DeltaError::SourceOutOfBounds);

    let target_out_of_bounds = delta(
        7,
        &base,
        vec![EdgeOperationV1::Add {
            source: 0,
            edge: edge(NEURON_SLOTS as u32),
        }],
        &after,
    );
    assert_rejected_preserves_graph(&base, &target_out_of_bounds, DeltaError::TargetOutOfBounds);

    let mut unknown_operator_edge = edge(1);
    unknown_operator_edge.operator_id = 4;
    let unknown_operator = delta(
        7,
        &base,
        vec![EdgeOperationV1::Add {
            source: 0,
            edge: unknown_operator_edge,
        }],
        &after,
    );
    assert_rejected_preserves_graph(&base, &unknown_operator, DeltaError::UnknownOperator);

    let mut unknown_delay_edge = edge(1);
    unknown_delay_edge.delay_class = 8;
    let unknown_delay = delta(
        7,
        &base,
        vec![EdgeOperationV1::Add {
            source: 0,
            edge: unknown_delay_edge,
        }],
        &after,
    );
    assert_rejected_preserves_graph(&base, &unknown_delay, DeltaError::UnknownDelayClass);

    let wrong_after = StructuralDeltaV1 {
        after_graph_digest: digest(55),
        ..delta(
            7,
            &base,
            vec![EdgeOperationV1::Add {
                source: 0,
                edge: edge(1),
            }],
            &after,
        )
    };
    assert_rejected_preserves_graph(&base, &wrong_after, DeltaError::AfterGraphDigestMismatch);
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
fn edge_capacity_is_rejected_without_changing_the_input_graph() {
    let base = graph_at_capacity();
    let value = delta(
        7,
        &base,
        vec![EdgeOperationV1::Add {
            source: 0,
            edge: edge(32),
        }],
        &base,
    );
    assert_rejected_preserves_graph(&base, &value, DeltaError::EdgeCapacityExceeded);
}
