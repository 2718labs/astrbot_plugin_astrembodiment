#![forbid(unsafe_code)]

use crate::RuntimeError;
use ae_attention::r7::{assemble_full_vector_load, FullVectorLoad};
use ae_contracts::{
    phase0_canonical_formula_digest_v1, wire, CommitStatus, EvidenceVector,
    NativeTelemetryReceiptV1, PerceptionProposalV1, SemanticVectorFormulaV2,
    SemanticVectorReceiptV2, StateSubcodeV1, TransitionReceipt, TransitionReceiptV2,
};
pub use ae_contracts::{
    NodeObservabilityComponentV1, NodeObservabilityCountsV1, NodeObservabilityProjectionWireV2,
    NodeObservabilityRegionV1, NodeObservabilityResidualStateV1, NodeObservabilityResidualsV1,
};
use ae_fixed::Fixed;
use ae_neurofield::{
    develop_graph, graph_digest, state_digest, GraphFormula, NeuralField, SparseGraph, Synapse,
    EDGE_CAPACITY, NEURON_SLOTS, REGION_LAYOUT,
};

use crate::semantic_dynamics_v2::{
    propagate_semantic_dynamics_v2, DynamicsError, DynamicsInputV2, PreparedSemanticDynamicsV2,
};

const SNAPSHOT_MAGIC_V2: &[u8] = b"AESEM2\0";
const SNAPSHOT_SCHEMA_V2: u16 = 2;
const SNAPSHOT_MAGIC_V3: &[u8] = b"AESEM3\0";
const SNAPSHOT_SCHEMA_V3: u16 = 3;
const EXPRESSION_FXP6_MAX: u32 = 1_000_000;
/// Frozen predecessor relaxation rate used only to authenticate AESEM2
/// history.  New writes always use the Phase-0 sparse dynamics below.
const LEGACY_NEUTRAL_RELAXATION_MAX_RATE: Fixed = Fixed::from_raw(125_000);
pub(crate) const LEGACY_FIELD_FXP6_SCALE: i64 = 1_000_000;
const REGION_NAMES: [&str; 9] = [
    "interoception_allostasis",
    "affective_valuation",
    "salience",
    "epistemic_fallibility",
    "social_boundary",
    "temper_inhibitory",
    "world_model_imagination",
    "global_workspace",
    "action_expression",
];

fn invalid_neural_state(subcode: StateSubcodeV1) -> RuntimeError {
    RuntimeError::invalid_neural_state(subcode)
}

