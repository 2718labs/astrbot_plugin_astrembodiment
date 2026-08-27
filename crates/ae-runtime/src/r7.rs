#![forbid(unsafe_code)]
// This is a deliberately unmounted compatibility island.  The release ABI
// cannot call it, while the retained R7/N1 source stays available for the
// private Store hydration path and future internal migration work.
#![allow(dead_code, unused_imports)]

mod private_projection_wire;
mod r7_atomic_projection;

use self::r7_atomic_projection::{
    compile_atomic_pre_output_wire_v1, R7SemanticProjectionBindingV1,
};
use ae_agent::r7::scaffold_contract;
use ae_attention::r7::assemble_load;
use ae_contracts::r7::{
    wire, ActionContract, CanonicalEvent, CommitStatus, EvidenceVector, InvariantResiduals,
    ScopeRef, SourceAuthority, TransitionReceipt, UserStimulus,
};
use ae_fixed::Fixed;
use ae_neurofield::{NeuralField, SparseGraph, NEURON_SLOTS};
use ae_renorm::empty_workspace;
use std::collections::BTreeMap;
use std::fmt;
use thiserror::Error;

pub(crate) use ae_contracts::r7::{
    AstrBotPublicSignalV1, AstrBotToolDispositionV1, AstrBotToolIngressV1, AstrBotToolOutcomeV1,
    DeliveryKnowledgeV1, HostEffectDispositionV1, HostEffectV1, HostIngressKindV1, HostIngressV1,
    HostSettlementStatusV1, HostSettlementV1,
};
pub(crate) use private_projection_wire::{
    discard_private_projection_transfer_v1, PrivateProjectionPayloadWireErrorV1,
    PrivateProjectionPayloadWireV1, PrivateProjectionTransferReceiptV1,
};
pub(crate) use r7_atomic_projection::{
    BoundedProjectionReferencesV1, NativeProjectionPayloadIngressV1,
    NativeProjectionPayloadProducerErrorV1, NativeProjectionPayloadProducerInputV1,
    NativeProjectionPayloadProducerV1, NativeProjectionUpdateV1, OrganismRuntimeErrorV1,
    PreOutputProjectionUpdateV1, R7PreOutputProjectionInputV1,
};

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum RuntimeError {
    #[error("invalid neural field")]
    InvalidNeuralField,
    #[error("invalid sparse graph")]
    InvalidSparseGraph,
    #[error("event kind not yet supported by scaffold")]
    UnsupportedEvent,
    #[error("invalid user stimulus")]
    InvalidUserStimulus,
    #[error("user stimulus base revision mismatch")]
    UserStimulusBaseRevisionMismatch,
    #[error("invalid semantic estimate")]
    InvalidSemanticEstimate,
    #[error("native semantic transition did not change state")]
    NativeStateUnchanged,
    #[error("native semantic transition formula identity mismatch")]
    NativeFormulaDigestMismatch,
    #[error("runtime revision overflow")]
    RevisionOverflow,
    #[error("private projection wire compilation was refused")]
    PrivateProjectionWireUnavailable,
    #[error("private projection wire was already consumed")]
    PrivateProjectionWireAlreadyConsumed,
    #[error("private projection wire does not bind the prepared semantic transition: {field}")]
    PrivateProjectionWireBindingMismatch { field: &'static str },
    #[error("invalid host ingress")]
    InvalidHostIngress,
    #[error("host base revision mismatch")]
    HostBaseRevisionMismatch,
    #[error("host process epoch mismatch")]
    HostProcessEpochMismatch,
    #[error("invalid host settlement")]
    InvalidHostSettlement,
    #[error("invalid host public effect")]
    InvalidHostPublicEffect,
    #[error("host effect registry full")]
    HostEffectRegistryFull,
    #[error("invalid astrbot tool ingress")]
    InvalidAstrBotToolIngress,
    #[error("invalid astrbot tool outcome")]
    InvalidAstrBotToolOutcome,
    #[error("astrbot tool process epoch mismatch")]
    AstrBotToolProcessEpochMismatch,
    #[error("astrbot tool base revision mismatch")]
    AstrBotToolBaseRevisionMismatch,
    #[error("astrbot tool invocation identity conflict")]
    AstrBotToolIdentityConflict,
    #[error("astrbot tool invocation expired")]
    AstrBotToolInvocationExpired,
    #[error("astrbot tool registry full")]
    AstrBotToolRegistryFull,
}

#[cfg(test)]
mod organism_compatibility_transfer_tests {
    mod committed_semantic_projection_path {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../ae-organism-runtime/tests/support/private_projection_runtime.rs"
        ));
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../ae-organism-runtime/tests/committed_semantic_projection_path.rs"
        ));

        committed_semantic_projection_path_test_contents!();
    }

    mod private_projection_payload_producer {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../ae-organism-runtime/tests/support/private_projection_runtime.rs"
        ));
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../ae-organism-runtime/tests/private_projection_payload_producer.rs"
        ));

        private_projection_payload_producer_test_contents!();
    }
}

const NATIVE_PUBLIC_EFFECT_TRIGGER_V1: &str = "[[AE_NATIVE_PUBLIC_EFFECT_V1]]";
const NATIVE_PUBLIC_EFFECT_TEXT_V1: &str = "AstrEmbodiment native public-effect v1.";
const NATIVE_PUBLIC_EFFECT_TTL_MS: u64 = 30_000;
const MAX_ISSUED_HOST_EFFECTS_V1: usize = 1_024;
const ASTRBOT_TOOL_TTL_MS: u64 = 30_000;
const MAX_ASTRBOT_TOOL_RECORDS_V1: usize = 1_024;
const NATIVE_SEMANTIC_FORMULA_DOMAIN_V1: &[u8] =
    b"astr-embodiment/native-semantic-transition-formula-v1";
const NATIVE_SEMANTIC_STATE_DOMAIN_V1: &[u8] = b"astr-embodiment/native-semantic-neural-field-v1";
const NATIVE_SEMANTIC_GRAPH_DOMAIN_V1: &[u8] = b"astr-embodiment/native-semantic-sparse-graph-v1";
const NATIVE_SEMANTIC_SCOPE_DOMAIN_V1: &[u8] = b"astr-embodiment/native-semantic-scope-v1";
const NATIVE_SEMANTIC_OPTION_ID_DOMAIN_V1: &[u8] = b"astr-embodiment/native-semantic-option-id-v1";
const NATIVE_SEMANTIC_EVENT_DOMAIN_V1: &[u8] = b"astr-embodiment/native-semantic-user-stimulus-v1";
const NATIVE_SEMANTIC_AUTHORITY_DOMAIN_V1: &[u8] = b"astr-embodiment/native-semantic-authority-v1";
const NATIVE_SEMANTIC_ACTION_DOMAIN_V1: &[u8] = b"astr-embodiment/native-semantic-action-v1";
const NATIVE_SEMANTIC_CONTRACT_DOMAIN_V1: &[u8] =
    b"astr-embodiment/native-semantic-action-contract-v1";
const NATIVE_SEMANTIC_ACTION_TTL_MS: u64 = 30_000;

#[derive(Clone, Debug, PartialEq, Eq)]
struct IssuedHostEffectV1 {
    disposition: HostEffectDispositionV1,
    effect_id: [u8; 32],
    action_id: [u8; 32],
    process_epoch_id: [u8; 16],
    adapter_type: String,
    adapter_id_binding: [u8; 32],
    scope_binding: [u8; 32],
    session_binding: [u8; 32],
    turn_binding: [u8; 32],
    expires_at_ms: u64,
    settlement: Option<(HostSettlementStatusV1, DeliveryKnowledgeV1)>,
}

impl IssuedHostEffectV1 {
    fn from_effect(effect: &HostEffectV1) -> Self {
        Self {
            disposition: effect.disposition,
            effect_id: effect.effect_id,
            action_id: effect.action_id,
            process_epoch_id: effect.process_epoch_id,
            adapter_type: effect.adapter_type.clone(),
            adapter_id_binding: effect.adapter_id_binding,
            scope_binding: effect.scope_binding,
            session_binding: effect.session_binding,
            turn_binding: effect.turn_binding,
            expires_at_ms: effect.expires_at_ms,
            settlement: None,
        }
    }

    fn matches_effect(&self, effect: &HostEffectV1) -> bool {
        self.disposition == effect.disposition
            && self.effect_id == effect.effect_id
            && self.action_id == effect.action_id
            && self.process_epoch_id == effect.process_epoch_id
            && self.adapter_type == effect.adapter_type
            && self.adapter_id_binding == effect.adapter_id_binding
            && self.scope_binding == effect.scope_binding
            && self.session_binding == effect.session_binding
            && self.turn_binding == effect.turn_binding
            && self.expires_at_ms == effect.expires_at_ms
    }

    fn matches_settlement(&self, settlement: &HostSettlementV1) -> bool {
        self.effect_id == settlement.effect_id
            && self.action_id == settlement.action_id
            && self.process_epoch_id == settlement.process_epoch_id
            && self.adapter_type == settlement.adapter_type
            && self.adapter_id_binding == settlement.adapter_id_binding
            && self.scope_binding == settlement.scope_binding
            && self.session_binding == settlement.session_binding
            && self.turn_binding == settlement.turn_binding
    }

