#![forbid(unsafe_code)]

use ae_contracts::{wire, EvidenceVector, PerceptionProposalV1};
use ae_fixed::Fixed;
use ae_neurofield::{
    graph_digest, state_digest, NeuralField, SparseGraph, Synapse, EDGE_CAPACITY, NEURON_SLOTS,
};
use ae_semantic_core::{
    decode_canonical_semantic_snapshot_v3, decode_native_telemetry_receipt_v1,
    decode_transition_receipt_v2, derive_user_stimulus_transition_v1,
    encode_native_telemetry_receipt_v1, encode_semantic_snapshot_v3, encode_transition_receipt_v2,
    SemanticCoreError, UserStimulusTransitionInputV1,
};

const FORMULA_DIGEST: [u8; 32] = [0x11; 32];
const SCOPE_DIGEST: [u8; 32] = [0x22; 32];
const EVENT_DIGEST: [u8; 32] = [0x33; 32];
const SOURCE_DIGEST: [u8; 32] = [0x44; 32];
const AUTHORITY_DIGEST: [u8; 32] = [0x55; 32];

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

fn two_edge_graph() -> SparseGraph {
    let mut row_offsets = vec![2; NEURON_SLOTS + 1];
    row_offsets[0] = 0;
    row_offsets[1] = 1;
    SparseGraph {
        row_offsets,
        edges: vec![edge(1, 1_000), edge(0, -1_000)],
    }
}

fn uniform_evidence(raw: i64) -> EvidenceVector {
    let value = fixed(raw);
    EvidenceVector {
        positive: value,
        affiliation: value,
        harm: value,
        boundary: value,
        repair: value,
        repetition: value,
        new_information: value,
        constraint_instability: value,
        epistemic_conflict: value,
        self_responsibility: value,
        other_responsibility: value,
        hostility: value,
        publicness: value,
        engagement: value,
        rejection: value,
    }
}

fn proposal() -> PerceptionProposalV1 {
    PerceptionProposalV1 {
        schema_version: PerceptionProposalV1::SCHEMA_VERSION,
        origin_digest: [0x61; 32],
        dimensions: uniform_evidence(200_000),
        estimator_confidence: fixed(500_000),
        protocol_version: PerceptionProposalV1::PROTOCOL_VERSION,
        request_nonce_digest: [0x63; 32],
    }
}

fn assert_node(field: &NeuralField, node: usize, expected: [i64; 8]) {
    assert_eq!(
        [
            field.potential[node].raw(),
            field.excitation[node].raw(),
            field.inhibition[node].raw(),
            field.adaptation[node].raw(),
            field.precision[node].raw(),
            field.prediction_error[node].raw(),
            field.eligibility[node].raw(),
            field.metabolic_reserve[node].raw(),
        ],
        expected
    );
}

