#![forbid(unsafe_code)]

//! Compatibility path for telemetry now owned by `ae-semantic-core`.

use ae_attention::emotion_matrix::FullVectorLoad;
use ae_contracts::{Digest, NativeTelemetryReceiptV1, StateSubcodeV1};
use ae_fixed::Fixed;
use ae_neurofield::REGION_LAYOUT;

use crate::semantic_dynamics_v2::PreparedSemanticDynamicsV2;
use crate::RuntimeError;

#[allow(unused_imports)]
pub(crate) use ae_semantic_core::semantic_telemetry_v1::{
    effective_vector_digest, local_vector_digest, regional_vector_digest,
};

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
    dynamics: &PreparedSemanticDynamicsV2,
    full_vector_load: &FullVectorLoad,
) -> Result<NativeTelemetryReceiptV1, RuntimeError> {
    ae_semantic_core::semantic_telemetry_v1::prepare_native_telemetry_v1(
        formula_digest,
        scope_digest,
        event_digest,
        source_digest,
        base_revision,
        next_revision,
        state_before,
        state_after,
        graph_before,
        graph_after,
        local_by_region,
        dynamics,
        full_vector_load,
    )
    .map_err(|error| match error {
        ae_semantic_core::SemanticCoreError::SemanticRevisionOverflow => {
            RuntimeError::SemanticRevisionOverflow
        }
        _ => RuntimeError::invalid_neural_state(StateSubcodeV1::SemanticClosureInvalid),
    })
}