fn state_subcode_for_dynamics_error(error: DynamicsError) -> StateSubcodeV1 {
    match error {
        DynamicsError::FieldStateInvalid => StateSubcodeV1::FieldStateInvalid,
        DynamicsError::GraphStateInvalid => StateSubcodeV1::GraphStateInvalid,
        DynamicsError::InvalidInput | DynamicsError::Arithmetic => StateSubcodeV1::DynamicsInvalid,
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedSemanticTransitionV2 {
    pub next_field: NeuralField,
    pub next_graph: SparseGraph,
    pub active_nodes: u32,
    pub full_vector_load: FullVectorLoad,
    pub local_by_region: [Fixed; REGION_LAYOUT.len()],
    pub dynamics: PreparedSemanticDynamicsV2,
}

/// The exact AESEM2 writer is retained solely for deterministic historical
/// attestation.  It is deliberately not a production write path.
#[derive(Clone, Debug)]
pub(crate) struct PreparedLegacyAesem2TransitionV1 {
    pub next_field: NeuralField,
    pub active_nodes: u32,
}

/// Aggregate, non-secret facts bound into the one-time field-domain receipt.
/// These are intentionally not per-node diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LegacyFieldDomainNormalizationV1 {
    pub source_common_max: i64,
    pub out_of_range_count: u32,
    pub potential_out_of_range_count: u32,
    pub excitation_out_of_range_count: u32,
    pub signal_mass_before: i128,
    pub signal_mass_after: i128,
}

#[derive(Clone, Debug)]
pub struct ExpressionProfileFxP6 {
    pub warmth: u32,
    pub sensitivity: u32,
    pub guardedness: u32,
    pub repair_orientation: u32,
    pub engagement: u32,
    pub epistemic_caution: u32,
}

#[derive(Clone, Debug)]
pub struct ExpressionProjectionV1 {
    pub revision: u64,
    pub profile_fxp6: ExpressionProfileFxP6,
}

fn legacy_full_vector_component_update(
    current: Fixed,
    baseline: Fixed,
    drive: Fixed,
    neutral_rate: Fixed,
) -> Result<(Fixed, Fixed), RuntimeError> {
    let displacement = current.saturating_sub(baseline);
    let recovery = displacement
        .checked_mul(neutral_rate)
        .ok_or_else(|| invalid_neural_state(StateSubcodeV1::DynamicsInvalid))?;
    Ok((
        current.saturating_add(drive).saturating_sub(recovery),
        recovery,
    ))
}

/// Reproduce the frozen AESEM2 writer exactly enough to authenticate every
/// persisted predecessor transition.  This function intentionally permits
/// finite P/E values outside the current unit domain; all other validation is
/// limited to the old writer's shape and proposal rules.
#[cfg(test)]
pub(crate) fn prepare_legacy_aesem2_transition_v1(
    field: &NeuralField,
    baseline: &NeuralField,
    proposal: &PerceptionProposalV1,
) -> Result<PreparedLegacyAesem2TransitionV1, RuntimeError> {
    proposal
        .validate_v1()
        .map_err(|_| RuntimeError::InvalidPerceptionProposal)?;
    replay_legacy_aesem2_transition_v1(
        field,
        baseline,
        &proposal.dimensions,
        proposal.estimator_confidence,
    )
}

pub(crate) fn replay_legacy_aesem2_transition_v1(
    field: &NeuralField,
    baseline: &NeuralField,
    dimensions: &EvidenceVector,
    estimator_confidence: Fixed,
) -> Result<PreparedLegacyAesem2TransitionV1, RuntimeError> {
    if !field.validate()
        || !baseline.validate()
        || !(Fixed::ZERO < estimator_confidence && estimator_confidence <= Fixed::ONE)
    {
        return Err(invalid_neural_state(StateSubcodeV1::FieldStateInvalid));
    }
    let full_vector_load = assemble_full_vector_load(dimensions)
        .map_err(|_| RuntimeError::InvalidPerceptionProposal)?;
    if full_vector_load.evaluated_dimension_count != 15
        || full_vector_load.injected_dimension_count != 15
    {
        return Err(RuntimeError::InvalidPerceptionProposal);
    }

    let mut next_field = field.clone();
    let mut active_nodes = 0_u32;
    for (region, &(start, count)) in REGION_LAYOUT.iter().enumerate() {
        let drive = full_vector_load.evidence_means[region]
            .checked_mul(estimator_confidence)
            .ok_or_else(|| invalid_neural_state(StateSubcodeV1::DynamicsInvalid))?;
        let neutral_rate = full_vector_load.neutral_means[region]
            .checked_mul(LEGACY_NEUTRAL_RELAXATION_MAX_RATE)
            .ok_or_else(|| invalid_neural_state(StateSubcodeV1::DynamicsInvalid))?;
        let end = start
            .checked_add(count)
            .filter(|end| *end <= NEURON_SLOTS)
            .ok_or_else(|| invalid_neural_state(StateSubcodeV1::DynamicsInvalid))?;
        for node in start..end {
            let (next_potential, potential_recovery) = legacy_full_vector_component_update(
                field.potential[node],
                baseline.potential[node],
                drive,
                neutral_rate,
            )?;
            let (next_excitation, excitation_recovery) = legacy_full_vector_component_update(
                field.excitation[node],
                baseline.excitation[node],
                drive,
                neutral_rate,
            )?;
            if drive == Fixed::ZERO
                && potential_recovery == Fixed::ZERO
                && excitation_recovery == Fixed::ZERO
            {
                continue;
            }
            active_nodes = active_nodes
                .checked_add(1)
                .ok_or_else(|| invalid_neural_state(StateSubcodeV1::DynamicsInvalid))?;
            next_field.potential[node] = next_potential;
            next_field.excitation[node] = next_excitation;
        }
    }
    if !next_field.validate() {
        return Err(invalid_neural_state(StateSubcodeV1::FieldStateInvalid));
    }
    Ok(PreparedLegacyAesem2TransitionV1 {
        next_field,
        active_nodes,
    })
}

fn checked_joint_scaled_fxp6(value: i64, common_max: i64) -> Result<i64, RuntimeError> {
    let numerator = i128::from(value)
        .checked_mul(i128::from(LEGACY_FIELD_FXP6_SCALE))
        .ok_or_else(|| invalid_neural_state(StateSubcodeV1::FieldStateInvalid))?;
    let denominator = i128::from(common_max);
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    let doubled_remainder = remainder
        .checked_mul(2)
        .ok_or_else(|| invalid_neural_state(StateSubcodeV1::FieldStateInvalid))?;
    let rounded = if doubled_remainder > denominator
        || (doubled_remainder == denominator && quotient % 2 != 0)
    {
        quotient
            .checked_add(1)
            .ok_or_else(|| invalid_neural_state(StateSubcodeV1::FieldStateInvalid))?
    } else {
        quotient
    };
    let scaled = i64::try_from(rounded)
        .map_err(|_| invalid_neural_state(StateSubcodeV1::FieldStateInvalid))?;
    if !(0..=LEGACY_FIELD_FXP6_SCALE).contains(&scaled) {
        return Err(invalid_neural_state(StateSubcodeV1::FieldStateInvalid));
    }
    Ok(scaled)
}

/// Normalize only the one closed AESEM2 legacy overflow shape.  It never
/// clamps values and it never mutates its input.  A return value of `None`
/// means the field is already in the unit domain; every other invalid shape
/// remains fail-closed under the existing field-state subcode.
pub(crate) fn normalize_legacy_aesem2_field_domain_v1(
    field: &NeuralField,
) -> Result<Option<(NeuralField, LegacyFieldDomainNormalizationV1)>, RuntimeError> {
    if !field.validate() {
        return Err(invalid_neural_state(StateSubcodeV1::FieldStateInvalid));
    }
    for values in [
        &field.inhibition,
        &field.adaptation,
        &field.precision,
        &field.prediction_error,
        &field.eligibility,
        &field.metabolic_reserve,
    ] {
        if values
            .iter()
            .any(|value| !(0..=LEGACY_FIELD_FXP6_SCALE).contains(&value.raw()))
        {
            return Err(invalid_neural_state(StateSubcodeV1::FieldStateInvalid));
        }
    }

    let mut common_max = 0_i64;
    let mut out_of_range_count = 0_u32;
    let mut potential_out_of_range_count = 0_u32;
    let mut excitation_out_of_range_count = 0_u32;
    let mut signal_mass_before = 0_i128;
    for (values, component_count) in [
        (&field.potential, &mut potential_out_of_range_count),
        (&field.excitation, &mut excitation_out_of_range_count),
    ] {
        for value in values {
            let raw = value.raw();
            if raw < 0 {
                return Err(invalid_neural_state(StateSubcodeV1::FieldStateInvalid));
            }
            common_max = common_max.max(raw);
            signal_mass_before = signal_mass_before
                .checked_add(i128::from(raw))
                .ok_or_else(|| invalid_neural_state(StateSubcodeV1::FieldStateInvalid))?;
            if raw > LEGACY_FIELD_FXP6_SCALE {
                *component_count = component_count
                    .checked_add(1)
                    .ok_or_else(|| invalid_neural_state(StateSubcodeV1::FieldStateInvalid))?;
                out_of_range_count = out_of_range_count
                    .checked_add(1)
                    .ok_or_else(|| invalid_neural_state(StateSubcodeV1::FieldStateInvalid))?;
            }
        }
    }
    if common_max <= LEGACY_FIELD_FXP6_SCALE {
        return Ok(None);
    }

    let mut normalized = field.clone();
    let mut signal_mass_after = 0_i128;
    for (source, destination) in [
        (&field.potential, &mut normalized.potential),
        (&field.excitation, &mut normalized.excitation),
    ] {
        for (before, after) in source.iter().zip(destination.iter_mut()) {
            let scaled = checked_joint_scaled_fxp6(before.raw(), common_max)?;
            *after = Fixed::from_raw(scaled);
            signal_mass_after = signal_mass_after
                .checked_add(i128::from(scaled))
                .ok_or_else(|| invalid_neural_state(StateSubcodeV1::FieldStateInvalid))?;
        }
    }
    if !normalized.validate()
        || normalized
            .potential
            .iter()
            .chain(normalized.excitation.iter())
            .any(|value| !(0..=LEGACY_FIELD_FXP6_SCALE).contains(&value.raw()))
    {
        return Err(invalid_neural_state(StateSubcodeV1::FieldStateInvalid));
    }
    Ok(Some((
        normalized,
        LegacyFieldDomainNormalizationV1 {
            source_common_max: common_max,
            out_of_range_count,
            potential_out_of_range_count,
            excitation_out_of_range_count,
            signal_mass_before,
            signal_mass_after,
        },
    )))
}

/// Phase 0 preparation materializes the deterministic graph exactly once and
/// then runs the immutable-before sparse-edge dynamics. AESEM2 snapshots are
/// decoded only for replay; new writes use this function.
pub(crate) fn prepare_semantic_transition_v2(
    field: &NeuralField,
    baseline: &NeuralField,
    graph: &SparseGraph,
    manifest_digest: &[u8; 32],
    development_seed_digest: &[u8; 32],
    proposal: &PerceptionProposalV1,
) -> Result<PreparedSemanticTransitionV2, RuntimeError> {
    proposal
        .validate_v1()
        .map_err(|_| RuntimeError::InvalidPerceptionProposal)?;
    let full_vector_load = assemble_full_vector_load(&proposal.dimensions)
        .map_err(|_| RuntimeError::InvalidPerceptionProposal)?;
    if full_vector_load.evaluated_dimension_count != 15
        || full_vector_load.injected_dimension_count != 15
    {
        return Err(RuntimeError::InvalidPerceptionProposal);
    }
    let next_graph = if graph.edges.is_empty() {
        develop_graph(manifest_digest, development_seed_digest, GraphFormula::V1)
            .map_err(|_| invalid_neural_state(StateSubcodeV1::GraphStateInvalid))?
    } else {
        graph.clone()
    };
    if !next_graph.validate() {
        return Err(invalid_neural_state(StateSubcodeV1::GraphStateInvalid));
    }
    let local_by_region = full_vector_load.evidence_means;
    // The Provider proposal and its supplied confidence are the complete
    // native input. No local estimator or secondary vector is merged here.
    let local_confidence_by_region = [proposal.estimator_confidence; REGION_LAYOUT.len()];
    let dynamics = propagate_semantic_dynamics_v2(DynamicsInputV2 {
        field,
        baseline,
        graph: &next_graph,
        local_by_region,
        local_confidence_by_region,
    })
    .map_err(|error| invalid_neural_state(state_subcode_for_dynamics_error(error)))?;
    let active_nodes = u32::try_from(
        (0..NEURON_SLOTS)
            .filter(|node| {
                field.potential[*node] != dynamics.next_field.potential[*node]
                    || field.excitation[*node] != dynamics.next_field.excitation[*node]
                    || field.inhibition[*node] != dynamics.next_field.inhibition[*node]
                    || field.adaptation[*node] != dynamics.next_field.adaptation[*node]
                    || field.precision[*node] != dynamics.next_field.precision[*node]
                    || field.prediction_error[*node] != dynamics.next_field.prediction_error[*node]
                    || field.eligibility[*node] != dynamics.next_field.eligibility[*node]
                    || field.metabolic_reserve[*node]
                        != dynamics.next_field.metabolic_reserve[*node]
            })
            .count(),
    )
    .map_err(|_| invalid_neural_state(StateSubcodeV1::DynamicsInvalid))?;
    Ok(PreparedSemanticTransitionV2 {
        next_field: dynamics.next_field.clone(),
        next_graph,
        active_nodes,
        full_vector_load,
        local_by_region,
        dynamics,
    })
}

/// Formula identity is fixed by the shared route/dynamics contract rather than
/// by a receipt or caller-provided digest.
pub(crate) fn phase0_semantic_formula_digest_v1(
    genesis_formula_digest: &[u8; 32],
) -> Result<[u8; 32], RuntimeError> {
    Ok(phase0_canonical_formula_digest_v1(genesis_formula_digest))
}

pub(crate) fn semantic_vector_receipt_v2(
    legacy: &TransitionReceipt,
    evaluated_dimension_count: u8,
    injected_dimension_count: u8,
    nonzero_evidence_dimension_count: u8,
) -> Result<TransitionReceiptV2, RuntimeError> {
    let neutral_baseline_dimension_count = evaluated_dimension_count
        .checked_sub(nonzero_evidence_dimension_count)
        .ok_or(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?;
    TransitionReceiptV2::from_legacy(
        legacy,
        SemanticVectorReceiptV2 {
            schema_version: SemanticVectorReceiptV2::SCHEMA_VERSION,
            formula: SemanticVectorFormulaV2::FullVectorRouteNeutralRelaxationV1,
            dimension_slot_count: 15,
            evaluated_dimension_count,
            injected_dimension_count,
            nonzero_evidence_dimension_count,
            neutral_baseline_dimension_count,
            unavailable_dimension_count: 0,
            state_changed: legacy.state_before != legacy.state_after,
        },
    )
    .ok_or(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))
}

pub(crate) fn semantic_v2_matches_legacy_receipt(
    semantic_receipt: &TransitionReceiptV2,
    legacy_receipt: &TransitionReceipt,
) -> bool {
    legacy_receipt.schema_version == 1
        && legacy_receipt.status == CommitStatus::Committed
        && legacy_receipt.action_contract.is_none()
        && semantic_receipt.validate()
        && semantic_receipt.formula_digest == legacy_receipt.formula_digest
        && semantic_receipt.scope_digest == legacy_receipt.scope_digest
        && semantic_receipt.event_digest == legacy_receipt.event_digest
        && semantic_receipt.authority_digest == legacy_receipt.authority_digest
        && semantic_receipt.base_revision == legacy_receipt.base_revision
        && semantic_receipt.next_revision == legacy_receipt.next_revision
        && semantic_receipt.state_before == legacy_receipt.state_before
        && semantic_receipt.state_after == legacy_receipt.state_after
        && semantic_receipt.graph_after == legacy_receipt.graph_after
        && semantic_receipt.action_contract == legacy_receipt.action_contract
        && semantic_receipt.active_nodes == legacy_receipt.active_nodes
        && semantic_receipt.active_edges == legacy_receipt.active_edges
        && semantic_receipt.residuals == legacy_receipt.residuals
        && semantic_receipt.status == legacy_receipt.status
}

fn mean_fxp6(sum: i128, count: usize) -> Result<i64, RuntimeError> {
    let count = i128::try_from(count)
        .ok()
        .filter(|count| *count > 0)
        .ok_or(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?;
    i64::try_from(sum / count)
        .map_err(|_| invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))
}

/// Observability for the Phase 0 sparse dynamics.  It reports what was
/// actually changed, rather than re-deriving the retired direct-only v1 rule.
pub(crate) fn node_observability_projection_v2(
    before: &NeuralField,
    after: &NeuralField,
    revision: u64,
) -> Result<NodeObservabilityProjectionWireV2, RuntimeError> {
    if !before.validate() || !after.validate() {
        return Err(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid));
    }
    let mut regions = Vec::with_capacity(REGION_LAYOUT.len());
    let mut selected_total = 0_u32;
    let mut activated_total = 0_u32;
    let mut changed_total = 0_u32;
    let mut potential_nonzero_after_total = 0_u32;
    let mut excitation_nonzero_after_total = 0_u32;
    let mut signal_nonzero_after_total = 0_u32;
    let mut expected_start = 0_usize;

    for (region, &(start, count)) in REGION_LAYOUT.iter().enumerate() {
        let end = start
            .checked_add(count)
            .filter(|end| *end <= NEURON_SLOTS && start == expected_start)
            .ok_or(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?;
        expected_start = end;
        let mut selected = 0_u32;
        let mut activated = 0_u32;
        let mut changed = 0_u32;
        let mut potential_before_sum = 0_i128;
        let mut potential_after_sum = 0_i128;
        let mut potential_delta_sum = 0_i128;
        let mut potential_changed = 0_u32;
        let mut potential_nonzero_after = 0_u32;
        let mut excitation_before_sum = 0_i128;
        let mut excitation_after_sum = 0_i128;
        let mut excitation_delta_sum = 0_i128;
        let mut excitation_changed = 0_u32;
        let mut excitation_nonzero_after = 0_u32;

        for node in start..end {
            let changes = [
                before.potential[node] != after.potential[node],
                before.excitation[node] != after.excitation[node],
                before.inhibition[node] != after.inhibition[node],
                before.adaptation[node] != after.adaptation[node],
                before.precision[node] != after.precision[node],
                before.prediction_error[node] != after.prediction_error[node],
                before.eligibility[node] != after.eligibility[node],
                before.metabolic_reserve[node] != after.metabolic_reserve[node],
            ];
            if changes.into_iter().any(|changed| changed) {
                selected = selected
                    .checked_add(1)
                    .ok_or(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?;
                changed = changed
                    .checked_add(1)
                    .ok_or(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?;
                selected_total = selected_total
                    .checked_add(1)
                    .ok_or(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?;
                changed_total = changed_total
                    .checked_add(1)
                    .ok_or(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?;
            }
            if changes[0] || changes[1] || changes[2] {
                activated = activated
                    .checked_add(1)
                    .ok_or(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?;
                activated_total = activated_total
                    .checked_add(1)
                    .ok_or(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?;
            }
            if changes[0] {
                potential_changed = potential_changed
                    .checked_add(1)
                    .ok_or(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?;
            }
            if changes[1] {
                excitation_changed = excitation_changed
                    .checked_add(1)
                    .ok_or(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?;
            }
            if after.potential[node] != Fixed::ZERO {
                potential_nonzero_after = potential_nonzero_after
                    .checked_add(1)
                    .ok_or(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?;
                potential_nonzero_after_total = potential_nonzero_after_total
                    .checked_add(1)
                    .ok_or(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?;
            }
            if after.excitation[node] != Fixed::ZERO {
                excitation_nonzero_after = excitation_nonzero_after
                    .checked_add(1)
                    .ok_or(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?;
                excitation_nonzero_after_total = excitation_nonzero_after_total
                    .checked_add(1)
                    .ok_or(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?;
            }
            if after.potential[node] != Fixed::ZERO
                || after.excitation[node] != Fixed::ZERO
                || after.inhibition[node] != Fixed::ZERO
            {
                signal_nonzero_after_total = signal_nonzero_after_total
                    .checked_add(1)
                    .ok_or(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?;
            }
            potential_before_sum += i128::from(before.potential[node].raw());
            potential_after_sum += i128::from(after.potential[node].raw());
            potential_delta_sum +=
                i128::from(after.potential[node].raw()) - i128::from(before.potential[node].raw());
            excitation_before_sum += i128::from(before.excitation[node].raw());
            excitation_after_sum += i128::from(after.excitation[node].raw());
            excitation_delta_sum += i128::from(after.excitation[node].raw())
                - i128::from(before.excitation[node].raw());
        }
        regions.push(NodeObservabilityRegionV1 {
            region_id: u8::try_from(region)
                .map_err(|_| invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?,
            region_name: REGION_NAMES[region].to_owned(),
            node_capacity: u32::try_from(count)
                .map_err(|_| invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?,
            selected_node_count: selected,
            activated_node_count: activated,
            changed_node_count: changed,
            potential: NodeObservabilityComponentV1 {
                before_mean_fxp6: mean_fxp6(potential_before_sum, count)?,
                after_mean_fxp6: mean_fxp6(potential_after_sum, count)?,
                delta_mean_fxp6: mean_fxp6(potential_delta_sum, count)?,
                changed_node_count: potential_changed,
                nonzero_after_count: potential_nonzero_after,
            },
            excitation: NodeObservabilityComponentV1 {
                before_mean_fxp6: mean_fxp6(excitation_before_sum, count)?,
                after_mean_fxp6: mean_fxp6(excitation_after_sum, count)?,
                delta_mean_fxp6: mean_fxp6(excitation_delta_sum, count)?,
                changed_node_count: excitation_changed,
                nonzero_after_count: excitation_nonzero_after,
            },
        });
    }
    if expected_start != NEURON_SLOTS || regions.len() != REGION_LAYOUT.len() {
        return Err(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid));
    }
    let projection = NodeObservabilityProjectionWireV2::new(
        revision,
        u32::try_from(NEURON_SLOTS)
            .map_err(|_| invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?,
        NodeObservabilityCountsV1 {
            selected_node_count: selected_total,
            activated_node_count: activated_total,
            changed_node_count: changed_total,
            potential_nonzero_after_count: potential_nonzero_after_total,
            excitation_nonzero_after_count: excitation_nonzero_after_total,
            signal_nonzero_after_count: signal_nonzero_after_total,
        },
        NodeObservabilityResidualsV1 {
            state: NodeObservabilityResidualStateV1::NotComputed,
            formula: None,
            values_fxp6: None,
        },
        regions,
    );
    if !projection.validate() {
        return Err(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid));
    }
    Ok(projection)
}

fn region_expression_signal_fxp6(field: &NeuralField, region: usize) -> Result<u32, RuntimeError> {
    if !field.validate() {
        return Err(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid));
    }
    let (start, count) = REGION_LAYOUT
        .get(region)
        .copied()
        .ok_or(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?;
    let end = start
        .checked_add(count)
        .filter(|end| *end <= NEURON_SLOTS)
        .ok_or(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?;
    let denominator = i128::try_from(count)
        .ok()
        .and_then(|count| count.checked_mul(2))
        .filter(|count| *count > 0)
        .ok_or(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?;
    let sum = (start..end).fold(0_i128, |total, node| {
        total + i128::from(field.potential[node].raw()) + i128::from(field.excitation[node].raw())
    });
    u32::try_from((sum / denominator).clamp(0, i128::from(EXPRESSION_FXP6_MAX)))
        .map_err(|_| invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))
}

fn expression_mean_fxp6(field: &NeuralField, regions: &[usize]) -> Result<u32, RuntimeError> {
    let count = u64::try_from(regions.len())
        .ok()
        .filter(|count| *count > 0)
        .ok_or(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))?;
    let sum = regions.iter().try_fold(0_u64, |total, region| {
        total
            .checked_add(u64::from(region_expression_signal_fxp6(field, *region)?))
            .ok_or(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))
    })?;
    u32::try_from(sum / count)
        .map_err(|_| invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid))
}

pub(crate) fn expression_projection_from_field_v1(
    field: &NeuralField,
    revision: u64,
) -> Result<ExpressionProjectionV1, RuntimeError> {
    Ok(ExpressionProjectionV1 {
        revision,
        profile_fxp6: ExpressionProfileFxP6 {
            warmth: expression_mean_fxp6(field, &[1, 8])?,
            sensitivity: expression_mean_fxp6(field, &[0, 1, 2])?,
            guardedness: expression_mean_fxp6(field, &[4, 5])?,
            repair_orientation: expression_mean_fxp6(field, &[3, 6, 7])?,
            engagement: expression_mean_fxp6(field, &[7, 8])?,
            epistemic_caution: expression_mean_fxp6(field, &[3, 2])?,
        },
    })
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], RuntimeError> {
        let end = self
            .position
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(invalid_neural_state(StateSubcodeV1::SnapshotWireInvalid))?;
        let value = &self.bytes[self.position..end];
        self.position = end;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16, RuntimeError> {
        let mut value = [0; 2];
        value.copy_from_slice(self.take(2)?);
        Ok(u16::from_le_bytes(value))
    }

    fn u32(&mut self) -> Result<u32, RuntimeError> {
        let mut value = [0; 4];
        value.copy_from_slice(self.take(4)?);
        Ok(u32::from_le_bytes(value))
    }

    fn fixed(&mut self) -> Result<Fixed, RuntimeError> {
        let mut value = [0; 8];
        value.copy_from_slice(self.take(8)?);
        Ok(Fixed::decode(value))
    }

    fn eof(&self) -> bool {
        self.position == self.bytes.len()
    }
}

fn encode_field(field: &NeuralField) -> Result<Vec<u8>, RuntimeError> {
    if !field.validate() {
        return Err(invalid_neural_state(StateSubcodeV1::FieldStateInvalid));
    }
    let mut out = Vec::with_capacity(8 * (4 + NEURON_SLOTS * 8));
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
        out.extend_from_slice(
            &(u32::try_from(values.len())
                .map_err(|_| invalid_neural_state(StateSubcodeV1::FieldStateInvalid))?)
            .to_le_bytes(),
        );
        for value in values {
            out.extend_from_slice(&value.encode());
        }
    }
    Ok(out)
}

fn decode_field(bytes: &[u8]) -> Result<NeuralField, RuntimeError> {
    let mut cursor = Cursor::new(bytes);
    let mut vectors = Vec::with_capacity(8);
    for _ in 0..8 {
        if usize::try_from(cursor.u32()?)
            .map_err(|_| invalid_neural_state(StateSubcodeV1::FieldStateInvalid))?
            != NEURON_SLOTS
        {
            return Err(invalid_neural_state(StateSubcodeV1::FieldStateInvalid));
        }
        let mut values = Vec::with_capacity(NEURON_SLOTS);
        for _ in 0..NEURON_SLOTS {
            values.push(cursor.fixed()?);
        }
        vectors.push(values);
    }
    if !cursor.eof() {
        return Err(invalid_neural_state(StateSubcodeV1::FieldStateInvalid));
    }
    let mut vectors = vectors.into_iter();
    let field = NeuralField {
        potential: vectors
            .next()
            .ok_or(invalid_neural_state(StateSubcodeV1::FieldStateInvalid))?,
        excitation: vectors
            .next()
            .ok_or(invalid_neural_state(StateSubcodeV1::FieldStateInvalid))?,
        inhibition: vectors
            .next()
            .ok_or(invalid_neural_state(StateSubcodeV1::FieldStateInvalid))?,
        adaptation: vectors
            .next()
            .ok_or(invalid_neural_state(StateSubcodeV1::FieldStateInvalid))?,
        precision: vectors
            .next()
            .ok_or(invalid_neural_state(StateSubcodeV1::FieldStateInvalid))?,
        prediction_error: vectors
            .next()
            .ok_or(invalid_neural_state(StateSubcodeV1::FieldStateInvalid))?,
        eligibility: vectors
            .next()
            .ok_or(invalid_neural_state(StateSubcodeV1::FieldStateInvalid))?,
        metabolic_reserve: vectors
            .next()
            .ok_or(invalid_neural_state(StateSubcodeV1::FieldStateInvalid))?,
    };
    if field.validate() {
        Ok(field)
    } else {
        Err(invalid_neural_state(StateSubcodeV1::FieldStateInvalid))
    }
}

fn encode_graph(graph: &SparseGraph) -> Result<Vec<u8>, RuntimeError> {
    if !graph.validate() {
        return Err(invalid_neural_state(StateSubcodeV1::GraphStateInvalid));
    }
    let mut out = Vec::with_capacity(4 + graph.row_offsets.len() * 4 + 4 + graph.edges.len() * 16);
    out.extend_from_slice(
        &(u32::try_from(graph.row_offsets.len())
            .map_err(|_| invalid_neural_state(StateSubcodeV1::GraphStateInvalid))?)
        .to_le_bytes(),
    );
    for offset in &graph.row_offsets {
        out.extend_from_slice(&offset.to_le_bytes());
    }
    out.extend_from_slice(
        &(u32::try_from(graph.edges.len())
            .map_err(|_| invalid_neural_state(StateSubcodeV1::GraphStateInvalid))?)
        .to_le_bytes(),
    );
    for edge in &graph.edges {
        out.extend_from_slice(&edge.target.to_le_bytes());
        out.extend_from_slice(&edge.weight.to_le_bytes());
        out.extend_from_slice(&edge.eligibility.to_le_bytes());
        out.extend_from_slice(&edge.stability.to_le_bytes());
        out.extend_from_slice(&edge.last_used_epoch.to_le_bytes());
        out.push(edge.operator_id);
        out.push(edge.delay_class);
        out.extend_from_slice(&edge.flags.to_le_bytes());
    }
    Ok(out)
}

fn decode_graph(bytes: &[u8]) -> Result<SparseGraph, RuntimeError> {
    let mut cursor = Cursor::new(bytes);
    let offsets_len = usize::try_from(cursor.u32()?)
        .map_err(|_| invalid_neural_state(StateSubcodeV1::GraphStateInvalid))?;
    if offsets_len != NEURON_SLOTS + 1 {
        return Err(invalid_neural_state(StateSubcodeV1::GraphStateInvalid));
    }
    let mut row_offsets = Vec::with_capacity(offsets_len);
    for _ in 0..offsets_len {
        row_offsets.push(cursor.u32()?);
    }
    let edge_len = usize::try_from(cursor.u32()?)
        .map_err(|_| invalid_neural_state(StateSubcodeV1::GraphStateInvalid))?;
    if edge_len > EDGE_CAPACITY {
        return Err(invalid_neural_state(StateSubcodeV1::GraphStateInvalid));
    }
    let mut edges = Vec::with_capacity(edge_len);
    for _ in 0..edge_len {
        let target = cursor.u32()?;
        let mut weight = [0; 2];
        weight.copy_from_slice(cursor.take(2)?);
        let mut eligibility = [0; 2];
        eligibility.copy_from_slice(cursor.take(2)?);
        let mut stability = [0; 2];
        stability.copy_from_slice(cursor.take(2)?);
        let mut last_used_epoch = [0; 2];
        last_used_epoch.copy_from_slice(cursor.take(2)?);
        let operator_id = cursor.take(1)?[0];
        let delay_class = cursor.take(1)?[0];
        let mut flags = [0; 2];
        flags.copy_from_slice(cursor.take(2)?);
        edges.push(Synapse {
            target,
            weight: i16::from_le_bytes(weight),
            eligibility: i16::from_le_bytes(eligibility),
            stability: u16::from_le_bytes(stability),
            last_used_epoch: u16::from_le_bytes(last_used_epoch),
            operator_id,
            delay_class,
            flags: u16::from_le_bytes(flags),
        });
    }
    if !cursor.eof() {
        return Err(invalid_neural_state(StateSubcodeV1::GraphStateInvalid));
    }
    let graph = SparseGraph { row_offsets, edges };
    if graph.validate() {
        Ok(graph)
    } else {
        Err(invalid_neural_state(StateSubcodeV1::GraphStateInvalid))
    }
}

pub(crate) fn decode_semantic_snapshot_v2(
    bytes: &[u8],
    expected_formula_digest: &[u8; 32],
    expected_state_digest: &[u8; 32],
    expected_graph_digest: &[u8; 32],
    legacy_receipt: &TransitionReceipt,
) -> Result<(NeuralField, SparseGraph, TransitionReceiptV2), RuntimeError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take(SNAPSHOT_MAGIC_V2.len())? != SNAPSHOT_MAGIC_V2
        || cursor.u16()? != SNAPSHOT_SCHEMA_V2
    {
        return Err(invalid_neural_state(StateSubcodeV1::SnapshotWireInvalid));
    }
    let field_len = usize::try_from(cursor.u32()?)
        .map_err(|_| invalid_neural_state(StateSubcodeV1::SnapshotWireInvalid))?;
    let field = decode_field(cursor.take(field_len)?)?;
    let graph_len = usize::try_from(cursor.u32()?)
        .map_err(|_| invalid_neural_state(StateSubcodeV1::SnapshotWireInvalid))?;
    let graph = decode_graph(cursor.take(graph_len)?)?;
    let receipt_len = usize::try_from(cursor.u32()?)
        .map_err(|_| invalid_neural_state(StateSubcodeV1::SnapshotWireInvalid))?;
    let receipt_bytes = cursor.take(receipt_len)?;
    if !cursor.eof() {
        return Err(invalid_neural_state(StateSubcodeV1::SnapshotWireInvalid));
    }
    let receipt = wire::decode_transition_receipt_v2(receipt_bytes)
        .map_err(|_| invalid_neural_state(StateSubcodeV1::SnapshotWireInvalid))?;
    if wire::encode_transition_receipt_v2(&receipt) != receipt_bytes {
        return Err(invalid_neural_state(StateSubcodeV1::SnapshotWireInvalid));
    }
    if !receipt.validate() {
        return Err(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid));
    }
    if receipt.formula_digest != *expected_formula_digest
        || receipt.state_after != *expected_state_digest
        || receipt.graph_after != *expected_graph_digest
        || !semantic_v2_matches_legacy_receipt(&receipt, legacy_receipt)
        || state_digest(&field, expected_formula_digest) != *expected_state_digest
        || graph_digest(&graph) != *expected_graph_digest
    {
        return Err(invalid_neural_state(
            StateSubcodeV1::SnapshotAttestationMismatch,
        ));
    }
    Ok((field, graph, receipt))
}

/// Frozen predecessor wire writer used only to construct an authenticated
/// AESEM2 persistence fixture.  The production runtime never emits AESEM2.
#[cfg(test)]
pub(crate) fn encode_semantic_snapshot_v2_for_test(
    formula_digest: &[u8; 32],
    field: &NeuralField,
    graph: &SparseGraph,
    receipt: &TransitionReceiptV2,
) -> Result<Vec<u8>, RuntimeError> {
    if !receipt.validate() {
        return Err(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid));
    }
    if receipt.formula_digest != *formula_digest
        || receipt.state_after != state_digest(field, formula_digest)
        || receipt.graph_after != graph_digest(graph)
    {
        return Err(invalid_neural_state(
            StateSubcodeV1::SnapshotAttestationMismatch,
        ));
    }
    let field_bytes = encode_field(field)?;
    let graph_bytes = encode_graph(graph)?;
    let receipt_bytes = wire::encode_transition_receipt_v2(receipt);
    let mut out = Vec::with_capacity(
        SNAPSHOT_MAGIC_V2.len()
            + 2
            + 4
            + field_bytes.len()
            + 4
            + graph_bytes.len()
            + 4
            + receipt_bytes.len(),
    );
    out.extend_from_slice(SNAPSHOT_MAGIC_V2);
    out.extend_from_slice(&SNAPSHOT_SCHEMA_V2.to_le_bytes());
    for bytes in [&field_bytes, &graph_bytes, &receipt_bytes] {
        out.extend_from_slice(
            &(u32::try_from(bytes.len())
                .map_err(|_| invalid_neural_state(StateSubcodeV1::SnapshotWireInvalid))?)
            .to_le_bytes(),
        );
        out.extend_from_slice(bytes);
    }
    Ok(out)
}

pub(crate) fn snapshot_is_aesem2(bytes: &[u8]) -> bool {
    bytes.starts_with(SNAPSHOT_MAGIC_V2)
}

pub(crate) fn semantic_v3_matches_legacy_receipt(
    telemetry: &NativeTelemetryReceiptV1,
    legacy_receipt: &TransitionReceipt,
) -> bool {
    legacy_receipt.schema_version == 1
        && legacy_receipt.status == CommitStatus::Committed
        && legacy_receipt.action_contract.is_none()
        && telemetry.validate()
        && telemetry.formula_digest == legacy_receipt.formula_digest
        && telemetry.scope_digest == legacy_receipt.scope_digest
        && telemetry.event_digest == legacy_receipt.event_digest
        && telemetry.base_revision == legacy_receipt.base_revision
        && telemetry.next_revision == legacy_receipt.next_revision
        && telemetry.state_before == legacy_receipt.state_before
        && telemetry.state_after == legacy_receipt.state_after
        && telemetry.graph_after == legacy_receipt.graph_after
        && telemetry.residuals == legacy_receipt.residuals
}

pub(crate) fn encode_semantic_snapshot_v3(
    formula_digest: &[u8; 32],
    field: &NeuralField,
    graph: &SparseGraph,
    telemetry: &NativeTelemetryReceiptV1,
) -> Result<Vec<u8>, RuntimeError> {
    if !telemetry.validate() {
        return Err(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid));
    }
    if telemetry.formula_digest != *formula_digest
        || telemetry.state_after != state_digest(field, formula_digest)
        || telemetry.graph_after != graph_digest(graph)
        || telemetry.compensation_digest != ae_contracts::legacy_reserved_zero_digest_v1()
    {
        return Err(invalid_neural_state(
            StateSubcodeV1::SnapshotAttestationMismatch,
        ));
    }
    let field_bytes = encode_field(field)?;
    let graph_bytes = encode_graph(graph)?;
    let telemetry_bytes = wire::encode_native_telemetry_receipt_v1(telemetry);
    let mut reserved_zero_bytes = Vec::with_capacity(REGION_LAYOUT.len() * 8);
    for _ in 0..REGION_LAYOUT.len() {
        reserved_zero_bytes.extend_from_slice(&Fixed::ZERO.encode());
    }
    let mut out = Vec::with_capacity(
        SNAPSHOT_MAGIC_V3.len()
            + 2
            + 4
            + field_bytes.len()
            + 4
            + graph_bytes.len()
            + 4
            + telemetry_bytes.len()
            + 4
            + reserved_zero_bytes.len(),
    );
    out.extend_from_slice(SNAPSHOT_MAGIC_V3);
    out.extend_from_slice(&SNAPSHOT_SCHEMA_V3.to_le_bytes());
    for bytes in [
        &field_bytes,
        &graph_bytes,
        &telemetry_bytes,
        &reserved_zero_bytes,
    ] {
        out.extend_from_slice(
            &(u32::try_from(bytes.len())
                .map_err(|_| invalid_neural_state(StateSubcodeV1::SnapshotWireInvalid))?)
            .to_le_bytes(),
        );
        out.extend_from_slice(bytes);
    }
    Ok(out)
}

pub(crate) fn decode_semantic_snapshot_v3(
    bytes: &[u8],
    expected_formula_digest: &[u8; 32],
    expected_state_digest: &[u8; 32],
    expected_graph_digest: &[u8; 32],
    legacy_receipt: &TransitionReceipt,
) -> Result<(NeuralField, SparseGraph, NativeTelemetryReceiptV1), RuntimeError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take(SNAPSHOT_MAGIC_V3.len())? != SNAPSHOT_MAGIC_V3
        || cursor.u16()? != SNAPSHOT_SCHEMA_V3
    {
        return Err(RuntimeError::LegacyUnattested);
    }
    let field_len = usize::try_from(cursor.u32()?)
        .map_err(|_| invalid_neural_state(StateSubcodeV1::SnapshotWireInvalid))?;
    let field = decode_field(cursor.take(field_len)?)?;
    let graph_len = usize::try_from(cursor.u32()?)
        .map_err(|_| invalid_neural_state(StateSubcodeV1::SnapshotWireInvalid))?;
    let graph = decode_graph(cursor.take(graph_len)?)?;
    let telemetry_len = usize::try_from(cursor.u32()?)
        .map_err(|_| invalid_neural_state(StateSubcodeV1::SnapshotWireInvalid))?;
    let telemetry_bytes = cursor.take(telemetry_len)?;
    // The early three-block precursor has the same all-zero reserved value.
    // Current writes always carry the fourth AESEM3 block; a non-zero legacy
    // value cannot be replayed safely because it may already have affected the
    // sealed field, so it fails closed rather than being ignored.
    if !cursor.eof() {
        let reserved_len = usize::try_from(cursor.u32()?)
            .map_err(|_| invalid_neural_state(StateSubcodeV1::SnapshotWireInvalid))?;
        if reserved_len != REGION_LAYOUT.len() * 8 {
            return Err(invalid_neural_state(StateSubcodeV1::SnapshotWireInvalid));
        }
        let reserved_bytes = cursor.take(reserved_len)?;
        if !cursor.eof() {
            return Err(invalid_neural_state(StateSubcodeV1::SnapshotWireInvalid));
        }
        for chunk in reserved_bytes.as_chunks::<8>().0 {
            let mut raw = [0u8; 8];
            raw.copy_from_slice(chunk);
            if Fixed::decode(raw) != Fixed::ZERO {
                return Err(invalid_neural_state(
                    StateSubcodeV1::Aesem3RetiredCompensationNonzero,
                ));
            }
        }
    }
    let telemetry = wire::decode_native_telemetry_receipt_v1(telemetry_bytes)
        .map_err(|_| invalid_neural_state(StateSubcodeV1::SnapshotWireInvalid))?;
    if wire::encode_native_telemetry_receipt_v1(&telemetry) != telemetry_bytes {
        return Err(invalid_neural_state(StateSubcodeV1::SnapshotWireInvalid));
    }
    if !telemetry.validate() {
        return Err(invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid));
    }
    if telemetry.formula_digest != *expected_formula_digest
        || telemetry.state_after != *expected_state_digest
        || telemetry.graph_after != *expected_graph_digest
        || telemetry.compensation_digest != ae_contracts::legacy_reserved_zero_digest_v1()
        || !semantic_v3_matches_legacy_receipt(&telemetry, legacy_receipt)
        || state_digest(&field, expected_formula_digest) != *expected_state_digest
        || graph_digest(&graph) != *expected_graph_digest
    {
        return Err(invalid_neural_state(
            StateSubcodeV1::SnapshotAttestationMismatch,
        ));
    }
    Ok((field, graph, telemetry))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ae_contracts::{
        CapacityTelemetryV1, EnergyTelemetryV1, EvidenceVector, InvariantResiduals,
        NativeTelemetryFormulaV1, NativeTelemetryPhaseV1, PerceptionProposalV1, StateSubcodeV1,
        NATIVE_TELEMETRY_RECEIPT_SCHEMA_V1,
    };

    fn valid_proposal() -> PerceptionProposalV1 {
        PerceptionProposalV1 {
            schema_version: PerceptionProposalV1::SCHEMA_VERSION,
            event_id: [1; 16],
            turn_id: [2; 16],
            observed_at_ms: 1,
            base_revision: 0,
            dimensions: EvidenceVector::default(),
            estimator_confidence: Fixed::ONE,
            protocol_version: PerceptionProposalV1::PROTOCOL_VERSION,
            request_nonce_digest: [3; 32],
        }
    }

    #[test]
    fn typed_node_wire_accepts_every_native_component_change() {
        let before = NeuralField::zeroed();
        for component in [
            "potential",
            "excitation",
            "inhibition",
            "adaptation",
            "precision",
            "prediction_error",
            "eligibility",
            "metabolic_reserve",
        ] {
            let mut after = before.clone();
            match component {
                "potential" => after.potential[0] = Fixed::ONE,
                "excitation" => after.excitation[0] = Fixed::ONE,
                "inhibition" => after.inhibition[0] = Fixed::ONE,
                "adaptation" => after.adaptation[0] = Fixed::ONE,
                "precision" => after.precision[0] = Fixed::ONE,
                "prediction_error" => after.prediction_error[0] = Fixed::ONE,
                "eligibility" => after.eligibility[0] = Fixed::ONE,
                "metabolic_reserve" => after.metabolic_reserve[0] = Fixed::ZERO,
                _ => unreachable!("fixed native component witness"),
            }

            let projection = node_observability_projection_v2(&before, &after, 7)
                .expect("valid field witness must project");
            assert!(projection.validate(), "{component}");
            assert_eq!(projection.counts.selected_node_count, 1, "{component}");
            assert_eq!(projection.counts.changed_node_count, 1, "{component}");
            assert_eq!(
                projection.counts.activated_node_count,
                u32::from(matches!(
                    component,
                    "potential" | "excitation" | "inhibition"
                )),
                "{component}"
            );
        }
    }

    #[test]
    fn out_of_unit_historical_field_is_classified_before_dynamics() {
        let mut field = NeuralField::zeroed();
        field.potential[0] = Fixed::from_raw(1_000_001);

        assert!(matches!(
            prepare_semantic_transition_v2(
                &field,
                &NeuralField::zeroed(),
                &SparseGraph::empty(),
                &[4; 32],
                &[5; 32],
                &valid_proposal(),
            ),
            Err(RuntimeError::InvalidNeuralState(
                StateSubcodeV1::FieldStateInvalid
            ))
        ));
    }

    #[test]
    fn legacy_field_domain_joint_max_normalization_is_exact_and_idempotent() {
        let mut field = NeuralField::zeroed();
        field.potential[0] = Fixed::from_raw(2_000_000);
        field.potential[1] = Fixed::from_raw(1_000_000);
        // These two values exercise ties-to-even without adding a second
        // scale: 3/2_000_000 rounds to 2, while 1/2_000_000 rounds to 0.
        field.excitation[0] = Fixed::from_raw(3);
        field.excitation[1] = Fixed::from_raw(1);
        let metabolic_before = field.metabolic_reserve.clone();

        let (normalized, metadata) = normalize_legacy_aesem2_field_domain_v1(&field)
            .unwrap()
            .expect("finite P/E overflow is the one migratable shape");
        assert_eq!(metadata.source_common_max, 2_000_000);
        assert_eq!(metadata.out_of_range_count, 1);
        assert_eq!(metadata.potential_out_of_range_count, 1);
        assert_eq!(metadata.excitation_out_of_range_count, 0);
        assert_eq!(normalized.potential[0], Fixed::ONE);
        assert_eq!(normalized.potential[1], Fixed::from_raw(500_000));
        assert_eq!(normalized.excitation[0], Fixed::from_raw(2));
        assert_eq!(normalized.excitation[1], Fixed::ZERO);
        assert_eq!(normalized.metabolic_reserve, metabolic_before);
        assert!(normalize_legacy_aesem2_field_domain_v1(&normalized)
            .unwrap()
            .is_none());
    }

    #[test]
    fn aesem3_reserved_zero_replays_and_nonzero_history_fails_closed() {
        let formula_digest = [0x31; 32];
        let field = NeuralField::zeroed();
        let graph = SparseGraph::empty();
        let state_after = state_digest(&field, &formula_digest);
        let graph_after = graph_digest(&graph);
        let residuals = InvariantResiduals {
            authority: Fixed::ZERO,
            continuity: Fixed::ZERO,
            energy: Fixed::from_raw(200_000),
            renormalization: Fixed::from_raw(100_000),
            capacity: Fixed::ZERO,
        };
        let telemetry = NativeTelemetryReceiptV1 {
            schema: NATIVE_TELEMETRY_RECEIPT_SCHEMA_V1.to_owned(),
            formula: NativeTelemetryFormulaV1::Phase0NativePropagationFxp6V1,
            formula_digest,
            scope_digest: [0x32; 32],
            event_digest: [0x33; 32],
            source_digest: [0x34; 32],
            base_revision: 0,
            next_revision: 1,
            phase: NativeTelemetryPhaseV1::Prepare,
            state_before: [0x35; 32],
            state_after,
            graph_before: graph_after,
            graph_after,
            local_digest: [0x36; 32],
            compensation_digest: ae_contracts::legacy_reserved_zero_digest_v1(),
            effective_digest: [0x37; 32],
            energy: EnergyTelemetryV1 {
                reserve_before: Fixed::ONE,
                reserve_after: Fixed::from_raw(500_000),
                recovered: Fixed::from_raw(100_000),
                spent: Fixed::from_raw(600_000),
                headroom: Fixed::from_raw(500_000),
                residual: Fixed::from_raw(200_000),
            },
            capacity: CapacityTelemetryV1 {
                upper_saturated_nodes: 2,
                node_limit: 4,
                node_headroom: Fixed::from_raw(500_000),
                edge_used: 1,
                edge_limit: 2,
                edge_headroom: Fixed::from_raw(500_000),
                headroom: Fixed::from_raw(500_000),
                residual: Fixed::ZERO,
            },
            residuals: residuals.clone(),
            residual_health: Fixed::from_raw(800_000),
            native_gate: Fixed::from_raw(500_000),
            checkpoint_digest: [0; 32],
            telemetry_digest: [0; 32],
        }
        .seal();
        let legacy_receipt = TransitionReceipt {
            schema_version: 1,
            formula_digest,
            scope_digest: telemetry.scope_digest,
            event_digest: telemetry.event_digest,
            authority_digest: [0x38; 32],
            base_revision: telemetry.base_revision,
            next_revision: telemetry.next_revision,
            state_before: telemetry.state_before,
            state_after,
            graph_after,
            action_contract: None,
            active_nodes: 0,
            active_edges: 0,
            residuals,
            status: CommitStatus::Committed,
        };

        let snapshot = encode_semantic_snapshot_v3(&formula_digest, &field, &graph, &telemetry)
            .expect("canonical reserved-zero snapshot encodes");
        assert!(decode_semantic_snapshot_v3(
            &snapshot,
            &formula_digest,
            &state_after,
            &graph_after,
            &legacy_receipt,
        )
        .is_ok());

        let truncated_wire = &snapshot[..SNAPSHOT_MAGIC_V3.len() + 2];
        assert!(matches!(
            decode_semantic_snapshot_v3(
                truncated_wire,
                &formula_digest,
                &state_after,
                &graph_after,
                &legacy_receipt,
            ),
            Err(RuntimeError::InvalidNeuralState(
                StateSubcodeV1::SnapshotWireInvalid
            ))
        ));

        let mut nonzero_history = snapshot;
        *nonzero_history
            .last_mut()
            .expect("AESEM3 has a reserved fourth block") = 1;
        assert!(matches!(
            decode_semantic_snapshot_v3(
                &nonzero_history,
                &formula_digest,
                &state_after,
                &graph_after,
                &legacy_receipt,
            ),
            Err(RuntimeError::InvalidNeuralState(
                StateSubcodeV1::Aesem3RetiredCompensationNonzero
            ))
        ));
    }
}
