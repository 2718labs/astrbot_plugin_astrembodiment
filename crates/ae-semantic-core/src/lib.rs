#![forbid(unsafe_code)]

//! Pure deterministic semantic transition derivation and canonical AESEM3 wire.

use ae_attention::emotion_matrix::{assemble_full_vector_load, FullVectorLoad};
use ae_contracts::{
    perception_dimension_values, phase0_canonical_formula_digest_v1, CommitStatus, Digest,
    InvariantResiduals, NativeTelemetryReceiptV1, PerceptionProposalV1, SemanticVectorFormulaV2,
    SemanticVectorReceiptV2, TransitionReceipt,
};
use ae_neurofield::{
    develop_graph, graph_digest, state_digest, GraphFormula, NeuralField, SparseGraph,
    NEURON_SLOTS, REGION_LAYOUT,
};
use thiserror::Error;

pub mod semantic_dynamics_v2;
pub mod semantic_telemetry_v1;

mod affect_projection;

pub use affect_projection::{potential_region_projection_v1, PotentialRegionProjectionV1};

mod matrix_time;

pub use ae_contracts::{MatrixSleepPhaseV1, MatrixTimeEpochV1};
pub use matrix_time::{
    advance_matrix_time_v1, matrix_time_formula_digest_v1, MatrixTimeAdvanceV1, MatrixTimeInputV1,
    MATRIX_TIME_FORMULA_V1, MATRIX_TIME_MAX_ELAPSED_MS, MATRIX_TIME_MAX_STEPS_PER_EVENT,
    MATRIX_TIME_QUANTUM_MS,
};

mod codec;

pub use codec::{
    decode_canonical_aesem3_blocks, decode_canonical_graph_v1,
    decode_canonical_semantic_snapshot_v3, decode_legacy_time_snapshot_v1,
    decode_native_telemetry_receipt_v1, decode_time_snapshot_v1, decode_transition_receipt_v2,
    encode_canonical_graph_v1, encode_native_telemetry_receipt_v1, encode_semantic_snapshot_v3,
    encode_time_snapshot_v1, encode_transition_receipt_v2, CanonicalAesem3Blocks,
    DecodedCanonicalSemanticSnapshotV3, DecodedLegacyTimeSnapshotV1, DecodedTimeSnapshotV1,
    NATIVE_TELEMETRY_RECEIPT_V1_WIRE_LEN, TIME_SNAPSHOT_WIRE_LEN_V1,
    TRANSITION_RECEIPT_V2_WIRE_LEN,
};
use semantic_dynamics_v2::{
    propagate_semantic_dynamics_v2, DynamicsError, DynamicsInputV2, PreparedSemanticDynamicsV2,
};
use semantic_telemetry_v1::prepare_native_telemetry_v1;

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum SemanticCoreError {
    #[error("perception proposal is invalid")]
    InvalidPerceptionProposal,
    #[error("semantic field state is invalid")]
    FieldStateInvalid,
    #[error("semantic graph state is invalid")]
    GraphStateInvalid,
    #[error("semantic dynamics input or arithmetic is invalid")]
    DynamicsInvalid,
    #[error("semantic revision overflows")]
    SemanticRevisionOverflow,
    #[error("semantic receipt closure is invalid")]
    SemanticClosureInvalid,
    #[error("semantic snapshot wire is invalid")]
    SnapshotWireInvalid,
    #[error("semantic snapshot magic or schema is invalid")]
    Aesem3MagicOrSchema,
    #[error("AESEM3 retired compensation data is nonzero")]
    Aesem3RetiredCompensationNonzero,
    #[error("semantic snapshot attestation does not close")]
    SnapshotAttestationMismatch,
}

