#![forbid(unsafe_code)]

use ae_fixed::Fixed;
use ae_neurofield::{graph_digest, state_digest, NeuralField, SparseGraph, NEURON_SLOTS};
use ae_semantic_core::{
    advance_matrix_time_v1, decode_time_snapshot_v1, encode_time_snapshot_v1,
    matrix_time_formula_digest_v1, DecodedTimeSnapshotV1, MatrixSleepPhaseV1, MatrixTimeEpochV1,
    MatrixTimeInputV1, MATRIX_TIME_MAX_ELAPSED_MS, MATRIX_TIME_MAX_STEPS_PER_EVENT,
    MATRIX_TIME_QUANTUM_MS,
};

fn filled_field(values: [i64; 8]) -> NeuralField {
    let fill = |raw| vec![Fixed::from_raw(raw); NEURON_SLOTS];
    NeuralField {
        potential: fill(values[0]),
        excitation: fill(values[1]),
        inhibition: fill(values[2]),
        adaptation: fill(values[3]),
        precision: fill(values[4]),
        prediction_error: fill(values[5]),
        eligibility: fill(values[6]),
        metabolic_reserve: fill(values[7]),
    }
}

#[test]
fn time_snapshot_is_strict_and_contains_no_evidence_receipt_or_telemetry() {
    let field = anchor();
    let graph = SparseGraph::empty();
    let semantic_formula_digest = [0x44; 32];
    let time_formula_digest = matrix_time_formula_digest_v1(&semantic_formula_digest);
    let epoch_before = MatrixTimeEpochV1 {
        anchor_state_digest: state_digest(&field, &semantic_formula_digest),
        ..epoch()
    };
    let advanced = advance_matrix_time_v1(MatrixTimeInputV1 {
        anchor_field: &field,
        genesis_baseline: &baseline(),
        epoch: &epoch_before,
        elapsed_ms: MATRIX_TIME_QUANTUM_MS,
        phase: MatrixSleepPhaseV1::Asleep,
        semantic_formula_digest,
    })
    .unwrap();
    let projection = DecodedTimeSnapshotV1 {
        semantic_formula_digest,
        time_formula_digest,
        epoch_before,
        epoch_after: advanced.epoch,
        requested_elapsed_ms: MATRIX_TIME_QUANTUM_MS,
        applied_elapsed_ms: MATRIX_TIME_QUANTUM_MS,
        capped_gap: false,
        pre_sleep_phase: MatrixSleepPhaseV1::Asleep,
        state_before: state_digest(&field, &semantic_formula_digest),
        state_after: state_digest(&advanced.field, &semantic_formula_digest),
        graph_digest: graph_digest(&graph),
        authority_digest: [0x55; 32],
        commitment_digest: [0x66; 32],
    };
    let bytes = encode_time_snapshot_v1(&projection).unwrap();
    // A time transition is a projection over the last authenticated
    // perception/Genesis anchor. It must never duplicate the 16K field or
    // sparse graph in every wake row.
    assert!(
        bytes.len() <= 1_024,
        "AESET1 must stay compact, got {} bytes",
        bytes.len()
    );
    assert!(bytes.starts_with(b"AESET1\0\x01\0"));
    let decoded = decode_time_snapshot_v1(&bytes).unwrap();
    assert_eq!(decoded, projection);
    assert_eq!(encode_time_snapshot_v1(&decoded).unwrap(), bytes);

    assert!(decode_time_snapshot_v1(&bytes[..bytes.len() - 1]).is_err());
    let mut trailing = bytes;
    trailing.push(0);
    assert!(decode_time_snapshot_v1(&trailing).is_err());
}

#[test]
fn time_snapshot_rejects_inconsistent_elapsed_projection() {
    let semantic_formula_digest = [0x44; 32];
    let time_formula_digest = matrix_time_formula_digest_v1(&semantic_formula_digest);
    let field = anchor();
    let mut epoch_before = epoch();
    epoch_before.anchor_state_digest = state_digest(&field, &semantic_formula_digest);
    let mut epoch_after = epoch_before.clone();
    epoch_after.awake_ticks = 1;
    let invalid = DecodedTimeSnapshotV1 {
        semantic_formula_digest,
        time_formula_digest,
        epoch_before,
        epoch_after,
        requested_elapsed_ms: 1,
        applied_elapsed_ms: 1,
        capped_gap: false,
        pre_sleep_phase: MatrixSleepPhaseV1::Awake,
        state_before: [1; 32],
        state_after: [2; 32],
        graph_digest: [3; 32],
        authority_digest: [4; 32],
        commitment_digest: [5; 32],
    };
    assert!(encode_time_snapshot_v1(&invalid).is_err());
}