#[test]
fn pure_transition_preserves_the_frozen_dynamics_and_wire_closure() {
    let field = before_field();
    let baseline = baseline_field();
    let graph = two_edge_graph();
    let proposal = proposal();
    let state_before = state_digest(&field, &FORMULA_DIGEST);
    let graph_before = graph_digest(&graph);

    let derived = derive_user_stimulus_transition_v1(UserStimulusTransitionInputV1 {
        field: &field,
        baseline: &baseline,
        graph: &graph,
        manifest_digest: [0x71; 32],
        development_seed_digest: [0x72; 32],
        proposal: &proposal,
        formula_digest: FORMULA_DIGEST,
        scope_digest: SCOPE_DIGEST,
        event_digest: EVENT_DIGEST,
        source_digest: SOURCE_DIGEST,
        authority_digest: AUTHORITY_DIGEST,
        semantic_base_revision: 7,
    })
    .expect("the authenticated pure transition must derive");

    assert_node(
        &derived.next_field,
        0,
        [587_500, 12_500, 0, 67_188, 500_000, 12_500, 18_750, 798_568],
    );
    assert_node(
        &derived.next_field,
        1,
        [
            762_500, 187_500, 0, 89_063, 500_000, 162_500, 93_750, 797_161,
        ],
    );
    assert_node(
        &derived.next_field,
        NEURON_SLOTS - 1,
        [
            675_000, 100_000, 0, 78_125, 500_000, 75_000, 50_000, 799_531,
        ],
    );
    assert_eq!(derived.active_nodes, NEURON_SLOTS as u32);
    assert_eq!(derived.active_edges, 2);
    assert_eq!(derived.state_before_digest, state_before);
    assert_eq!(derived.graph_before_digest, graph_before);
    assert_eq!(
        derived.state_after_digest,
        state_digest(&derived.next_field, &FORMULA_DIGEST)
    );
    assert_eq!(
        derived.graph_after_digest,
        graph_digest(&derived.next_graph)
    );
    assert_eq!(derived.semantic_receipt.base_revision, 7);
    assert_eq!(derived.semantic_receipt.next_revision, 8);
    assert_eq!(derived.semantic_receipt.authority_digest, AUTHORITY_DIGEST);
    assert_eq!(
        derived
            .semantic_receipt
            .semantic_vector
            .nonzero_evidence_dimension_count,
        15
    );
    assert_eq!(derived.telemetry.base_revision, 7);
    assert_eq!(derived.telemetry.next_revision, 8);
    assert_eq!(derived.telemetry.source_digest, SOURCE_DIGEST);
    assert_eq!(derived.telemetry.energy.reserve_before, fixed(800_000));
    assert_eq!(derived.telemetry.energy.reserve_after, fixed(797_161));
    assert_eq!(derived.telemetry.energy.recovered, fixed(5_000));
    assert_eq!(derived.telemetry.energy.spent, fixed(5_469));
    assert_eq!(
        derived.telemetry.capacity.edge_limit as usize,
        EDGE_CAPACITY
    );
    assert_eq!(derived.telemetry.capacity.edge_headroom, fixed(999_996));
    assert_eq!(derived.telemetry.native_gate, fixed(797_161));

    assert_eq!(derived.semantic_receipt_bytes.len(), 302);
    assert_eq!(derived.telemetry_bytes.len(), 588);
    assert_eq!(derived.snapshot_bytes.len(), 1_114_873);
    assert!(derived.snapshot_bytes.starts_with(b"AESEM3\0\x03\0"));
    assert_eq!(
        wire::domain_hash(
            b"ae-semantic-core/test/semantic-receipt-v1",
            &[&derived.semantic_receipt_bytes]
        ),
        [
            33, 219, 155, 48, 42, 76, 147, 204, 187, 157, 254, 194, 34, 201, 193, 161, 184, 102,
            62, 153, 127, 184, 70, 143, 178, 136, 253, 183, 211, 203, 6, 115,
        ]
    );
    assert_eq!(
        wire::domain_hash(
            b"ae-semantic-core/test/semantic-telemetry-v1",
            &[&derived.telemetry_bytes]
        ),
        [
            229, 68, 100, 44, 122, 172, 55, 213, 218, 238, 209, 27, 90, 160, 165, 226, 34, 249,
            201, 90, 86, 38, 217, 3, 212, 63, 104, 25, 165, 87, 191, 227,
        ]
    );
    assert_eq!(
        wire::domain_hash(
            b"ae-semantic-core/test/aesem3-v1",
            &[&derived.snapshot_bytes]
        ),
        [
            121, 20, 160, 149, 15, 150, 240, 49, 191, 58, 86, 215, 201, 28, 5, 238, 60, 196, 140,
            81, 35, 39, 247, 255, 77, 59, 60, 21, 164, 162, 90, 115,
        ]
    );
    assert_eq!(
        derived.state_after_digest,
        [
            129, 43, 164, 67, 211, 134, 37, 204, 225, 41, 196, 214, 149, 135, 27, 194, 153, 65, 29,
            35, 235, 241, 5, 111, 126, 218, 147, 90, 74, 201, 116, 236,
        ]
    );
    assert_eq!(
        derived.graph_after_digest,
        [
            206, 217, 148, 161, 78, 12, 29, 182, 251, 232, 5, 127, 157, 10, 88, 206, 150, 126, 6,
            172, 122, 52, 64, 199, 50, 49, 148, 141, 6, 76, 193, 184,
        ]
    );
    assert_eq!(
        derived.telemetry.telemetry_digest,
        [
            81, 191, 67, 170, 238, 213, 203, 153, 120, 157, 200, 197, 248, 130, 198, 32, 122, 180,
            83, 143, 108, 115, 100, 244, 161, 76, 19, 29, 40, 230, 117, 158,
        ]
    );
    assert_eq!(
        derived.route_digest,
        [
            30, 136, 103, 6, 53, 84, 178, 250, 172, 169, 128, 169, 205, 128, 76, 232, 149, 1, 252,
            108, 135, 10, 119, 7, 254, 178, 144, 194, 101, 203, 167, 101,
        ]
    );

    let receipt = decode_transition_receipt_v2(&derived.semantic_receipt_bytes).unwrap();
    let telemetry = decode_native_telemetry_receipt_v1(&derived.telemetry_bytes).unwrap();
    assert_eq!(receipt, derived.semantic_receipt);
    assert_eq!(telemetry, derived.telemetry);
    assert_eq!(
        encode_transition_receipt_v2(&receipt).unwrap(),
        derived.semantic_receipt_bytes
    );
    assert_eq!(
        encode_native_telemetry_receipt_v1(&telemetry).unwrap(),
        derived.telemetry_bytes
    );

    let decoded = decode_canonical_semantic_snapshot_v3(&derived.snapshot_bytes).unwrap();
    assert_eq!(
        state_digest(&decoded.field, &decoded.telemetry.formula_digest),
        derived.state_after_digest
    );
    assert_eq!(graph_digest(&decoded.graph), derived.graph_after_digest);
    assert_eq!(decoded.telemetry, derived.telemetry);
    assert_eq!(
        encode_semantic_snapshot_v3(
            &decoded.telemetry.formula_digest,
            &decoded.field,
            &decoded.graph,
            &decoded.telemetry,
        )
        .unwrap(),
        derived.snapshot_bytes
    );
}

