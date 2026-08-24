#![forbid(unsafe_code)]

//! Frozen 1.0.0 production contracts for AstrEmbodiment.
//!
//! All wire-exchanged structs are closed schemas: unknown JSON fields are
//! rejected instead of being silently ignored. Identity and persistence use
//! the canonical binary codecs in [wire]; JSON is for the FFI boundary and
//! debugging only and never participates in a digest.

use ae_fixed::Fixed;
use serde::{Deserialize, Serialize};
use std::fmt;

pub type Digest = [u8; 32];
pub type Id128 = [u8; 16];

pub mod hex {
    //! Serde helpers: digests and opaque tokens cross the FFI as lowercase hex
    //! strings. The canonical binary wire forms are unaffected.

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn encode16(bytes: &[u8; 16]) -> String {
        let mut out = String::with_capacity(32);
        for byte in bytes {
            out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap());
            out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap());
        }
        out
    }

    pub fn encode32(bytes: &[u8; 32]) -> String {
        let mut out = String::with_capacity(64);
        for byte in bytes {
            out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap());
            out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap());
        }
        out
    }

    pub fn decode16(text: &str) -> Result<[u8; 16], String> {
        let trimmed = text.trim();
        if trimmed.len() != 32 {
            return Err(format!("expected 32 hex chars, got {}", trimmed.len()));
        }
        let mut out = [0u8; 16];
        for (index, chunk) in trimmed.as_bytes().as_chunks::<2>().0.iter().enumerate() {
            let high = (chunk[0] as char).to_digit(16).ok_or("invalid hex")?;
            let low = (chunk[1] as char).to_digit(16).ok_or("invalid hex")?;
            out[index] = ((high << 4) | low) as u8;
        }
        Ok(out)
    }

    pub fn decode32(text: &str) -> Result<[u8; 32], String> {
        let trimmed = text.trim();
        if trimmed.len() != 64 {
            return Err(format!("expected 64 hex chars, got {}", trimmed.len()));
        }
        let mut out = [0u8; 32];
        for (index, chunk) in trimmed.as_bytes().as_chunks::<2>().0.iter().enumerate() {
            let high = (chunk[0] as char).to_digit(16).ok_or("invalid hex")?;
            let low = (chunk[1] as char).to_digit(16).ok_or("invalid hex")?;
            out[index] = ((high << 4) | low) as u8;
        }
        Ok(out)
    }

    pub mod d16 {
        use super::*;

        pub fn serialize<S: Serializer>(
            value: &[u8; 16],
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            serializer.serialize_str(&encode16(value))
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<[u8; 16], D::Error> {
            let text = String::deserialize(deserializer)?;
            decode16(&text).map_err(serde::de::Error::custom)
        }
    }

    pub mod d32 {
        use super::*;

        pub fn serialize<S: Serializer>(
            value: &[u8; 32],
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            serializer.serialize_str(&encode32(value))
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<[u8; 32], D::Error> {
            let text = String::deserialize(deserializer)?;
            decode32(&text).map_err(serde::de::Error::custom)
        }
    }

    pub mod d16_opt {
        use super::*;

        pub fn serialize<S: Serializer>(
            value: &Option<[u8; 16]>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            match value {
                Some(bytes) => serializer.serialize_some(&encode16(bytes)),
                None => serializer.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<[u8; 16]>, D::Error> {
            Option::<String>::deserialize(deserializer)?
                .map(|text| decode16(&text).map_err(serde::de::Error::custom))
                .transpose()
        }
    }

    pub mod d32_opt {
        use super::*;

        pub fn serialize<S: Serializer>(
            value: &Option<[u8; 32]>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            match value {
                Some(bytes) => serializer.serialize_some(&encode32(bytes)),
                None => serializer.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<[u8; 32]>, D::Error> {
            Option::<String>::deserialize(deserializer)?
                .map(|text| decode32(&text).map_err(serde::de::Error::custom))
                .transpose()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    PersonaConfig,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeRef {
    #[serde(with = "crate::hex::d16")]
    pub bot_token: Id128,
    #[serde(with = "crate::hex::d16")]
    pub persona_token: Id128,
    #[serde(with = "crate::hex::d16_opt")]
    pub relation_token: Option<Id128>,
    #[serde(with = "crate::hex::d16")]
    pub session_token: Id128,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonaSelectionKind {
    SessionForced,
    Conversation,
    ProviderDefault,
    WebchatSpecial,
    ExplicitDefault,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaScopeRef {
    #[serde(with = "crate::hex::d16")]
    pub bot_token: Id128,
    #[serde(with = "crate::hex::d16")]
    pub persona_token: Id128,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaSourceRef {
    pub scope: PersonaScopeRef,
    #[serde(with = "crate::hex::d32")]
    pub source_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub capability_digest: Digest,
    pub selection: PersonaSelectionKind,
    pub prompt_chars: u32,
    pub begin_dialog_count: u16,
    pub mood_dialog_count: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonalityVector {
    pub baseline_warmth: Fixed,
    pub baseline_patience: Fixed,
    pub sensitivity: Fixed,
    pub irritability: Fixed,
    pub composure: Fixed,
    pub epistemic_pride: Fixed,
    pub epistemic_openness: Fixed,
    pub boundary_strength: Fixed,
    pub forgiveness: Fixed,
    pub attachment_propensity: Fixed,
    pub expression_drive: Fixed,
    pub curiosity: Fixed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpressionPhenotype {
    pub warmth: Fixed,
    pub directness: Fixed,
    pub verbosity: Fixed,
    pub self_disclosure: Fixed,
    pub humor: Fixed,
    pub formality: Fixed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllostaticSetpoints {
    pub energy: Fixed,
    pub arousal: Fixed,
    pub contact_need: Fixed,
    pub quiet_need: Fixed,
    pub expression_pressure: Fixed,
    pub exploration_drive: Fixed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpistemicPriors {
    pub verification_drive: Fixed,
    pub confidence_style: Fixed,
    pub correction_defensiveness: Fixed,
    pub repair_after_error: Fixed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SocialPriors {
    pub stranger_distance: Fixed,
    pub approach_threshold: Fixed,
    pub rejection_sensitivity: Fixed,
    pub reciprocity_expectation: Fixed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionGenesis {
    pub gain: Fixed,
    pub inhibitory_tone: Fixed,
    pub time_scale: Fixed,
    pub plasticity_gate: Fixed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenesisManifestProposal {
    pub schema_version: u16,
    pub source: PersonaSourceRef,
    pub traits: PersonalityVector,
    pub trait_confidence: PersonalityVector,
    pub expression: ExpressionPhenotype,
    pub allostasis: AllostaticSetpoints,
    pub epistemic: EpistemicPriors,
    pub social: SocialPriors,
    #[serde(with = "crate::hex::d32")]
    pub compiler_protocol_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub compiler_model_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenesisManifest {
    /// Canonical, immutable phenotype data. Content identity deliberately excludes
    /// Bot/Persona scope, capabilities, compiler model and timestamps: equal modeled
    /// data must have exactly one SeedCode regardless of where or how it was compiled.
    /// The self digest is computed over the canonical body with this field zeroed.
    pub schema_version: u16,
    pub traits: PersonalityVector,
    pub expression: ExpressionPhenotype,
    pub allostasis: AllostaticSetpoints,
    pub epistemic: EpistemicPriors,
    pub social: SocialPriors,
    #[serde(with = "crate::hex::d32")]
    pub manifest_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenesisRecord {
    #[serde(with = "crate::hex::d32")]
    pub seed_code_digest: Digest,
    pub manifest: GenesisManifest,
    pub source: PersonaSourceRef,
    #[serde(with = "crate::hex::d32")]
    pub compiler_protocol_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub compiler_model_digest: Digest,
    pub compiled_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaGenesisRequest {
    pub source: PersonaSourceRef,
    pub proposal: GenesisManifestProposal,
    /// Formula is intentionally excluded from SeedCode identity. It identifies
    /// the laws used to instantiate one concrete brain from the Manifest.
    #[serde(with = "crate::hex::d32")]
    pub formula_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub incarnation_nonce: Digest,
    #[serde(with = "crate::hex::d32_opt")]
    pub parent_incarnation_id: Option<Digest>,
    pub observed_at_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenesisStatus {
    Committed,
    Rejected,
    Superseded,
    RetryWait,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenesisReceipt {
    pub schema_version: u16,
    /// Content identity of the immutable GenesisManifest.
    #[serde(with = "crate::hex::d32")]
    pub seed_code_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub manifest_digest: Digest,
    /// Identity of one concrete birth; may differ for multiple incarnations of
    /// the same Manifest and never replaces SeedCode.
    #[serde(with = "crate::hex::d32")]
    pub incarnation_id: Digest,
    #[serde(with = "crate::hex::d32")]
    pub formula_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub persona_source_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub compiler_protocol_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub compiler_model_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub development_seed_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub initial_snapshot_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub graph_digest: Digest,
    pub equilibrium_residual: Fixed,
    pub energy_residual: Fixed,
    pub capacity_residual: Fixed,
    pub sample_fit_residual: Fixed,
    pub status: GenesisStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenesisCapsule {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d32")]
    pub seed_code_digest: Digest,
    pub manifest: GenesisManifest,
    #[serde(with = "crate::hex::d32")]
    pub provenance_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub capsule_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IncarnationRef {
    #[serde(with = "crate::hex::d32")]
    pub seed_code_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub manifest_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub incarnation_id: Digest,
    #[serde(with = "crate::hex::d32")]
    pub formula_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub active_snapshot_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CausalRef {
    #[serde(with = "crate::hex::d16")]
    pub turn_id: Id128,
    #[serde(with = "crate::hex::d16_opt")]
    pub action_id: Option<Id128>,
    #[serde(with = "crate::hex::d16_opt")]
    pub delivery_id: Option<Id128>,
    #[serde(with = "crate::hex::d16_opt")]
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

/// Closed request-local evidence proposal for the semantic perception preview.
/// It deliberately contains neither provider text nor authority/action policy.
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
        formatter.write_str(match self {
            Self::InvalidSchemaVersion => "invalid perception proposal schema",
            Self::InvalidProtocolVersion => "invalid perception proposal protocol",
            Self::InvalidIdentity => "invalid perception proposal identity",
            Self::InvalidObservedAt => "invalid perception proposal observed time",
            Self::InvalidDimensions => "invalid perception proposal dimensions",
            Self::InvalidConfidence => "invalid perception proposal confidence",
            Self::ZeroRequestNonce => "invalid perception proposal nonce",
        })
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
        if self.event_id.iter().all(|byte| *byte == 0) || self.turn_id.iter().all(|byte| *byte == 0)
        {
            return Err(PerceptionProposalErrorV1::InvalidIdentity);
        }
        if self.observed_at_ms == 0 {
            return Err(PerceptionProposalErrorV1::InvalidObservedAt);
        }
        if perception_dimension_values(&self.dimensions)
            .into_iter()
            .any(|value| !(Fixed::ZERO..=Fixed::ONE).contains(&value))
        {
            return Err(PerceptionProposalErrorV1::InvalidDimensions);
        }
        if !(Fixed::ZERO < self.estimator_confidence && self.estimator_confidence <= Fixed::ONE) {
            return Err(PerceptionProposalErrorV1::InvalidConfidence);
        }
        if self.request_nonce_digest.iter().all(|byte| *byte == 0) {
            return Err(PerceptionProposalErrorV1::ZeroRequestNonce);
        }
        Ok(())
    }

    /// Canonically commits the closed proposal together with the caller's
    /// public request scope. The durable semantic namespace is deliberately
    /// not an input to this commitment.
    pub fn estimator_digest_v1(&self, scope: &ScopeRef) -> Digest {
        let schema_version = self.schema_version.to_le_bytes();
        let values = perception_dimension_values(&self.dimensions).map(Fixed::encode);
        let confidence = self.estimator_confidence.encode();
        let protocol_version = self.protocol_version.to_le_bytes();
        let scope_digest = wire::scope_digest(scope);
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

pub fn perception_dimension_values(evidence: &EvidenceVector) -> [Fixed; 15] {
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticEstimate {
    pub schema_version: u16,
    pub dimensions: EvidenceVector,
    pub estimator_confidence: Fixed,
    #[serde(with = "crate::hex::d32")]
    pub estimator_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserStimulus {
    #[serde(with = "crate::hex::d16")]
    pub event_id: Id128,
    pub scope: ScopeRef,
    pub causal: CausalRef,
    pub observed_at_ms: u64,
    pub evidence: SemanticEstimate,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserReaction {
    #[serde(with = "crate::hex::d16")]
    pub event_id: Id128,
    pub scope: ScopeRef,
    pub causal: CausalRef,
    pub observed_at_ms: u64,
    pub evidence: SemanticEstimate,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorrectionClaim {
    #[serde(with = "crate::hex::d16")]
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
#[serde(deny_unknown_fields)]
pub struct CorrectionVerdictEvent {
    #[serde(with = "crate::hex::d16")]
    pub event_id: Id128,
    pub scope: ScopeRef,
    pub causal: CausalRef,
    pub verdict: VerdictKind,
    pub confidence: Fixed,
    pub contradiction: Fixed,
    pub hostility: Fixed,
    #[serde(with = "crate::hex::d32")]
    pub evidence_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelfActionCandidate {
    #[serde(with = "crate::hex::d16")]
    pub event_id: Id128,
    pub scope: ScopeRef,
    pub causal: CausalRef,
    #[serde(with = "crate::hex::d32")]
    pub visible_action_digest: Digest,
    pub claims: Vec<ClaimCommitment>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolActionDescriptor {
    #[serde(with = "crate::hex::d16")]
    pub action_id: Id128,
    pub tool_class: u16,
    pub side_effect_class: u8,
    #[serde(with = "crate::hex::d32")]
    pub argument_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub authorization_digest: Digest,
    #[serde(with = "crate::hex::d32_opt")]
    pub result_digest: Option<Digest>,
    pub stage: ActionEffectStage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryOutcome {
    #[serde(with = "crate::hex::d16")]
    pub event_id: Id128,
    pub scope: ScopeRef,
    pub causal: CausalRef,
    pub delivered: bool,
    #[serde(with = "crate::hex::d32")]
    pub visible_action_digest: Digest,
    pub delivered_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeAdvance {
    #[serde(with = "crate::hex::d16")]
    pub event_id: Id128,
    pub scope: ScopeRef,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminAction {
    #[serde(with = "crate::hex::d16")]
    pub event_id: Id128,
    pub scope: ScopeRef,
    pub operation: String,
    #[serde(with = "crate::hex::d32")]
    pub nonce_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum CanonicalEvent {
    UserStimulus(UserStimulus),
    UserReaction(UserReaction),
    CorrectionClaim(CorrectionClaim),
    CorrectionVerdict(CorrectionVerdictEvent),
    SelfActionCandidate(SelfActionCandidate),
    DeliveryOutcome(DeliveryOutcome),
    SettlementEvidence(SettlementEvidence),
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
            Self::SettlementEvidence(e) => e.source,
            Self::TimeAdvance(_) => SourceAuthority::TimeAdvance,
            Self::AdminAction(_) => SourceAuthority::AdminAction,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimCommitment {
    #[serde(with = "crate::hex::d16")]
    pub claim_id: Id128,
    pub confidence: Fixed,
    pub assertiveness: Fixed,
    pub stakes: Fixed,
    pub audience_publicness: Fixed,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimDescriptor {
    #[serde(with = "crate::hex::d16")]
    pub claim_id: Id128,
    #[serde(with = "crate::hex::d16")]
    pub action_id: Id128,
    #[serde(with = "crate::hex::d32")]
    pub text_digest: Digest,
    pub confidence: Fixed,
    pub assertiveness: Fixed,
    pub stakes: Fixed,
    pub delivered: bool,
    pub expires_at_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionEffectStage {
    Proposed,
    Authorized,
    Started,
    Executed,
    Decorated,
    Delivered,
    Settled,
    Failed,
    Cancelled,
    UnknownTerminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementKind {
    ExplicitAcceptance,
    ExplicitRejection,
    RepairAcknowledged,
    ConfirmedSelfError,
    RejectedChallenge,
    VerifiedBoundaryViolation,
    ConfirmedFrictionPattern,
    ToolResult,
    DeliveryTerminal,
    StrongContinuation,
    AmbiguousObservation,
}

/// The only event class that may request irreversible Relation learning.
/// Raw user text, semantic estimates and delivery lifecycle events first create
/// candidates; an independently validated, causally bound settlement is required
/// before the authority projection can expose any residual coordinate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettlementEvidence {
    #[serde(with = "crate::hex::d16")]
    pub settlement_id: Id128,
    pub scope: ScopeRef,
    pub causal: CausalRef,
    pub kind: SettlementKind,
    pub source: SourceAuthority,
    pub confidence: Fixed,
    pub evidence_level: u8,
    #[serde(with = "crate::hex::d32")]
    pub evidence_digest: Digest,
    pub observed_at_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct ActionContract {
    #[serde(with = "crate::hex::d16")]
    pub action_id: Id128,
    #[serde(with = "crate::hex::d16")]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct TransitionReceipt {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d32")]
    pub formula_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub scope_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub event_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub authority_digest: Digest,
    pub base_revision: u64,
    pub next_revision: u64,
    #[serde(with = "crate::hex::d32")]
    pub state_before: Digest,
    #[serde(with = "crate::hex::d32")]
    pub state_after: Digest,
    #[serde(with = "crate::hex::d32")]
    pub graph_after: Digest,
    #[serde(with = "crate::hex::d32_opt")]
    pub action_contract: Option<Digest>,
    pub active_nodes: u32,
    pub active_edges: u32,
    pub residuals: InvariantResiduals,
    pub status: CommitStatus,
}

pub const SEMANTIC_VECTOR_RECEIPT_SCHEMA_V2: &str = "astr-embodiment.semantic-vector-receipt.v2";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SemanticVectorFormulaV2 {
    #[serde(rename = "full-vector-route-neutral-relaxation-v1")]
    FullVectorRouteNeutralRelaxationV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticVectorReceiptV2 {
    pub schema_version: u16,
    pub formula: SemanticVectorFormulaV2,
    pub dimension_slot_count: u8,
    pub evaluated_dimension_count: u8,
    pub injected_dimension_count: u8,
    pub nonzero_evidence_dimension_count: u8,
    pub neutral_baseline_dimension_count: u8,
    pub unavailable_dimension_count: u8,
    pub state_changed: bool,
}

impl SemanticVectorReceiptV2 {
    pub const SCHEMA_VERSION: u16 = 2;

    pub fn validate(&self) -> bool {
        self.schema_version == Self::SCHEMA_VERSION
            && self.dimension_slot_count == 15
            && self.evaluated_dimension_count == 15
            && self.injected_dimension_count == 15
            && self.unavailable_dimension_count == 0
            && self
                .nonzero_evidence_dimension_count
                .checked_add(self.neutral_baseline_dimension_count)
                == Some(self.evaluated_dimension_count)
    }
}

/// A separate semantic attestation. Its fixed v1-shaped prefix deliberately
/// leaves the persisted D1 `TransitionReceipt` byte codec unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionReceiptV2 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d32")]
    pub formula_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub scope_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub event_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub authority_digest: Digest,
    pub base_revision: u64,
    pub next_revision: u64,
    #[serde(with = "crate::hex::d32")]
    pub state_before: Digest,
    #[serde(with = "crate::hex::d32")]
    pub state_after: Digest,
    #[serde(with = "crate::hex::d32")]
    pub graph_after: Digest,
    #[serde(with = "crate::hex::d32_opt")]
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

pub mod wire {
    //! Canonical binary wire codec (fixed layout, little-endian, closed
    //! boundaries). Every struct here has exactly one encoding; a digest is
    //! computed over the encoded bytes with a domain-separated BLAKE3 key.
    //! JSON is never an identity codec.

    use super::*;
    use thiserror::Error;

    pub const WIRE_SCHEMA_VERSION: u16 = 1;

    pub const MANIFEST_BODY_DOMAIN: &[u8] = b"ae.genesis.manifest-body.v1";
    pub const EVENT_DOMAIN: &[u8] = b"ae.event.v1";
    pub const ACTION_CONTRACT_DOMAIN: &[u8] = b"ae.action-contract.v1";
    pub const TRANSITION_RECEIPT_DOMAIN: &[u8] = b"ae.transition-receipt.v1";
    pub const TRANSITION_RECEIPT_V2_DOMAIN: &[u8] = b"ae.transition-receipt.v2";
    pub const SCOPE_DOMAIN: &[u8] = b"ae.journal.scope.v1";
    pub const AUTHORITY_DOMAIN: &[u8] = b"ae.authority.v1";
    pub const CAPSULE_DOMAIN: &[u8] = b"ae.genesis-capsule.v1";
    pub const STATE_DOMAIN: &[u8] = b"ae.neural-state.v1";
    pub const GRAPH_DOMAIN: &[u8] = b"ae.graph.v1";
    pub const SNAPSHOT_DOMAIN: &[u8] = b"ae.snapshot.v1";

    pub const MAX_WIRE_STRING: usize = 4096;
    pub const MAX_CLAIMS: usize = 64;

    /// 2 bytes schema version + 32 fixed-point values (8 bytes each).
    pub const MANIFEST_BODY_LEN: usize = 2 + 32 * 8;

    pub const KIND_USER_STIMULUS: u8 = 1;
    pub const KIND_USER_REACTION: u8 = 2;
    pub const KIND_CORRECTION_CLAIM: u8 = 3;
    pub const KIND_CORRECTION_VERDICT: u8 = 4;
    pub const KIND_SELF_ACTION_CANDIDATE: u8 = 5;
    pub const KIND_DELIVERY_OUTCOME: u8 = 6;
    pub const KIND_SETTLEMENT_EVIDENCE: u8 = 7;
    pub const KIND_TIME_ADVANCE: u8 = 8;
    pub const KIND_ADMIN_ACTION: u8 = 9;

    #[derive(Debug, Error, PartialEq, Eq)]
    pub enum WireError {
        #[error("wire byte boundary violation (need {0} bytes, have {1})")]
        Boundary(usize, usize),
        #[error("wire trailing bytes after complete message ({0} bytes)")]
        TrailingBytes(usize),
        #[error("wire schema version {0} is not supported")]
        SchemaVersion(u16),
        #[error("wire unknown event kind {0}")]
        UnknownKind(u8),
        #[error("wire invalid enum code {0}")]
        InvalidEnum(&'static str),
        #[error("wire string is not valid UTF-8")]
        InvalidUtf8,
        #[error("wire string exceeds length limit")]
        StringTooLong,
        #[error("wire claim count exceeds limit")]
        TooManyClaims,
    }

    /// Domain-separated hash: BLAKE3(domain || 0x00 || len(field) || field ...)
    /// with u64 little-endian field lengths.
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

    pub fn encode_fixed(fixed: Fixed) -> [u8; 8] {
        fixed.encode()
    }

    pub fn decode_fixed(bytes: [u8; 8]) -> Fixed {
        Fixed::decode(bytes)
    }

    // ---------------------------------------------------------------- reader

    pub struct Reader<'a> {
        data: &'a [u8],
        pos: usize,
    }

    impl<'a> Reader<'a> {
        pub fn new(data: &'a [u8]) -> Self {
            Self { data, pos: 0 }
        }

        pub fn remaining(&self) -> usize {
            self.data.len() - self.pos
        }

        fn take(&mut self, count: usize) -> Result<&'a [u8], WireError> {
            if self.remaining() < count {
                return Err(WireError::Boundary(count, self.remaining()));
            }
            let slice = &self.data[self.pos..self.pos + count];
            self.pos += count;
            Ok(slice)
        }

        pub fn u8(&mut self) -> Result<u8, WireError> {
            Ok(self.take(1)?[0])
        }

        pub fn bool(&mut self) -> Result<bool, WireError> {
            match self.u8()? {
                0 => Ok(false),
                1 => Ok(true),
                _ => Err(WireError::InvalidEnum("boolean")),
            }
        }

        pub fn u16(&mut self) -> Result<u16, WireError> {
            let bytes: [u8; 2] = self.take(2)?.try_into().unwrap();
            Ok(u16::from_le_bytes(bytes))
        }

        pub fn u32(&mut self) -> Result<u32, WireError> {
            let bytes: [u8; 4] = self.take(4)?.try_into().unwrap();
            Ok(u32::from_le_bytes(bytes))
        }

        pub fn u64(&mut self) -> Result<u64, WireError> {
            let bytes: [u8; 8] = self.take(8)?.try_into().unwrap();
            Ok(u64::from_le_bytes(bytes))
        }

        pub fn fixed(&mut self) -> Result<Fixed, WireError> {
            let bytes: [u8; 8] = self.take(8)?.try_into().unwrap();
            Ok(decode_fixed(bytes))
        }

        pub fn id(&mut self) -> Result<Id128, WireError> {
            let bytes: [u8; 16] = self.take(16)?.try_into().unwrap();
            Ok(bytes)
        }

        pub fn digest(&mut self) -> Result<Digest, WireError> {
            let bytes: [u8; 32] = self.take(32)?.try_into().unwrap();
            Ok(bytes)
        }

        pub fn opt_id(&mut self) -> Result<Option<Id128>, WireError> {
            if self.bool()? {
                Ok(Some(self.id()?))
            } else {
                Ok(None)
            }
        }

        pub fn opt_digest(&mut self) -> Result<Option<Digest>, WireError> {
            if self.bool()? {
                Ok(Some(self.digest()?))
            } else {
                Ok(None)
            }
        }

        pub fn string(&mut self) -> Result<String, WireError> {
            let length = self.u32()? as usize;
            if length > MAX_WIRE_STRING {
                return Err(WireError::StringTooLong);
            }
            let bytes = self.take(length)?;
            String::from_utf8(bytes.to_vec()).map_err(|_| WireError::InvalidUtf8)
        }

        pub fn finish(self) -> Result<(), WireError> {
            if self.pos != self.data.len() {
                return Err(WireError::TrailingBytes(self.data.len() - self.pos));
            }
            Ok(())
        }
    }

    fn push_u16(out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u64(out: &mut Vec<u8>, value: u64) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn push_bool(out: &mut Vec<u8>, value: bool) {
        out.push(u8::from(value));
    }

    fn push_id(out: &mut Vec<u8>, value: &Id128) {
        out.extend_from_slice(value);
    }

    fn push_digest(out: &mut Vec<u8>, value: &Digest) {
        out.extend_from_slice(value);
    }

    fn push_opt_id(out: &mut Vec<u8>, value: &Option<Id128>) {
        push_bool(out, value.is_some());
        if let Some(id) = value {
            push_id(out, id);
        }
    }

    fn push_opt_digest(out: &mut Vec<u8>, value: &Option<Digest>) {
        push_bool(out, value.is_some());
        if let Some(digest) = value {
            push_digest(out, digest);
        }
    }

    fn push_string(out: &mut Vec<u8>, value: &str) {
        debug_assert!(value.len() <= MAX_WIRE_STRING);
        push_u32(out, value.len() as u32);
        out.extend_from_slice(value.as_bytes());
    }

    // ---------------------------------------------------------------- scope

    pub fn encode_scope(scope: &ScopeRef) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 * 3 + 17);
        push_id(&mut out, &scope.bot_token);
        push_id(&mut out, &scope.persona_token);
        push_opt_id(&mut out, &scope.relation_token);
        push_id(&mut out, &scope.session_token);
        out
    }

    fn decode_scope(reader: &mut Reader<'_>) -> Result<ScopeRef, WireError> {
        Ok(ScopeRef {
            bot_token: reader.id()?,
            persona_token: reader.id()?,
            relation_token: reader.opt_id()?,
            session_token: reader.id()?,
        })
    }

    pub fn scope_digest(scope: &ScopeRef) -> Digest {
        domain_hash(SCOPE_DOMAIN, &[&encode_scope(scope)])
    }

    /// Digest of the stable commit lane: (Bot, Persona, Relation?) without
    /// the session token. Revisions and the journal hash chain advance per
    /// persona/relation writer, not per conversation.
    pub fn persona_scope_digest(
        bot_token: &Id128,
        persona_token: &Id128,
        relation_token: Option<&Id128>,
    ) -> Digest {
        let mut body = Vec::with_capacity(49);
        body.extend_from_slice(bot_token);
        body.extend_from_slice(persona_token);
        push_bool(&mut body, relation_token.is_some());
        if let Some(relation) = relation_token {
            body.extend_from_slice(relation);
        }
        domain_hash(SCOPE_DOMAIN, &[&body])
    }

    // ---------------------------------------------------------------- causal

    fn encode_causal(causal: &CausalRef, out: &mut Vec<u8>) {
        push_id(out, &causal.turn_id);
        push_opt_id(out, &causal.action_id);
        push_opt_id(out, &causal.delivery_id);
        push_opt_id(out, &causal.claim_id);
        push_u64(out, causal.base_revision);
    }

    fn decode_causal(reader: &mut Reader<'_>) -> Result<CausalRef, WireError> {
        Ok(CausalRef {
            turn_id: reader.id()?,
            action_id: reader.opt_id()?,
            delivery_id: reader.opt_id()?,
            claim_id: reader.opt_id()?,
            base_revision: reader.u64()?,
        })
    }

    // ---------------------------------------------------------------- manifest

    fn manifest_fixed_values(manifest: &GenesisManifest) -> [Fixed; 32] {
        let t = &manifest.traits;
        let e = &manifest.expression;
        let a = &manifest.allostasis;
        let p = &manifest.epistemic;
        let s = &manifest.social;
        [
            t.baseline_warmth,
            t.baseline_patience,
            t.sensitivity,
            t.irritability,
            t.composure,
            t.epistemic_pride,
            t.epistemic_openness,
            t.boundary_strength,
            t.forgiveness,
            t.attachment_propensity,
            t.expression_drive,
            t.curiosity,
            e.warmth,
            e.directness,
            e.verbosity,
            e.self_disclosure,
            e.humor,
            e.formality,
            a.energy,
            a.arousal,
            a.contact_need,
            a.quiet_need,
            a.expression_pressure,
            a.exploration_drive,
            p.verification_drive,
            p.confidence_style,
            p.correction_defensiveness,
            p.repair_after_error,
            s.stranger_distance,
            s.approach_threshold,
            s.rejection_sensitivity,
            s.reciprocity_expectation,
        ]
    }

    /// Canonical binary ABI v1: fixed field order, little-endian i64
    /// fixed-point values, no self digest. The manifest_digest field is
    /// ignored during encoding (it is the digest of the rest).
    pub fn encode_manifest_body(manifest: &GenesisManifest) -> Vec<u8> {
        let mut out = Vec::with_capacity(MANIFEST_BODY_LEN);
        push_u16(&mut out, manifest.schema_version);
        for value in manifest_fixed_values(manifest) {
            out.extend_from_slice(&encode_fixed(value));
        }
        debug_assert_eq!(out.len(), MANIFEST_BODY_LEN);
        out
    }

    pub fn decode_manifest_body(bytes: &[u8]) -> Result<GenesisManifest, WireError> {
        if bytes.len() != MANIFEST_BODY_LEN {
            return Err(WireError::Boundary(MANIFEST_BODY_LEN, bytes.len()));
        }
        let mut reader = Reader::new(bytes);
        let schema_version = reader.u16()?;
        let mut values = [Fixed::ZERO; 32];
        for slot in &mut values {
            *slot = reader.fixed()?;
        }
        reader.finish()?;
        let t = &values[0..12];
        let e = &values[12..18];
        let a = &values[18..24];
        let p = &values[24..28];
        let s = &values[28..32];
        Ok(GenesisManifest {
            schema_version,
            traits: PersonalityVector {
                baseline_warmth: t[0],
                baseline_patience: t[1],
                sensitivity: t[2],
                irritability: t[3],
                composure: t[4],
                epistemic_pride: t[5],
                epistemic_openness: t[6],
                boundary_strength: t[7],
                forgiveness: t[8],
                attachment_propensity: t[9],
                expression_drive: t[10],
                curiosity: t[11],
            },
            expression: ExpressionPhenotype {
                warmth: e[0],
                directness: e[1],
                verbosity: e[2],
                self_disclosure: e[3],
                humor: e[4],
                formality: e[5],
            },
            allostasis: AllostaticSetpoints {
                energy: a[0],
                arousal: a[1],
                contact_need: a[2],
                quiet_need: a[3],
                expression_pressure: a[4],
                exploration_drive: a[5],
            },
            epistemic: EpistemicPriors {
                verification_drive: p[0],
                confidence_style: p[1],
                correction_defensiveness: p[2],
                repair_after_error: p[3],
            },
            social: SocialPriors {
                stranger_distance: s[0],
                approach_threshold: s[1],
                rejection_sensitivity: s[2],
                reciprocity_expectation: s[3],
            },
            manifest_digest: [0; 32],
        })
    }

    pub fn manifest_body_digest(manifest: &GenesisManifest) -> Digest {
        domain_hash(MANIFEST_BODY_DOMAIN, &[&encode_manifest_body(manifest)])
    }

    // ---------------------------------------------------------------- evidence

    fn encode_evidence_vector(out: &mut Vec<u8>, evidence: &EvidenceVector) {
        for value in [
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
        ] {
            out.extend_from_slice(&encode_fixed(value));
        }
    }

    fn decode_evidence_vector(reader: &mut Reader<'_>) -> Result<EvidenceVector, WireError> {
        Ok(EvidenceVector {
            positive: reader.fixed()?,
            affiliation: reader.fixed()?,
            harm: reader.fixed()?,
            boundary: reader.fixed()?,
            repair: reader.fixed()?,
            repetition: reader.fixed()?,
            new_information: reader.fixed()?,
            constraint_instability: reader.fixed()?,
            epistemic_conflict: reader.fixed()?,
            self_responsibility: reader.fixed()?,
            other_responsibility: reader.fixed()?,
            hostility: reader.fixed()?,
            publicness: reader.fixed()?,
            engagement: reader.fixed()?,
            rejection: reader.fixed()?,
        })
    }

    fn encode_semantic_estimate(out: &mut Vec<u8>, estimate: &SemanticEstimate) {
        push_u16(out, estimate.schema_version);
        encode_evidence_vector(out, &estimate.dimensions);
        out.extend_from_slice(&encode_fixed(estimate.estimator_confidence));
        push_digest(out, &estimate.estimator_digest);
    }

    fn decode_semantic_estimate(reader: &mut Reader<'_>) -> Result<SemanticEstimate, WireError> {
        Ok(SemanticEstimate {
            schema_version: reader.u16()?,
            dimensions: decode_evidence_vector(reader)?,
            estimator_confidence: reader.fixed()?,
            estimator_digest: reader.digest()?,
        })
    }

    // ---------------------------------------------------------------- events

    pub fn event_kind_code(event: &CanonicalEvent) -> u8 {
        match event {
            CanonicalEvent::UserStimulus(_) => KIND_USER_STIMULUS,
            CanonicalEvent::UserReaction(_) => KIND_USER_REACTION,
            CanonicalEvent::CorrectionClaim(_) => KIND_CORRECTION_CLAIM,
            CanonicalEvent::CorrectionVerdict(_) => KIND_CORRECTION_VERDICT,
            CanonicalEvent::SelfActionCandidate(_) => KIND_SELF_ACTION_CANDIDATE,
            CanonicalEvent::DeliveryOutcome(_) => KIND_DELIVERY_OUTCOME,
            CanonicalEvent::SettlementEvidence(_) => KIND_SETTLEMENT_EVIDENCE,
            CanonicalEvent::TimeAdvance(_) => KIND_TIME_ADVANCE,
            CanonicalEvent::AdminAction(_) => KIND_ADMIN_ACTION,
        }
    }

    pub fn event_kind_name(event: &CanonicalEvent) -> &'static str {
        match event {
            CanonicalEvent::UserStimulus(_) => "user_stimulus",
            CanonicalEvent::UserReaction(_) => "user_reaction",
            CanonicalEvent::CorrectionClaim(_) => "correction_claim",
            CanonicalEvent::CorrectionVerdict(_) => "correction_verdict",
            CanonicalEvent::SelfActionCandidate(_) => "self_action_candidate",
            CanonicalEvent::DeliveryOutcome(_) => "delivery_outcome",
            CanonicalEvent::SettlementEvidence(_) => "settlement_evidence",
            CanonicalEvent::TimeAdvance(_) => "time_advance",
            CanonicalEvent::AdminAction(_) => "admin_action",
        }
    }

    pub fn source_authority_code(source: SourceAuthority) -> u8 {
        match source {
            SourceAuthority::UserObserved => 1,
            SourceAuthority::ExplicitFeedback => 2,
            SourceAuthority::PlatformObserved => 3,
            SourceAuthority::VerifierResult => 4,
            SourceAuthority::SelfAction => 5,
            SourceAuthority::SelfCritique => 6,
            SourceAuthority::TimeAdvance => 7,
            SourceAuthority::AdminAction => 8,
            SourceAuthority::PersonaConfig => 9,
        }
    }

    pub fn source_authority_from_code(code: u8) -> Option<SourceAuthority> {
        Some(match code {
            1 => SourceAuthority::UserObserved,
            2 => SourceAuthority::ExplicitFeedback,
            3 => SourceAuthority::PlatformObserved,
            4 => SourceAuthority::VerifierResult,
            5 => SourceAuthority::SelfAction,
            6 => SourceAuthority::SelfCritique,
            7 => SourceAuthority::TimeAdvance,
            8 => SourceAuthority::AdminAction,
            9 => SourceAuthority::PersonaConfig,
            _ => return None,
        })
    }

    pub fn settlement_kind_code(kind: SettlementKind) -> u8 {
        match kind {
            SettlementKind::ExplicitAcceptance => 1,
            SettlementKind::ExplicitRejection => 2,
            SettlementKind::RepairAcknowledged => 3,
            SettlementKind::ConfirmedSelfError => 4,
            SettlementKind::RejectedChallenge => 5,
            SettlementKind::VerifiedBoundaryViolation => 6,
            SettlementKind::ConfirmedFrictionPattern => 7,
            SettlementKind::ToolResult => 8,
            SettlementKind::DeliveryTerminal => 9,
            SettlementKind::StrongContinuation => 10,
            SettlementKind::AmbiguousObservation => 11,
        }
    }

    pub fn settlement_kind_from_code(code: u8) -> Option<SettlementKind> {
        Some(match code {
            1 => SettlementKind::ExplicitAcceptance,
            2 => SettlementKind::ExplicitRejection,
            3 => SettlementKind::RepairAcknowledged,
            4 => SettlementKind::ConfirmedSelfError,
            5 => SettlementKind::RejectedChallenge,
            6 => SettlementKind::VerifiedBoundaryViolation,
            7 => SettlementKind::ConfirmedFrictionPattern,
            8 => SettlementKind::ToolResult,
            9 => SettlementKind::DeliveryTerminal,
            10 => SettlementKind::StrongContinuation,
            11 => SettlementKind::AmbiguousObservation,
            _ => return None,
        })
    }

    pub fn verdict_kind_code(kind: VerdictKind) -> u8 {
        match kind {
            VerdictKind::ConfirmedSelfError => 1,
            VerdictKind::RejectedChallenge => 2,
            VerdictKind::SharedAmbiguity => 3,
            VerdictKind::HostFailure => 4,
            VerdictKind::Unresolved => 5,
        }
    }

    pub fn verdict_kind_from_code(code: u8) -> Option<VerdictKind> {
        Some(match code {
            1 => VerdictKind::ConfirmedSelfError,
            2 => VerdictKind::RejectedChallenge,
            3 => VerdictKind::SharedAmbiguity,
            4 => VerdictKind::HostFailure,
            5 => VerdictKind::Unresolved,
            _ => return None,
        })
    }

    pub fn commit_status_code(status: CommitStatus) -> u8 {
        match status {
            CommitStatus::Committed => 1,
            CommitStatus::Rejected => 2,
            CommitStatus::Superseded => 3,
            CommitStatus::Stale => 4,
        }
    }

    pub fn commit_status_from_code(code: u8) -> Option<CommitStatus> {
        Some(match code {
            1 => CommitStatus::Committed,
            2 => CommitStatus::Rejected,
            3 => CommitStatus::Superseded,
            4 => CommitStatus::Stale,
            _ => return None,
        })
    }

    pub fn genesis_status_code(status: GenesisStatus) -> u8 {
        match status {
            GenesisStatus::Committed => 1,
            GenesisStatus::Rejected => 2,
            GenesisStatus::Superseded => 3,
            GenesisStatus::RetryWait => 4,
        }
    }

    pub fn genesis_status_from_code(code: u8) -> Option<GenesisStatus> {
        Some(match code {
            1 => GenesisStatus::Committed,
            2 => GenesisStatus::Rejected,
            3 => GenesisStatus::Superseded,
            4 => GenesisStatus::RetryWait,
            _ => return None,
        })
    }

    fn decode_scope_and_causal_payload(
        reader: &mut Reader<'_>,
    ) -> Result<(ScopeRef, CausalRef), WireError> {
        Ok((decode_scope(reader)?, decode_causal(reader)?))
    }

    pub fn encode_event(event: &CanonicalEvent) -> Vec<u8> {
        let mut out = Vec::with_capacity(128);
        push_u16(&mut out, WIRE_SCHEMA_VERSION);
        out.push(event_kind_code(event));
        match event {
            CanonicalEvent::UserStimulus(e) => {
                push_id(&mut out, &e.event_id);
                out.extend_from_slice(&encode_scope(&e.scope));
                encode_causal(&e.causal, &mut out);
                push_u64(&mut out, e.observed_at_ms);
                encode_semantic_estimate(&mut out, &e.evidence);
            }
            CanonicalEvent::UserReaction(e) => {
                push_id(&mut out, &e.event_id);
                out.extend_from_slice(&encode_scope(&e.scope));
                encode_causal(&e.causal, &mut out);
                push_u64(&mut out, e.observed_at_ms);
                encode_semantic_estimate(&mut out, &e.evidence);
            }
            CanonicalEvent::CorrectionClaim(e) => {
                push_id(&mut out, &e.event_id);
                out.extend_from_slice(&encode_scope(&e.scope));
                encode_causal(&e.causal, &mut out);
                out.extend_from_slice(&encode_fixed(e.specificity));
                out.extend_from_slice(&encode_fixed(e.supplied_evidence));
                out.extend_from_slice(&encode_fixed(e.hostility));
                out.extend_from_slice(&encode_fixed(e.publicness));
            }
            CanonicalEvent::CorrectionVerdict(e) => {
                push_id(&mut out, &e.event_id);
                out.extend_from_slice(&encode_scope(&e.scope));
                encode_causal(&e.causal, &mut out);
                out.push(verdict_kind_code(e.verdict));
                out.extend_from_slice(&encode_fixed(e.confidence));
                out.extend_from_slice(&encode_fixed(e.contradiction));
                out.extend_from_slice(&encode_fixed(e.hostility));
                push_digest(&mut out, &e.evidence_digest);
            }
            CanonicalEvent::SelfActionCandidate(e) => {
                push_id(&mut out, &e.event_id);
                out.extend_from_slice(&encode_scope(&e.scope));
                encode_causal(&e.causal, &mut out);
                push_digest(&mut out, &e.visible_action_digest);
                push_u32(&mut out, e.claims.len() as u32);
                for claim in &e.claims {
                    push_id(&mut out, &claim.claim_id);
                    out.extend_from_slice(&encode_fixed(claim.confidence));
                    out.extend_from_slice(&encode_fixed(claim.assertiveness));
                    out.extend_from_slice(&encode_fixed(claim.stakes));
                    out.extend_from_slice(&encode_fixed(claim.audience_publicness));
                    push_u64(&mut out, claim.expires_at_ms);
                }
            }
            CanonicalEvent::DeliveryOutcome(e) => {
                push_id(&mut out, &e.event_id);
                out.extend_from_slice(&encode_scope(&e.scope));
                encode_causal(&e.causal, &mut out);
                push_bool(&mut out, e.delivered);
                push_digest(&mut out, &e.visible_action_digest);
                push_u64(&mut out, e.delivered_at_ms);
            }
            CanonicalEvent::SettlementEvidence(e) => {
                push_id(&mut out, &e.settlement_id);
                out.extend_from_slice(&encode_scope(&e.scope));
                encode_causal(&e.causal, &mut out);
                out.push(settlement_kind_code(e.kind));
                out.push(source_authority_code(e.source));
                out.extend_from_slice(&encode_fixed(e.confidence));
                out.push(e.evidence_level);
                push_digest(&mut out, &e.evidence_digest);
                push_u64(&mut out, e.observed_at_ms);
            }
            CanonicalEvent::TimeAdvance(e) => {
                push_id(&mut out, &e.event_id);
                out.extend_from_slice(&encode_scope(&e.scope));
                push_u64(&mut out, e.elapsed_ms);
            }
            CanonicalEvent::AdminAction(e) => {
                push_id(&mut out, &e.event_id);
                out.extend_from_slice(&encode_scope(&e.scope));
                push_string(&mut out, &e.operation);
                push_digest(&mut out, &e.nonce_digest);
            }
        }
        out
    }

    pub fn decode_event(bytes: &[u8]) -> Result<CanonicalEvent, WireError> {
        let mut reader = Reader::new(bytes);
        let schema_version = reader.u16()?;
        if schema_version != WIRE_SCHEMA_VERSION {
            return Err(WireError::SchemaVersion(schema_version));
        }
        let kind = reader.u8()?;
        let event = match kind {
            KIND_USER_STIMULUS => {
                let event_id = reader.id()?;
                let (scope, causal) = decode_scope_and_causal_payload(&mut reader)?;
                let observed_at_ms = reader.u64()?;
                let evidence = decode_semantic_estimate(&mut reader)?;
                CanonicalEvent::UserStimulus(UserStimulus {
                    event_id,
                    scope,
                    causal,
                    observed_at_ms,
                    evidence,
                })
            }
            KIND_USER_REACTION => {
                let event_id = reader.id()?;
                let (scope, causal) = decode_scope_and_causal_payload(&mut reader)?;
                let observed_at_ms = reader.u64()?;
                let evidence = decode_semantic_estimate(&mut reader)?;
                CanonicalEvent::UserReaction(UserReaction {
                    event_id,
                    scope,
                    causal,
                    observed_at_ms,
                    evidence,
                })
            }
            KIND_CORRECTION_CLAIM => {
                let event_id = reader.id()?;
                let (scope, causal) = decode_scope_and_causal_payload(&mut reader)?;
                CanonicalEvent::CorrectionClaim(CorrectionClaim {
                    event_id,
                    scope,
                    causal,
                    specificity: reader.fixed()?,
                    supplied_evidence: reader.fixed()?,
                    hostility: reader.fixed()?,
                    publicness: reader.fixed()?,
                })
            }
            KIND_CORRECTION_VERDICT => {
                let event_id = reader.id()?;
                let (scope, causal) = decode_scope_and_causal_payload(&mut reader)?;
                let verdict = verdict_kind_from_code(reader.u8()?)
                    .ok_or(WireError::InvalidEnum("verdict kind"))?;
                CanonicalEvent::CorrectionVerdict(CorrectionVerdictEvent {
                    event_id,
                    scope,
                    causal,
                    verdict,
                    confidence: reader.fixed()?,
                    contradiction: reader.fixed()?,
                    hostility: reader.fixed()?,
                    evidence_digest: reader.digest()?,
                })
            }
            KIND_SELF_ACTION_CANDIDATE => {
                let event_id = reader.id()?;
                let (scope, causal) = decode_scope_and_causal_payload(&mut reader)?;
                let visible_action_digest = reader.digest()?;
                let count = reader.u32()? as usize;
                if count > MAX_CLAIMS {
                    return Err(WireError::TooManyClaims);
                }
                let mut claims = Vec::with_capacity(count);
                for _ in 0..count {
                    claims.push(ClaimCommitment {
                        claim_id: reader.id()?,
                        confidence: reader.fixed()?,
                        assertiveness: reader.fixed()?,
                        stakes: reader.fixed()?,
                        audience_publicness: reader.fixed()?,
                        expires_at_ms: reader.u64()?,
                    });
                }
                CanonicalEvent::SelfActionCandidate(SelfActionCandidate {
                    event_id,
                    scope,
                    causal,
                    visible_action_digest,
                    claims,
                })
            }
            KIND_DELIVERY_OUTCOME => {
                let event_id = reader.id()?;
                let (scope, causal) = decode_scope_and_causal_payload(&mut reader)?;
                CanonicalEvent::DeliveryOutcome(DeliveryOutcome {
                    event_id,
                    scope,
                    causal,
                    delivered: reader.bool()?,
                    visible_action_digest: reader.digest()?,
                    delivered_at_ms: reader.u64()?,
                })
            }
            KIND_SETTLEMENT_EVIDENCE => {
                let settlement_id = reader.id()?;
                let (scope, causal) = decode_scope_and_causal_payload(&mut reader)?;
                let kind = settlement_kind_from_code(reader.u8()?)
                    .ok_or(WireError::InvalidEnum("settlement kind"))?;
                let source = source_authority_from_code(reader.u8()?)
                    .ok_or(WireError::InvalidEnum("source authority"))?;
                CanonicalEvent::SettlementEvidence(SettlementEvidence {
                    settlement_id,
                    scope,
                    causal,
                    kind,
                    source,
                    confidence: reader.fixed()?,
                    evidence_level: reader.u8()?,
                    evidence_digest: reader.digest()?,
                    observed_at_ms: reader.u64()?,
                })
            }
            KIND_TIME_ADVANCE => {
                let event_id = reader.id()?;
                let scope = decode_scope(&mut reader)?;
                CanonicalEvent::TimeAdvance(TimeAdvance {
                    event_id,
                    scope,
                    elapsed_ms: reader.u64()?,
                })
            }
            KIND_ADMIN_ACTION => {
                let event_id = reader.id()?;
                let scope = decode_scope(&mut reader)?;
                CanonicalEvent::AdminAction(AdminAction {
                    event_id,
                    scope,
                    operation: reader.string()?,
                    nonce_digest: reader.digest()?,
                })
            }
            other => return Err(WireError::UnknownKind(other)),
        };
        reader.finish()?;
        Ok(event)
    }

    pub fn event_digest(event: &CanonicalEvent) -> Digest {
        domain_hash(EVENT_DOMAIN, &[&encode_event(event)])
    }

    // ---------------------------------------------------------------- action contract

    pub fn encode_action_contract(contract: &ActionContract) -> Vec<u8> {
        let mut out = Vec::with_capacity(140);
        push_id(&mut out, &contract.action_id);
        push_id(&mut out, &contract.turn_id);
        let c = &contract.continuous;
        for value in [
            c.answer,
            c.verify,
            c.acknowledge_error,
            c.repair,
            c.ask_evidence,
            c.set_boundary,
            c.withdraw,
            c.proactive_reach,
            c.warmth,
            c.directness,
            c.verbosity,
            c.confidence_ceiling,
        ] {
            out.extend_from_slice(&encode_fixed(value));
        }
        push_bool(&mut out, contract.must_verify);
        push_bool(&mut out, contract.must_acknowledge_error);
        push_bool(&mut out, contract.must_correct_claim);
        push_bool(&mut out, contract.may_set_boundary);
        push_bool(&mut out, contract.may_withdraw);
        push_bool(&mut out, contract.must_not_seek_reassurance);
        push_u64(&mut out, contract.expires_at_ms);
        out
    }

    pub fn decode_action_contract(bytes: &[u8]) -> Result<ActionContract, WireError> {
        let mut reader = Reader::new(bytes);
        let action_id = reader.id()?;
        let turn_id = reader.id()?;
        let mut values = [Fixed::ZERO; 12];
        for slot in &mut values {
            *slot = reader.fixed()?;
        }
        let contract = ActionContract {
            action_id,
            turn_id,
            continuous: ActionVector {
                answer: values[0],
                verify: values[1],
                acknowledge_error: values[2],
                repair: values[3],
                ask_evidence: values[4],
                set_boundary: values[5],
                withdraw: values[6],
                proactive_reach: values[7],
                warmth: values[8],
                directness: values[9],
                verbosity: values[10],
                confidence_ceiling: values[11],
            },
            must_verify: reader.bool()?,
            must_acknowledge_error: reader.bool()?,
            must_correct_claim: reader.bool()?,
            may_set_boundary: reader.bool()?,
            may_withdraw: reader.bool()?,
            must_not_seek_reassurance: reader.bool()?,
            expires_at_ms: reader.u64()?,
        };
        reader.finish()?;
        Ok(contract)
    }

    pub fn action_contract_digest(contract: &ActionContract) -> Digest {
        domain_hash(ACTION_CONTRACT_DOMAIN, &[&encode_action_contract(contract)])
    }

    // ---------------------------------------------------------------- transition receipt

    pub fn encode_transition_receipt(receipt: &TransitionReceipt) -> Vec<u8> {
        let mut out = Vec::with_capacity(260);
        push_u16(&mut out, receipt.schema_version);
        push_digest(&mut out, &receipt.formula_digest);
        push_digest(&mut out, &receipt.scope_digest);
        push_digest(&mut out, &receipt.event_digest);
        push_digest(&mut out, &receipt.authority_digest);
        push_u64(&mut out, receipt.base_revision);
        push_u64(&mut out, receipt.next_revision);
        push_digest(&mut out, &receipt.state_before);
        push_digest(&mut out, &receipt.state_after);
        push_digest(&mut out, &receipt.graph_after);
        push_opt_digest(&mut out, &receipt.action_contract);
        push_u32(&mut out, receipt.active_nodes);
        push_u32(&mut out, receipt.active_edges);
        let r = &receipt.residuals;
        for value in [
            r.authority,
            r.continuity,
            r.energy,
            r.renormalization,
            r.capacity,
        ] {
            out.extend_from_slice(&encode_fixed(value));
        }
        out.push(commit_status_code(receipt.status));
        out
    }

    pub fn decode_transition_receipt(bytes: &[u8]) -> Result<TransitionReceipt, WireError> {
        let mut reader = Reader::new(bytes);
        let receipt = TransitionReceipt {
            schema_version: reader.u16()?,
            formula_digest: reader.digest()?,
            scope_digest: reader.digest()?,
            event_digest: reader.digest()?,
            authority_digest: reader.digest()?,
            base_revision: reader.u64()?,
            next_revision: reader.u64()?,
            state_before: reader.digest()?,
            state_after: reader.digest()?,
            graph_after: reader.digest()?,
            action_contract: reader.opt_digest()?,
            active_nodes: reader.u32()?,
            active_edges: reader.u32()?,
            residuals: InvariantResiduals {
                authority: reader.fixed()?,
                continuity: reader.fixed()?,
                energy: reader.fixed()?,
                renormalization: reader.fixed()?,
                capacity: reader.fixed()?,
            },
            status: commit_status_from_code(reader.u8()?)
                .ok_or(WireError::InvalidEnum("commit status"))?,
        };
        reader.finish()?;
        Ok(receipt)
    }

    pub fn receipt_digest(receipt: &TransitionReceipt) -> Digest {
        domain_hash(
            TRANSITION_RECEIPT_DOMAIN,
            &[&encode_transition_receipt(receipt)],
        )
    }

    // ------------------------------------------------------ transition receipt v2

    fn semantic_vector_formula_v2_code(formula: SemanticVectorFormulaV2) -> u8 {
        match formula {
            SemanticVectorFormulaV2::FullVectorRouteNeutralRelaxationV1 => 1,
        }
    }

    fn semantic_vector_formula_v2_from_code(code: u8) -> Option<SemanticVectorFormulaV2> {
        Some(match code {
            1 => SemanticVectorFormulaV2::FullVectorRouteNeutralRelaxationV1,
            _ => return None,
        })
    }

    pub fn encode_transition_receipt_v2(receipt: &TransitionReceiptV2) -> Vec<u8> {
        let mut out = Vec::with_capacity(272);
        push_u16(&mut out, receipt.schema_version);
        push_digest(&mut out, &receipt.formula_digest);
        push_digest(&mut out, &receipt.scope_digest);
        push_digest(&mut out, &receipt.event_digest);
        push_digest(&mut out, &receipt.authority_digest);
        push_u64(&mut out, receipt.base_revision);
        push_u64(&mut out, receipt.next_revision);
        push_digest(&mut out, &receipt.state_before);
        push_digest(&mut out, &receipt.state_after);
        push_digest(&mut out, &receipt.graph_after);
        push_opt_digest(&mut out, &receipt.action_contract);
        push_u32(&mut out, receipt.active_nodes);
        push_u32(&mut out, receipt.active_edges);
        for value in [
            receipt.residuals.authority,
            receipt.residuals.continuity,
            receipt.residuals.energy,
            receipt.residuals.renormalization,
            receipt.residuals.capacity,
        ] {
            out.extend_from_slice(&encode_fixed(value));
        }
        out.push(commit_status_code(receipt.status));
        let vector = &receipt.semantic_vector;
        push_u16(&mut out, vector.schema_version);
        out.push(semantic_vector_formula_v2_code(vector.formula));
        out.push(vector.dimension_slot_count);
        out.push(vector.evaluated_dimension_count);
        out.push(vector.injected_dimension_count);
        out.push(vector.nonzero_evidence_dimension_count);
        out.push(vector.neutral_baseline_dimension_count);
        out.push(vector.unavailable_dimension_count);
        push_bool(&mut out, vector.state_changed);
        out
    }

    pub fn decode_transition_receipt_v2(bytes: &[u8]) -> Result<TransitionReceiptV2, WireError> {
        let mut reader = Reader::new(bytes);
        let schema_version = reader.u16()?;
        if schema_version != TransitionReceiptV2::SCHEMA_VERSION {
            return Err(WireError::SchemaVersion(schema_version));
        }
        let receipt = TransitionReceiptV2 {
            schema_version,
            formula_digest: reader.digest()?,
            scope_digest: reader.digest()?,
            event_digest: reader.digest()?,
            authority_digest: reader.digest()?,
            base_revision: reader.u64()?,
            next_revision: reader.u64()?,
            state_before: reader.digest()?,
            state_after: reader.digest()?,
            graph_after: reader.digest()?,
            action_contract: reader.opt_digest()?,
            active_nodes: reader.u32()?,
            active_edges: reader.u32()?,
            residuals: InvariantResiduals {
                authority: reader.fixed()?,
                continuity: reader.fixed()?,
                energy: reader.fixed()?,
                renormalization: reader.fixed()?,
                capacity: reader.fixed()?,
            },
            status: commit_status_from_code(reader.u8()?)
                .ok_or(WireError::InvalidEnum("commit status"))?,
            semantic_vector: SemanticVectorReceiptV2 {
                schema_version: reader.u16()?,
                formula: semantic_vector_formula_v2_from_code(reader.u8()?)
                    .ok_or(WireError::InvalidEnum("semantic vector formula v2"))?,
                dimension_slot_count: reader.u8()?,
                evaluated_dimension_count: reader.u8()?,
                injected_dimension_count: reader.u8()?,
                nonzero_evidence_dimension_count: reader.u8()?,
                neutral_baseline_dimension_count: reader.u8()?,
                unavailable_dimension_count: reader.u8()?,
                state_changed: reader.bool()?,
            },
        };
        reader.finish()?;
        if !receipt.validate() {
            return Err(WireError::InvalidEnum("transition receipt v2"));
        }
        Ok(receipt)
    }

    pub fn transition_receipt_v2_digest(receipt: &TransitionReceiptV2) -> Digest {
        domain_hash(
            TRANSITION_RECEIPT_V2_DOMAIN,
            &[&encode_transition_receipt_v2(receipt)],
        )
    }

    // ---------------------------------------------------------------- genesis receipt

    pub fn encode_genesis_receipt(receipt: &GenesisReceipt) -> Vec<u8> {
        let mut out = Vec::with_capacity(340);
        push_u16(&mut out, receipt.schema_version);
        for digest in [
            &receipt.seed_code_digest,
            &receipt.manifest_digest,
            &receipt.incarnation_id,
            &receipt.formula_digest,
            &receipt.persona_source_digest,
            &receipt.compiler_protocol_digest,
            &receipt.compiler_model_digest,
            &receipt.development_seed_digest,
            &receipt.initial_snapshot_digest,
            &receipt.graph_digest,
        ] {
            push_digest(&mut out, digest);
        }
        for value in [
            receipt.equilibrium_residual,
            receipt.energy_residual,
            receipt.capacity_residual,
            receipt.sample_fit_residual,
        ] {
            out.extend_from_slice(&encode_fixed(value));
        }
        out.push(genesis_status_code(receipt.status));
        out
    }

    pub fn decode_genesis_receipt(bytes: &[u8]) -> Result<GenesisReceipt, WireError> {
        let mut reader = Reader::new(bytes);
        let receipt = GenesisReceipt {
            schema_version: reader.u16()?,
            seed_code_digest: reader.digest()?,
            manifest_digest: reader.digest()?,
            incarnation_id: reader.digest()?,
            formula_digest: reader.digest()?,
            persona_source_digest: reader.digest()?,
            compiler_protocol_digest: reader.digest()?,
            compiler_model_digest: reader.digest()?,
            development_seed_digest: reader.digest()?,
            initial_snapshot_digest: reader.digest()?,
            graph_digest: reader.digest()?,
            equilibrium_residual: reader.fixed()?,
            energy_residual: reader.fixed()?,
            capacity_residual: reader.fixed()?,
            sample_fit_residual: reader.fixed()?,
            status: genesis_status_from_code(reader.u8()?)
                .ok_or(WireError::InvalidEnum("genesis status"))?,
        };
        reader.finish()?;
        Ok(receipt)
    }

    pub fn genesis_receipt_digest(receipt: &GenesisReceipt) -> Digest {
        domain_hash(
            TRANSITION_RECEIPT_DOMAIN,
            &[&encode_genesis_receipt(receipt)],
        )
    }

    // ---------------------------------------------------------------- capsule

    pub fn encode_capsule_body(capsule: &GenesisCapsule) -> Vec<u8> {
        let mut out = Vec::with_capacity(2 + 32 + MANIFEST_BODY_LEN + 64);
        push_u16(&mut out, capsule.schema_version);
        push_digest(&mut out, &capsule.seed_code_digest);
        out.extend_from_slice(&encode_manifest_body(&capsule.manifest));
        push_digest(&mut out, &capsule.provenance_digest);
        push_digest(&mut out, &capsule.capsule_digest);
        out
    }

    pub fn decode_capsule_body(bytes: &[u8]) -> Result<GenesisCapsule, WireError> {
        let mut reader = Reader::new(bytes);
        let schema_version = reader.u16()?;
        let seed_code_digest = reader.digest()?;
        let manifest_bytes: &[u8] = reader.take(MANIFEST_BODY_LEN)?;
        let manifest = decode_manifest_body(manifest_bytes)?;
        let provenance_digest = reader.digest()?;
        let capsule_digest = reader.digest()?;
        reader.finish()?;
        Ok(GenesisCapsule {
            schema_version,
            seed_code_digest,
            manifest,
            provenance_digest,
            capsule_digest,
        })
    }

    /// Capsule identity covers schema, seed digest, canonical manifest body
    /// and provenance. The embedded capsule_digest field is excluded (it is
    /// the digest of everything else).
    pub fn capsule_digest(capsule: &GenesisCapsule) -> Digest {
        domain_hash(
            CAPSULE_DOMAIN,
            &[
                &capsule.schema_version.to_le_bytes(),
                &capsule.seed_code_digest,
                &encode_manifest_body(&capsule.manifest),
                &capsule.provenance_digest,
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wire::*;

    fn scope() -> ScopeRef {
        ScopeRef {
            bot_token: [7; 16],
            persona_token: [9; 16],
            relation_token: None,
            session_token: [3; 16],
        }
    }

    fn sample_manifest() -> GenesisManifest {
        let mut manifest = GenesisManifest {
            schema_version: 1,
            traits: PersonalityVector {
                baseline_warmth: Fixed::from_raw(600_000),
                ..PersonalityVector::default()
            },
            expression: ExpressionPhenotype::default(),
            allostasis: AllostaticSetpoints::default(),
            epistemic: EpistemicPriors::default(),
            social: SocialPriors::default(),
            manifest_digest: [0; 32],
        };
        manifest.manifest_digest = manifest_body_digest(&manifest);
        manifest
    }

    #[test]
    fn manifest_body_has_fixed_layout() {
        let manifest = sample_manifest();
        let bytes = encode_manifest_body(&manifest);
        assert_eq!(bytes.len(), MANIFEST_BODY_LEN);
        assert_eq!(bytes[0], 1);
        assert_eq!(bytes[1], 0);
        // baseline_warmth = 600_000 = 0x0009_27C0 little-endian.
        assert_eq!(&bytes[2..10], &[0xC0, 0x27, 0x09, 0x00, 0, 0, 0, 0]);
        let decoded = decode_manifest_body(&bytes).unwrap();
        assert_eq!(decoded.traits, manifest.traits);
        assert_eq!(decoded.manifest_digest, [0; 32]);
    }

    #[test]
    fn manifest_decode_rejects_wrong_length() {
        let bytes = encode_manifest_body(&sample_manifest());
        assert_eq!(
            decode_manifest_body(&bytes[..bytes.len() - 1]).unwrap_err(),
            WireError::Boundary(MANIFEST_BODY_LEN, bytes.len() - 1)
        );
    }

    #[test]
    fn closed_schema_rejects_unknown_json_field() {
        let proposal = GenesisManifestProposal {
            schema_version: 1,
            source: PersonaSourceRef {
                scope: PersonaScopeRef {
                    bot_token: [0xAA; 16],
                    persona_token: [0xBB; 16],
                },
                source_digest: [0xCC; 32],
                capability_digest: [0xDD; 32],
                selection: PersonaSelectionKind::Conversation,
                prompt_chars: 10,
                begin_dialog_count: 0,
                mood_dialog_count: 0,
            },
            traits: PersonalityVector::default(),
            trait_confidence: PersonalityVector::default(),
            expression: ExpressionPhenotype::default(),
            allostasis: AllostaticSetpoints::default(),
            epistemic: EpistemicPriors::default(),
            social: SocialPriors::default(),
            compiler_protocol_digest: [0xEE; 32],
            compiler_model_digest: [0xFF; 32],
        };
        let mut json = serde_json::to_value(&proposal).unwrap();
        serde_json::from_value::<GenesisManifestProposal>(json.clone()).unwrap();
        json["neural_topology"] = serde_json::json!({});
        let err = serde_json::from_value::<GenesisManifestProposal>(json).unwrap_err();
        assert!(err.to_string().contains("unknown field"), "{err}");
    }

    #[test]
    fn canonical_event_rejects_unknown_json_field() {
        let mut json = serde_json::to_value(CanonicalEvent::TimeAdvance(TimeAdvance {
            event_id: [1; 16],
            scope: scope(),
            elapsed_ms: 5,
        }))
        .unwrap();
        json["payload"]["secret"] = serde_json::json!(1);
        let err = serde_json::from_value::<CanonicalEvent>(json).unwrap_err();
        assert!(err.to_string().contains("unknown field"), "{err}");
    }

    #[test]
    fn event_codec_round_trips_every_kind() {
        let causal = CausalRef {
            turn_id: [1; 16],
            action_id: Some([2; 16]),
            delivery_id: None,
            claim_id: Some([3; 16]),
            base_revision: 7,
        };
        let estimate = SemanticEstimate {
            schema_version: 1,
            dimensions: EvidenceVector {
                positive: Fixed::from_raw(100),
                ..EvidenceVector::default()
            },
            estimator_confidence: Fixed::from_raw(500_000),
            estimator_digest: [4; 32],
        };
        let events = [
            CanonicalEvent::UserStimulus(UserStimulus {
                event_id: [5; 16],
                scope: scope(),
                causal: causal.clone(),
                observed_at_ms: 10,
                evidence: estimate.clone(),
            }),
            CanonicalEvent::UserReaction(UserReaction {
                event_id: [6; 16],
                scope: scope(),
                causal: causal.clone(),
                observed_at_ms: 11,
                evidence: estimate.clone(),
            }),
            CanonicalEvent::CorrectionClaim(CorrectionClaim {
                event_id: [7; 16],
                scope: scope(),
                causal: causal.clone(),
                specificity: Fixed::ONE,
                supplied_evidence: Fixed::ZERO,
                hostility: Fixed::from_raw(10),
                publicness: Fixed::from_raw(20),
            }),
            CanonicalEvent::CorrectionVerdict(CorrectionVerdictEvent {
                event_id: [8; 16],
                scope: scope(),
                causal: causal.clone(),
                verdict: VerdictKind::RejectedChallenge,
                confidence: Fixed::from_raw(30),
                contradiction: Fixed::from_raw(40),
                hostility: Fixed::ZERO,
                evidence_digest: [9; 32],
            }),
            CanonicalEvent::SelfActionCandidate(SelfActionCandidate {
                event_id: [10; 16],
                scope: scope(),
                causal: causal.clone(),
                visible_action_digest: [11; 32],
                claims: vec![ClaimCommitment {
                    claim_id: [12; 16],
                    confidence: Fixed::from_raw(50),
                    assertiveness: Fixed::from_raw(60),
                    stakes: Fixed::from_raw(70),
                    audience_publicness: Fixed::from_raw(80),
                    expires_at_ms: 90,
                }],
            }),
            CanonicalEvent::DeliveryOutcome(DeliveryOutcome {
                event_id: [13; 16],
                scope: scope(),
                causal: causal.clone(),
                delivered: true,
                visible_action_digest: [14; 32],
                delivered_at_ms: 15,
            }),
            CanonicalEvent::SettlementEvidence(SettlementEvidence {
                settlement_id: [16; 16],
                scope: scope(),
                causal: causal.clone(),
                kind: SettlementKind::ExplicitAcceptance,
                source: SourceAuthority::ExplicitFeedback,
                confidence: Fixed::from_raw(17),
                evidence_level: 2,
                evidence_digest: [18; 32],
                observed_at_ms: 19,
            }),
            CanonicalEvent::TimeAdvance(TimeAdvance {
                event_id: [20; 16],
                scope: scope(),
                elapsed_ms: 21,
            }),
            CanonicalEvent::AdminAction(AdminAction {
                event_id: [22; 16],
                scope: scope(),
                operation: "migration".to_string(),
                nonce_digest: [23; 32],
            }),
        ];
        for event in events {
            let bytes = encode_event(&event);
            let decoded = decode_event(&bytes).unwrap();
            assert_eq!(decoded, event);
            assert_eq!(event_kind_code(&decoded), event_kind_code(&event));
        }
    }

    #[test]
    fn event_decode_rejects_trailing_bytes_and_unknown_kind() {
        let event = CanonicalEvent::TimeAdvance(TimeAdvance {
            event_id: [1; 16],
            scope: scope(),
            elapsed_ms: 1,
        });
        let mut bytes = encode_event(&event);
        bytes.push(0xFF);
        assert_eq!(
            decode_event(&bytes).unwrap_err(),
            WireError::TrailingBytes(1)
        );
        bytes[2] = 99;
        assert_eq!(
            decode_event(&bytes).unwrap_err(),
            WireError::UnknownKind(99)
        );
    }

    #[test]
    fn receipt_and_contract_codecs_round_trip() {
        let contract = ActionContract {
            action_id: [1; 16],
            turn_id: [2; 16],
            continuous: ActionVector {
                answer: Fixed::ONE,
                directness: Fixed::from_raw(500_000),
                verbosity: Fixed::from_raw(500_000),
                confidence_ceiling: Fixed::from_raw(700_000),
                ..ActionVector::default()
            },
            must_verify: false,
            must_acknowledge_error: false,
            must_correct_claim: false,
            may_set_boundary: true,
            may_withdraw: true,
            must_not_seek_reassurance: true,
            expires_at_ms: 0,
        };
        let contract_bytes = encode_action_contract(&contract);
        assert_eq!(decode_action_contract(&contract_bytes).unwrap(), contract);

        let receipt = TransitionReceipt {
            schema_version: 1,
            formula_digest: [3; 32],
            scope_digest: [4; 32],
            event_digest: [5; 32],
            authority_digest: [6; 32],
            base_revision: 0,
            next_revision: 1,
            state_before: [7; 32],
            state_after: [8; 32],
            graph_after: [9; 32],
            action_contract: Some(action_contract_digest(&contract)),
            active_nodes: 16_384,
            active_edges: 0,
            residuals: InvariantResiduals::default(),
            status: CommitStatus::Committed,
        };
        let receipt_bytes = encode_transition_receipt(&receipt);
        assert_eq!(decode_transition_receipt(&receipt_bytes).unwrap(), receipt);
        assert_ne!(receipt_digest(&receipt), [0; 32]);

        let mut without_contract = receipt.clone();
        without_contract.action_contract = None;
        let round =
            decode_transition_receipt(&encode_transition_receipt(&without_contract)).unwrap();
        assert_eq!(round.action_contract, None);
    }

    #[test]
    fn scope_digest_is_stable_and_relation_sensitive() {
        let mut scoped = scope();
        let base = scope_digest(&scoped);
        assert_eq!(base, scope_digest(&scoped));
        scoped.relation_token = Some([42; 16]);
        assert_ne!(base, scope_digest(&scoped));
    }

    #[test]
    fn hex_serde_round_trips() {
        let source = PersonaSourceRef {
            scope: PersonaScopeRef {
                bot_token: [0xAB; 16],
                persona_token: [0x0F; 16],
            },
            source_digest: [0xCD; 32],
            capability_digest: [0xEF; 32],
            selection: PersonaSelectionKind::Conversation,
            prompt_chars: 10,
            begin_dialog_count: 0,
            mood_dialog_count: 0,
        };
        let json = serde_json::to_value(&source).unwrap();
        assert_eq!(
            json["scope"]["bot_token"],
            serde_json::json!("ab".repeat(16))
        );
        assert_eq!(
            json["scope"]["persona_token"],
            serde_json::json!("0f".repeat(16))
        );
        assert_eq!(json["source_digest"], serde_json::json!("cd".repeat(32)));
        assert_eq!(
            json["capability_digest"],
            serde_json::json!("ef".repeat(32))
        );
        let back: PersonaSourceRef = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(back, source);

        let mut invalid = json;
        invalid["source_digest"] = serde_json::json!("zz");
        assert!(serde_json::from_value::<PersonaSourceRef>(invalid).is_err());
    }

    #[test]
    fn genesis_receipt_codec_round_trips() {
        let receipt = GenesisReceipt {
            schema_version: 1,
            seed_code_digest: [1; 32],
            manifest_digest: [2; 32],
            incarnation_id: [3; 32],
            formula_digest: [4; 32],
            persona_source_digest: [5; 32],
            compiler_protocol_digest: [6; 32],
            compiler_model_digest: [7; 32],
            development_seed_digest: [8; 32],
            initial_snapshot_digest: [9; 32],
            graph_digest: [10; 32],
            equilibrium_residual: Fixed::ZERO,
            energy_residual: Fixed::ZERO,
            capacity_residual: Fixed::ZERO,
            sample_fit_residual: Fixed::ZERO,
            status: GenesisStatus::Committed,
        };
        let bytes = encode_genesis_receipt(&receipt);
        assert_eq!(decode_genesis_receipt(&bytes).unwrap(), receipt);
    }
}