#[test]
fn frozen_one_quantum_and_mixed_phase_values_pin_rounding_and_coefficients() {
    let awake = advance(&epoch(), MATRIX_TIME_QUANTUM_MS, MatrixSleepPhaseV1::Awake);
    assert_eq!(
        [
            awake.field.potential[0].raw(),
            awake.field.excitation[0].raw(),
            awake.field.inhibition[0].raw(),
            awake.field.adaptation[0].raw(),
            awake.field.precision[0].raw(),
            awake.field.prediction_error[0].raw(),
            awake.field.eligibility[0].raw(),
            awake.field.metabolic_reserve[0].raw(),
        ],
        [892_188, 493_750, 394_531, 291_406, 698_438, 197_188, 98_594, 501_562]
    );

    let drowsy = advance(
        &awake.epoch,
        MATRIX_TIME_QUANTUM_MS,
        MatrixSleepPhaseV1::Drowsy,
    );
    let mixed = advance(
        &drowsy.epoch,
        MATRIX_TIME_QUANTUM_MS,
        MatrixSleepPhaseV1::Asleep,
    );
    assert_eq!(
        [
            mixed.field.potential[0].raw(),
            mixed.field.excitation[0].raw(),
            mixed.field.inhibition[0].raw(),
            mixed.field.adaptation[0].raw(),
            mixed.field.precision[0].raw(),
            mixed.field.prediction_error[0].raw(),
            mixed.field.eligibility[0].raw(),
            mixed.field.metabolic_reserve[0].raw(),
        ],
        [847_006, 457_605, 362_904, 243_536, 689_401, 180_922, 90_461, 510_869]
    );
}

fn anchor() -> NeuralField {
    filled_field([
        900_000, 500_000, 400_000, 300_000, 700_000, 200_000, 100_000, 500_000,
    ])
}

fn baseline() -> NeuralField {
    filled_field([
        400_000, 100_000, 50_000, 25_000, 600_000, 20_000, 10_000, 250_000,
    ])
}

fn epoch() -> MatrixTimeEpochV1 {
    let field = anchor();
    MatrixTimeEpochV1 {
        schema_version: MatrixTimeEpochV1::SCHEMA_VERSION,
        anchor_semantic_revision: 7,
        anchor_state_digest: state_digest(&field, &[0x44; 32]),
        awake_ticks: 0,
        drowsy_ticks: 0,
        asleep_ticks: 0,
        awake_remainder_ms: 0,
        drowsy_remainder_ms: 0,
        asleep_remainder_ms: 0,
    }
}

fn advance(
    epoch: &MatrixTimeEpochV1,
    elapsed_ms: u64,
    phase: MatrixSleepPhaseV1,
) -> ae_semantic_core::MatrixTimeAdvanceV1 {
    advance_matrix_time_v1(MatrixTimeInputV1 {
        anchor_field: &anchor(),
        genesis_baseline: &baseline(),
        epoch,
        elapsed_ms,
        phase,
        semantic_formula_digest: [0x44; 32],
    })
    .unwrap()
}

#[test]
fn sleep_time_advance_relaxes_without_inventing_evidence() {
    let awake = advance(&epoch(), 3_600_000, MatrixSleepPhaseV1::Awake);
    let asleep = advance(&epoch(), 3_600_000, MatrixSleepPhaseV1::Asleep);

    assert!(
        (asleep.field.potential[0].raw() - baseline().potential[0].raw()).abs()
            < (awake.field.potential[0].raw() - baseline().potential[0].raw()).abs()
    );
    assert!(asleep.field.adaptation[0] < awake.field.adaptation[0]);
    assert!(asleep.field.metabolic_reserve[0] > awake.field.metabolic_reserve[0]);
    assert_eq!(awake.applied_elapsed_ms, 3_600_000);
    assert_eq!(awake.step_count, 6);
    assert!(!awake.capped_gap);
}

