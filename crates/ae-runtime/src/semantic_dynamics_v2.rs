#![forbid(unsafe_code)]

//! Deterministic Phase 0 sparse semantic dynamics.
//!
//! This module deliberately works on the immutable input field for an entire
//! step (Jacobi semantics).  In particular, an edge never observes a source
//! value which was changed earlier in the same traversal.

use ae_fixed::Fixed;
use ae_neurofield::{NeuralField, SparseGraph, EDGE_CAPACITY, NEURON_SLOTS, REGION_LAYOUT};

pub const FXP6_SCALE: i64 = 1_000_000;
pub const PROPAGATION_RATE_FXP6: Fixed = Fixed::from_raw(125_000);
pub const NEUTRAL_RATE_FXP6: Fixed = Fixed::from_raw(125_000);
pub const ADAPTATION_RATE_FXP6: Fixed = Fixed::from_raw(125_000);
pub const RESERVE_RECOVERY_RATE_FXP6: Fixed = Fixed::from_raw(25_000);
pub const ENERGY_COST_RATE_FXP6: Fixed = Fixed::from_raw(100_000);
pub const DYNAMICS_FORMULA_V2: &str = "phase0-native-propagation-fxp6-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DynamicsError {
    InvalidInput,
    Arithmetic,
}

/// Inputs are all fixed-point values. `local_by_region` and
/// `local_confidence_by_region` are independent: a global estimator confidence
/// must never be silently expanded into a per-region confidence vector.
#[derive(Clone, Debug)]
pub struct DynamicsInputV2<'a> {
    pub field: &'a NeuralField,
    pub baseline: &'a NeuralField,
    pub graph: &'a SparseGraph,
    pub local_by_region: [Fixed; REGION_LAYOUT.len()],
    pub local_confidence_by_region: [Fixed; REGION_LAYOUT.len()],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnergyLedgerV1 {
    pub reserve_before_min: Fixed,
    pub reserve_after_min: Fixed,
    pub recovered_mean: Fixed,
    pub spent_mean: Fixed,
    pub residual_mean: Fixed,
}

#[derive(Clone, Debug)]
pub struct PreparedSemanticDynamicsV2 {
    pub next_field: NeuralField,
    pub effective_by_region: [Fixed; REGION_LAYOUT.len()],
    pub direct_by_region: [Fixed; REGION_LAYOUT.len()],
    pub propagated_edge_count: u32,
    pub upper_saturated_nodes: u32,
    pub energy: EnergyLedgerV1,
    pub renormalization_residual: Fixed,
}

fn checked_i64(value: i128) -> Result<i64, DynamicsError> {
    i64::try_from(value).map_err(|_| DynamicsError::Arithmetic)
}

fn fixed_from_i128(value: i128) -> Result<Fixed, DynamicsError> {
    Ok(Fixed::from_raw(checked_i64(value)?))
}

fn abs_i128(value: i128) -> Result<i128, DynamicsError> {
    value.checked_abs().ok_or(DynamicsError::Arithmetic)
}

fn require_unit(value: Fixed) -> Result<(), DynamicsError> {
    if (Fixed::ZERO..=Fixed::ONE).contains(&value) {
        Ok(())
    } else {
        Err(DynamicsError::InvalidInput)
    }
}

fn validate_field(field: &NeuralField) -> Result<(), DynamicsError> {
    if !field.validate() {
        return Err(DynamicsError::InvalidInput);
    }
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
        for value in values {
            require_unit(*value)?;
        }
    }
    Ok(())
}

fn clamp_i128(value: i128, lower: i128, upper: i128) -> Result<(Fixed, i128), DynamicsError> {
    if lower > upper {
        return Err(DynamicsError::InvalidInput);
    }
    let clamped = value.clamp(lower, upper);
    let loss = abs_i128(
        value
            .checked_sub(clamped)
            .ok_or(DynamicsError::Arithmetic)?,
    )?;
    Ok((fixed_from_i128(clamped)?, loss))
}

fn clamp_unit(value: i128) -> Result<(Fixed, i128), DynamicsError> {
    clamp_i128(value, 0, i128::from(FXP6_SCALE))
}

fn clamp_signed_unit(value: i128) -> Result<(Fixed, i128), DynamicsError> {
    clamp_i128(value, -i128::from(FXP6_SCALE), i128::from(FXP6_SCALE))
}

/// Fixed-point product with the Phase 0 positive-rounding rule.
pub fn mul6_raw(left: i64, right: i64) -> Result<i64, DynamicsError> {
    let product = i128::from(left)
        .checked_mul(i128::from(right))
        .ok_or(DynamicsError::Arithmetic)?;
    let rounded = product
        .checked_add(500_000)
        .ok_or(DynamicsError::Arithmetic)?
        / i128::from(FXP6_SCALE);
    checked_i64(rounded)
}