    fn record_settlement(
        &mut self,
        status: HostSettlementStatusV1,
        delivery: DeliveryKnowledgeV1,
    ) -> Result<(), RuntimeError> {
        let next = (status, delivery);
        match self.settlement {
            None => self.settlement = Some(next),
            Some(previous) if previous == next => {}
            Some(_) if status == HostSettlementStatusV1::DuplicateSuppressed => {
                self.settlement = Some(next)
            }
            Some(_) => return Err(RuntimeError::InvalidHostSettlement),
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AstrBotToolRegistryRecordV1 {
    outcome_id: [u8; 32],
    invocation_id: [u8; 32],
    process_epoch_id: [u8; 32],
    adapter_binding: [u8; 32],
    session_binding: [u8; 32],
    turn_binding: [u8; 32],
    event_binding: [u8; 32],
    revision: u64,
    expires_at_ms: u64,
    disposition: AstrBotToolDispositionV1,
    public_signal: Option<AstrBotPublicSignalV1>,
}

impl AstrBotToolRegistryRecordV1 {
    fn from_outcome(outcome: &AstrBotToolOutcomeV1, expires_at_ms: u64) -> Self {
        Self {
            outcome_id: outcome.outcome_id,
            invocation_id: outcome.invocation_id,
            process_epoch_id: outcome.process_epoch_id,
            adapter_binding: outcome.adapter_binding,
            session_binding: outcome.session_binding,
            turn_binding: outcome.turn_binding,
            event_binding: outcome.event_binding,
            revision: outcome.revision,
            expires_at_ms,
            disposition: outcome.disposition,
            public_signal: outcome.public_signal,
        }
    }

    fn matches_ingress(&self, ingress: &AstrBotToolIngressV1) -> bool {
        self.invocation_id == ingress.invocation_id
            && self.process_epoch_id == ingress.process_epoch_id
            && self.adapter_binding == ingress.adapter_binding
            && self.session_binding == ingress.session_binding
            && self.turn_binding == ingress.turn_binding
            && self.event_binding == ingress.event_binding
    }

    fn outcome(&self) -> AstrBotToolOutcomeV1 {
        AstrBotToolOutcomeV1 {
            schema_version: 1,
            outcome_id: self.outcome_id,
            invocation_id: self.invocation_id,
            process_epoch_id: self.process_epoch_id,
            adapter_binding: self.adapter_binding,
            session_binding: self.session_binding,
            turn_binding: self.turn_binding,
            event_binding: self.event_binding,
            revision: self.revision,
            disposition: self.disposition,
            public_signal: self.public_signal,
        }
    }
}

pub(crate) struct AstrRuntime {
    pub(crate) field: NeuralField,
    pub(crate) graph: SparseGraph,
    pub(crate) revision: u64,
    pub(crate) formula_digest: [u8; 32],
    host_process_epoch: Option<[u8; 16]>,
    last_host_settlement: Option<HostSettlementV1>,
    issued_host_effects: BTreeMap<[u8; 32], IssuedHostEffectV1>,
    astrbot_tool_process_epoch: Option<[u8; 32]>,
    astrbot_tool_registry: BTreeMap<[u8; 32], AstrBotToolRegistryRecordV1>,
}

impl fmt::Debug for AstrRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AstrRuntime")
            .field("revision", &self.revision)
            .field("formula_digest", &self.formula_digest)
            .field(
                "host_process_epoch_bound",
                &self.host_process_epoch.is_some(),
            )
            .field("issued_host_effect_count", &self.issued_host_effects.len())
            .field(
                "astrbot_tool_process_epoch_bound",
                &self.astrbot_tool_process_epoch.is_some(),
            )
            .field(
                "astrbot_tool_registry_count",
                &self.astrbot_tool_registry.len(),
            )
            .finish()
    }
}

pub(crate) struct RuntimeDecision {
    pub(crate) contract: ae_contracts::r7::ActionContract,
    pub(crate) receipt: TransitionReceipt,
    private_projection_wire: PrivateProjectionPayloadWireV1,
}

impl RuntimeDecision {
    /// Transfers the exactly-bound one-shot private projection capability only
    /// after the runtime has atomically committed its matching semantic state.
    pub(crate) fn into_private_projection_wire(self) -> PrivateProjectionPayloadWireV1 {
        self.private_projection_wire
    }
}

/// A pure candidate prepared from the production runtime's durable HotBrain.
/// It contains no mutable runtime state and must be committed by the outer
/// `ae_runtime::AstrRuntime` only after its projection wire is validated.
pub(crate) struct PreparedProductionUserStimulusTransitionV1 {
    pub(crate) next_field: NeuralField,
    pub(crate) active_nodes: u32,
}

struct PreparedSemanticTransitionV1 {
    next_field: NeuralField,
    next_revision: u64,
    state_after: [u8; 32],
    turn_id: [u8; 16],
    scope_digest: [u8; 32],
    event_digest: [u8; 32],
    authority_digest: [u8; 32],
    projection_turn_binding: [u8; 32],
    projection_binding: R7SemanticProjectionBindingV1,
    contract: ActionContract,
    receipt: TransitionReceipt,
}

fn committed_semantic_projection_turn_binding(
    next_revision: u64,
    state_after: &[u8; 32],
    turn_id: &[u8; 16],
    scope_digest: &[u8; 32],
    event_digest: &[u8; 32],
    authority_digest: &[u8; 32],
) -> [u8; 32] {
    let revision = next_revision.to_be_bytes();
    wire::domain_hash(
        b"astr-embodiment/r7/committed-semantic-transition-binding-v1",
        &[
            &revision,
            state_after,
            turn_id,
            scope_digest,
            event_digest,
            authority_digest,
        ],
    )
}

fn all_zero_128(value: &[u8; 16]) -> bool {
    value.iter().all(|byte| *byte == 0)
}

fn all_zero_digest(value: &[u8; 32]) -> bool {
    value.iter().all(|byte| *byte == 0)
}

fn native_formula_digest() -> [u8; 32] {
    wire::domain_hash(
        NATIVE_SEMANTIC_FORMULA_DOMAIN_V1,
        &[
            b"input:canonical-user-stimulus-v1",
            b"validation:closed-typed-estimate-v1",
            b"attention:positive-harm-epistemic-conflict-boundary-v1",
            b"state:potential-and-excitation-regional-load-times-confidence-v1",
            b"contract:scaffold-identity-bound-to-transition-v1",
            b"receipt:canonical-domain-hashes-v1",
        ],
    )
}

fn evidence_values(evidence: &EvidenceVector) -> [Fixed; 15] {
    [
        evidence.positive,
        evidence.affiliation,
        evidence.harm,
        evidence.boundary,
        evidence.repair,
        evidence.repetition,
        evidence.new_information,
        evidence.constraint_instability,
        evidence.epistemic_conflict,
        evidence.self_responsibility,
        evidence.other_responsibility,
        evidence.hostility,
        evidence.publicness,
        evidence.engagement,
        evidence.rejection,
    ]
}

fn fixed_values_digest(domain: &[u8], values: &[Fixed]) -> [u8; 32] {
    let mut encoded = Vec::with_capacity(values.len().saturating_mul(std::mem::size_of::<i64>()));
    for value in values {
        encoded.extend_from_slice(&value.raw().to_be_bytes());
    }
    wire::domain_hash(domain, &[&encoded])
}

fn neural_field_digest(field: &NeuralField) -> [u8; 32] {
    let potential = fixed_values_digest(
        b"astr-embodiment/neural-field/potential-v1",
        &field.potential,
    );
    let excitation = fixed_values_digest(
        b"astr-embodiment/neural-field/excitation-v1",
        &field.excitation,
    );
    let inhibition = fixed_values_digest(
        b"astr-embodiment/neural-field/inhibition-v1",
        &field.inhibition,
    );
    let adaptation = fixed_values_digest(
        b"astr-embodiment/neural-field/adaptation-v1",
        &field.adaptation,
    );
    let precision = fixed_values_digest(
        b"astr-embodiment/neural-field/precision-v1",
        &field.precision,
    );
    let prediction_error = fixed_values_digest(
        b"astr-embodiment/neural-field/prediction-error-v1",
        &field.prediction_error,
    );
    let eligibility = fixed_values_digest(
        b"astr-embodiment/neural-field/eligibility-v1",
        &field.eligibility,
    );
    let metabolic_reserve = fixed_values_digest(
        b"astr-embodiment/neural-field/metabolic-reserve-v1",
        &field.metabolic_reserve,
    );
    wire::domain_hash(
        NATIVE_SEMANTIC_STATE_DOMAIN_V1,
        &[
            &potential,
            &excitation,
            &inhibition,
            &adaptation,
            &precision,
            &prediction_error,
            &eligibility,
            &metabolic_reserve,
        ],
    )
}

fn sparse_graph_digest(graph: &SparseGraph) -> [u8; 32] {
    let mut row_offsets = Vec::with_capacity(graph.row_offsets.len().saturating_mul(4));
    for offset in &graph.row_offsets {
        row_offsets.extend_from_slice(&offset.to_be_bytes());
    }
    let mut edges = Vec::with_capacity(graph.edges.len().saturating_mul(16));
    for edge in &graph.edges {
        edges.extend_from_slice(&edge.target.to_be_bytes());
        edges.extend_from_slice(&edge.weight.to_be_bytes());
        edges.extend_from_slice(&edge.eligibility.to_be_bytes());
        edges.extend_from_slice(&edge.stability.to_be_bytes());
        edges.extend_from_slice(&edge.last_used_epoch.to_be_bytes());
        edges.push(edge.operator_id);
        edges.push(edge.delay_class);
        edges.extend_from_slice(&edge.flags.to_be_bytes());
    }
    wire::domain_hash(NATIVE_SEMANTIC_GRAPH_DOMAIN_V1, &[&row_offsets, &edges])
}

fn option_id_digest(value: Option<[u8; 16]>) -> [u8; 32] {
    match value {
        None => wire::domain_hash(NATIVE_SEMANTIC_OPTION_ID_DOMAIN_V1, &[b"none"]),
        Some(id) => wire::domain_hash(NATIVE_SEMANTIC_OPTION_ID_DOMAIN_V1, &[b"some", &id]),
    }
}

fn scope_digest(scope: &ScopeRef) -> [u8; 32] {
    let relation = option_id_digest(scope.relation_token);
    wire::domain_hash(
        NATIVE_SEMANTIC_SCOPE_DOMAIN_V1,
        &[
            &scope.bot_token,
            &scope.persona_token,
            &relation,
            &scope.session_token,
        ],
    )
}

fn semantic_estimate_digest(
    evidence: &EvidenceVector,
    confidence: Fixed,
    digest: &[u8; 32],
) -> [u8; 32] {
    let dimensions = fixed_values_digest(
        b"astr-embodiment/native-semantic-evidence-vector-v1",
        &evidence_values(evidence),
    );
    let confidence = confidence.raw().to_be_bytes();
    wire::domain_hash(
        b"astr-embodiment/native-semantic-estimate-v1",
        &[&dimensions, &confidence, digest],
    )
}

fn user_stimulus_digest(stimulus: &UserStimulus, scope: &[u8; 32]) -> [u8; 32] {
    let action_id = option_id_digest(stimulus.causal.action_id);
    let delivery_id = option_id_digest(stimulus.causal.delivery_id);
    let claim_id = option_id_digest(stimulus.causal.claim_id);
    let observed_at_ms = stimulus.observed_at_ms.to_be_bytes();
    let base_revision = stimulus.causal.base_revision.to_be_bytes();
    let schema_version = stimulus.evidence.schema_version.to_be_bytes();
    let estimate = semantic_estimate_digest(
        &stimulus.evidence.dimensions,
        stimulus.evidence.estimator_confidence,
        &stimulus.evidence.estimator_digest,
    );
    wire::domain_hash(
        NATIVE_SEMANTIC_EVENT_DOMAIN_V1,
        &[
            &stimulus.event_id,
            scope,
            &stimulus.causal.turn_id,
            &action_id,
            &delivery_id,
            &claim_id,
            &observed_at_ms,
            &base_revision,
            &schema_version,
            &estimate,
        ],
    )
}

fn authority_name(authority: SourceAuthority) -> &'static [u8] {
    match authority {
        SourceAuthority::UserObserved => b"user_observed",
        SourceAuthority::ExplicitFeedback => b"explicit_feedback",
        SourceAuthority::PlatformObserved => b"platform_observed",
        SourceAuthority::VerifierResult => b"verifier_result",
        SourceAuthority::SelfAction => b"self_action",
        SourceAuthority::SelfCritique => b"self_critique",
        SourceAuthority::TimeAdvance => b"time_advance",
        SourceAuthority::AdminAction => b"admin_action",
    }
}