#[test]
fn cumulative_epoch_makes_time_chunking_exact() {
    let once = advance(&epoch(), 3_600_000, MatrixSleepPhaseV1::Awake);
    let mut chunks = epoch();
    for _ in 0..6 {
        chunks = advance(&chunks, MATRIX_TIME_QUANTUM_MS, MatrixSleepPhaseV1::Awake).epoch;
    }
    let chunked = advance(&chunks, 0, MatrixSleepPhaseV1::Awake);

    assert_eq!(once.field.potential, chunked.field.potential);
    assert_eq!(once.field.adaptation, chunked.field.adaptation);
    assert_eq!(
        once.field.metabolic_reserve,
        chunked.field.metabolic_reserve
    );
    assert_eq!(once.epoch, chunked.epoch);

    let five = advance(&epoch(), 300_000, MatrixSleepPhaseV1::Drowsy);
    assert_eq!(five.step_count, 0);
    assert_eq!(five.epoch.drowsy_remainder_ms, 300_000);
    let ten = advance(&five.epoch, 300_000, MatrixSleepPhaseV1::Drowsy);
    let direct = advance(&epoch(), 600_000, MatrixSleepPhaseV1::Drowsy);
    assert_eq!(ten.field.potential, direct.field.potential);
    assert_eq!(ten.epoch, direct.epoch);
}

#[test]
fn phase_order_is_fixed_across_different_event_chunking() {
    let awake = advance(&epoch(), 1_200_000, MatrixSleepPhaseV1::Awake);
    let mixed = advance(&awake.epoch, 1_800_000, MatrixSleepPhaseV1::Asleep);

    let sleep_first = advance(&epoch(), 1_800_000, MatrixSleepPhaseV1::Asleep);
    let reordered = advance(&sleep_first.epoch, 1_200_000, MatrixSleepPhaseV1::Awake);

    assert_eq!(mixed.epoch, reordered.epoch);
    assert_eq!(mixed.field.potential, reordered.field.potential);
    assert_eq!(mixed.field.adaptation, reordered.field.adaptation);
    assert_eq!(
        mixed.field.metabolic_reserve,
        reordered.field.metabolic_reserve
    );
}

#[test]
fn sub_quantum_time_keeps_its_sleep_phase_attribution() {
    let awake_half = advance(&epoch(), 300_000, MatrixSleepPhaseV1::Awake);
    let asleep_half = advance(&awake_half.epoch, 300_000, MatrixSleepPhaseV1::Asleep);
    assert_eq!(asleep_half.epoch.awake_ticks, 0);
    assert_eq!(asleep_half.epoch.asleep_ticks, 0);
    assert_eq!(asleep_half.epoch.awake_remainder_ms, 300_000);
    assert_eq!(asleep_half.epoch.asleep_remainder_ms, 300_000);

    let awake_full = advance(&asleep_half.epoch, 300_000, MatrixSleepPhaseV1::Awake);
    assert_eq!(awake_full.epoch.awake_ticks, 1);
    assert_eq!(awake_full.epoch.asleep_ticks, 0);
    assert_eq!(awake_full.epoch.awake_remainder_ms, 0);
    assert_eq!(awake_full.epoch.asleep_remainder_ms, 300_000);
}

#[test]
fn elapsed_is_bounded_and_invalid_epoch_fails_closed() {
    assert_eq!(MATRIX_TIME_MAX_STEPS_PER_EVENT, 1_008);
    let bounded = advance(
        &epoch(),
        MATRIX_TIME_MAX_ELAPSED_MS + MATRIX_TIME_QUANTUM_MS,
        MatrixSleepPhaseV1::Asleep,
    );
    assert!(bounded.capped_gap);
    assert_eq!(bounded.applied_elapsed_ms, MATRIX_TIME_MAX_ELAPSED_MS);
    assert_eq!(bounded.step_count, MATRIX_TIME_MAX_STEPS_PER_EVENT);

    let mut invalid = epoch();
    invalid.awake_remainder_ms = MATRIX_TIME_QUANTUM_MS;
    assert!(advance_matrix_time_v1(MatrixTimeInputV1 {
        anchor_field: &anchor(),
        genesis_baseline: &baseline(),
        epoch: &invalid,
        elapsed_ms: 1,
        phase: MatrixSleepPhaseV1::Awake,
        semantic_formula_digest: [0x44; 32],
    })
    .is_err());

    let mut wrong_anchor = epoch();
    wrong_anchor.anchor_state_digest = [0x99; 32];
    assert!(advance_matrix_time_v1(MatrixTimeInputV1 {
        anchor_field: &anchor(),
        genesis_baseline: &baseline(),
        epoch: &wrong_anchor,
        elapsed_ms: MATRIX_TIME_QUANTUM_MS,
        phase: MatrixSleepPhaseV1::Awake,
        semantic_formula_digest: [0x44; 32],
    })
    .is_err());
}