/// Signed fixed-point product, rounded away from zero by magnitude.
pub fn smul6_raw(value: i64, gain: i64) -> Result<i64, DynamicsError> {
    if !(0..=FXP6_SCALE).contains(&gain) {
        return Err(DynamicsError::InvalidInput);
    }
    let magnitude = i128::from(value)
        .checked_abs()
        .ok_or(DynamicsError::Arithmetic)?
        .checked_mul(i128::from(gain))
        .ok_or(DynamicsError::Arithmetic)?
        .checked_add(500_000)
        .ok_or(DynamicsError::Arithmetic)?
        / i128::from(FXP6_SCALE);
    let signed = if value < 0 {
        magnitude.checked_neg().ok_or(DynamicsError::Arithmetic)?
    } else {
        magnitude
    };
    checked_i64(signed)
}

/// Ratio with a half-denominator rounding term. The caller supplies a bounded
/// numerator; this helper never turns an unavailable denominator into zero.
pub fn ratio6_raw(numerator: usize, denominator: usize) -> Result<i64, DynamicsError> {
    if denominator == 0 {
        return Err(DynamicsError::InvalidInput);
    }
    let numerator = i128::try_from(numerator).map_err(|_| DynamicsError::Arithmetic)?;
    let denominator = i128::try_from(denominator).map_err(|_| DynamicsError::Arithmetic)?;
    let scaled = numerator
        .checked_mul(i128::from(FXP6_SCALE))
        .ok_or(DynamicsError::Arithmetic)?
        .checked_add(denominator / 2)
        .ok_or(DynamicsError::Arithmetic)?
        / denominator;
    checked_i64(scaled)
}

fn signed_round_div(numerator: i128, denominator: i128) -> Result<i128, DynamicsError> {
    if denominator <= 0 {
        return Err(DynamicsError::InvalidInput);
    }
    let magnitude = abs_i128(numerator)?
        .checked_add(denominator / 2)
        .ok_or(DynamicsError::Arithmetic)?
        / denominator;
    if numerator < 0 {
        magnitude.checked_neg().ok_or(DynamicsError::Arithmetic)
    } else {
        Ok(magnitude)
    }
}

fn region_index(node: usize) -> Result<usize, DynamicsError> {
    REGION_LAYOUT
        .iter()
        .position(|(start, count)| node >= *start && node < start.saturating_add(*count))
        .ok_or(DynamicsError::InvalidInput)
}

fn average_fixed(sum: i128, count: usize) -> Result<Fixed, DynamicsError> {
    let count = i128::try_from(count)
        .map_err(|_| DynamicsError::Arithmetic)?
        .max(1);
    fixed_from_i128(
        sum.checked_add(count / 2)
            .ok_or(DynamicsError::Arithmetic)?
            / count,
    )
}

fn update_max_loss(maximum: &mut i128, loss: i128) {
    *maximum = (*maximum).max(loss);
}