impl From<DynamicsError> for SemanticCoreError {
    fn from(error: DynamicsError) -> Self {
        match error {
            DynamicsError::FieldStateInvalid => Self::FieldStateInvalid,
            DynamicsError::GraphStateInvalid => Self::GraphStateInvalid,
            DynamicsError::InvalidInput | DynamicsError::Arithmetic => Self::DynamicsInvalid,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PreparedSemanticTransitionV2 {
    pub next_field: NeuralField,
    pub next_graph: SparseGraph,
    pub active_nodes: u32,
    pub full_vector_load: FullVectorLoad,
    pub local_by_region: [ae_fixed::Fixed; REGION_LAYOUT.len()],
    pub dynamics: PreparedSemanticDynamicsV2,
}

/// Frozen semantic-sidecar receipt. Its revision is the semantic cursor and is
/// deliberately independent from the canonical journal cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransitionReceiptV2 {
    pub schema_version: u16,
    pub formula_digest: Digest,
    pub scope_digest: Digest,
    pub event_digest: Digest,
    pub authority_digest: Digest,
    pub base_revision: u64,
    pub next_revision: u64,
    pub state_before: Digest,
    pub state_after: Digest,
    pub graph_after: Digest,
    pub action_contract: Option<Digest>,
    pub active_nodes: u32,
    pub active_edges: u32,
    pub residuals: InvariantResiduals,
    pub status: CommitStatus,
    pub semantic_vector: SemanticVectorReceiptV2,
}

impl TransitionReceiptV2 {
    pub const SCHEMA_VERSION: u16 = 2;

    pub fn from_legacy(
        legacy: &TransitionReceipt,
        semantic_vector: SemanticVectorReceiptV2,
    ) -> Option<Self> {
        if legacy.schema_version != 1 {
            return None;
        }
        let receipt = Self {
            schema_version: Self::SCHEMA_VERSION,
            formula_digest: legacy.formula_digest,
            scope_digest: legacy.scope_digest,
            event_digest: legacy.event_digest,
            authority_digest: legacy.authority_digest,
            base_revision: legacy.base_revision,
            next_revision: legacy.next_revision,
            state_before: legacy.state_before,
            state_after: legacy.state_after,
            graph_after: legacy.graph_after,
            action_contract: legacy.action_contract,
            active_nodes: legacy.active_nodes,
            active_edges: legacy.active_edges,
            residuals: legacy.residuals.clone(),
            status: legacy.status,
            semantic_vector,
        };
        receipt.validate().then_some(receipt)
    }

    pub fn validate(&self) -> bool {
        self.schema_version == Self::SCHEMA_VERSION
            && self.status == CommitStatus::Committed
            && self.action_contract.is_none()
            && self.base_revision.checked_add(1) == Some(self.next_revision)
            && self.semantic_vector.validate()
            && self.semantic_vector.state_changed == (self.state_before != self.state_after)
    }
}

/// Phase-0 preparation materializes the deterministic graph exactly once and
/// then runs the immutable-before sparse-edge dynamics.
pub fn prepare_semantic_transition_v2(
    field: &NeuralField,
    baseline: &NeuralField,
    graph: &SparseGraph,
    manifest_digest: &Digest,
    development_seed_digest: &Digest,
    proposal: &PerceptionProposalV1,
) -> Result<PreparedSemanticTransitionV2, SemanticCoreError> {
    proposal
        .validate_v1()
        .map_err(|_| SemanticCoreError::InvalidPerceptionProposal)?;
    let full_vector_load = assemble_full_vector_load(&proposal.dimensions)
        .map_err(|_| SemanticCoreError::InvalidPerceptionProposal)?;
    if full_vector_load.evaluated_dimension_count != 15
        || full_vector_load.injected_dimension_count != 15
    {
        return Err(SemanticCoreError::InvalidPerceptionProposal);
    }
    let next_graph = if graph.edges.is_empty() {
        develop_graph(manifest_digest, development_seed_digest, GraphFormula::V1)
            .map_err(|_| SemanticCoreError::GraphStateInvalid)?
    } else {
        graph.clone()
    };
    if !next_graph.validate() {
        return Err(SemanticCoreError::GraphStateInvalid);
    }
    let local_by_region = full_vector_load.evidence_means;
    let local_confidence_by_region = [proposal.estimator_confidence; REGION_LAYOUT.len()];
    let dynamics = propagate_semantic_dynamics_v2(DynamicsInputV2 {
        field,
        baseline,
        graph: &next_graph,
        local_by_region,
        local_confidence_by_region,
    })
    .map_err(SemanticCoreError::from)?;
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
    .map_err(|_| SemanticCoreError::DynamicsInvalid)?;
    Ok(PreparedSemanticTransitionV2 {
        next_field: dynamics.next_field.clone(),
        next_graph,
        active_nodes,
        full_vector_load,
        local_by_region,
        dynamics,
    })
}

pub fn phase0_semantic_formula_digest_v1(
    genesis_formula_digest: &Digest,
) -> Result<Digest, SemanticCoreError> {
    Ok(phase0_canonical_formula_digest_v1(genesis_formula_digest))
}

pub fn semantic_vector_receipt_v2(
    legacy: &TransitionReceipt,
    evaluated_dimension_count: u8,
    injected_dimension_count: u8,
    nonzero_evidence_dimension_count: u8,
) -> Result<TransitionReceiptV2, SemanticCoreError> {
    let neutral_baseline_dimension_count = evaluated_dimension_count
        .checked_sub(nonzero_evidence_dimension_count)
        .ok_or(SemanticCoreError::SemanticClosureInvalid)?;
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
    .ok_or(SemanticCoreError::SemanticClosureInvalid)
}

pub fn semantic_v2_matches_legacy_receipt(
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

#[derive(Clone, Copy, Debug)]
pub struct UserStimulusTransitionInputV1<'a> {
    pub field: &'a NeuralField,
    pub baseline: &'a NeuralField,
    pub graph: &'a SparseGraph,
    pub manifest_digest: Digest,
    pub development_seed_digest: Digest,
    pub proposal: &'a PerceptionProposalV1,
    pub formula_digest: Digest,
    pub scope_digest: Digest,
    pub event_digest: Digest,
    pub source_digest: Digest,
    pub authority_digest: Digest,
    pub semantic_base_revision: u64,
}

#[derive(Clone, Debug)]
pub struct DerivedUserStimulusTransitionV1 {
    pub next_field: NeuralField,
    pub next_graph: SparseGraph,
    pub active_nodes: u32,
    pub active_edges: u32,
    pub route_digest: Digest,
    pub state_before_digest: Digest,
    pub state_after_digest: Digest,
    pub graph_before_digest: Digest,
    pub graph_after_digest: Digest,
    pub semantic_receipt: TransitionReceiptV2,
    pub telemetry: NativeTelemetryReceiptV1,
    pub semantic_receipt_bytes: Vec<u8>,
    pub telemetry_bytes: Vec<u8>,
    pub snapshot_bytes: Vec<u8>,
}

pub type TransitionInputV1<'a> = UserStimulusTransitionInputV1<'a>;
pub type DerivedTransitionV1 = DerivedUserStimulusTransitionV1;

/// Derive one complete semantic sidecar set without journal or storage access.
/// The caller remains responsible for authenticating all supplied identities
/// and atomically committing the returned canonical bytes.
pub fn derive_user_stimulus_transition_v1(
    input: UserStimulusTransitionInputV1<'_>,
) -> Result<DerivedUserStimulusTransitionV1, SemanticCoreError> {
    let next_revision = input
        .semantic_base_revision
        .checked_add(1)
        .ok_or(SemanticCoreError::SemanticRevisionOverflow)?;
    let prepared = prepare_semantic_transition_v2(
        input.field,
        input.baseline,
        input.graph,
        &input.manifest_digest,
        &input.development_seed_digest,
        input.proposal,
    )?;
    let state_before_digest = state_digest(input.field, &input.formula_digest);
    let state_after_digest = state_digest(&prepared.next_field, &input.formula_digest);
    let graph_before_digest = graph_digest(input.graph);
    let graph_after_digest = graph_digest(&prepared.next_graph);
    let active_edges = u32::try_from(prepared.next_graph.edges.len())
        .map_err(|_| SemanticCoreError::SemanticClosureInvalid)?;
    let telemetry = prepare_native_telemetry_v1(
        input.formula_digest,
        input.scope_digest,
        input.event_digest,
        input.source_digest,
        input.semantic_base_revision,
        next_revision,
        state_before_digest,
        state_after_digest,
        graph_before_digest,
        graph_after_digest,
        &prepared.local_by_region,
        &prepared.dynamics,
        &prepared.full_vector_load,
    )?;
    let legacy_semantic_receipt = TransitionReceipt {
        schema_version: 1,
        formula_digest: input.formula_digest,
        scope_digest: input.scope_digest,
        event_digest: input.event_digest,
        authority_digest: input.authority_digest,
        base_revision: input.semantic_base_revision,
        next_revision,
        state_before: state_before_digest,
        state_after: state_after_digest,
        graph_after: graph_after_digest,
        action_contract: None,
        active_nodes: prepared.active_nodes,
        active_edges,
        residuals: telemetry.residuals.clone(),
        status: CommitStatus::Committed,
    };
    let nonzero_evidence_dimension_count = u8::try_from(
        perception_dimension_values(&input.proposal.dimensions)
            .into_iter()
            .filter(|value| *value != ae_fixed::Fixed::ZERO)
            .count(),
    )
    .map_err(|_| SemanticCoreError::SemanticClosureInvalid)?;
    let semantic_receipt = semantic_vector_receipt_v2(
        &legacy_semantic_receipt,
        prepared.full_vector_load.evaluated_dimension_count,
        prepared.full_vector_load.injected_dimension_count,
        nonzero_evidence_dimension_count,
    )?;
    let semantic_receipt_bytes = encode_transition_receipt_v2(&semantic_receipt)?;
    let telemetry_bytes = encode_native_telemetry_receipt_v1(&telemetry)?;
    let snapshot_bytes = encode_semantic_snapshot_v3(
        &input.formula_digest,
        &prepared.next_field,
        &prepared.next_graph,
        &telemetry,
    )?;
    Ok(DerivedUserStimulusTransitionV1 {
        next_field: prepared.next_field,
        next_graph: prepared.next_graph,
        active_nodes: prepared.active_nodes,
        active_edges,
        route_digest: prepared.full_vector_load.route_digest,
        state_before_digest,
        state_after_digest,
        graph_before_digest,
        graph_after_digest,
        semantic_receipt,
        telemetry,
        semantic_receipt_bytes,
        telemetry_bytes,
        snapshot_bytes,
    })
}