fn action_contract_digest(contract: &ActionContract) -> [u8; 32] {
    let continuous = fixed_values_digest(
        b"astr-embodiment/native-semantic-action-vector-v1",
        &[
            contract.continuous.answer,
            contract.continuous.verify,
            contract.continuous.acknowledge_error,
            contract.continuous.repair,
            contract.continuous.ask_evidence,
            contract.continuous.set_boundary,
            contract.continuous.withdraw,
            contract.continuous.proactive_reach,
            contract.continuous.warmth,
            contract.continuous.directness,
            contract.continuous.verbosity,
            contract.continuous.confidence_ceiling,
        ],
    );
    let flags = [
        u8::from(contract.must_verify),
        u8::from(contract.must_acknowledge_error),
        u8::from(contract.must_correct_claim),
        u8::from(contract.may_set_boundary),
        u8::from(contract.may_withdraw),
        u8::from(contract.must_not_seek_reassurance),
    ];
    let expires_at_ms = contract.expires_at_ms.to_be_bytes();
    wire::domain_hash(
        NATIVE_SEMANTIC_CONTRACT_DOMAIN_V1,
        &[
            &contract.action_id,
            &contract.turn_id,
            &continuous,
            &flags,
            &expires_at_ms,
        ],
    )
}

fn validate_user_stimulus(stimulus: &UserStimulus, revision: u64) -> Result<(), RuntimeError> {
    if all_zero_128(&stimulus.event_id)
        || all_zero_128(&stimulus.scope.bot_token)
        || all_zero_128(&stimulus.scope.persona_token)
        || all_zero_128(&stimulus.scope.session_token)
        || all_zero_128(&stimulus.causal.turn_id)
        || stimulus.observed_at_ms == 0
        || [
            stimulus.scope.relation_token,
            stimulus.causal.action_id,
            stimulus.causal.delivery_id,
            stimulus.causal.claim_id,
        ]
        .into_iter()
        .flatten()
        .any(|id| all_zero_128(&id))
    {
        return Err(RuntimeError::InvalidUserStimulus);
    }
    if stimulus.causal.base_revision != revision {
        return Err(RuntimeError::UserStimulusBaseRevisionMismatch);
    }
    let evidence = &stimulus.evidence;
    let dimensions = evidence_values(&evidence.dimensions);
    if evidence.schema_version != 1
        || all_zero_digest(&evidence.estimator_digest)
        || evidence.estimator_confidence <= Fixed::ZERO
        || evidence.estimator_confidence > Fixed::ONE
        || dimensions
            .into_iter()
            .any(|value| value < Fixed::ZERO || value > Fixed::ONE)
        || dimensions.into_iter().all(|value| value == Fixed::ZERO)
    {
        return Err(RuntimeError::InvalidSemanticEstimate);
    }
    Ok(())
}

fn native_action_contract(
    stimulus: &UserStimulus,
    scope: &[u8; 32],
    event: &[u8; 32],
    authority: &[u8; 32],
    state_after: &[u8; 32],
) -> Result<ActionContract, RuntimeError> {
    let action_digest = wire::domain_hash(
        NATIVE_SEMANTIC_ACTION_DOMAIN_V1,
        &[scope, event, authority, state_after],
    );
    let mut action_id = [0; 16];
    action_id.copy_from_slice(&action_digest[..16]);
    let expires_at_ms = stimulus
        .observed_at_ms
        .checked_add(NATIVE_SEMANTIC_ACTION_TTL_MS)
        .ok_or(RuntimeError::InvalidUserStimulus)?;
    let mut contract = scaffold_contract(&empty_workspace(), stimulus.causal.turn_id);
    contract.action_id = action_id;
    contract.expires_at_ms = expires_at_ms;
    Ok(contract)
}

impl AstrRuntime {
    /// Constructs an isolated, non-durable fixture harness for Rust-only
    /// Host/projection tests. Production `ae_runtime::AstrRuntime` never owns
    /// or delegates semantic state to this scaffold.
    pub(crate) fn scaffold() -> Self {
        Self {
            field: NeuralField::zeroed(),
            graph: SparseGraph {
                row_offsets: vec![0; NEURON_SLOTS + 1],
                edges: Vec::new(),
            },
            revision: 0,
            formula_digest: native_formula_digest(),
            host_process_epoch: None,
            last_host_settlement: None,
            issued_host_effects: BTreeMap::new(),
            astrbot_tool_process_epoch: None,
            astrbot_tool_registry: BTreeMap::new(),
        }
    }

    pub(crate) fn current_revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn last_host_settlement(&self) -> Option<&HostSettlementV1> {
        self.last_host_settlement.as_ref()
    }

    fn prune_completed_issued_effects(&mut self, observed_at_ms: u64) {
        self.issued_host_effects.retain(|_, record| {
            record.settlement.is_none() || record.expires_at_ms >= observed_at_ms
        });
    }

    fn register_issued_effect(&mut self, effect: &HostEffectV1) -> Result<(), RuntimeError> {
        if let Some(existing) = self.issued_host_effects.get(&effect.effect_id) {
            return if existing.matches_effect(effect) {
                Ok(())
            } else {
                Err(RuntimeError::InvalidHostPublicEffect)
            };
        }
        if self.issued_host_effects.len() >= MAX_ISSUED_HOST_EFFECTS_V1 {
            return Err(RuntimeError::HostEffectRegistryFull);
        }
        self.issued_host_effects
            .insert(effect.effect_id, IssuedHostEffectV1::from_effect(effect));
        Ok(())
    }

    pub(crate) fn apply_astrbot_tool_v1(
        &mut self,
        ingress: AstrBotToolIngressV1,
    ) -> Result<AstrBotToolOutcomeV1, RuntimeError> {
        if let Some(record) = self.astrbot_tool_registry.get(&ingress.invocation_id) {
            if ingress.validate_shape().is_err() || !record.matches_ingress(&ingress) {
                return Err(RuntimeError::AstrBotToolIdentityConflict);
            }
        } else {
            ingress
                .validate_shape()
                .map_err(|_| RuntimeError::InvalidAstrBotToolIngress)?;
        }

        if let Some(epoch) = self.astrbot_tool_process_epoch {
            if epoch != ingress.process_epoch_id {
                return Err(RuntimeError::AstrBotToolProcessEpochMismatch);
            }
        }

        if let Some(record) = self.astrbot_tool_registry.get(&ingress.invocation_id) {
            if ingress.observed_at_ms >= record.expires_at_ms {
                self.astrbot_tool_registry.remove(&ingress.invocation_id);
                return Err(RuntimeError::AstrBotToolInvocationExpired);
            }
            let outcome = record.outcome();
            outcome
                .validate_shape()
                .map_err(|_| RuntimeError::InvalidAstrBotToolOutcome)?;
            return Ok(outcome);
        }

        if ingress.base_revision != self.revision {
            return Err(RuntimeError::AstrBotToolBaseRevisionMismatch);
        }

        self.astrbot_tool_registry
            .retain(|_, record| record.expires_at_ms > ingress.observed_at_ms);
        if self.astrbot_tool_registry.len() >= MAX_ASTRBOT_TOOL_RECORDS_V1 {
            return Err(RuntimeError::AstrBotToolRegistryFull);
        }

        let next_revision = self
            .revision
            .checked_add(1)
            .ok_or(RuntimeError::InvalidAstrBotToolOutcome)?;
        let (disposition, public_signal) =
            if ingress.current_event_text == NATIVE_PUBLIC_EFFECT_TRIGGER_V1 {
                (
                    AstrBotToolDispositionV1::PublicSignal,
                    Some(AstrBotPublicSignalV1::Observed),
                )
            } else {
                (AstrBotToolDispositionV1::Silence, None)
            };
        let outcome =
            AstrBotToolOutcomeV1::for_ingress(&ingress, next_revision, disposition, public_signal)
                .map_err(|_| RuntimeError::InvalidAstrBotToolOutcome)?;
        let expires_at_ms = ingress
            .observed_at_ms
            .checked_add(ASTRBOT_TOOL_TTL_MS)
            .ok_or(RuntimeError::InvalidAstrBotToolIngress)?;

        self.astrbot_tool_registry.insert(
            ingress.invocation_id,
            AstrBotToolRegistryRecordV1::from_outcome(&outcome, expires_at_ms),
        );
        if self.astrbot_tool_process_epoch.is_none() {
            self.astrbot_tool_process_epoch = Some(ingress.process_epoch_id);
        }
        self.revision = next_revision;
        Ok(outcome)
    }

