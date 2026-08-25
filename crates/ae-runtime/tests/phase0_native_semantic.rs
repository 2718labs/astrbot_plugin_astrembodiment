use ae_fixed::Fixed;
use ae_neurofield::{NeuralField, SparseGraph, Synapse, NEURON_SLOTS, REGION_LAYOUT};
use ae_runtime::semantic_dynamics_v2::{propagate_semantic_dynamics_v2, DynamicsInputV2};

#[test]
fn sparse_edge_propagates_source_signal_to_target_with_immutable_jacobi_state() {
    let mut field = NeuralField::zeroed();
    let baseline = NeuralField::zeroed();
    let source = 0_usize;
    let target = REGION_LAYOUT[1].0;
    field.potential[source] = Fixed::ONE;

    let mut graph = SparseGraph::empty();
    graph.edges.push(Synapse {
        target: u32::try_from(target).expect("target fits u32"),
        weight: 1_000,
        ..Synapse::default()
    });
    for offset in graph.row_offsets.iter_mut().skip(1) {
        *offset = 1;
    }
    assert!(graph.validate());
    assert_eq!(graph.row_offsets.len(), NEURON_SLOTS + 1);

    let result = propagate_semantic_dynamics_v2(DynamicsInputV2 {
        field: &field,
        baseline: &baseline,
        graph: &graph,
        local_by_region: [Fixed::ZERO; REGION_LAYOUT.len()],
        local_confidence_by_region: [Fixed::ZERO; REGION_LAYOUT.len()],
    })
    .expect("valid edge fixture must prepare");

    assert_eq!(result.propagated_edge_count, 1);
    assert_eq!(
        result.next_field.potential[target],
        Fixed::from_raw(125_000)
    );
    assert_eq!(
        result.next_field.excitation[target],
        Fixed::from_raw(125_000)
    );
}
