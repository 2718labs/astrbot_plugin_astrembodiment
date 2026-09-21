#![forbid(unsafe_code)]

use ae_contracts::StateSubcodeV1;
use ae_fixed::Fixed;
use ae_neurofield::{
    graph_digest, state_digest, NeuralField, SparseGraph, Synapse, NEURON_SLOTS, REGION_LAYOUT,
};
use ae_runtime::semantic_dynamics_v2::{
    mul6_raw, propagate_semantic_dynamics_v2, DynamicsError, DynamicsInputV2,
};
use ae_runtime::RuntimeError;

const FORMULA_DIGEST: [u8; 32] = [0x11; 32];

fn fixed(raw: i64) -> Fixed {
    Fixed::from_raw(raw)
}

fn filled_field(
    potential: i64,
    excitation: i64,
    inhibition: i64,
    adaptation: i64,
    precision: i64,
    prediction_error: i64,
    eligibility: i64,
    metabolic_reserve: i64,
) -> NeuralField {
    let fill = |raw| vec![fixed(raw); NEURON_SLOTS];
    NeuralField {
        potential: fill(potential),
        excitation: fill(excitation),
        inhibition: fill(inhibition),
        adaptation: fill(adaptation),
        precision: fill(precision),
        prediction_error: fill(prediction_error),
        eligibility: fill(eligibility),
        metabolic_reserve: fill(metabolic_reserve),
    }
}

fn before_field() -> NeuralField {
    filled_field(
        600_000, 200_000, 100_000, 50_000, 300_000, 75_000, 25_000, 800_000,
    )
}

fn baseline_field() -> NeuralField {
    filled_field(400_000, 0, 0, 0, 0, 0, 0, 1_000_000)
}

fn edge(target: u32, weight: i16) -> Synapse {
    Synapse {
        target,
        weight,
        eligibility: 0,
        stability: 0,
        last_used_epoch: 0,
        operator_id: 0,
        delay_class: 0,
        flags: 0,
    }
}

/// Source 0 excites node 1; source 1 inhibits node 0. Every later row is
/// empty. This is a real 16K CSR graph, not a reduced stand-in.
fn two_edge_graph() -> SparseGraph {
    let mut row_offsets = vec![2; NEURON_SLOTS + 1];
    row_offsets[0] = 0;
    row_offsets[1] = 1;
    SparseGraph {
        row_offsets,
        edges: vec![edge(1, 1_000), edge(0, -1_000)],
    }
}

fn field_bytes(field: &NeuralField) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(NEURON_SLOTS * 8 * 8 + 32);
    for values in [
        &field.potential,
        &field.excitation,
        &field.inhibition,
        &field.adaptation,
        &field.precision,
        &field.prediction_error,
        &field.eligibility,
        &field.metabolic_reserve,
    ] {
        bytes.extend_from_slice(&(values.len() as u32).to_le_bytes());
        for value in values {
            bytes.extend_from_slice(&value.encode());
        }
    }
    bytes
}

fn assert_node(field: &NeuralField, node: usize, expected: [i64; 8]) {
    let actual = [
        field.potential[node].raw(),
        field.excitation[node].raw(),
        field.inhibition[node].raw(),
        field.adaptation[node].raw(),
        field.precision[node].raw(),
        field.prediction_error[node].raw(),
        field.eligibility[node].raw(),
        field.metabolic_reserve[node].raw(),
    ];
    assert_eq!(
        actual, expected,
        "unexpected eight-DOF state at node {node}"
    );
}

fn assert_typed_error(error: RuntimeError, expected: StateSubcodeV1) {
    match error {
        RuntimeError::InvalidNeuralState(actual) => assert_eq!(actual, expected),
        other => panic!("expected typed INVALID_NEURAL_STATE, got {other:?}"),
    }
}