#[test]
fn canonical_aesem3_decoder_rejects_truncation_trailing_and_reserved_history() {
    let field = before_field();
    let baseline = baseline_field();
    let graph = two_edge_graph();
    let proposal = proposal();
    let derived = derive_user_stimulus_transition_v1(UserStimulusTransitionInputV1 {
        field: &field,
        baseline: &baseline,
        graph: &graph,
        manifest_digest: [0x71; 32],
        development_seed_digest: [0x72; 32],
        proposal: &proposal,
        formula_digest: FORMULA_DIGEST,
        scope_digest: SCOPE_DIGEST,
        event_digest: EVENT_DIGEST,
        source_digest: SOURCE_DIGEST,
        authority_digest: AUTHORITY_DIGEST,
        semantic_base_revision: 7,
    })
    .unwrap();

    let exact_three_blocks = &derived.snapshot_bytes[..derived.snapshot_bytes.len() - (4 + 9 * 8)];
    assert!(matches!(
        decode_canonical_semantic_snapshot_v3(exact_three_blocks),
        Err(SemanticCoreError::SnapshotWireInvalid)
    ));
    assert!(matches!(
        decode_canonical_semantic_snapshot_v3(
            &derived.snapshot_bytes[..derived.snapshot_bytes.len() - 1]
        ),
        Err(SemanticCoreError::SnapshotWireInvalid)
    ));

    let mut trailing = derived.snapshot_bytes.clone();
    trailing.push(0);
    assert!(matches!(
        decode_canonical_semantic_snapshot_v3(&trailing),
        Err(SemanticCoreError::SnapshotWireInvalid)
    ));

    let mut retired_history = derived.snapshot_bytes;
    *retired_history.last_mut().unwrap() = 1;
    assert!(matches!(
        decode_canonical_semantic_snapshot_v3(&retired_history),
        Err(SemanticCoreError::Aesem3RetiredCompensationNonzero)
    ));
}
