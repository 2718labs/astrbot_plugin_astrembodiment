#![forbid(unsafe_code)]

//! Pure, deterministic sleep-aware relaxation of the semantic neural field.
//!
//! Time events are deliberately evidence-free.  The Store owns the anchor and
//! cumulative epoch, while this module performs only bounded fixed-point math.

use ae_contracts::{wire, Digest, MatrixSleepPhaseV1, MatrixTimeEpochV1};
use ae_fixed::{Fixed, SCALE};
use ae_neurofield::{state_digest, NeuralField, NEURON_SLOTS};

use crate::SemanticCoreError;

pub const MATRIX_TIME_FORMULA_V1: &str = "matrix-time-advance-fxp6-v1";
pub const MATRIX_TIME_QUANTUM_MS: u64 = 600_000;
pub const MATRIX_TIME_MAX_ELAPSED_MS: u64 = 604_800_000;
pub const MATRIX_TIME_MAX_STEPS_PER_EVENT: u64 = 1_008;

const MATRIX_TIME_FORMULA_DOMAIN_V1: &[u8] = b"astr-embodiment/matrix-time-advance-fxp6-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PhaseCoefficientsV1 {
    normal: i64,
    adaptation: i64,
    reserve_gap: i64,
}

const AWAKE: PhaseCoefficientsV1 = PhaseCoefficientsV1 {
    normal: 984_375,
    adaptation: 968_750,
    reserve_gap: 996_875,
};
const DROWSY: PhaseCoefficientsV1 = PhaseCoefficientsV1 {
    normal: 968_750,
    adaptation: 937_500,
    reserve_gap: 993_750,
};
const ASLEEP: PhaseCoefficientsV1 = PhaseCoefficientsV1 {
    normal: 937_500,
    adaptation: 875_000,
    reserve_gap: 987_500,
};

#[derive(Clone, Copy, Debug)]
pub struct MatrixTimeInputV1<'a> {
    pub anchor_field: &'a NeuralField,
    pub genesis_baseline: &'a NeuralField,
    pub epoch: &'a MatrixTimeEpochV1,
    pub elapsed_ms: u64,
    pub phase: MatrixSleepPhaseV1,
    pub semantic_formula_digest: Digest,
}

#[derive(Clone, Debug)]
pub struct MatrixTimeAdvanceV1 {
    pub field: NeuralField,
    pub epoch: MatrixTimeEpochV1,
    pub applied_elapsed_ms: u64,
    pub step_count: u64,
    pub capped_gap: bool,
}

pub fn matrix_time_formula_digest_v1(semantic_formula_digest: &Digest) -> Digest {
    let mut constants = Vec::with_capacity(8 * 12);
    for coefficient in [AWAKE, DROWSY, ASLEEP] {
        constants.extend_from_slice(&coefficient.normal.to_le_bytes());
        constants.extend_from_slice(&coefficient.adaptation.to_le_bytes());
        constants.extend_from_slice(&coefficient.reserve_gap.to_le_bytes());
    }
    constants.extend_from_slice(&MATRIX_TIME_QUANTUM_MS.to_le_bytes());
    constants.extend_from_slice(&MATRIX_TIME_MAX_ELAPSED_MS.to_le_bytes());
    constants.extend_from_slice(&MATRIX_TIME_MAX_STEPS_PER_EVENT.to_le_bytes());
    wire::domain_hash(
        MATRIX_TIME_FORMULA_DOMAIN_V1,
        &[
            semantic_formula_digest,
            MATRIX_TIME_FORMULA_V1.as_bytes(),
            &constants,
        ],
    )
}

fn validate_unit_field(field: &NeuralField) -> bool {
    field.validate()
        && [
            &field.potential,
            &field.excitation,
            &field.inhibition,
            &field.adaptation,
            &field.precision,
            &field.prediction_error,
            &field.eligibility,
            &field.metabolic_reserve,
        ]
        .into_iter()
        .flatten()
        .all(|value| (Fixed::ZERO..=Fixed::ONE).contains(value))
}

/// Multiply FXP6 values with symmetric round-to-nearest, ties away from zero.
fn mul_fxp6(left: i64, right: i64) -> Result<i64, SemanticCoreError> {
    let product = i128::from(left)
        .checked_mul(i128::from(right))
        .ok_or(SemanticCoreError::DynamicsInvalid)?;
    let magnitude = product
        .checked_abs()
        .and_then(|value| value.checked_add(i128::from(SCALE / 2)))
        .ok_or(SemanticCoreError::DynamicsInvalid)?
        / i128::from(SCALE);
    let signed = if product < 0 {
        magnitude
            .checked_neg()
            .ok_or(SemanticCoreError::DynamicsInvalid)?
    } else {
        magnitude
    };
    i64::try_from(signed).map_err(|_| SemanticCoreError::DynamicsInvalid)
}

fn pow_fxp6(mut base: i64, mut exponent: u64) -> Result<i64, SemanticCoreError> {
    if !(0..=SCALE).contains(&base) {
        return Err(SemanticCoreError::DynamicsInvalid);
    }
    let mut result = SCALE;
    while exponent != 0 {
        if exponent & 1 == 1 {
            result = mul_fxp6(result, base)?;
        }
        exponent >>= 1;
        if exponent != 0 {
            base = mul_fxp6(base, base)?;
        }
    }
    Ok(result)
}