#[test]
fn full_16k_two_edge_jacobi_step_matches_exact_dynamics_goldens() {
    let before = before_field();
    let baseline = baseline_field();
    let graph = two_edge_graph();
    assert!(before.validate());
    assert!(baseline.validate());
    assert!(graph.validate());
    assert_eq!(graph.edges.len(), 2);

    let before_bytes = field_bytes(&before);
    let before_digest = state_digest(&before, &FORMULA_DIGEST);
    let graph_bytes = graph.canonical_bytes();
    let graph_before = graph_digest(&graph);
    let local = [fixed(200_000); REGION_LAYOUT.len()];
    let confidence = [fixed(500_000); REGION_LAYOUT.len()];

    let prepared = propagate_semantic_dynamics_v2(DynamicsInputV2 {
        field: &before,
        baseline: &baseline,
        graph: &graph,
        local_by_region: local,
        local_confidence_by_region: confidence,
    })
    .expect("the pinned full-field fixture must prepare");

    assert_eq!(field_bytes(&before), before_bytes);
    assert_eq!(state_digest(&before, &FORMULA_DIGEST), before_digest);
    assert_eq!(graph.canonical_bytes(), graph_bytes);
    assert_eq!(graph_digest(&graph), graph_before);
    assert_ne!(
        state_digest(&prepared.next_field, &FORMULA_DIGEST),
        before_digest
    );
    assert!(prepared.next_field.validate());
    for values in [
        &prepared.next_field.potential,
        &prepared.next_field.excitation,
        &prepared.next_field.inhibition,
        &prepared.next_field.adaptation,
        &prepared.next_field.precision,
        &prepared.next_field.prediction_error,
        &prepared.next_field.eligibility,
        &prepared.next_field.metabolic_reserve,
    ] {
        assert_eq!(values.len(), NEURON_SLOTS);
    }

    // Independently hand-derived FXP6 goldens. Node 0 receives the negative
    // edge, node 1 receives the positive edge, and node 2 represents all
    // 16,382 edge-free nodes.
    assert_node(
        &prepared.next_field,
        0,
        [587_500, 12_500, 0, 67_188, 500_000, 12_500, 18_750, 798_568],
    );
    assert_node(
        &prepared.next_field,
        1,
        [
            762_500, 187_500, 0, 89_063, 500_000, 162_500, 93_750, 797_161,
        ],
    );
    assert_node(
        &prepared.next_field,
        2,
        [
            675_000, 100_000, 0, 78_125, 500_000, 75_000, 50_000, 799_531,
        ],
    );
    assert_node(
        &prepared.next_field,
        NEURON_SLOTS - 1,
        [
            675_000, 100_000, 0, 78_125, 500_000, 75_000, 50_000, 799_531,
        ],
    );

    // A sequential-update mutant lets source 1 observe node 1 after source 0
    // writes it, yielding 556250 at node 0. The exact golden above must remain
    // distinct, proving immutable-before Jacobi reads.
    assert_ne!(prepared.next_field.potential[0], fixed(556_250));
    assert_eq!(prepared.effective_by_region, [fixed(200_000); 9]);
    assert_eq!(prepared.direct_by_region, [fixed(100_000); 9]);
    assert_eq!(prepared.propagated_edge_count, 2);
    assert_eq!(prepared.upper_saturated_nodes, 0);
    assert_eq!(prepared.energy.reserve_before_min, fixed(800_000));
    assert_eq!(prepared.energy.reserve_after_min, fixed(797_161));
    assert_eq!(prepared.energy.recovered_mean, fixed(5_000));
    assert_eq!(prepared.energy.spent_mean, fixed(5_469));
    assert_eq!(prepared.energy.residual_mean, Fixed::ZERO);
    assert_eq!(prepared.renormalization_residual, Fixed::ZERO);
}

#[test]
fn overflow_and_invalid_shape_fail_without_partial_field() {
    let before = before_field();
    let baseline = baseline_field();
    let graph = two_edge_graph();
    let before_bytes = field_bytes(&before);
    let before_digest = state_digest(&before, &FORMULA_DIGEST);
    let graph_bytes = graph.canonical_bytes();
    let local = [fixed(200_000); REGION_LAYOUT.len()];
    let confidence = [fixed(500_000); REGION_LAYOUT.len()];

    let overflow = mul6_raw(i64::MAX, i64::MAX).expect_err("FXP6 output must fail i64 narrowing");
    assert_eq!(overflow, DynamicsError::Arithmetic);
    assert_typed_error(
        RuntimeError::from(overflow),
        StateSubcodeV1::DynamicsInvalid,
    );

    let mut invalid_field = before.clone();
    invalid_field.eligibility.pop();
    let invalid_field_bytes = field_bytes(&invalid_field);
    let invalid_field_digest = state_digest(&invalid_field, &FORMULA_DIGEST);
    let field_error = propagate_semantic_dynamics_v2(DynamicsInputV2 {
        field: &invalid_field,
        baseline: &baseline,
        graph: &graph,
        local_by_region: local,
        local_confidence_by_region: confidence,
    })
    .expect_err("a seven-complete-plus-one-short field must fail closed");
    assert_eq!(field_error, DynamicsError::FieldStateInvalid);
    assert_typed_error(
        RuntimeError::from(field_error),
        StateSubcodeV1::FieldStateInvalid,
    );
    assert_eq!(field_bytes(&invalid_field), invalid_field_bytes);
    assert_eq!(
        state_digest(&invalid_field, &FORMULA_DIGEST),
        invalid_field_digest
    );

    let mut invalid_graph = graph.clone();
    invalid_graph.row_offsets[2] = 0;
    let invalid_graph_bytes = invalid_graph.canonical_bytes();
    let graph_error = propagate_semantic_dynamics_v2(DynamicsInputV2 {
        field: &before,
        baseline: &baseline,
        graph: &invalid_graph,
        local_by_region: local,
        local_confidence_by_region: confidence,
    })
    .expect_err("a decreasing CSR offset must fail closed");
    assert_eq!(graph_error, DynamicsError::GraphStateInvalid);
    assert_typed_error(
        RuntimeError::from(graph_error),
        StateSubcodeV1::GraphStateInvalid,
    );
    assert_eq!(invalid_graph.canonical_bytes(), invalid_graph_bytes);

    assert_eq!(field_bytes(&before), before_bytes);
    assert_eq!(state_digest(&before, &FORMULA_DIGEST), before_digest);
    assert_eq!(graph.canonical_bytes(), graph_bytes);
}