    pub(crate) fn apply_host_ingress_v1(
        &mut self,
        ingress: HostIngressV1,
    ) -> Result<Option<HostEffectV1>, RuntimeError> {
        ingress
            .validate_shape()
            .map_err(|_| RuntimeError::InvalidHostIngress)?;
        if ingress.base_revision != self.revision {
            return Err(RuntimeError::HostBaseRevisionMismatch);
        }

        match ingress.kind {
            HostIngressKindV1::CurrentEvent => {
                if let Some(epoch) = self.host_process_epoch {
                    if epoch != ingress.process_epoch_id {
                        return Err(RuntimeError::HostProcessEpochMismatch);
                    }
                }
                let next_revision = self
                    .revision
                    .checked_add(1)
                    .ok_or(RuntimeError::InvalidHostPublicEffect)?;
                let revision = next_revision.to_le_bytes();
                let effect = if ingress.current_event_text.as_deref()
                    == Some(NATIVE_PUBLIC_EFFECT_TRIGGER_V1)
                {
                    let action_id = wire::domain_hash(
                        b"astr-embodiment/native-public-action-v1",
                        &[
                            &ingress.process_epoch_id,
                            &ingress.adapter_id_binding,
                            &ingress.scope_binding,
                            &ingress.session_binding,
                            &ingress.turn_binding,
                            &ingress.event_id,
                        ],
                    );
                    let authority_evidence_digest = wire::domain_hash(
                        b"astr-embodiment/explicit-public-trigger-authority-v1",
                        &[&action_id, &ingress.event_id],
                    );
                    let policy_evidence_digest = wire::domain_hash(
                        b"astr-embodiment/fixed-public-text-policy-v1",
                        &[&action_id, NATIVE_PUBLIC_EFFECT_TEXT_V1.as_bytes()],
                    );
                    let expires_at_ms = ingress
                        .observed_at_ms
                        .checked_add(NATIVE_PUBLIC_EFFECT_TTL_MS)
                        .ok_or(RuntimeError::InvalidHostPublicEffect)?;
                    HostEffectV1::public_for_ingress_v1(
                        &ingress,
                        action_id,
                        NATIVE_PUBLIC_EFFECT_TEXT_V1.to_owned(),
                        authority_evidence_digest,
                        policy_evidence_digest,
                        expires_at_ms,
                    )
                    .map_err(|_| RuntimeError::InvalidHostPublicEffect)?
                } else {
                    let action_id = wire::domain_hash(
                        b"astr-embodiment/host-silence-action-v1",
                        &[&ingress.ingress_id, &ingress.turn_binding, &revision],
                    );
                    HostEffectV1::silence_for_ingress(&ingress, action_id)
                };
                effect
                    .validate_shape()
                    .map_err(|_| RuntimeError::InvalidHostPublicEffect)?;
                self.prune_completed_issued_effects(ingress.observed_at_ms);
                self.register_issued_effect(&effect)?;
                if self.host_process_epoch.is_none() {
                    self.host_process_epoch = Some(ingress.process_epoch_id);
                }
                self.revision = next_revision;
                Ok(Some(effect))
            }
            HostIngressKindV1::EffectSettlement => {
                if self.host_process_epoch != Some(ingress.process_epoch_id) {
                    return Err(RuntimeError::HostProcessEpochMismatch);
                }
                let settlement = ingress
                    .settlement
                    .ok_or(RuntimeError::InvalidHostSettlement)?;
                let issued = self
                    .issued_host_effects
                    .get_mut(&settlement.effect_id)
                    .ok_or(RuntimeError::InvalidHostSettlement)?;
                if !issued.matches_settlement(&settlement) {
                    return Err(RuntimeError::InvalidHostSettlement);
                }
                issued.record_settlement(settlement.status, settlement.delivery)?;
                self.last_host_settlement = Some(settlement);
                self.revision = self
                    .revision
                    .checked_add(1)
                    .ok_or(RuntimeError::InvalidHostSettlement)?;
                Ok(None)
            }
        }
    }

    /// Evaluates a closed user stimulus and atomically compiles its exact
    /// private projection capability. The crate-private runtime never accepts
    /// a callback, byte buffer, custom sealer, or caller-supplied wire.
    pub(crate) fn apply_user_stimulus_with_private_projection_wire_v1(
        &mut self,
        event: &CanonicalEvent,
        input: &R7PreOutputProjectionInputV1,
    ) -> Result<RuntimeDecision, RuntimeError> {
        let candidate = self.prepare_user_stimulus_transition_v1(event)?;
        let wire = compile_atomic_pre_output_wire_v1(&candidate.projection_binding, input)
            .map_err(|_| RuntimeError::PrivateProjectionWireUnavailable)?;
        self.commit_prepared_projection_wire_v1(candidate, wire)
    }

    fn commit_prepared_projection_wire_v1(
        &mut self,
        candidate: PreparedSemanticTransitionV1,
        wire: PrivateProjectionPayloadWireV1,
    ) -> Result<RuntimeDecision, RuntimeError> {
        self.validate_prepared_projection_wire_v1(&candidate, &wire)?;
        let PreparedSemanticTransitionV1 {
            next_field,
            next_revision,
            contract,
            receipt,
            ..
        } = candidate;
        self.field = next_field;
        self.revision = next_revision;
        Ok(RuntimeDecision {
            contract,
            receipt,
            private_projection_wire: wire,
        })
    }

    fn prepare_user_stimulus_transition_v1(
        &self,
        event: &CanonicalEvent,
    ) -> Result<PreparedSemanticTransitionV1, RuntimeError> {
        if !self.field.validate() {
            return Err(RuntimeError::InvalidNeuralField);
        }
        if !self.graph.validate() {
            return Err(RuntimeError::InvalidSparseGraph);
        }
        if self.formula_digest != native_formula_digest() {
            return Err(RuntimeError::NativeFormulaDigestMismatch);
        }

        let stimulus = match event {
            CanonicalEvent::UserStimulus(stimulus) => stimulus,
            _ => return Err(RuntimeError::UnsupportedEvent),
        };
        validate_user_stimulus(stimulus, self.revision)?;

        let scope = scope_digest(&stimulus.scope);
        let event_digest = user_stimulus_digest(stimulus, &scope);
        let authority_digest = wire::domain_hash(
            NATIVE_SEMANTIC_AUTHORITY_DOMAIN_V1,
            &[
                authority_name(event.authority()),
                &stimulus.evidence.estimator_digest,
                &event_digest,
            ],
        );
        let state_before = neural_field_digest(&self.field);
        let load = assemble_load(&stimulus.evidence.dimensions, NEURON_SLOTS as u32);
        if load.active_nodes.is_empty() {
            return Err(RuntimeError::InvalidSemanticEstimate);
        }

        let confidence = stimulus.evidence.estimator_confidence;
        if load.regional_loads.is_empty() {
            return Err(RuntimeError::InvalidSemanticEstimate);
        }

        let mut next_field = self.field.clone();
        for (position, node) in load.active_nodes.iter().enumerate() {
            let index = *node as usize;
            let regional_load = load.regional_loads[position % load.regional_loads.len()]
                .checked_mul(confidence)
                .ok_or(RuntimeError::InvalidSemanticEstimate)?;
            next_field.potential[index] = next_field.potential[index].saturating_add(regional_load);
            next_field.excitation[index] =
                next_field.excitation[index].saturating_add(regional_load);
        }
        if !next_field.validate() {
            return Err(RuntimeError::InvalidNeuralField);
        }
        let state_after = neural_field_digest(&next_field);
        if state_before == state_after {
            return Err(RuntimeError::NativeStateUnchanged);
        }
        let graph_after = sparse_graph_digest(&self.graph);
        let contract = native_action_contract(
            stimulus,
            &scope,
            &event_digest,
            &authority_digest,
            &state_after,
        )?;
        let action_contract = action_contract_digest(&contract);
        let next_revision = self
            .revision
            .checked_add(1)
            .ok_or(RuntimeError::RevisionOverflow)?;
        let projection_turn_binding = committed_semantic_projection_turn_binding(
            next_revision,
            &state_after,
            &stimulus.causal.turn_id,
            &scope,
            &event_digest,
            &authority_digest,
        );
        let projection_binding = R7SemanticProjectionBindingV1::new(
            next_revision,
            state_after,
            stimulus.causal.turn_id,
            scope,
            event_digest,
            authority_digest,
            projection_turn_binding,
        );
        let receipt = TransitionReceipt {
            schema_version: 1,
            formula_digest: self.formula_digest,
            scope_digest: scope,
            event_digest,
            authority_digest,
            base_revision: self.revision,
            next_revision,
            state_before,
            state_after,
            graph_after,
            action_contract: Some(action_contract),
            active_nodes: u32::try_from(load.active_nodes.len())
                .map_err(|_| RuntimeError::InvalidNeuralField)?,
            active_edges: self.graph.edges.len() as u32,
            residuals: InvariantResiduals::default(),
            status: CommitStatus::Committed,
        };
        Ok(PreparedSemanticTransitionV1 {
            next_field,
            next_revision,
            state_after,
            turn_id: stimulus.causal.turn_id,
            scope_digest: scope,
            event_digest,
            authority_digest,
            projection_turn_binding,
            projection_binding,
            contract,
            receipt,
        })
    }

    fn validate_prepared_projection_wire_v1(
        &self,
        candidate: &PreparedSemanticTransitionV1,
        wire: &PrivateProjectionPayloadWireV1,
    ) -> Result<(), RuntimeError> {
        wire.validate_live_canonical_v1()
            .map_err(|error| match error {
                PrivateProjectionPayloadWireErrorV1::AlreadyConsumed => {
                    RuntimeError::PrivateProjectionWireAlreadyConsumed
                }
                _ => RuntimeError::PrivateProjectionWireUnavailable,
            })?;
        let metadata = wire.binding_metadata();
        if metadata.revision() != candidate.next_revision {
            return Err(RuntimeError::PrivateProjectionWireBindingMismatch { field: "revision" });
        }
        for (field, actual, expected) in [
            (
                "turn_id",
                metadata.turn_id().as_slice(),
                candidate.turn_id.as_slice(),
            ),
            (
                "turn_binding",
                metadata.turn_binding().as_slice(),
                candidate.projection_turn_binding.as_slice(),
            ),
            (
                "source_state_digest",
                metadata.source_state_digest().as_slice(),
                candidate.state_after.as_slice(),
            ),
        ] {
            if actual != expected {
                return Err(RuntimeError::PrivateProjectionWireBindingMismatch { field });
            }
        }
        // `projection_turn_binding` canonically commits all of these values,
        // including the full closed event digest, so the fixed wire header need
        // not grow or drift.
        let expected_turn_binding = committed_semantic_projection_turn_binding(
            candidate.next_revision,
            &candidate.state_after,
            &candidate.turn_id,
            &candidate.scope_digest,
            &candidate.event_digest,
            &candidate.authority_digest,
        );
        if metadata.turn_binding() != &expected_turn_binding {
            return Err(RuntimeError::PrivateProjectionWireBindingMismatch {
                field: "scope_event_authority_binding",
            });
        }
        Ok(())
    }
}

