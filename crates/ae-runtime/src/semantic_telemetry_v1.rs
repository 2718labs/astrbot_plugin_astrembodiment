#![forbid(unsafe_code)]

//! Canonical Phase 0 telemetry construction.
//!
//! The receipt is built exclusively from an already-prepared native dynamics
//! result.  No caller can submit headroom, residual, capacity, or gate values.

use ae_attention::r7::FullVectorLoad;
use ae_contracts::{
    wire, CapacityTelemetryV1, Digest, EnergyTelemetryV1, InvariantResiduals,
    NativeTelemetryFormulaV1, NativeTelemetryPhaseV1, NativeTelemetryReceiptV1,
    NATIVE_TELEMETRY_RECEIPT_SCHEMA_V1,
};
use ae_fixed::Fixed;
use ae_neurofield::{EDGE_CAPACITY, NEURON_SLOTS, REGION_LAYOUT};

use crate::semantic_dynamics_v2::{
    ratio6_raw, PreparedSemanticDynamicsV2, ADAPTATION_RATE_FXP6, DYNAMICS_FORMULA_V2,
    ENERGY_COST_RATE_FXP6, NEUTRAL_RATE_FXP6, PROPAGATION_RATE_FXP6, RESERVE_RECOVERY_RATE_FXP6,
};
use crate::RuntimeError;

pub const PHASE0_FORMULA_DIGEST_DOMAIN_V1: &[u8] =
    b"astr-embodiment/phase0-native-propagation-fxp6-v1";
const LOCAL_VECTOR_DIGEST_DOMAIN_V1: &[u8] = b"astr-embodiment/phase0-local-vector-v1";
const COMPENSATION_VECTOR_DIGEST_DOMAIN_V1: &[u8] =
    b"astr-embodiment/phase0-compensation-vector-v1";
const EFFECTIVE_VECTOR_DIGEST_DOMAIN_V1: &[u8] = b"astr-embodiment/phase0-effective-vector-v1";

pub(crate) fn phase0_formula_digest_v1(
    genesis_formula_digest: &Digest,
    route_digest: &Digest,
) -> Digest {
    let mut constants = Vec::with_capacity(8 * 5 + 32);
    constants.extend_from_slice(b"graph-formula-v1");
    for value in [
        PROPAGATION_RATE_FXP6,
        NEUTRAL_RATE_FXP6,
        ADAPTATION_RATE_FXP6,
        RESERVE_RECOVERY_RATE_FXP6,
        ENERGY_COST_RATE_FXP6,
    ] {
        constants.extend_from_slice(&value.raw().to_le_bytes());
    }
    wire::domain_hash(
        PHASE0_FORMULA_DIGEST_DOMAIN_V1,
        &[
            genesis_formula_digest,
            route_digest,
            DYNAMICS_FORMULA_V2.as_bytes(),
            &constants,
        ],
    )
}

pub(crate) fn regional_vector_digest(
    domain: &[u8],
    values: &[Fixed; REGION_LAYOUT.len()],
) -> Digest {
    let mut bytes = Vec::with_capacity(values.len() * 8);
    for value in values {
        bytes.extend_from_slice(&value.encode());
    }
    wire::domain_hash(domain, &[&bytes])
}

pub(crate) fn local_vector_digest(values: &[Fixed; REGION_LAYOUT.len()]) -> Digest {
    regional_vector_digest(LOCAL_VECTOR_DIGEST_DOMAIN_V1, values)
}

pub(crate) fn compensation_vector_digest(values: &[Fixed; REGION_LAYOUT.len()]) -> Digest {
    regional_vector_digest(COMPENSATION_VECTOR_DIGEST_DOMAIN_V1, values)
}

pub(crate) fn effective_vector_digest(values: &[Fixed; REGION_LAYOUT.len()]) -> Digest {
    regional_vector_digest(EFFECTIVE_VECTOR_DIGEST_DOMAIN_V1, values)
}

fn bounded_headroom(used: usize, limit: usize) -> Result<Fixed, RuntimeError> {
    if used > limit {
        return Err(RuntimeError::InvalidNeuralState);
    }
    let used_ratio = ratio6_raw(used, limit).map_err(|_| RuntimeError::InvalidNeuralState)?;
    Ok(Fixed::from_raw(
        Fixed::ONE
            .raw()
            .checked_sub(used_ratio)
            .ok_or(RuntimeError::InvalidNeuralState)?,
    ))
}