/// Prepare one actual sparse-edge propagation step. No mutation is made to the
/// input field or graph; callers can safely derive receipts before committing.
pub fn propagate_semantic_dynamics_v2(
    input: DynamicsInputV2<'_>,
) -> Result<PreparedSemanticDynamicsV2, DynamicsError> {
    validate_field(input.field)?;
    validate_field(input.baseline)?;
    if !input.graph.validate() || input.graph.edges.len() > EDGE_CAPACITY {
        return Err(DynamicsError::InvalidInput);
    }

    let mut effective_by_region = [Fixed::ZERO; REGION_LAYOUT.len()];
    let mut direct_by_region = [Fixed::ZERO; REGION_LAYOUT.len()];
    let mut maximum_clamp_loss = 0_i128;
    for region in 0..REGION_LAYOUT.len() {
        require_unit(input.local_by_region[region])?;
        require_unit(input.local_confidence_by_region[region])?;
        let effective = input.local_by_region[region];
        effective_by_region[region] = effective;
        direct_by_region[region] = Fixed::from_raw(mul6_raw(
            effective.raw(),
            input.local_confidence_by_region[region].raw(),
        )?);
    }

    // Every source is computed before any next-field value is written.
    let mut source = vec![0_i64; NEURON_SLOTS];
    for (node, source_slot) in source.iter_mut().enumerate() {
        let raw = i128::from(input.field.potential[node].raw())
            .checked_add(i128::from(input.field.excitation[node].raw()))
            .and_then(|value| value.checked_sub(i128::from(input.field.inhibition[node].raw())))
            .ok_or(DynamicsError::Arithmetic)?;
        *source_slot = clamp_signed_unit(raw)?.0.raw();
    }

    let mut weighted = vec![0_i128; NEURON_SLOTS];
    let mut mass = vec![0_i128; NEURON_SLOTS];
    for node in 0..NEURON_SLOTS {
        let start = usize::try_from(input.graph.row_offsets[node])
            .map_err(|_| DynamicsError::InvalidInput)?;
        let end = usize::try_from(input.graph.row_offsets[node + 1])
            .map_err(|_| DynamicsError::InvalidInput)?;
        for edge in &input.graph.edges[start..end] {
            let target = usize::try_from(edge.target).map_err(|_| DynamicsError::InvalidInput)?;
            let weight = i128::from(edge.weight)
                .checked_mul(1_000)
                .ok_or(DynamicsError::Arithmetic)?;
            weighted[target] = weighted[target]
                .checked_add(
                    i128::from(source[node])
                        .checked_mul(weight)
                        .ok_or(DynamicsError::Arithmetic)?,
                )
                .ok_or(DynamicsError::Arithmetic)?;
            mass[target] = mass[target]
                .checked_add(abs_i128(weight)?)
                .ok_or(DynamicsError::Arithmetic)?;
        }
    }

    let mut next_field = input.field.clone();
    let mut reserve_before_min = Fixed::ONE;
    let mut reserve_after_min = Fixed::ONE;
    let mut recovered_sum = 0_i128;
    let mut spent_sum = 0_i128;
    let mut energy_residual_sum = 0_i128;
    let mut upper_saturated_nodes = 0_usize;

    for node in 0..NEURON_SLOTS {
        let region = region_index(node)?;
        let edge_mean = if mass[node] == 0 {
            0
        } else {
            checked_i64(signed_round_div(weighted[node], mass[node])?)?
        };
        let edge_drive = smul6_raw(edge_mean, PROPAGATION_RATE_FXP6.raw())?;
        let requested_drive = i128::from(direct_by_region[region].raw())
            .checked_add(i128::from(edge_drive))
            .ok_or(DynamicsError::Arithmetic)?;
        let (drive, drive_loss) = clamp_signed_unit(requested_drive)?;
        update_max_loss(&mut maximum_clamp_loss, drive_loss);

        let displacement = i128::from(input.field.potential[node].raw())
            .checked_sub(i128::from(input.baseline.potential[node].raw()))
            .ok_or(DynamicsError::Arithmetic)?;
        let recovery = smul6_raw(checked_i64(displacement)?, NEUTRAL_RATE_FXP6.raw())?;
        let potential_unclamped = i128::from(input.field.potential[node].raw())
            .checked_add(i128::from(drive.raw()))
            .and_then(|value| value.checked_sub(i128::from(recovery)))
            .ok_or(DynamicsError::Arithmetic)?;
        let (potential, potential_loss) = clamp_unit(potential_unclamped)?;
        update_max_loss(&mut maximum_clamp_loss, potential_loss);

        let (excitation, excitation_loss) = clamp_unit(i128::from(drive.raw()).max(0))?;
        update_max_loss(&mut maximum_clamp_loss, excitation_loss);
        let (inhibition, inhibition_loss) = clamp_unit(
            i128::from(drive.raw())
                .checked_neg()
                .ok_or(DynamicsError::Arithmetic)?
                .max(0),
        )?;
        update_max_loss(&mut maximum_clamp_loss, inhibition_loss);
        let prediction_raw = abs_i128(
            i128::from(drive.raw())
                .checked_sub(i128::from(recovery))
                .ok_or(DynamicsError::Arithmetic)?,
        )?;
        let (prediction_error, prediction_loss) = clamp_unit(prediction_raw)?;
        update_max_loss(&mut maximum_clamp_loss, prediction_loss);
        let precision = input.local_confidence_by_region[region];
        let adaptation_delta = abs_i128(
            i128::from(potential.raw())
                .checked_sub(i128::from(input.baseline.potential[node].raw()))
                .ok_or(DynamicsError::Arithmetic)?,
        )?
        .checked_sub(i128::from(input.field.adaptation[node].raw()))
        .ok_or(DynamicsError::Arithmetic)?;
        let adaptation_adjustment =
            smul6_raw(checked_i64(adaptation_delta)?, ADAPTATION_RATE_FXP6.raw())?;
        let (adaptation, adaptation_loss) = clamp_unit(
            i128::from(input.field.adaptation[node].raw())
                .checked_add(i128::from(adaptation_adjustment))
                .ok_or(DynamicsError::Arithmetic)?,
        )?;
        update_max_loss(&mut maximum_clamp_loss, adaptation_loss);
        let eligibility_numerator = i128::from(input.field.eligibility[node].raw())
            .checked_add(i128::from(prediction_error.raw()))
            .and_then(|value| value.checked_add(1))
            .ok_or(DynamicsError::Arithmetic)?;
        let (eligibility, eligibility_loss) = clamp_unit(eligibility_numerator / 2)?;
        update_max_loss(&mut maximum_clamp_loss, eligibility_loss);

        let delta_sum = [
            abs_i128(i128::from(potential.raw()) - i128::from(input.field.potential[node].raw()))?,
            abs_i128(
                i128::from(excitation.raw()) - i128::from(input.field.excitation[node].raw()),
            )?,
            abs_i128(
                i128::from(inhibition.raw()) - i128::from(input.field.inhibition[node].raw()),
            )?,
            abs_i128(
                i128::from(adaptation.raw()) - i128::from(input.field.adaptation[node].raw()),
            )?,
            abs_i128(
                i128::from(prediction_error.raw())
                    - i128::from(input.field.prediction_error[node].raw()),
            )?,
            abs_i128(
                i128::from(eligibility.raw()) - i128::from(input.field.eligibility[node].raw()),
            )?,
        ]
        .into_iter()
        .try_fold(0_i128, |total, delta| {
            total.checked_add(delta).ok_or(DynamicsError::Arithmetic)
        })?;
        let work = delta_sum.checked_add(5).ok_or(DynamicsError::Arithmetic)? / 6;
        let reserve_before = input.field.metabolic_reserve[node];
        let recovered = mul6_raw(
            checked_i64(i128::from(FXP6_SCALE) - i128::from(reserve_before.raw()))?,
            RESERVE_RECOVERY_RATE_FXP6.raw(),
        )?;
        let spent = mul6_raw(checked_i64(work)?, ENERGY_COST_RATE_FXP6.raw())?;
        let reserve_unclamped = i128::from(reserve_before.raw())
            .checked_add(i128::from(recovered))
            .and_then(|value| value.checked_sub(i128::from(spent)))
            .ok_or(DynamicsError::Arithmetic)?;
        let (reserve, reserve_loss) = clamp_unit(reserve_unclamped)?;
        update_max_loss(&mut maximum_clamp_loss, reserve_loss);
        energy_residual_sum = energy_residual_sum
            .checked_add(reserve_loss)
            .ok_or(DynamicsError::Arithmetic)?;
        recovered_sum = recovered_sum
            .checked_add(i128::from(recovered))
            .ok_or(DynamicsError::Arithmetic)?;
        spent_sum = spent_sum
            .checked_add(i128::from(spent))
            .ok_or(DynamicsError::Arithmetic)?;
        reserve_before_min = reserve_before_min.min(reserve_before);
        reserve_after_min = reserve_after_min.min(reserve);

        next_field.potential[node] = potential;
        next_field.excitation[node] = excitation;
        next_field.inhibition[node] = inhibition;
        next_field.adaptation[node] = adaptation;
        next_field.precision[node] = precision;
        next_field.prediction_error[node] = prediction_error;
        next_field.eligibility[node] = eligibility;
        next_field.metabolic_reserve[node] = reserve;
        // Capacity saturation is a signal-bound measurement.  Reserve is a
        // separate energy ledger: a fully charged fresh field must not make
        // every node look structurally saturated.
        if [
            potential,
            excitation,
            inhibition,
            adaptation,
            prediction_error,
            eligibility,
        ]
        .into_iter()
        .any(|value| value == Fixed::ONE)
        {
            // Precision is an input-certainty control, not occupied neural
            // capacity. A fully confident literal estimate must not fabricate
            // a field-wide saturation signal.
            upper_saturated_nodes = upper_saturated_nodes
                .checked_add(1)
                .ok_or(DynamicsError::Arithmetic)?;
        }
    }

    validate_field(&next_field)?;
    let propagated_edge_count =
        u32::try_from(input.graph.edges.len()).map_err(|_| DynamicsError::Arithmetic)?;
    let upper_saturated_nodes =
        u32::try_from(upper_saturated_nodes).map_err(|_| DynamicsError::Arithmetic)?;
    Ok(PreparedSemanticDynamicsV2 {
        next_field,
        effective_by_region,
        direct_by_region,
        propagated_edge_count,
        upper_saturated_nodes,
        energy: EnergyLedgerV1 {
            reserve_before_min,
            reserve_after_min,
            recovered_mean: average_fixed(recovered_sum, NEURON_SLOTS)?,
            spent_mean: average_fixed(spent_sum, NEURON_SLOTS)?,
            residual_mean: average_fixed(energy_residual_sum, NEURON_SLOTS)?,
        },
        renormalization_residual: Fixed::from_raw(checked_i64(
            maximum_clamp_loss.min(i128::from(FXP6_SCALE)),
        )?),
    })
}