fn combined_factor(
    epoch: &MatrixTimeEpochV1,
    coefficient: impl Fn(PhaseCoefficientsV1) -> i64,
) -> Result<i64, SemanticCoreError> {
    // This order is part of the formula contract.  Event arrival order cannot
    // influence rounding because every view is recomputed from the anchor.
    let awake = pow_fxp6(coefficient(AWAKE), epoch.awake_ticks)?;
    let drowsy = pow_fxp6(coefficient(DROWSY), epoch.drowsy_ticks)?;
    let asleep = pow_fxp6(coefficient(ASLEEP), epoch.asleep_ticks)?;
    mul_fxp6(mul_fxp6(awake, drowsy)?, asleep)
}

fn relax(anchor: i64, target: i64, factor: i64) -> Result<Fixed, SemanticCoreError> {
    let displacement = anchor
        .checked_sub(target)
        .ok_or(SemanticCoreError::DynamicsInvalid)?;
    let retained = mul_fxp6(displacement, factor)?;
    let next = target
        .checked_add(retained)
        .ok_or(SemanticCoreError::DynamicsInvalid)?;
    if !(0..=SCALE).contains(&next) {
        return Err(SemanticCoreError::DynamicsInvalid);
    }
    Ok(Fixed::from_raw(next))
}

pub fn advance_matrix_time_v1(
    input: MatrixTimeInputV1<'_>,
) -> Result<MatrixTimeAdvanceV1, SemanticCoreError> {
    if !validate_unit_field(input.anchor_field)
        || !validate_unit_field(input.genesis_baseline)
        || input.epoch.schema_version != MatrixTimeEpochV1::SCHEMA_VERSION
        || input.semantic_formula_digest.iter().all(|byte| *byte == 0)
        || state_digest(input.anchor_field, &input.semantic_formula_digest)
            != input.epoch.anchor_state_digest
        || input.epoch.awake_remainder_ms >= MATRIX_TIME_QUANTUM_MS
        || input.epoch.drowsy_remainder_ms >= MATRIX_TIME_QUANTUM_MS
        || input.epoch.asleep_remainder_ms >= MATRIX_TIME_QUANTUM_MS
    {
        return Err(SemanticCoreError::FieldStateInvalid);
    }

    let capped_gap = input.elapsed_ms > MATRIX_TIME_MAX_ELAPSED_MS;
    let applied_elapsed_ms = input.elapsed_ms.min(MATRIX_TIME_MAX_ELAPSED_MS);
    let selected_remainder = match input.phase {
        MatrixSleepPhaseV1::Awake => input.epoch.awake_remainder_ms,
        MatrixSleepPhaseV1::Drowsy => input.epoch.drowsy_remainder_ms,
        MatrixSleepPhaseV1::Asleep => input.epoch.asleep_remainder_ms,
    };
    let accumulated = selected_remainder
        .checked_add(applied_elapsed_ms)
        .ok_or(SemanticCoreError::DynamicsInvalid)?;
    let step_count = accumulated / MATRIX_TIME_QUANTUM_MS;
    if step_count > MATRIX_TIME_MAX_STEPS_PER_EVENT {
        return Err(SemanticCoreError::DynamicsInvalid);
    }
    let mut epoch = input.epoch.clone();
    let (selected_ticks, selected_remainder) = match input.phase {
        MatrixSleepPhaseV1::Awake => (&mut epoch.awake_ticks, &mut epoch.awake_remainder_ms),
        MatrixSleepPhaseV1::Drowsy => (&mut epoch.drowsy_ticks, &mut epoch.drowsy_remainder_ms),
        MatrixSleepPhaseV1::Asleep => (&mut epoch.asleep_ticks, &mut epoch.asleep_remainder_ms),
    };
    *selected_remainder = accumulated % MATRIX_TIME_QUANTUM_MS;
    *selected_ticks = selected_ticks
        .checked_add(step_count)
        .ok_or(SemanticCoreError::DynamicsInvalid)?;

    let normal_factor = combined_factor(&epoch, |value| value.normal)?;
    let adaptation_factor = combined_factor(&epoch, |value| value.adaptation)?;
    let reserve_factor = combined_factor(&epoch, |value| value.reserve_gap)?;
    let mut field = input.anchor_field.clone();
    for node in 0..NEURON_SLOTS {
        field.potential[node] = relax(
            input.anchor_field.potential[node].raw(),
            input.genesis_baseline.potential[node].raw(),
            normal_factor,
        )?;
        field.excitation[node] = relax(
            input.anchor_field.excitation[node].raw(),
            input.genesis_baseline.excitation[node].raw(),
            normal_factor,
        )?;
        field.inhibition[node] = relax(
            input.anchor_field.inhibition[node].raw(),
            input.genesis_baseline.inhibition[node].raw(),
            normal_factor,
        )?;
        field.adaptation[node] = relax(
            input.anchor_field.adaptation[node].raw(),
            input.genesis_baseline.adaptation[node].raw(),
            adaptation_factor,
        )?;
        field.precision[node] = relax(
            input.anchor_field.precision[node].raw(),
            input.genesis_baseline.precision[node].raw(),
            normal_factor,
        )?;
        field.prediction_error[node] = relax(
            input.anchor_field.prediction_error[node].raw(),
            input.genesis_baseline.prediction_error[node].raw(),
            normal_factor,
        )?;
        field.eligibility[node] = relax(
            input.anchor_field.eligibility[node].raw(),
            input.genesis_baseline.eligibility[node].raw(),
            normal_factor,
        )?;
        // Reserve is a capacity, not a Genesis personality dimension.  Its
        // unique equilibrium is fully recovered regardless of old fixtures.
        field.metabolic_reserve[node] = relax(
            input.anchor_field.metabolic_reserve[node].raw(),
            Fixed::ONE.raw(),
            reserve_factor,
        )?;
    }

    Ok(MatrixTimeAdvanceV1 {
        field,
        epoch,
        applied_elapsed_ms,
        step_count,
        capped_gap,
    })
}