pub(crate) fn prepare_production_user_stimulus_transition_v1(
    event: &CanonicalEvent,
    field: &NeuralField,
    graph: &SparseGraph,
    revision: u64,
) -> Result<PreparedProductionUserStimulusTransitionV1, RuntimeError> {
    if !field.validate() {
        return Err(RuntimeError::InvalidNeuralField);
    }
    if !graph.validate() {
        return Err(RuntimeError::InvalidSparseGraph);
    }
    let stimulus = match event {
        CanonicalEvent::UserStimulus(stimulus) => stimulus,
        _ => return Err(RuntimeError::UnsupportedEvent),
    };
    validate_user_stimulus(stimulus, revision)?;
    let load = assemble_load(&stimulus.evidence.dimensions, NEURON_SLOTS as u32);
    if load.active_nodes.is_empty() || load.regional_loads.is_empty() {
        return Err(RuntimeError::InvalidSemanticEstimate);
    }

    let mut next_field = field.clone();
    for (position, node) in load.active_nodes.iter().enumerate() {
        let regional_load = load.regional_loads[position % load.regional_loads.len()]
            .checked_mul(stimulus.evidence.estimator_confidence)
            .ok_or(RuntimeError::InvalidSemanticEstimate)?;
        let index = *node as usize;
        next_field.potential[index] = next_field.potential[index].saturating_add(regional_load);
        next_field.excitation[index] = next_field.excitation[index].saturating_add(regional_load);
    }
    if !next_field.validate() {
        return Err(RuntimeError::InvalidNeuralField);
    }
    Ok(PreparedProductionUserStimulusTransitionV1 {
        next_field,
        active_nodes: u32::try_from(load.active_nodes.len())
            .map_err(|_| RuntimeError::InvalidNeuralField)?,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn compile_and_validate_production_private_projection_wire_v1(
    event: &CanonicalEvent,
    next_revision: u64,
    state_after: [u8; 32],
    scope_digest: [u8; 32],
    event_digest: [u8; 32],
    authority_digest: [u8; 32],
    input: &R7PreOutputProjectionInputV1,
) -> Result<PrivateProjectionPayloadWireV1, RuntimeError> {
    let stimulus = match event {
        CanonicalEvent::UserStimulus(stimulus) => stimulus,
        _ => return Err(RuntimeError::UnsupportedEvent),
    };
    let turn_id = stimulus.causal.turn_id;
    let projection_turn_binding = committed_semantic_projection_turn_binding(
        next_revision,
        &state_after,
        &turn_id,
        &scope_digest,
        &event_digest,
        &authority_digest,
    );
    let binding = R7SemanticProjectionBindingV1::new(
        next_revision,
        state_after,
        turn_id,
        scope_digest,
        event_digest,
        authority_digest,
        projection_turn_binding,
    );
    let wire = compile_atomic_pre_output_wire_v1(&binding, input)
        .map_err(|_| RuntimeError::PrivateProjectionWireUnavailable)?;
    validate_projection_wire_v1(
        next_revision,
        &state_after,
        &turn_id,
        &scope_digest,
        &event_digest,
        &authority_digest,
        &projection_turn_binding,
        &wire,
    )?;
    Ok(wire)
}

#[allow(clippy::too_many_arguments)]
fn validate_projection_wire_v1(
    next_revision: u64,
    state_after: &[u8; 32],
    turn_id: &[u8; 16],
    scope_digest: &[u8; 32],
    event_digest: &[u8; 32],
    authority_digest: &[u8; 32],
    projection_turn_binding: &[u8; 32],
    wire: &PrivateProjectionPayloadWireV1,
) -> Result<(), RuntimeError> {
    wire.validate_live_canonical_v1()
        .map_err(|error| match error {
            PrivateProjectionPayloadWireErrorV1::AlreadyConsumed => {
                RuntimeError::PrivateProjectionWireAlreadyConsumed
            }
            _ => RuntimeError::PrivateProjectionWireUnavailable,
        })?;
    let metadata = wire.binding_metadata();
    if metadata.revision() != next_revision {
        return Err(RuntimeError::PrivateProjectionWireBindingMismatch { field: "revision" });
    }
    for (field, actual, expected) in [
        ("turn_id", metadata.turn_id().as_slice(), turn_id.as_slice()),
        (
            "turn_binding",
            metadata.turn_binding().as_slice(),
            projection_turn_binding.as_slice(),
        ),
        (
            "source_state_digest",
            metadata.source_state_digest().as_slice(),
            state_after.as_slice(),
        ),
    ] {
        if actual != expected {
            return Err(RuntimeError::PrivateProjectionWireBindingMismatch { field });
        }
    }
    let expected_turn_binding = committed_semantic_projection_turn_binding(
        next_revision,
        state_after,
        turn_id,
        scope_digest,
        event_digest,
        authority_digest,
    );
    if metadata.turn_binding() != &expected_turn_binding {
        return Err(RuntimeError::PrivateProjectionWireBindingMismatch {
            field: "scope_event_authority_binding",
        });
    }
    Ok(())
}

#[cfg(test)]
mod issued_registry_privacy_tests {
    use super::*;

    const RAW_SESSION_ORIGIN: &str = "PRIVATE_RAW_SESSION_ORIGIN_SENTINEL";
    const PROVIDER_PROMPT: &str = "PRIVATE_PROVIDER_PROMPT_SENTINEL";
    const PROVIDER_CONTEXT: &str = "PRIVATE_PROVIDER_CONTEXT_SENTINEL";
    const PROVIDER_TOOL: &str = "PRIVATE_PROVIDER_TOOL_SENTINEL";
    const EMOTION: &str = "PRIVATE_EMOTION_SENTINEL";
    const EXOCORTEX: &str = "PRIVATE_EXOCORTEX_SENTINEL";
    const AUTHORITY_SOURCE: &str = "PRIVATE_AUTHORITY_SOURCE_SENTINEL";
    const POLICY_SOURCE: &str = "PRIVATE_POLICY_SOURCE_SENTINEL";
    const EXCEPTION: &str = "PRIVATE_EXCEPTION_SENTINEL";
    const HISTORY: &str = "PRIVATE_HISTORY_SENTINEL";

    #[test]
    fn issued_registry_excludes_raw_current_public_and_private_source_text() {
        let ingress = HostIngressV1 {
            schema_version: 1,
            kind: HostIngressKindV1::CurrentEvent,
            ingress_id: [1; 32],
            process_epoch_id: [7; 16],
            adapter_type: "lark".to_owned(),
            adapter_id_binding: [2; 32],
            scope_binding: [3; 32],
            session_binding: [4; 32],
            turn_binding: [5; 32],
            event_id: [6; 32],
            observed_at_ms: 1_000,
            base_revision: 0,
            current_event_text: Some(NATIVE_PUBLIC_EFFECT_TRIGGER_V1.to_owned()),
            settlement: None,
        };
        let mut runtime = AstrRuntime::scaffold();
        let effect = runtime.apply_host_ingress_v1(ingress).unwrap().unwrap();
        assert_eq!(effect.disposition, HostEffectDispositionV1::PublicEffect);
        assert_eq!(
            effect
                .public_payload
                .as_ref()
                .map(|payload| payload.text.as_str()),
            Some(NATIVE_PUBLIC_EFFECT_TEXT_V1)
        );

        let record = runtime.issued_host_effects.get(&effect.effect_id).unwrap();
        assert_eq!(record.disposition, HostEffectDispositionV1::PublicEffect);
        assert_eq!(record.effect_id, effect.effect_id);
        assert_eq!(record.action_id, effect.action_id);
        assert_eq!(record.adapter_type, "lark");
        assert_eq!(record.expires_at_ms, effect.expires_at_ms);
        assert_eq!(record.settlement, None);

        let rendered = format!("{record:?}");
        for forbidden in [
            NATIVE_PUBLIC_EFFECT_TRIGGER_V1,
            NATIVE_PUBLIC_EFFECT_TEXT_V1,
            RAW_SESSION_ORIGIN,
            PROVIDER_PROMPT,
            PROVIDER_CONTEXT,
            PROVIDER_TOOL,
            EMOTION,
            EXOCORTEX,
            AUTHORITY_SOURCE,
            POLICY_SOURCE,
            EXCEPTION,
            HISTORY,
        ] {
            assert!(!rendered.contains(forbidden));
        }
    }
}

#[cfg(test)]
mod atomic_private_projection_wire_tests {
    use super::private_projection_wire::{
        test_only_tampered_wire_for_metadata_with_probe_v1,
        test_only_wire_for_metadata_with_probe_v1, PrivateProjectionPayloadWireBindingMetadataV1,
        TestOnlyZeroizationProbeV1,
    };
    use super::*;

    fn closed_stimulus() -> CanonicalEvent {
        CanonicalEvent::UserStimulus(UserStimulus {
            event_id: [61; 16],
            scope: ScopeRef {
                bot_token: [62; 16],
                persona_token: [63; 16],
                relation_token: Some([64; 16]),
                session_token: [65; 16],
            },
            causal: ae_contracts::r7::CausalRef {
                turn_id: [12; 16],
                action_id: None,
                delivery_id: None,
                claim_id: None,
                base_revision: 0,
            },
            observed_at_ms: 1_000,
            evidence: ae_contracts::r7::SemanticEstimate {
                schema_version: 1,
                dimensions: EvidenceVector {
                    positive: Fixed::from_raw(300_000),
                    harm: Fixed::from_raw(100_000),
                    epistemic_conflict: Fixed::from_raw(200_000),
                    boundary: Fixed::from_raw(150_000),
                    ..EvidenceVector::default()
                },
                estimator_confidence: Fixed::from_raw(800_000),
                estimator_digest: [66; 32],
            },
        })
    }

    #[test]
    fn preconsumed_binding_correct_wire_cannot_advance_runtime_state_or_revision() {
        let event = closed_stimulus();
        let mut runtime = AstrRuntime::scaffold();
        let potential_before = runtime.field.potential.clone();
        let excitation_before = runtime.field.excitation.clone();
        let candidate = runtime
            .prepare_user_stimulus_transition_v1(&event)
            .expect("closed semantic candidate");
        let metadata = PrivateProjectionPayloadWireBindingMetadataV1::new(
            candidate.next_revision,
            candidate.turn_id,
            candidate.projection_turn_binding,
            [18; 32],
            candidate.state_after,
        )
        .expect("candidate metadata is binding-correct");
        let probe = TestOnlyZeroizationProbeV1::default();
        let mut wire = test_only_wire_for_metadata_with_probe_v1(metadata, probe.clone())
            .expect("canonical test capability");
        let transfer = wire
            .begin_transfer_once_v1()
            .expect("pre-transfer exact capability");
        drop(transfer);
        probe.assert_zeroized_observations(3);

        assert!(matches!(
            runtime.commit_prepared_projection_wire_v1(candidate, wire),
            Err(RuntimeError::PrivateProjectionWireAlreadyConsumed)
        ));
        assert_eq!(runtime.current_revision(), 0);
        assert_eq!(runtime.field.potential, potential_before);
        assert_eq!(runtime.field.excitation, excitation_before);
    }

    #[test]
    fn tampered_binding_correct_wire_cannot_advance_runtime_state_or_revision() {
        let event = closed_stimulus();
        let mut runtime = AstrRuntime::scaffold();
        let potential_before = runtime.field.potential.clone();
        let excitation_before = runtime.field.excitation.clone();
        let candidate = runtime
            .prepare_user_stimulus_transition_v1(&event)
            .expect("closed semantic candidate");
        let metadata = PrivateProjectionPayloadWireBindingMetadataV1::new(
            candidate.next_revision,
            candidate.turn_id,
            candidate.projection_turn_binding,
            [18; 32],
            candidate.state_after,
        )
        .expect("candidate metadata is binding-correct");
        let probe = TestOnlyZeroizationProbeV1::default();
        let wire = test_only_tampered_wire_for_metadata_with_probe_v1(metadata, probe.clone())
            .expect("tampered capability keeps matching private metadata");

        assert!(matches!(
            runtime.commit_prepared_projection_wire_v1(candidate, wire),
            Err(RuntimeError::PrivateProjectionWireUnavailable)
        ));
        assert_eq!(runtime.current_revision(), 0);
        assert_eq!(runtime.field.potential, potential_before);
        assert_eq!(runtime.field.excitation, excitation_before);
        probe.assert_zeroized_observations(3);
    }

    #[test]
    fn binding_correct_other_event_wire_cannot_advance_the_target_runtime() {
        let event = closed_stimulus();
        let mut target_runtime = AstrRuntime::scaffold();
        let potential_before = target_runtime.field.potential.clone();
        let excitation_before = target_runtime.field.excitation.clone();
        let target_candidate = target_runtime
            .prepare_user_stimulus_transition_v1(&event)
            .expect("target candidate");

        let mut other_event = closed_stimulus();
        let CanonicalEvent::UserStimulus(stimulus) = &mut other_event else {
            panic!("fixture is a user stimulus");
        };
        stimulus.event_id = [79; 16];
        let other_candidate = AstrRuntime::scaffold()
            .prepare_user_stimulus_transition_v1(&other_event)
            .expect("other-event candidate");
        let other_metadata = PrivateProjectionPayloadWireBindingMetadataV1::new(
            other_candidate.next_revision,
            other_candidate.turn_id,
            other_candidate.projection_turn_binding,
            [18; 32],
            other_candidate.state_after,
        )
        .expect("other-event metadata");
        let probe = TestOnlyZeroizationProbeV1::default();
        let other_wire = test_only_wire_for_metadata_with_probe_v1(other_metadata, probe.clone())
            .expect("canonical other-event capability");

        assert!(matches!(
            target_runtime.commit_prepared_projection_wire_v1(target_candidate, other_wire),
            Err(RuntimeError::PrivateProjectionWireBindingMismatch { .. })
        ));
        assert_eq!(target_runtime.current_revision(), 0);
        assert_eq!(target_runtime.field.potential, potential_before);
        assert_eq!(target_runtime.field.excitation, excitation_before);
        probe.assert_zeroized_observations(3);
    }

    #[test]
    fn invalid_and_stale_semantics_leave_runtime_unchanged_before_projection_compilation() {
        let runtime = AstrRuntime::scaffold();
        let potential_before = runtime.field.potential.clone();
        let excitation_before = runtime.field.excitation.clone();

        let mut invalid = closed_stimulus();
        let CanonicalEvent::UserStimulus(stimulus) = &mut invalid else {
            panic!("fixture is a user stimulus");
        };
        stimulus.evidence.estimator_confidence = Fixed::ZERO;
        assert!(matches!(
            runtime.prepare_user_stimulus_transition_v1(&invalid),
            Err(RuntimeError::InvalidSemanticEstimate)
        ));

        let mut stale = closed_stimulus();
        let CanonicalEvent::UserStimulus(stimulus) = &mut stale else {
            panic!("fixture is a user stimulus");
        };
        stimulus.causal.base_revision = 1;
        assert!(matches!(
            runtime.prepare_user_stimulus_transition_v1(&stale),
            Err(RuntimeError::UserStimulusBaseRevisionMismatch)
        ));
        assert_eq!(runtime.current_revision(), 0);
        assert_eq!(runtime.field.potential, potential_before);
        assert_eq!(runtime.field.excitation, excitation_before);
    }
}

#[cfg(test)]
mod user_stimulus_state_transition_semantic_regressions {
    use super::private_projection_wire::{
        test_only_wire_for_metadata_v1, PrivateProjectionPayloadWireBindingMetadataV1,
    };
    use super::*;
    use ae_contracts::r7::{
        wire, ActionContract, CanonicalEvent, CausalRef, EvidenceVector, ScopeRef,
        SemanticEstimate, UserStimulus,
    };

    // The public typed-input pipeline is covered by the active organism
    // committed_semantic_projection_path tests. This crate-private harness
    // restores the baseline semantic-candidate/sole-commit matrix without
    // recreating a public callback, sealer, producer, or wire constructor.
    fn commit_semantic_transition_for_test(
        runtime: &mut AstrRuntime,
        event: &CanonicalEvent,
    ) -> Result<RuntimeDecision, RuntimeError> {
        let candidate = runtime.prepare_user_stimulus_transition_v1(event)?;
        let metadata = PrivateProjectionPayloadWireBindingMetadataV1::new(
            candidate.next_revision,
            candidate.turn_id,
            candidate.projection_turn_binding,
            [18; 32],
            candidate.state_after,
        )
        .map_err(|_| RuntimeError::PrivateProjectionWireUnavailable)?;
        let wire = test_only_wire_for_metadata_v1(metadata)
            .map_err(|_| RuntimeError::PrivateProjectionWireUnavailable)?;
        runtime.commit_prepared_projection_wire_v1(candidate, wire)
    }

    const NATIVE_FORMULA_DIGEST_HEX_V1: &str =
        "632bfe32268a280aa56189d5a198550502707d79069c9f2fa76f74aa977f957d";

    fn closed_stimulus(positive_delta: i64) -> CanonicalEvent {
        closed_stimulus_with_relation(positive_delta, Some([4; 16]))
    }

    fn closed_stimulus_with_relation(
        positive_delta: i64,
        relation_token: Option<[u8; 16]>,
    ) -> CanonicalEvent {
        CanonicalEvent::UserStimulus(UserStimulus {
            event_id: [1; 16],
            scope: ScopeRef {
                bot_token: [2; 16],
                persona_token: [3; 16],
                relation_token,
                session_token: [5; 16],
            },
            causal: CausalRef {
                turn_id: [6; 16],
                action_id: None,
                delivery_id: None,
                claim_id: None,
                base_revision: 0,
            },
            observed_at_ms: 1,
            evidence: SemanticEstimate {
                schema_version: 1,
                dimensions: EvidenceVector {
                    positive: Fixed::from_raw(110_000 + positive_delta),
                    affiliation: Fixed::from_raw(120_000),
                    harm: Fixed::from_raw(130_000),
                    boundary: Fixed::from_raw(140_000),
                    repair: Fixed::from_raw(150_000),
                    repetition: Fixed::from_raw(160_000),
                    new_information: Fixed::from_raw(170_000),
                    constraint_instability: Fixed::from_raw(180_000),
                    epistemic_conflict: Fixed::from_raw(190_000),
                    self_responsibility: Fixed::from_raw(200_000),
                    other_responsibility: Fixed::from_raw(210_000),
                    hostility: Fixed::from_raw(220_000),
                    publicness: Fixed::from_raw(230_000),
                    engagement: Fixed::from_raw(240_000),
                    rejection: Fixed::from_raw(250_000),
                },
                estimator_confidence: Fixed::from_raw(800_000),
                estimator_digest: [7; 32],
            },
        })
    }

    fn has_digest(digest: [u8; 32]) -> bool {
        digest != [0; 32]
    }

    #[derive(Clone)]
    struct RuntimeSnapshot {
        potential: Vec<Fixed>,
        excitation: Vec<Fixed>,
        inhibition: Vec<Fixed>,
        adaptation: Vec<Fixed>,
        precision: Vec<Fixed>,
        prediction_error: Vec<Fixed>,
        eligibility: Vec<Fixed>,
        metabolic_reserve: Vec<Fixed>,
        row_offsets: Vec<u32>,
        edges: Vec<ae_neurofield::Synapse>,
        revision: u64,
        formula_digest: [u8; 32],
    }

    fn snapshot(runtime: &AstrRuntime) -> RuntimeSnapshot {
        RuntimeSnapshot {
            potential: runtime.field.potential.clone(),
            excitation: runtime.field.excitation.clone(),
            inhibition: runtime.field.inhibition.clone(),
            adaptation: runtime.field.adaptation.clone(),
            precision: runtime.field.precision.clone(),
            prediction_error: runtime.field.prediction_error.clone(),
            eligibility: runtime.field.eligibility.clone(),
            metabolic_reserve: runtime.field.metabolic_reserve.clone(),
            row_offsets: runtime.graph.row_offsets.clone(),
            edges: runtime.graph.edges.clone(),
            revision: runtime.current_revision(),
            formula_digest: runtime.formula_digest,
        }
    }

    fn assert_unchanged(runtime: &AstrRuntime, before: &RuntimeSnapshot) {
        assert_eq!(runtime.field.potential, before.potential);
        assert_eq!(runtime.field.excitation, before.excitation);
        assert_eq!(runtime.field.inhibition, before.inhibition);
        assert_eq!(runtime.field.adaptation, before.adaptation);
        assert_eq!(runtime.field.precision, before.precision);
        assert_eq!(runtime.field.prediction_error, before.prediction_error);
        assert_eq!(runtime.field.eligibility, before.eligibility);
        assert_eq!(runtime.field.metabolic_reserve, before.metabolic_reserve);
        assert_eq!(runtime.graph.row_offsets, before.row_offsets);
        assert_eq!(runtime.graph.edges, before.edges);
        assert_eq!(runtime.current_revision(), before.revision);
        assert_eq!(runtime.formula_digest, before.formula_digest);
    }

    fn fixed_digest(domain: &[u8], values: &[Fixed]) -> [u8; 32] {
        let mut encoded = Vec::with_capacity(values.len() * std::mem::size_of::<i64>());
        for value in values {
            encoded.extend_from_slice(&value.raw().to_be_bytes());
        }
        wire::domain_hash(domain, &[&encoded])
    }

    fn evidence_values(evidence: &EvidenceVector) -> [Fixed; 15] {
        [
            evidence.positive,
            evidence.affiliation,
            evidence.harm,
            evidence.boundary,
            evidence.repair,
            evidence.repetition,
            evidence.new_information,
            evidence.constraint_instability,
            evidence.epistemic_conflict,
            evidence.self_responsibility,
            evidence.other_responsibility,
            evidence.hostility,
            evidence.publicness,
            evidence.engagement,
            evidence.rejection,
        ]
    }

    fn option_id_digest(value: Option<[u8; 16]>) -> [u8; 32] {
        match value {
            None => wire::domain_hash(b"astr-embodiment/native-semantic-option-id-v1", &[b"none"]),
            Some(id) => wire::domain_hash(
                b"astr-embodiment/native-semantic-option-id-v1",
                &[b"some", &id],
            ),
        }
    }

    fn expected_scope_digest(scope: &ScopeRef) -> [u8; 32] {
        let relation = option_id_digest(scope.relation_token);
        wire::domain_hash(
            b"astr-embodiment/native-semantic-scope-v1",
            &[
                &scope.bot_token,
                &scope.persona_token,
                &relation,
                &scope.session_token,
            ],
        )
    }

    fn expected_estimate_digest(estimate: &SemanticEstimate) -> [u8; 32] {
        let dimensions = fixed_digest(
            b"astr-embodiment/native-semantic-evidence-vector-v1",
            &evidence_values(&estimate.dimensions),
        );
        let confidence = estimate.estimator_confidence.raw().to_be_bytes();
        wire::domain_hash(
            b"astr-embodiment/native-semantic-estimate-v1",
            &[&dimensions, &confidence, &estimate.estimator_digest],
        )
    }

    fn expected_event_digest(stimulus: &UserStimulus, scope: [u8; 32]) -> [u8; 32] {
        let action = option_id_digest(stimulus.causal.action_id);
        let delivery = option_id_digest(stimulus.causal.delivery_id);
        let claim = option_id_digest(stimulus.causal.claim_id);
        let observed_at_ms = stimulus.observed_at_ms.to_be_bytes();
        let base_revision = stimulus.causal.base_revision.to_be_bytes();
        let schema_version = stimulus.evidence.schema_version.to_be_bytes();
        let estimate = expected_estimate_digest(&stimulus.evidence);
        wire::domain_hash(
            b"astr-embodiment/native-semantic-user-stimulus-v1",
            &[
                &stimulus.event_id,
                &scope,
                &stimulus.causal.turn_id,
                &action,
                &delivery,
                &claim,
                &observed_at_ms,
                &base_revision,
                &schema_version,
                &estimate,
            ],
        )
    }

    fn expected_field_digest(runtime: &AstrRuntime) -> [u8; 32] {
        let potential = fixed_digest(
            b"astr-embodiment/neural-field/potential-v1",
            &runtime.field.potential,
        );
        let excitation = fixed_digest(
            b"astr-embodiment/neural-field/excitation-v1",
            &runtime.field.excitation,
        );
        let inhibition = fixed_digest(
            b"astr-embodiment/neural-field/inhibition-v1",
            &runtime.field.inhibition,
        );
        let adaptation = fixed_digest(
            b"astr-embodiment/neural-field/adaptation-v1",
            &runtime.field.adaptation,
        );
        let precision = fixed_digest(
            b"astr-embodiment/neural-field/precision-v1",
            &runtime.field.precision,
        );
        let prediction_error = fixed_digest(
            b"astr-embodiment/neural-field/prediction-error-v1",
            &runtime.field.prediction_error,
        );
        let eligibility = fixed_digest(
            b"astr-embodiment/neural-field/eligibility-v1",
            &runtime.field.eligibility,
        );
        let metabolic_reserve = fixed_digest(
            b"astr-embodiment/neural-field/metabolic-reserve-v1",
            &runtime.field.metabolic_reserve,
        );
        wire::domain_hash(
            b"astr-embodiment/native-semantic-neural-field-v1",
            &[
                &potential,
                &excitation,
                &inhibition,
                &adaptation,
                &precision,
                &prediction_error,
                &eligibility,
                &metabolic_reserve,
            ],
        )
    }

    fn expected_graph_digest(runtime: &AstrRuntime) -> [u8; 32] {
        let mut row_offsets = Vec::with_capacity(runtime.graph.row_offsets.len() * 4);
        for offset in &runtime.graph.row_offsets {
            row_offsets.extend_from_slice(&offset.to_be_bytes());
        }
        let mut edges = Vec::with_capacity(runtime.graph.edges.len() * 16);
        for edge in &runtime.graph.edges {
            edges.extend_from_slice(&edge.target.to_be_bytes());
            edges.extend_from_slice(&edge.weight.to_be_bytes());
            edges.extend_from_slice(&edge.eligibility.to_be_bytes());
            edges.extend_from_slice(&edge.stability.to_be_bytes());
            edges.extend_from_slice(&edge.last_used_epoch.to_be_bytes());
            edges.push(edge.operator_id);
            edges.push(edge.delay_class);
            edges.extend_from_slice(&edge.flags.to_be_bytes());
        }
        wire::domain_hash(
            b"astr-embodiment/native-semantic-sparse-graph-v1",
            &[&row_offsets, &edges],
        )
    }

    fn expected_contract_digest(contract: &ActionContract) -> [u8; 32] {
        let continuous = fixed_digest(
            b"astr-embodiment/native-semantic-action-vector-v1",
            &[
                contract.continuous.answer,
                contract.continuous.verify,
                contract.continuous.acknowledge_error,
                contract.continuous.repair,
                contract.continuous.ask_evidence,
                contract.continuous.set_boundary,
                contract.continuous.withdraw,
                contract.continuous.proactive_reach,
                contract.continuous.warmth,
                contract.continuous.directness,
                contract.continuous.verbosity,
                contract.continuous.confidence_ceiling,
            ],
        );
        let flags = [
            u8::from(contract.must_verify),
            u8::from(contract.must_acknowledge_error),
            u8::from(contract.must_correct_claim),
            u8::from(contract.may_set_boundary),
            u8::from(contract.may_withdraw),
            u8::from(contract.must_not_seek_reassurance),
        ];
        let expires_at_ms = contract.expires_at_ms.to_be_bytes();
        wire::domain_hash(
            b"astr-embodiment/native-semantic-action-contract-v1",
            &[
                &contract.action_id,
                &contract.turn_id,
                &continuous,
                &flags,
                &expires_at_ms,
            ],
        )
    }

    fn digest_hex(digest: [u8; 32]) -> String {
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn closed_nonzero_user_stimulus_commits_a_bound_native_transition() {
        let event = closed_stimulus(0);
        let mut runtime = AstrRuntime::scaffold();
        let field_before = runtime.field.potential.clone();

        let decision = commit_semantic_transition_for_test(&mut runtime, &event)
            .expect("closed typed user stimulus is accepted without a raw-text boundary");

        assert!(runtime
            .field
            .potential
            .iter()
            .zip(field_before.iter())
            .any(|(after, before)| after != before));
        assert!(has_digest(decision.receipt.state_before));
        assert!(has_digest(decision.receipt.state_after));
        assert_ne!(decision.receipt.state_before, decision.receipt.state_after);
        assert!(has_digest(decision.receipt.graph_after));
        assert!(has_digest(decision.receipt.event_digest));
        assert!(has_digest(decision.receipt.authority_digest));
        assert!(has_digest(decision.receipt.action_contract.expect(
            "committed transition carries an action-contract digest"
        )));
        assert_ne!(decision.contract.action_id, [0; 16]);
        assert!(decision.receipt.active_nodes > 0);
        assert_eq!(runtime.current_revision(), 1);
    }

    #[test]
    fn closed_evidence_is_deterministic_and_distinguishes_native_state() {
        let event = closed_stimulus(0);
        let contrasting_event = closed_stimulus(100_000);

        let mut first = AstrRuntime::scaffold();
        let first_decision =
            commit_semantic_transition_for_test(&mut first, &event).expect("first transition");
        let mut repeated = AstrRuntime::scaffold();
        let repeated_decision =
            commit_semantic_transition_for_test(&mut repeated, &event).expect("repeat transition");
        let mut contrasting = AstrRuntime::scaffold();
        let contrasting_decision =
            commit_semantic_transition_for_test(&mut contrasting, &contrasting_event)
                .expect("contrasting transition");

        assert_eq!(first.field.potential, repeated.field.potential);
        assert_eq!(
            first_decision.receipt.state_after,
            repeated_decision.receipt.state_after
        );
        assert_eq!(
            first_decision.receipt.action_contract,
            repeated_decision.receipt.action_contract
        );
        assert_ne!(first.field.potential[0], contrasting.field.potential[0]);
        assert_ne!(
            first_decision.receipt.state_after,
            contrasting_decision.receipt.state_after
        );
        assert_ne!(
            first_decision.receipt.action_contract,
            contrasting_decision.receipt.action_contract
        );
    }

    #[test]
    fn zero_confidence_fails_closed_without_mutating_runtime_state() {
        let mut event = closed_stimulus(0);
        let CanonicalEvent::UserStimulus(stimulus) = &mut event else {
            unreachable!("fixture is a user stimulus");
        };
        stimulus.evidence.estimator_confidence = Fixed::ZERO;

        let mut runtime = AstrRuntime::scaffold();
        let before = snapshot(&runtime);
        assert!(commit_semantic_transition_for_test(&mut runtime, &event).is_err());
        assert_unchanged(&runtime, &before);
    }

    #[test]
    fn zero_estimator_digest_fails_closed_without_mutating_runtime_state() {
        let mut event = closed_stimulus(0);
        let CanonicalEvent::UserStimulus(stimulus) = &mut event else {
            unreachable!("fixture is a user stimulus");
        };
        stimulus.evidence.estimator_digest = [0; 32];

        let mut runtime = AstrRuntime::scaffold();
        let before = snapshot(&runtime);
        assert!(commit_semantic_transition_for_test(&mut runtime, &event).is_err());
        assert_unchanged(&runtime, &before);
    }

    #[test]
    fn sparse_evidence_is_accepted_when_current_attention_has_effective_drive() {
        let mut event = closed_stimulus(0);
        let CanonicalEvent::UserStimulus(stimulus) = &mut event else {
            unreachable!("fixture is a user stimulus");
        };
        stimulus.evidence.dimensions = EvidenceVector {
            positive: Fixed::from_raw(500_000),
            ..EvidenceVector::default()
        };

        let mut runtime = AstrRuntime::scaffold();
        let before = snapshot(&runtime);
        let decision = commit_semantic_transition_for_test(&mut runtime, &event)
            .expect("sparse typed evidence with active attention is valid");
        assert_ne!(decision.receipt.state_before, decision.receipt.state_after);
        assert_ne!(runtime.field.potential, before.potential);
    }

    #[test]
    fn fully_zero_evidence_fails_closed_without_mutating_runtime_state() {
        let mut event = closed_stimulus(0);
        let CanonicalEvent::UserStimulus(stimulus) = &mut event else {
            unreachable!("fixture is a user stimulus");
        };
        stimulus.evidence.dimensions = EvidenceVector::default();

        let mut runtime = AstrRuntime::scaffold();
        let before = snapshot(&runtime);
        assert!(commit_semantic_transition_for_test(&mut runtime, &event).is_err());
        assert_unchanged(&runtime, &before);
    }

    #[test]
    fn slice_a_aggregate_attention_collides_for_equal_sum_different_composition() {
        let mut first_event = closed_stimulus(0);
        let CanonicalEvent::UserStimulus(first_stimulus) = &mut first_event else {
            unreachable!("fixture is a user stimulus");
        };
        first_stimulus.evidence.dimensions = EvidenceVector {
            positive: Fixed::from_raw(300_000),
            harm: Fixed::from_raw(100_000),
            ..EvidenceVector::default()
        };

        let mut second_event = closed_stimulus(0);
        let CanonicalEvent::UserStimulus(second_stimulus) = &mut second_event else {
            unreachable!("fixture is a user stimulus");
        };
        second_stimulus.evidence.dimensions = EvidenceVector {
            positive: Fixed::from_raw(100_000),
            harm: Fixed::from_raw(300_000),
            ..EvidenceVector::default()
        };

        let mut first_runtime = AstrRuntime::scaffold();
        let first = commit_semantic_transition_for_test(&mut first_runtime, &first_event)
            .expect("first sparse aggregate activation");
        let mut second_runtime = AstrRuntime::scaffold();
        let second = commit_semantic_transition_for_test(&mut second_runtime, &second_event)
            .expect("second sparse aggregate activation");

        assert_ne!(first.receipt.event_digest, second.receipt.event_digest);
        assert_eq!(first.receipt.state_after, second.receipt.state_after);
        assert_eq!(
            first_runtime.field.potential,
            second_runtime.field.potential
        );
    }

    #[test]
    fn canonical_receipt_digests_bind_every_returned_typed_field() {
        let event = closed_stimulus(0);
        let CanonicalEvent::UserStimulus(stimulus) = &event else {
            unreachable!("fixture is a user stimulus");
        };
        let mut runtime = AstrRuntime::scaffold();
        let expected_before = expected_field_digest(&runtime);
        let decision =
            commit_semantic_transition_for_test(&mut runtime, &event).expect("closed transition");
        let expected_scope = expected_scope_digest(&stimulus.scope);
        let expected_event = expected_event_digest(stimulus, expected_scope);
        let expected_authority = wire::domain_hash(
            b"astr-embodiment/native-semantic-authority-v1",
            &[
                b"user_observed",
                &stimulus.evidence.estimator_digest,
                &expected_event,
            ],
        );

        assert_eq!(
            digest_hex(runtime.formula_digest),
            NATIVE_FORMULA_DIGEST_HEX_V1
        );
        assert_eq!(decision.receipt.scope_digest, expected_scope);
        assert_eq!(decision.receipt.event_digest, expected_event);
        assert_eq!(decision.receipt.authority_digest, expected_authority);
        assert_eq!(decision.receipt.state_before, expected_before);
        assert_eq!(
            decision.receipt.state_after,
            expected_field_digest(&runtime)
        );
        assert_eq!(
            decision.receipt.graph_after,
            expected_graph_digest(&runtime)
        );
        assert_eq!(
            decision.receipt.action_contract,
            Some(expected_contract_digest(&decision.contract))
        );
    }

    #[test]
    fn optional_bindings_remain_canonical_and_legal_relation_absence_is_supported() {
        let mut zero_optional = closed_stimulus(0);
        let CanonicalEvent::UserStimulus(stimulus) = &mut zero_optional else {
            unreachable!("fixture is a user stimulus");
        };
        stimulus.causal.action_id = Some([0; 16]);
        let mut runtime = AstrRuntime::scaffold();
        let before = snapshot(&runtime);
        assert!(commit_semantic_transition_for_test(&mut runtime, &zero_optional).is_err());
        assert_unchanged(&runtime, &before);

        let without_relation = closed_stimulus_with_relation(0, None);
        let CanonicalEvent::UserStimulus(stimulus) = &without_relation else {
            unreachable!("fixture is a user stimulus");
        };
        let mut relationless_runtime = AstrRuntime::scaffold();
        let decision =
            commit_semantic_transition_for_test(&mut relationless_runtime, &without_relation)
                .expect("relation is an optional typed binding");
        assert_eq!(
            decision.receipt.scope_digest,
            expected_scope_digest(&stimulus.scope)
        );
    }

    #[test]
    fn zero_relation_binding_and_mutated_formula_fail_closed() {
        let mut zero_relation = closed_stimulus(0);
        let CanonicalEvent::UserStimulus(stimulus) = &mut zero_relation else {
            unreachable!("fixture is a user stimulus");
        };
        stimulus.scope.relation_token = Some([0; 16]);
        let mut relation_runtime = AstrRuntime::scaffold();
        let before_relation = snapshot(&relation_runtime);
        assert!(
            commit_semantic_transition_for_test(&mut relation_runtime, &zero_relation).is_err()
        );
        assert_unchanged(&relation_runtime, &before_relation);

        let event = closed_stimulus(0);
        let mut formula_runtime = AstrRuntime::scaffold();
        formula_runtime.formula_digest = [0; 32];
        let before_formula = snapshot(&formula_runtime);
        assert!(commit_semantic_transition_for_test(&mut formula_runtime, &event).is_err());
        assert_unchanged(&formula_runtime, &before_formula);
    }

    #[test]
    fn stale_replay_overflow_and_late_failure_leave_full_runtime_unchanged() {
        let mut stale = closed_stimulus(0);
        let CanonicalEvent::UserStimulus(stimulus) = &mut stale else {
            unreachable!("fixture is a user stimulus");
        };
        stimulus.causal.base_revision = 1;
        let mut runtime = AstrRuntime::scaffold();
        let before_stale = snapshot(&runtime);
        assert!(commit_semantic_transition_for_test(&mut runtime, &stale).is_err());
        assert_unchanged(&runtime, &before_stale);

        let replay = closed_stimulus(0);
        commit_semantic_transition_for_test(&mut runtime, &replay).expect("first transition");
        let before_replay = snapshot(&runtime);
        assert!(commit_semantic_transition_for_test(&mut runtime, &replay).is_err());
        assert_unchanged(&runtime, &before_replay);

        let mut overflow = closed_stimulus(0);
        let CanonicalEvent::UserStimulus(stimulus) = &mut overflow else {
            unreachable!("fixture is a user stimulus");
        };
        stimulus.causal.base_revision = u64::MAX;
        let mut overflow_runtime = AstrRuntime::scaffold();
        overflow_runtime.revision = u64::MAX;
        let before_overflow = snapshot(&overflow_runtime);
        assert!(commit_semantic_transition_for_test(&mut overflow_runtime, &overflow).is_err());
        assert_unchanged(&overflow_runtime, &before_overflow);

        let mut late_failure = closed_stimulus(0);
        let CanonicalEvent::UserStimulus(stimulus) = &mut late_failure else {
            unreachable!("fixture is a user stimulus");
        };
        stimulus.observed_at_ms = u64::MAX;
        let mut late_runtime = AstrRuntime::scaffold();
        let before_late = snapshot(&late_runtime);
        assert!(commit_semantic_transition_for_test(&mut late_runtime, &late_failure).is_err());
        assert_unchanged(&late_runtime, &before_late);
    }
}
