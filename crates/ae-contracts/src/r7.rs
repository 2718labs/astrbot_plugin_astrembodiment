#![forbid(unsafe_code)]

use ae_fixed::Fixed;
use serde::{Deserialize, Serialize};
use std::fmt;

pub type Digest = [u8; 32];
pub type Id128 = [u8; 16];

pub mod wire {
    use crate::Digest;

    pub fn domain_hash(domain: &[u8], fields: &[&[u8]]) -> Digest {
        let mut hasher = blake3::Hasher::new();
        hasher.update(domain);
        hasher.update(&[0]);
        for field in fields {
            hasher.update(&(field.len() as u64).to_le_bytes());
            hasher.update(field);
        }
        *hasher.finalize().as_bytes()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceAuthority {
    UserObserved,
    ExplicitFeedback,
    PlatformObserved,
    VerifierResult,
    SelfAction,
    SelfCritique,
    TimeAdvance,
    AdminAction,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeRef {
    pub bot_token: Id128,
    pub persona_token: Id128,
    pub relation_token: Option<Id128>,
    pub session_token: Id128,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CausalRef {
    pub turn_id: Id128,
    pub action_id: Option<Id128>,
    pub delivery_id: Option<Id128>,
    pub claim_id: Option<Id128>,
    pub base_revision: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceVector {
    pub positive: Fixed,
    pub affiliation: Fixed,
    pub harm: Fixed,
    pub boundary: Fixed,
    pub repair: Fixed,
    pub repetition: Fixed,
    pub new_information: Fixed,
    pub constraint_instability: Fixed,
    pub epistemic_conflict: Fixed,
    pub self_responsibility: Fixed,
    pub other_responsibility: Fixed,
    pub hostility: Fixed,
    pub publicness: Fixed,
    pub engagement: Fixed,
    pub rejection: Fixed,
}

/// Closed request-local evidence proposal for the SPC1 semantic perception
/// preview.  This type deliberately contains no authority, action, policy,
/// wire, callback, provider, or text material.  The estimator commitment is
/// derived by Rust from this exact field order and the bound request scope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerceptionProposalV1 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d16")]
    pub event_id: Id128,
    #[serde(with = "crate::hex::d16")]
    pub turn_id: Id128,
    pub observed_at_ms: u64,
    pub base_revision: u64,
    pub dimensions: EvidenceVector,
    pub estimator_confidence: Fixed,
    pub protocol_version: u16,
    #[serde(with = "crate::hex::d32")]
    pub request_nonce_digest: Digest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PerceptionProposalErrorV1 {
    InvalidSchemaVersion,
    InvalidProtocolVersion,
    InvalidIdentity,
    InvalidObservedAt,
    InvalidDimensions,
    InvalidConfidence,
    ZeroRequestNonce,
}

impl fmt::Display for PerceptionProposalErrorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidSchemaVersion => "invalid perception proposal schema",
            Self::InvalidProtocolVersion => "invalid perception proposal protocol",
            Self::InvalidIdentity => "invalid perception proposal identity",
            Self::InvalidObservedAt => "invalid perception proposal observed time",
            Self::InvalidDimensions => "invalid perception proposal dimensions",
            Self::InvalidConfidence => "invalid perception proposal confidence",
            Self::ZeroRequestNonce => "invalid perception proposal nonce",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for PerceptionProposalErrorV1 {}

impl PerceptionProposalV1 {
    pub const SCHEMA_VERSION: u16 = 1;
    pub const PROTOCOL_VERSION: u16 = 1;
    pub const DIGEST_DOMAIN_V1: &'static [u8] = b"astr-embodiment/semantic-perception-proposal-v1";

    pub fn validate_v1(&self) -> Result<(), PerceptionProposalErrorV1> {
        if self.schema_version != Self::SCHEMA_VERSION {
            return Err(PerceptionProposalErrorV1::InvalidSchemaVersion);
        }
        if self.protocol_version != Self::PROTOCOL_VERSION {
            return Err(PerceptionProposalErrorV1::InvalidProtocolVersion);
        }
        if proposal_all_zero_id(&self.event_id) || proposal_all_zero_id(&self.turn_id) {
            return Err(PerceptionProposalErrorV1::InvalidIdentity);
        }
        if self.observed_at_ms == 0 {
            return Err(PerceptionProposalErrorV1::InvalidObservedAt);
        }
        let values = [
            self.dimensions.positive,
            self.dimensions.affiliation,
            self.dimensions.harm,
            self.dimensions.boundary,
            self.dimensions.repair,
            self.dimensions.repetition,
            self.dimensions.new_information,
            self.dimensions.constraint_instability,
            self.dimensions.epistemic_conflict,
            self.dimensions.self_responsibility,
            self.dimensions.other_responsibility,
            self.dimensions.hostility,
            self.dimensions.publicness,
            self.dimensions.engagement,
            self.dimensions.rejection,
        ];
        if values
            .iter()
            .any(|value| *value < Fixed::ZERO || *value > Fixed::ONE)
            || values.iter().all(|value| *value == Fixed::ZERO)
        {
            return Err(PerceptionProposalErrorV1::InvalidDimensions);
        }
        if self.estimator_confidence <= Fixed::ZERO || self.estimator_confidence > Fixed::ONE {
            return Err(PerceptionProposalErrorV1::InvalidConfidence);
        }
        if proposal_all_zero_digest(&self.request_nonce_digest) {
            return Err(PerceptionProposalErrorV1::ZeroRequestNonce);
        }
        Ok(())
    }

    /// Derive the estimator commitment using the canonical fixed field order.
    /// The caller supplies only the already-bound scope; no caller-supplied
    /// digest or authority can influence this result.
    pub fn estimator_digest_v1(&self, scope: &ScopeRef) -> Digest {
        let schema_version = self.schema_version.to_le_bytes();
        let values = [
            self.dimensions.positive.encode(),
            self.dimensions.affiliation.encode(),
            self.dimensions.harm.encode(),
            self.dimensions.boundary.encode(),
            self.dimensions.repair.encode(),
            self.dimensions.repetition.encode(),
            self.dimensions.new_information.encode(),
            self.dimensions.constraint_instability.encode(),
            self.dimensions.epistemic_conflict.encode(),
            self.dimensions.self_responsibility.encode(),
            self.dimensions.other_responsibility.encode(),
            self.dimensions.hostility.encode(),
            self.dimensions.publicness.encode(),
            self.dimensions.engagement.encode(),
            self.dimensions.rejection.encode(),
        ];
        let confidence = self.estimator_confidence.encode();
        let protocol_version = self.protocol_version.to_le_bytes();
        let scope_digest = scope_digest(scope);
        let base_revision = self.base_revision.to_le_bytes();
        let mut fields: Vec<&[u8]> = Vec::with_capacity(22);
        fields.push(&schema_version);
        fields.extend(values.iter().map(|value| value.as_slice()));
        fields.push(&confidence);
        fields.push(&protocol_version);
        fields.push(&self.request_nonce_digest);
        fields.push(&self.event_id);
        fields.push(&scope_digest);
        fields.push(&self.turn_id);
        fields.push(&base_revision);
        wire::domain_hash(Self::DIGEST_DOMAIN_V1, &fields)
    }
}

fn proposal_all_zero_id(value: &Id128) -> bool {
    value.iter().all(|byte| *byte == 0)
}

fn proposal_all_zero_digest(value: &Digest) -> bool {
    value.iter().all(|byte| *byte == 0)
}

fn scope_digest(scope: &ScopeRef) -> Digest {
    let mut body = Vec::with_capacity(16 * 4 + 1);
    body.extend_from_slice(&scope.bot_token);
    body.extend_from_slice(&scope.persona_token);
    match scope.relation_token {
        Some(relation) => {
            body.push(1);
            body.extend_from_slice(&relation);
        }
        None => body.push(0),
    }
    body.extend_from_slice(&scope.session_token);
    wire::domain_hash(b"astr-embodiment/semantic-perception-scope-v1", &[&body])
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticEstimate {
    pub schema_version: u16,
    pub dimensions: EvidenceVector,
    pub estimator_confidence: Fixed,
    pub estimator_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserStimulus {
    pub event_id: Id128,
    pub scope: ScopeRef,
    pub causal: CausalRef,
    pub observed_at_ms: u64,
    pub evidence: SemanticEstimate,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserReaction {
    pub event_id: Id128,
    pub scope: ScopeRef,
    pub causal: CausalRef,
    pub observed_at_ms: u64,
    pub evidence: SemanticEstimate,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrectionClaim {
    pub event_id: Id128,
    pub scope: ScopeRef,
    pub causal: CausalRef,
    pub specificity: Fixed,
    pub supplied_evidence: Fixed,
    pub hostility: Fixed,
    pub publicness: Fixed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictKind {
    ConfirmedSelfError,
    RejectedChallenge,
    SharedAmbiguity,
    HostFailure,
    Unresolved,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrectionVerdictEvent {
    pub event_id: Id128,
    pub scope: ScopeRef,
    pub causal: CausalRef,
    pub verdict: VerdictKind,
    pub confidence: Fixed,
    pub contradiction: Fixed,
    pub hostility: Fixed,
    pub evidence_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelfActionCandidate {
    pub event_id: Id128,
    pub scope: ScopeRef,
    pub causal: CausalRef,
    pub visible_action_digest: Digest,
    pub claims: Vec<ClaimCommitment>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryOutcome {
    pub event_id: Id128,
    pub scope: ScopeRef,
    pub causal: CausalRef,
    pub delivered: bool,
    pub visible_action_digest: Digest,
    pub delivered_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeAdvance {
    pub event_id: Id128,
    pub scope: ScopeRef,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminAction {
    pub event_id: Id128,
    pub scope: ScopeRef,
    pub operation: String,
    pub nonce_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum CanonicalEvent {
    UserStimulus(UserStimulus),
    UserReaction(UserReaction),
    CorrectionClaim(CorrectionClaim),
    CorrectionVerdict(CorrectionVerdictEvent),
    SelfActionCandidate(SelfActionCandidate),
    DeliveryOutcome(DeliveryOutcome),
    TimeAdvance(TimeAdvance),
    AdminAction(AdminAction),
}

impl CanonicalEvent {
    pub fn authority(&self) -> SourceAuthority {
        match self {
            Self::UserStimulus(_) | Self::CorrectionClaim(_) => SourceAuthority::UserObserved,
            Self::UserReaction(_) => SourceAuthority::ExplicitFeedback,
            Self::CorrectionVerdict(_) => SourceAuthority::VerifierResult,
            Self::SelfActionCandidate(_) => SourceAuthority::SelfAction,
            Self::DeliveryOutcome(_) => SourceAuthority::PlatformObserved,
            Self::TimeAdvance(_) => SourceAuthority::TimeAdvance,
            Self::AdminAction(_) => SourceAuthority::AdminAction,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimCommitment {
    pub claim_id: Id128,
    pub confidence: Fixed,
    pub assertiveness: Fixed,
    pub stakes: Fixed,
    pub audience_publicness: Fixed,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionVector {
    pub answer: Fixed,
    pub verify: Fixed,
    pub acknowledge_error: Fixed,
    pub repair: Fixed,
    pub ask_evidence: Fixed,
    pub set_boundary: Fixed,
    pub withdraw: Fixed,
    pub proactive_reach: Fixed,
    pub warmth: Fixed,
    pub directness: Fixed,
    pub verbosity: Fixed,
    pub confidence_ceiling: Fixed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionContract {
    pub action_id: Id128,
    pub turn_id: Id128,
    pub continuous: ActionVector,
    pub must_verify: bool,
    pub must_acknowledge_error: bool,
    pub must_correct_claim: bool,
    pub may_set_boundary: bool,
    pub may_withdraw: bool,
    pub must_not_seek_reassurance: bool,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvariantResiduals {
    pub authority: Fixed,
    pub continuity: Fixed,
    pub energy: Fixed,
    pub renormalization: Fixed,
    pub capacity: Fixed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitStatus {
    Committed,
    Rejected,
    Superseded,
    Stale,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionReceipt {
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
}

pub const HOST_SCHEMA_V1: u16 = 1;
pub const LARK_PUBLIC_EFFECT_V1: &str = "LARK_PUBLIC_EFFECT_V1";
pub const PUBLIC_TEXT_V1: &str = "PUBLIC_TEXT_V1";
pub const ASTRBOT_TOOL_SCHEMA_V1: u16 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostIngressKindV1 {
    CurrentEvent,
    EffectSettlement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostEffectDispositionV1 {
    Silence,
    PublicEffect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryKnowledgeV1 {
    NotDispatched,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostSettlementStatusV1 {
    Silenced,
    RejectedSchema,
    RejectedIngressKind,
    RejectedPlatform,
    RejectedAdapterIdentity,
    RejectedScope,
    RejectedSession,
    RejectedTurn,
    RejectedAction,
    RejectedProcessEpoch,
    RejectedCapability,
    RejectedAuthority,
    RejectedPolicy,
    RejectedExpired,
    RejectedPayloadClass,
    RejectedPayloadShape,
    IdempotencyConflict,
    DuplicateSuppressed,
    FailedBeforeDispatch,
    DispatchReturnedNoTypedReceipt,
    DeliveryUnknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicTextV1 {
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostSettlementV1 {
    pub schema_version: u16,
    pub settlement_id: Digest,
    pub effect_id: Digest,
    pub process_epoch_id: Id128,
    pub adapter_type: String,
    pub adapter_id_binding: Digest,
    pub scope_binding: Digest,
    pub session_binding: Digest,
    pub turn_binding: Digest,
    pub action_id: Digest,
    pub status: HostSettlementStatusV1,
    pub delivery: DeliveryKnowledgeV1,
    pub observed_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostIngressV1 {
    pub schema_version: u16,
    pub kind: HostIngressKindV1,
    pub ingress_id: Digest,
    pub process_epoch_id: Id128,
    pub adapter_type: String,
    pub adapter_id_binding: Digest,
    pub scope_binding: Digest,
    pub session_binding: Digest,
    pub turn_binding: Digest,
    pub event_id: Digest,
    pub observed_at_ms: u64,
    pub base_revision: u64,
    pub current_event_text: Option<String>,
    pub settlement: Option<HostSettlementV1>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostEffectV1 {
    pub schema_version: u16,
    pub disposition: HostEffectDispositionV1,
    pub effect_id: Digest,
    pub process_epoch_id: Id128,
    pub adapter_type: String,
    pub adapter_id_binding: Digest,
    pub scope_binding: Digest,
    pub session_binding: Digest,
    pub turn_binding: Digest,
    pub action_id: Digest,
    pub capability_id: String,
    pub authority_evidence_digest: Digest,
    pub policy_evidence_digest: Digest,
    pub authority_granted: bool,
    pub policy_granted: bool,
    pub payload_class: String,
    pub public_payload: Option<PublicTextV1>,
    pub expires_at_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AstrBotToolDispositionV1 {
    Silence,
    PublicSignal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AstrBotPublicSignalV1 {
    Observed,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AstrBotToolIngressV1 {
    pub schema_version: u16,
    pub invocation_id: Digest,
    pub process_epoch_id: Digest,
    pub adapter_binding: Digest,
    pub session_binding: Digest,
    pub turn_binding: Digest,
    pub event_binding: Digest,
    pub observed_at_ms: u64,
    pub base_revision: u64,
    pub current_event_text: String,
}

impl fmt::Debug for AstrBotToolIngressV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AstrBotToolIngressV1")
            .field("schema_version", &self.schema_version)
            .field("invocation_id", &self.invocation_id)
            .field("process_epoch_id", &self.process_epoch_id)
            .field("adapter_binding", &self.adapter_binding)
            .field("session_binding", &self.session_binding)
            .field("turn_binding", &self.turn_binding)
            .field("event_binding", &self.event_binding)
            .field("observed_at_ms", &self.observed_at_ms)
            .field("base_revision", &self.base_revision)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AstrBotToolOutcomeV1 {
    pub schema_version: u16,
    pub outcome_id: Digest,
    pub invocation_id: Digest,
    pub process_epoch_id: Digest,
    pub adapter_binding: Digest,
    pub session_binding: Digest,
    pub turn_binding: Digest,
    pub event_binding: Digest,
    pub revision: u64,
    pub disposition: AstrBotToolDispositionV1,
    pub public_signal: Option<AstrBotPublicSignalV1>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AstrBotToolContractErrorV1 {
    InvalidSchema,
    InvalidIngressShape,
    InvalidOutcomeShape,
}

impl fmt::Display for AstrBotToolContractErrorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidSchema => "invalid astrbot tool schema",
            Self::InvalidIngressShape => "invalid astrbot tool ingress",
            Self::InvalidOutcomeShape => "invalid astrbot tool outcome",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for AstrBotToolContractErrorV1 {}

impl AstrBotToolIngressV1 {
    pub fn validate_shape(&self) -> Result<(), AstrBotToolContractErrorV1> {
        if self.schema_version != ASTRBOT_TOOL_SCHEMA_V1 {
            return Err(AstrBotToolContractErrorV1::InvalidSchema);
        }
        if all_zero(&self.invocation_id)
            || all_zero(&self.process_epoch_id)
            || all_zero(&self.adapter_binding)
            || all_zero(&self.session_binding)
            || all_zero(&self.turn_binding)
            || all_zero(&self.event_binding)
            || self.observed_at_ms == 0
            || self.current_event_text.contains('\0')
            || self.current_event_text.chars().count() > 16_384
            || self.current_event_text.len() > 65_536
            || self.invocation_id != self.recompute_invocation_id()
        {
            return Err(AstrBotToolContractErrorV1::InvalidIngressShape);
        }
        Ok(())
    }

    pub fn recompute_invocation_id(&self) -> Digest {
        let text_sha256 = sha256_digest(self.current_event_text.as_bytes());
        let base_revision = self.base_revision.to_be_bytes();
        wire::domain_hash(
            b"astr-embodiment/astrbot-v4273-tool-invocation-v1",
            &[
                &self.process_epoch_id,
                &self.adapter_binding,
                &self.session_binding,
                &self.turn_binding,
                &self.event_binding,
                &text_sha256,
                &base_revision,
            ],
        )
    }
}

impl AstrBotToolOutcomeV1 {
    pub fn for_ingress(
        ingress: &AstrBotToolIngressV1,
        revision: u64,
        disposition: AstrBotToolDispositionV1,
        public_signal: Option<AstrBotPublicSignalV1>,
    ) -> Result<Self, AstrBotToolContractErrorV1> {
        ingress.validate_shape()?;
        let mut outcome = Self {
            schema_version: ASTRBOT_TOOL_SCHEMA_V1,
            outcome_id: [0; 32],
            invocation_id: ingress.invocation_id,
            process_epoch_id: ingress.process_epoch_id,
            adapter_binding: ingress.adapter_binding,
            session_binding: ingress.session_binding,
            turn_binding: ingress.turn_binding,
            event_binding: ingress.event_binding,
            revision,
            disposition,
            public_signal,
        };
        outcome.outcome_id = outcome.recompute_outcome_id();
        outcome.validate_shape()?;
        Ok(outcome)
    }

    pub fn validate_shape(&self) -> Result<(), AstrBotToolContractErrorV1> {
        if self.schema_version != ASTRBOT_TOOL_SCHEMA_V1 {
            return Err(AstrBotToolContractErrorV1::InvalidSchema);
        }
        let signal_shape_is_valid = matches!(
            (self.disposition, self.public_signal),
            (AstrBotToolDispositionV1::Silence, None)
                | (
                    AstrBotToolDispositionV1::PublicSignal,
                    Some(AstrBotPublicSignalV1::Observed)
                )
        );
        if all_zero(&self.outcome_id)
            || all_zero(&self.invocation_id)
            || all_zero(&self.process_epoch_id)
            || all_zero(&self.adapter_binding)
            || all_zero(&self.session_binding)
            || all_zero(&self.turn_binding)
            || all_zero(&self.event_binding)
            || self.revision == 0
            || !signal_shape_is_valid
            || self.outcome_id != self.recompute_outcome_id()
        {
            return Err(AstrBotToolContractErrorV1::InvalidOutcomeShape);
        }
        Ok(())
    }

    pub fn recompute_outcome_id(&self) -> Digest {
        let revision = self.revision.to_be_bytes();
        let disposition = match self.disposition {
            AstrBotToolDispositionV1::Silence => b"silence".as_slice(),
            AstrBotToolDispositionV1::PublicSignal => b"public_signal".as_slice(),
        };
        let signal = match self.public_signal {
            None => b"".as_slice(),
            Some(AstrBotPublicSignalV1::Observed) => b"observed".as_slice(),
        };
        wire::domain_hash(
            b"astr-embodiment/astrbot-v4273-tool-outcome-v1",
            &[
                &self.invocation_id,
                &self.process_epoch_id,
                &self.adapter_binding,
                &self.session_binding,
                &self.turn_binding,
                &self.event_binding,
                &revision,
                disposition,
                signal,
            ],
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostContractErrorV1 {
    InvalidSchema,
    InvalidIngressShape,
    InvalidEffectShape,
    InvalidSettlementShape,
    InvalidHexLength,
    InvalidPublicText,
}

impl fmt::Display for HostContractErrorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidSchema => "invalid host schema",
            Self::InvalidIngressShape => "invalid host ingress shape",
            Self::InvalidEffectShape => "invalid host effect shape",
            Self::InvalidSettlementShape => "invalid host settlement shape",
            Self::InvalidHexLength => "invalid host identifier",
            Self::InvalidPublicText => "invalid public text",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for HostContractErrorV1 {}

impl PublicTextV1 {
    pub fn new(text: String) -> Result<Self, HostContractErrorV1> {
        if text.chars().count() > 4_096 || text.contains('\0') {
            return Err(HostContractErrorV1::InvalidPublicText);
        }
        Ok(Self { text })
    }
}

impl HostEffectV1 {
    pub fn silence_for_ingress(ingress: &HostIngressV1, action_id: Digest) -> Self {
        let mut effect = Self {
            schema_version: HOST_SCHEMA_V1,
            disposition: HostEffectDispositionV1::Silence,
            effect_id: [0; 32],
            process_epoch_id: ingress.process_epoch_id,
            adapter_type: ingress.adapter_type.clone(),
            adapter_id_binding: ingress.adapter_id_binding,
            scope_binding: ingress.scope_binding,
            session_binding: ingress.session_binding,
            turn_binding: ingress.turn_binding,
            action_id,
            capability_id: String::new(),
            authority_evidence_digest: [0; 32],
            policy_evidence_digest: [0; 32],
            authority_granted: false,
            policy_granted: false,
            payload_class: String::new(),
            public_payload: None,
            expires_at_ms: ingress.observed_at_ms,
        };
        effect.effect_id = effect.recompute_effect_id();
        effect
    }

    pub fn public_for_ingress_v1(
        ingress: &HostIngressV1,
        action_id: Digest,
        public_text: String,
        authority_evidence_digest: Digest,
        policy_evidence_digest: Digest,
        expires_at_ms: u64,
    ) -> Result<Self, HostContractErrorV1> {
        ingress.validate_shape()?;
        if ingress.kind != HostIngressKindV1::CurrentEvent
            || all_zero(&action_id)
            || all_zero(&authority_evidence_digest)
            || all_zero(&policy_evidence_digest)
            || expires_at_ms <= ingress.observed_at_ms
        {
            return Err(HostContractErrorV1::InvalidEffectShape);
        }

        let mut effect = Self {
            schema_version: HOST_SCHEMA_V1,
            disposition: HostEffectDispositionV1::PublicEffect,
            effect_id: [0; 32],
            process_epoch_id: ingress.process_epoch_id,
            adapter_type: ingress.adapter_type.clone(),
            adapter_id_binding: ingress.adapter_id_binding,
            scope_binding: ingress.scope_binding,
            session_binding: ingress.session_binding,
            turn_binding: ingress.turn_binding,
            action_id,
            capability_id: LARK_PUBLIC_EFFECT_V1.to_owned(),
            authority_evidence_digest,
            policy_evidence_digest,
            authority_granted: true,
            policy_granted: true,
            payload_class: PUBLIC_TEXT_V1.to_owned(),
            public_payload: Some(PublicTextV1::new(public_text)?),
            expires_at_ms,
        };
        effect.effect_id = effect.recompute_effect_id();
        effect.validate_shape()?;
        Ok(effect)
    }

    pub fn validate_shape(&self) -> Result<(), HostContractErrorV1> {
        if self.schema_version != HOST_SCHEMA_V1 {
            return Err(HostContractErrorV1::InvalidSchema);
        }
        if self.adapter_type != "lark" {
            return Err(HostContractErrorV1::InvalidEffectShape);
        }
        if all_zero_id(&self.process_epoch_id)
            || all_zero(&self.adapter_id_binding)
            || all_zero(&self.scope_binding)
            || all_zero(&self.session_binding)
            || all_zero(&self.turn_binding)
            || all_zero(&self.action_id)
            || self.effect_id != self.recompute_effect_id()
        {
            return Err(HostContractErrorV1::InvalidEffectShape);
        }
        match self.disposition {
            HostEffectDispositionV1::Silence => {
                if self.public_payload.is_some()
                    || !self.capability_id.is_empty()
                    || !self.payload_class.is_empty()
                    || self.authority_granted
                    || self.policy_granted
                {
                    return Err(HostContractErrorV1::InvalidEffectShape);
                }
            }
            HostEffectDispositionV1::PublicEffect => {
                if self.capability_id != LARK_PUBLIC_EFFECT_V1
                    || self.payload_class != PUBLIC_TEXT_V1
                    || !self.authority_granted
                    || !self.policy_granted
                    || all_zero(&self.authority_evidence_digest)
                    || all_zero(&self.policy_evidence_digest)
                    || self.public_payload.is_none()
                {
                    return Err(HostContractErrorV1::InvalidEffectShape);
                }
                let payload = self.public_payload.as_ref().expect("checked is_some");
                PublicTextV1::new(payload.text.clone())?;
            }
        }
        Ok(())
    }

    pub fn recompute_effect_id(&self) -> Digest {
        let expires_at_ms = self.expires_at_ms.to_le_bytes();
        wire::domain_hash(
            b"astr-embodiment/host-effect-v1",
            &[
                &self.process_epoch_id,
                &self.scope_binding,
                &self.adapter_id_binding,
                &self.session_binding,
                &self.turn_binding,
                &self.action_id,
                self.capability_id.as_bytes(),
                self.payload_class.as_bytes(),
                &expires_at_ms,
            ],
        )
    }
}

impl HostSettlementV1 {
    pub fn for_effect(
        effect: &HostEffectV1,
        status: HostSettlementStatusV1,
        delivery: DeliveryKnowledgeV1,
        observed_at_ms: u64,
    ) -> Self {
        let observed = observed_at_ms.to_le_bytes();
        let settlement_id = wire::domain_hash(
            b"astr-embodiment/host-settlement-v1",
            &[
                &effect.effect_id,
                settlement_status_name(status).as_bytes(),
                delivery_knowledge_name(delivery).as_bytes(),
                &observed,
            ],
        );
        Self {
            schema_version: HOST_SCHEMA_V1,
            settlement_id,
            effect_id: effect.effect_id,
            process_epoch_id: effect.process_epoch_id,
            adapter_type: effect.adapter_type.clone(),
            adapter_id_binding: effect.adapter_id_binding,
            scope_binding: effect.scope_binding,
            session_binding: effect.session_binding,
            turn_binding: effect.turn_binding,
            action_id: effect.action_id,
            status,
            delivery,
            observed_at_ms,
        }
    }
}

impl HostIngressV1 {
    pub fn validate_shape(&self) -> Result<(), HostContractErrorV1> {
        if self.schema_version != HOST_SCHEMA_V1 {
            return Err(HostContractErrorV1::InvalidSchema);
        }
        if self.adapter_type != "lark"
            || all_zero(&self.ingress_id)
            || all_zero_id(&self.process_epoch_id)
            || all_zero(&self.adapter_id_binding)
            || all_zero(&self.scope_binding)
            || all_zero(&self.session_binding)
            || all_zero(&self.turn_binding)
            || all_zero(&self.event_id)
        {
            return Err(HostContractErrorV1::InvalidIngressShape);
        }
        match self.kind {
            HostIngressKindV1::CurrentEvent => {
                let text = self
                    .current_event_text
                    .as_ref()
                    .ok_or(HostContractErrorV1::InvalidIngressShape)?;
                if text.chars().count() > 16_384 || self.settlement.is_some() {
                    return Err(HostContractErrorV1::InvalidIngressShape);
                }
            }
            HostIngressKindV1::EffectSettlement => {
                if self.current_event_text.is_some() {
                    return Err(HostContractErrorV1::InvalidIngressShape);
                }
                let settlement = self
                    .settlement
                    .as_ref()
                    .ok_or(HostContractErrorV1::InvalidSettlementShape)?;
                if settlement.schema_version != HOST_SCHEMA_V1
                    || settlement.process_epoch_id != self.process_epoch_id
                    || settlement.adapter_type != self.adapter_type
                    || settlement.adapter_id_binding != self.adapter_id_binding
                    || settlement.scope_binding != self.scope_binding
                    || settlement.session_binding != self.session_binding
                    || settlement.turn_binding != self.turn_binding
                    || settlement.effect_id != self.event_id
                    || all_zero(&settlement.action_id)
                {
                    return Err(HostContractErrorV1::InvalidSettlementShape);
                }
            }
        }
        Ok(())
    }

    pub fn for_settlement(settlement: HostSettlementV1, base_revision: u64) -> Self {
        let revision = base_revision.to_le_bytes();
        let ingress_id = wire::domain_hash(
            b"astr-embodiment/host-settlement-ingress-v1",
            &[&settlement.settlement_id, &revision],
        );
        Self {
            schema_version: HOST_SCHEMA_V1,
            kind: HostIngressKindV1::EffectSettlement,
            ingress_id,
            process_epoch_id: settlement.process_epoch_id,
            adapter_type: settlement.adapter_type.clone(),
            adapter_id_binding: settlement.adapter_id_binding,
            scope_binding: settlement.scope_binding,
            session_binding: settlement.session_binding,
            turn_binding: settlement.turn_binding,
            event_id: settlement.effect_id,
            observed_at_ms: settlement.observed_at_ms,
            base_revision,
            current_event_text: None,
            settlement: Some(settlement),
        }
    }
}

fn sha256_digest(input: &[u8]) -> Digest {
    const INITIAL: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    const ROUND: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let bit_length = (input.len() as u64) * 8;
    let mut padded = Vec::with_capacity(input.len() + 72);
    padded.extend_from_slice(input);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_length.to_be_bytes());

    let mut hash = INITIAL;
    for chunk in padded.as_chunks::<64>().0 {
        let mut words = [0_u32; 64];
        for (index, word) in words[..16].iter_mut().enumerate() {
            let offset = index * 4;
            *word = u32::from_be_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }
        for index in 16..64 {
            let sigma0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let sigma1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(sigma0)
                .wrapping_add(words[index - 7])
                .wrapping_add(sigma1);
        }

        let mut a = hash[0];
        let mut b = hash[1];
        let mut c = hash[2];
        let mut d = hash[3];
        let mut e = hash[4];
        let mut f = hash[5];
        let mut g = hash[6];
        let mut h = hash[7];

        for index in 0..64 {
            let upper_sigma1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ ((!e) & g);
            let temporary1 = h
                .wrapping_add(upper_sigma1)
                .wrapping_add(choice)
                .wrapping_add(ROUND[index])
                .wrapping_add(words[index]);
            let upper_sigma0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temporary2 = upper_sigma0.wrapping_add(majority);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temporary1);
            d = c;
            c = b;
            b = a;
            a = temporary1.wrapping_add(temporary2);
        }

        hash[0] = hash[0].wrapping_add(a);
        hash[1] = hash[1].wrapping_add(b);
        hash[2] = hash[2].wrapping_add(c);
        hash[3] = hash[3].wrapping_add(d);
        hash[4] = hash[4].wrapping_add(e);
        hash[5] = hash[5].wrapping_add(f);
        hash[6] = hash[6].wrapping_add(g);
        hash[7] = hash[7].wrapping_add(h);
    }

    let mut output = [0_u8; 32];
    for (index, word) in hash.iter().enumerate() {
        output[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    output
}

fn all_zero(value: &Digest) -> bool {
    value.iter().all(|byte| *byte == 0)
}

fn all_zero_id(value: &Id128) -> bool {
    value.iter().all(|byte| *byte == 0)
}

fn settlement_status_name(status: HostSettlementStatusV1) -> &'static str {
    match status {
        HostSettlementStatusV1::Silenced => "silenced",
        HostSettlementStatusV1::RejectedSchema => "rejected_schema",
        HostSettlementStatusV1::RejectedIngressKind => "rejected_ingress_kind",
        HostSettlementStatusV1::RejectedPlatform => "rejected_platform",
        HostSettlementStatusV1::RejectedAdapterIdentity => "rejected_adapter_identity",
        HostSettlementStatusV1::RejectedScope => "rejected_scope",
        HostSettlementStatusV1::RejectedSession => "rejected_session",
        HostSettlementStatusV1::RejectedTurn => "rejected_turn",
        HostSettlementStatusV1::RejectedAction => "rejected_action",
        HostSettlementStatusV1::RejectedProcessEpoch => "rejected_process_epoch",
        HostSettlementStatusV1::RejectedCapability => "rejected_capability",
        HostSettlementStatusV1::RejectedAuthority => "rejected_authority",
        HostSettlementStatusV1::RejectedPolicy => "rejected_policy",
        HostSettlementStatusV1::RejectedExpired => "rejected_expired",
        HostSettlementStatusV1::RejectedPayloadClass => "rejected_payload_class",
        HostSettlementStatusV1::RejectedPayloadShape => "rejected_payload_shape",
        HostSettlementStatusV1::IdempotencyConflict => "idempotency_conflict",
        HostSettlementStatusV1::DuplicateSuppressed => "duplicate_suppressed",
        HostSettlementStatusV1::FailedBeforeDispatch => "failed_before_dispatch",
        HostSettlementStatusV1::DispatchReturnedNoTypedReceipt => {
            "dispatch_returned_no_typed_receipt"
        }
        HostSettlementStatusV1::DeliveryUnknown => "delivery_unknown",
    }
}

fn delivery_knowledge_name(delivery: DeliveryKnowledgeV1) -> &'static str {
    match delivery {
        DeliveryKnowledgeV1::NotDispatched => "not_dispatched",
        DeliveryKnowledgeV1::Unknown => "unknown",
    }
}