/// Seal real telemetry after both the sparse graph and the next field are
/// known. Capacity reads only structural counts: active nodes are deliberately
/// not a capacity proxy.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_native_telemetry_v1(
    formula_digest: Digest,
    scope_digest: Digest,
    event_digest: Digest,
    source_digest: Digest,
    base_revision: u64,
    next_revision: u64,
    state_before: Digest,
    state_after: Digest,
    graph_before: Digest,
    graph_after: Digest,
    local_by_region: &[Fixed; REGION_LAYOUT.len()],
    compensation_by_region: &[Fixed; REGION_LAYOUT.len()],
    dynamics: &PreparedSemanticDynamicsV2,
    full_vector_load: &FullVectorLoad,
) -> Result<NativeTelemetryReceiptV1, RuntimeError> {
    if full_vector_load.evaluated_dimension_count != 15
        || full_vector_load.injected_dimension_count != 15
        || next_revision
            != base_revision
                .checked_add(1)
                .ok_or(RuntimeError::SemanticRevisionOverflow)?
        || usize::try_from(dynamics.propagated_edge_count)
            .map_err(|_| RuntimeError::InvalidNeuralState)?
            > EDGE_CAPACITY
        || usize::try_from(dynamics.upper_saturated_nodes)
            .map_err(|_| RuntimeError::InvalidNeuralState)?
            > NEURON_SLOTS
    {
        return Err(RuntimeError::InvalidNeuralState);
    }
    let node_headroom = bounded_headroom(
        usize::try_from(dynamics.upper_saturated_nodes)
            .map_err(|_| RuntimeError::InvalidNeuralState)?,
        NEURON_SLOTS,
    )?;
    let edge_headroom = bounded_headroom(
        usize::try_from(dynamics.propagated_edge_count)
            .map_err(|_| RuntimeError::InvalidNeuralState)?,
        EDGE_CAPACITY,
    )?;
    let capacity_headroom = node_headroom.min(edge_headroom);
    let residuals = InvariantResiduals {
        // Structural authority and continuity are checked by the verified
        // proposal/store transaction before this PREPARE object is built.
        authority: Fixed::ZERO,
        continuity: Fixed::ZERO,
        energy: dynamics.energy.residual_mean,
        renormalization: dynamics.renormalization_residual,
        capacity: Fixed::ZERO,
    };
    let largest_residual = [
        residuals.authority,
        residuals.continuity,
        residuals.energy,
        residuals.renormalization,
        residuals.capacity,
    ]
    .into_iter()
    .max()
    .ok_or(RuntimeError::InvalidNeuralState)?;
    let residual_health = Fixed::ONE.saturating_sub(largest_residual);
    let native_gate = dynamics
        .energy
        .reserve_after_min
        .min(capacity_headroom)
        .min(residual_health);
    let receipt = NativeTelemetryReceiptV1 {
        schema: NATIVE_TELEMETRY_RECEIPT_SCHEMA_V1.to_owned(),
        formula: NativeTelemetryFormulaV1::Phase0NativePropagationFxp6V1,
        formula_digest,
        scope_digest,
        event_digest,
        source_digest,
        base_revision,
        next_revision,
        phase: NativeTelemetryPhaseV1::Prepare,
        state_before,
        state_after,
        graph_before,
        graph_after,
        local_digest: local_vector_digest(local_by_region),
        compensation_digest: compensation_vector_digest(compensation_by_region),
        effective_digest: effective_vector_digest(&dynamics.effective_by_region),
        energy: EnergyTelemetryV1 {
            reserve_before: dynamics.energy.reserve_before_min,
            reserve_after: dynamics.energy.reserve_after_min,
            recovered: dynamics.energy.recovered_mean,
            spent: dynamics.energy.spent_mean,
            headroom: dynamics.energy.reserve_after_min,
            residual: dynamics.energy.residual_mean,
        },
        capacity: CapacityTelemetryV1 {
            upper_saturated_nodes: dynamics.upper_saturated_nodes,
            node_limit: u32::try_from(NEURON_SLOTS)
                .map_err(|_| RuntimeError::InvalidNeuralState)?,
            node_headroom,
            edge_used: dynamics.propagated_edge_count,
            edge_limit: u32::try_from(EDGE_CAPACITY)
                .map_err(|_| RuntimeError::InvalidNeuralState)?,
            edge_headroom,
            headroom: capacity_headroom,
            residual: Fixed::ZERO,
        },
        residuals,
        residual_health,
        native_gate,
        checkpoint_digest: [0; 32],
        telemetry_digest: [0; 32],
    }
    .seal();
    if !receipt.validate() {
        return Err(RuntimeError::InvalidNeuralState);
    }
    Ok(receipt)
}
