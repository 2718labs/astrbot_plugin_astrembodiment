//! Frozen Phase-0 emotion evidence contracts restored from the audited R7 source.
//!
//! The alpha3 crate-level evidence, authority, and wire types remain canonical;
//! this module only adds the frozen routing and observability contracts around
//! those existing types.

use crate::{
    wire, ActionContract, AffectTrendV1, ApplyInteractionResultV1, Digest, EvidenceVector, Id128,
    InteractionFactBatchV1, InvariantResiduals, ScopeRef, SourceAuthority, AFFECT_FXP6_SCALE_V1,
    AFFECT_REGION_COUNT_V1,
};
use ae_fixed::Fixed;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Closed, content-free classification for an `INVALID_NEURAL_STATE` rejection.
///
/// This type is deliberately not serialized into any receipt, snapshot, or
/// persistence record. It only crosses the native error boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StateSubcodeV1 {
    BaselineStateInvalid,
    FieldStateInvalid,
    GraphStateInvalid,
    DynamicsInvalid,
    SemanticClosureInvalid,
    SnapshotWireInvalid,
    SnapshotAttestationMismatch,
    Aesem3RetiredCompensationNonzero,
    RelationScopeMissing,
    UnknownInvalidNeuralState,
}

impl StateSubcodeV1 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BaselineStateInvalid => "BASELINE_STATE_INVALID",
            Self::FieldStateInvalid => "FIELD_STATE_INVALID",
            Self::GraphStateInvalid => "GRAPH_STATE_INVALID",
            Self::DynamicsInvalid => "DYNAMICS_INVALID",
            Self::SemanticClosureInvalid => "SEMANTIC_CLOSURE_INVALID",
            Self::SnapshotWireInvalid => "SNAPSHOT_WIRE_INVALID",
            Self::SnapshotAttestationMismatch => "SNAPSHOT_ATTESTATION_MISMATCH",
            Self::Aesem3RetiredCompensationNonzero => "AESEM3_RETIRED_COMPENSATION_NONZERO",
            Self::RelationScopeMissing => "RELATION_SCOPE_MISSING",
            Self::UnknownInvalidNeuralState => "UNKNOWN_INVALID_NEURAL_STATE",
        }
    }
}

impl fmt::Display for StateSubcodeV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub const PHASE0_NATIVE_FORMULA_DIGEST_DOMAIN_V1: &[u8] =
    b"astr-embodiment/phase0-native-propagation-fxp6-v1";
pub const PHASE0_NATIVE_GRAPH_FORMULA_V1: &[u8] = b"graph-formula-v1";
pub const PHASE0_NATIVE_DYNAMICS_FORMULA_V1: &str = "phase0-native-propagation-fxp6-v1";
pub const PHASE0_NATIVE_PROPAGATION_RATE_FXP6: Fixed = Fixed::from_raw(125_000);
pub const PHASE0_NATIVE_NEUTRAL_RATE_FXP6: Fixed = Fixed::from_raw(125_000);
pub const PHASE0_NATIVE_ADAPTATION_RATE_FXP6: Fixed = Fixed::from_raw(125_000);
pub const PHASE0_NATIVE_RESERVE_RECOVERY_RATE_FXP6: Fixed = Fixed::from_raw(25_000);
pub const PHASE0_NATIVE_ENERGY_COST_RATE_FXP6: Fixed = Fixed::from_raw(100_000);

/// Frozen ten-minute phase used by the deterministic semantic time lane.
///
/// This value is derived from the committed pre-transition autonomous state;
/// it is never accepted as caller authority by the Store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatrixSleepPhaseV1 {
    Awake,
    Drowsy,
    Asleep,
}

/// Durable accumulator that makes elapsed-time chunking byte deterministic.
/// The anchor field itself remains in the semantic snapshot at
/// `anchor_semantic_revision`; this closed cursor only records how the view is
/// derived from that immutable anchor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatrixTimeEpochV1 {
    pub schema_version: u16,
    pub anchor_semantic_revision: u64,
    #[serde(with = "crate::hex::d32")]
    pub anchor_state_digest: Digest,
    pub awake_ticks: u64,
    pub drowsy_ticks: u64,
    pub asleep_ticks: u64,
    /// Sub-quantum elapsed time is kept per phase. A single shared remainder
    /// would silently relabel time when sleep changes between wake events.
    pub awake_remainder_ms: u64,
    pub drowsy_remainder_ms: u64,
    pub asleep_remainder_ms: u64,
}

impl MatrixTimeEpochV1 {
    pub const SCHEMA_VERSION: u16 = 1;

    pub fn accounted_elapsed_ms(&self, quantum_ms: u64) -> Option<u64> {
        self.awake_ticks
            .checked_add(self.drowsy_ticks)?
            .checked_add(self.asleep_ticks)?
            .checked_mul(quantum_ms)?
            .checked_add(self.awake_remainder_ms)?
            .checked_add(self.drowsy_remainder_ms)?
            .checked_add(self.asleep_remainder_ms)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticTransitionKindV1 {
    Perception,
    Time,
}

impl SemanticTransitionKindV1 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Perception => "perception",
            Self::Time => "time",
        }
    }

    // Existing closed parser returns Option; preserve that public contract.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "perception" => Some(Self::Perception),
            "time" => Some(Self::Time),
            _ => None,
        }
    }
}

/// Store-derived authority for one evidence-free semantic time transition.
/// It deliberately contains no relation, recipient, content or delivery data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticTimeAuthorityV1 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d32")]
    pub persona_scope: Digest,
    pub semantic_base_revision: u64,
    pub semantic_revision: u64,
    pub journal_revision: u64,
    #[serde(with = "crate::hex::d16")]
    pub event_id: Id128,
    #[serde(with = "crate::hex::d32")]
    pub event_digest: Digest,
    pub pre_sleep_phase: MatrixSleepPhaseV1,
    pub interval_start_utc_ms: u64,
    pub interval_end_utc_ms: u64,
    pub raw_elapsed_ms: u64,
    pub applied_elapsed_ms: u64,
    pub capped_gap: bool,
    #[serde(with = "crate::hex::d32")]
    pub incarnation_id: Digest,
    #[serde(with = "crate::hex::d32")]
    pub manifest_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub route_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub semantic_formula_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub time_formula_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub state_before: Digest,
    #[serde(with = "crate::hex::d32")]
    pub state_after: Digest,
    #[serde(with = "crate::hex::d32")]
    pub graph_digest: Digest,
    pub epoch_before: MatrixTimeEpochV1,
    pub epoch_after: MatrixTimeEpochV1,
    #[serde(with = "crate::hex::d32")]
    pub autonomy_state_before: Digest,
    #[serde(with = "crate::hex::d32")]
    pub autonomy_state_after: Digest,
    #[serde(with = "crate::hex::d32")]
    pub journal_delta_digest: Digest,
}

impl SemanticTimeAuthorityV1 {
    pub const SCHEMA_VERSION: u16 = 1;
    pub const MAX_APPLIED_ELAPSED_MS: u64 = 604_800_000;
    pub const QUANTUM_MS: u64 = 600_000;

    pub fn validate_v1(&self) -> bool {
        let digests = [
            self.persona_scope,
            self.event_digest,
            self.incarnation_id,
            self.manifest_digest,
            self.route_digest,
            self.semantic_formula_digest,
            self.time_formula_digest,
            self.state_before,
            self.state_after,
            self.graph_digest,
            self.autonomy_state_before,
            self.autonomy_state_after,
            self.journal_delta_digest,
        ];
        let elapsed_valid = self
            .interval_end_utc_ms
            .checked_sub(self.interval_start_utc_ms)
            == Some(self.raw_elapsed_ms)
            && self.raw_elapsed_ms > 0
            && self.applied_elapsed_ms == self.raw_elapsed_ms.min(Self::MAX_APPLIED_ELAPSED_MS)
            && self.capped_gap == (self.raw_elapsed_ms > Self::MAX_APPLIED_ELAPSED_MS);
        let epoch_delta = self
            .epoch_after
            .accounted_elapsed_ms(Self::QUANTUM_MS)
            .zip(self.epoch_before.accounted_elapsed_ms(Self::QUANTUM_MS))
            .and_then(|(after, before)| after.checked_sub(before));
        self.schema_version == Self::SCHEMA_VERSION
            && self.semantic_base_revision.checked_add(1) == Some(self.semantic_revision)
            && self.journal_revision > 0
            && self.event_id != [0; 16]
            && digests.iter().all(|value| *value != [0; 32])
            && elapsed_valid
            && self.epoch_before.schema_version == MatrixTimeEpochV1::SCHEMA_VERSION
            && self.epoch_after.schema_version == MatrixTimeEpochV1::SCHEMA_VERSION
            && self.epoch_before.anchor_semantic_revision
                == self.epoch_after.anchor_semantic_revision
            && self.epoch_before.anchor_state_digest == self.epoch_after.anchor_state_digest
            && epoch_delta == Some(self.applied_elapsed_ms)
            && self.epoch_after.awake_remainder_ms < Self::QUANTUM_MS
            && self.epoch_after.drowsy_remainder_ms < Self::QUANTUM_MS
            && self.epoch_after.asleep_remainder_ms < Self::QUANTUM_MS
    }
}

pub const PHASE0_SEMANTIC_ROUTE_DIGEST_DOMAIN_V1: &[u8] =
    b"astr-embodiment/semantic-evidence-route-neutral-v1";
pub const PHASE0_SEMANTIC_ROUTE_PRIMARY_COEFFICIENT_FXP6: Fixed = Fixed::ONE;
pub const PHASE0_SEMANTIC_ROUTE_SECONDARY_COEFFICIENT_FXP6: Fixed = Fixed::from_raw(500_000);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Phase0SemanticRouteRuleV1 {
    pub primary: u8,
    pub secondary: Option<u8>,
}

/// Frozen fifteen-slot semantic route used by both native dynamics and the
/// durable formula-authenticity fence.
pub const PHASE0_SEMANTIC_ROUTE_RULES_V1: [Phase0SemanticRouteRuleV1; 15] = [
    Phase0SemanticRouteRuleV1 {
        primary: 1,
        secondary: Some(8),
    },
    Phase0SemanticRouteRuleV1 {
        primary: 1,
        secondary: Some(8),
    },
    Phase0SemanticRouteRuleV1 {
        primary: 0,
        secondary: Some(5),
    },
    Phase0SemanticRouteRuleV1 {
        primary: 4,
        secondary: Some(5),
    },
    Phase0SemanticRouteRuleV1 {
        primary: 3,
        secondary: Some(8),
    },
    Phase0SemanticRouteRuleV1 {
        primary: 2,
        secondary: Some(7),
    },
    Phase0SemanticRouteRuleV1 {
        primary: 6,
        secondary: Some(2),
    },
    Phase0SemanticRouteRuleV1 {
        primary: 2,
        secondary: Some(3),
    },
    Phase0SemanticRouteRuleV1 {
        primary: 3,
        secondary: Some(7),
    },
    Phase0SemanticRouteRuleV1 {
        primary: 3,
        secondary: Some(7),
    },
    Phase0SemanticRouteRuleV1 {
        primary: 4,
        secondary: Some(7),
    },
    Phase0SemanticRouteRuleV1 {
        primary: 5,
        secondary: Some(4),
    },
    Phase0SemanticRouteRuleV1 {
        primary: 4,
        secondary: Some(7),
    },
    Phase0SemanticRouteRuleV1 {
        primary: 8,
        secondary: Some(7),
    },
    Phase0SemanticRouteRuleV1 {
        primary: 0,
        secondary: Some(4),
    },
];

/// Derive the frozen fifteen-slot route commitment without caller inputs.
pub fn phase0_semantic_route_digest_v1() -> Digest {
    let mut route_bytes = Vec::with_capacity(PHASE0_SEMANTIC_ROUTE_RULES_V1.len() * 18);
    for route in PHASE0_SEMANTIC_ROUTE_RULES_V1 {
        route_bytes.push(route.primary);
        route_bytes.push(route.secondary.unwrap_or(u8::MAX));
        route_bytes.extend_from_slice(
            &PHASE0_SEMANTIC_ROUTE_PRIMARY_COEFFICIENT_FXP6
                .raw()
                .to_be_bytes(),
        );
        route_bytes.extend_from_slice(
            &PHASE0_SEMANTIC_ROUTE_SECONDARY_COEFFICIENT_FXP6
                .raw()
                .to_be_bytes(),
        );
    }
    wire::domain_hash(PHASE0_SEMANTIC_ROUTE_DIGEST_DOMAIN_V1, &[&route_bytes])
}

fn phase0_native_formula_digest_v1(
    genesis_formula_digest: &Digest,
    route_digest: &Digest,
) -> Digest {
    let mut constants = Vec::with_capacity(8 * 5 + PHASE0_NATIVE_GRAPH_FORMULA_V1.len());
    constants.extend_from_slice(PHASE0_NATIVE_GRAPH_FORMULA_V1);
    for value in [
        PHASE0_NATIVE_PROPAGATION_RATE_FXP6,
        PHASE0_NATIVE_NEUTRAL_RATE_FXP6,
        PHASE0_NATIVE_ADAPTATION_RATE_FXP6,
        PHASE0_NATIVE_RESERVE_RECOVERY_RATE_FXP6,
        PHASE0_NATIVE_ENERGY_COST_RATE_FXP6,
    ] {
        constants.extend_from_slice(&value.raw().to_le_bytes());
    }
    wire::domain_hash(
        PHASE0_NATIVE_FORMULA_DIGEST_DOMAIN_V1,
        &[
            genesis_formula_digest,
            route_digest,
            PHASE0_NATIVE_DYNAMICS_FORMULA_V1.as_bytes(),
            &constants,
        ],
    )
}

/// Derive the sole Phase-0 formula digest from Genesis authority and the
/// frozen route commitment.
pub fn phase0_canonical_formula_digest_v1(genesis_formula_digest: &Digest) -> Digest {
    phase0_native_formula_digest_v1(genesis_formula_digest, &phase0_semantic_route_digest_v1())
}

/// Store-attested origin of one inbound semantic-perception request.
///
/// Every field is derived from a committed `InteractionFactBatch` and the
/// active Genesis binding.  Estimators may echo this value but may not mint or
/// alter it: the Store compares it byte-for-byte with its private challenge
/// row before accepting evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerceptionOriginCommitmentV1 {
    pub schema_version: u16,
    pub source_authority: SourceAuthority,
    #[serde(with = "crate::hex::d32")]
    pub source_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub model_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub provider_digest: Digest,
    pub scope: ScopeRef,
    #[serde(with = "crate::hex::d32")]
    pub scope_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub persona_scope: Digest,
    pub relation_present: bool,
    #[serde(with = "crate::hex::d32")]
    pub relation_scope: Digest,
    /// Relation-local semantic event identity.  For a Host-observed inbound
    /// proposal this is exactly the committed origin turn ID; it is not the
    /// database-global InteractionFactBatch or InteractionFact identity.
    #[serde(with = "crate::hex::d16")]
    pub event_id: Id128,
    #[serde(with = "crate::hex::d16")]
    pub turn_id: Id128,
    pub observed_at_ms: u64,
    pub canonical_base_revision: u64,
    #[serde(with = "crate::hex::d32")]
    pub incarnation_id: Digest,
    #[serde(with = "crate::hex::d32")]
    pub manifest_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub origin_event_digest: Digest,
    pub origin_journal_revision: u64,
    #[serde(with = "crate::hex::d32")]
    pub origin_digest: Digest,
}

impl PerceptionOriginCommitmentV1 {
    pub const SCHEMA_VERSION: u16 = 1;
    pub const DIGEST_DOMAIN_V1: &'static [u8] = b"astr-embodiment/perception-origin-commitment-v1";

    pub fn digest_v1(&self) -> Digest {
        let schema = self.schema_version.to_le_bytes();
        let authority = [wire::source_authority_code(self.source_authority)];
        let scope_bytes = wire::encode_scope(&self.scope);
        let relation_present = [u8::from(self.relation_present)];
        let observed_at = self.observed_at_ms.to_le_bytes();
        let base_revision = self.canonical_base_revision.to_le_bytes();
        let origin_revision = self.origin_journal_revision.to_le_bytes();
        wire::domain_hash(
            Self::DIGEST_DOMAIN_V1,
            &[
                &schema,
                &authority,
                &self.source_digest,
                &self.model_digest,
                &self.provider_digest,
                &scope_bytes,
                &self.scope_digest,
                &self.persona_scope,
                &relation_present,
                &self.relation_scope,
                &self.event_id,
                &self.turn_id,
                &observed_at,
                &base_revision,
                &self.incarnation_id,
                &self.manifest_digest,
                &self.origin_event_digest,
                &origin_revision,
            ],
        )
    }

    pub fn validate_v1(&self) -> bool {
        let persona_scope =
            wire::persona_scope_digest(&self.scope.bot_token, &self.scope.persona_token, None);
        let relation_scope = self.scope.relation_token.as_ref().map(|relation| {
            wire::persona_scope_digest(
                &self.scope.bot_token,
                &self.scope.persona_token,
                Some(relation),
            )
        });
        self.schema_version == Self::SCHEMA_VERSION
            && self.source_authority == SourceAuthority::UserObserved
            && self.source_digest.iter().any(|byte| *byte != 0)
            && self.model_digest.iter().any(|byte| *byte != 0)
            && self.provider_digest.iter().any(|byte| *byte != 0)
            && self.scope.bot_token.iter().any(|byte| *byte != 0)
            && self.scope.persona_token.iter().any(|byte| *byte != 0)
            && self.scope.session_token.iter().any(|byte| *byte != 0)
            && self
                .scope
                .relation_token
                .as_ref()
                .map(|relation| relation.iter().any(|byte| *byte != 0))
                .unwrap_or(false)
            && self.scope_digest == wire::scope_digest(&self.scope)
            && self.persona_scope == persona_scope
            && self.relation_present
            && relation_scope == Some(self.relation_scope)
            && self.event_id.iter().any(|byte| *byte != 0)
            && self.turn_id.iter().any(|byte| *byte != 0)
            && self.observed_at_ms != 0
            && self.origin_journal_revision != 0
            && self.canonical_base_revision >= self.origin_journal_revision
            && self.incarnation_id.iter().any(|byte| *byte != 0)
            && self.manifest_digest.iter().any(|byte| *byte != 0)
            && self.origin_event_digest.iter().any(|byte| *byte != 0)
            && self.origin_digest == self.digest_v1()
    }

    /// Core-only projection. The historical relation-required validator above
    /// remains unchanged; only Store-authenticated core origins use this form.
    pub fn validate_core_v1(&self) -> bool {
        if self.scope.relation_token.is_some()
            || self.relation_present
            || self.relation_scope != self.persona_scope
            || self.scope.session_token != self.turn_id
            || self.event_id != self.turn_id
        {
            return false;
        }
        // Apply precisely the same non-relation constraints through the legacy
        // validator without changing any public bytes or origin digest.
        let mut legacy = self.clone();
        legacy.scope.relation_token = Some([1; 16]);
        legacy.scope_digest = wire::scope_digest(&legacy.scope);
        legacy.relation_present = true;
        legacy.relation_scope = wire::persona_scope_digest(
            &legacy.scope.bot_token,
            &legacy.scope.persona_token,
            Some(&[1; 16]),
        );
        legacy.origin_digest = legacy.digest_v1();
        self.scope_digest == wire::scope_digest(&self.scope)
            && self.origin_digest == self.digest_v1()
            && legacy.validate_v1()
    }
}

/// Closed request-local evidence proposal for semantic perception preview.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerceptionProposalV1 {
    pub schema_version: u16,
    /// Opaque identifier of the Store-attested inbound origin.  The proposal
    /// deliberately does not carry caller-selected scope, authority, time or
    /// model fields; the Store reloads those fields from its challenge row.
    #[serde(with = "crate::hex::d32")]
    pub origin_digest: Digest,
    pub dimensions: EvidenceVector,
    pub estimator_confidence: Fixed,
    pub protocol_version: u16,
    #[serde(with = "crate::hex::d32")]
    pub request_nonce_digest: Digest,
}

/// Store-minted, one-event authorization context for an external semantic
/// estimator.  The challenge commits the complete session scope, canonical
/// journal base and currently active Genesis incarnation.  It contains no
/// evidence and grants no authority for another event, turn or scope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerceptionChallengeV1 {
    pub schema_version: u16,
    pub origin: PerceptionOriginCommitmentV1,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    #[serde(with = "crate::hex::d32")]
    pub request_nonce_digest: Digest,
}

impl PerceptionChallengeV1 {
    pub const SCHEMA_VERSION: u16 = 1;
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
        if self.origin_digest.iter().all(|byte| *byte == 0) {
            return Err(PerceptionProposalErrorV1::InvalidIdentity);
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

    /// Commit the estimator output to its exact Store-attested request
    /// context. The incarnation is explicit even though the one-time nonce is
    /// also incarnation-bound; this prevents a future nonce-domain migration
    /// from silently weakening estimator identity.
    pub fn estimator_digest_v1(&self) -> Digest {
        let schema_version = self.schema_version.to_le_bytes();
        let values = perception_dimension_values(&self.dimensions).map(Fixed::encode);
        let confidence = self.estimator_confidence.encode();
        let protocol_version = self.protocol_version.to_le_bytes();
        let mut fields: Vec<&[u8]> = Vec::with_capacity(20);
        fields.push(&schema_version);
        fields.extend(values.iter().map(|value| value.as_slice()));
        fields.push(&confidence);
        fields.push(&protocol_version);
        fields.push(&self.request_nonce_digest);
        fields.push(&self.origin_digest);
        wire::domain_hash(Self::DIGEST_DOMAIN_V1, &fields)
    }
}

/// The only affect state exposed to AstrBot's ordinary reply model.
///
/// Neural identities, formula digests, evidence, graph structure and history
/// remain private to Native.  The reply lane receives only a bounded current
/// projection and cannot use it as mutation authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyAffectV1 {
    pub semantic_revision: u64,
    pub personality_revision: u64,
    pub confidence_fxp6: u32,
    pub region_mean_fxp6: [i64; AFFECT_REGION_COUNT_V1],
    pub region_delta_fxp6: [i64; AFFECT_REGION_COUNT_V1],
    pub trend: AffectTrendV1,
}

impl ReplyAffectV1 {
    pub fn validate_v1(&self) -> bool {
        let Some(delta_sum) = self
            .region_delta_fxp6
            .iter()
            .try_fold(0_i128, |sum, value| sum.checked_add(i128::from(*value)))
        else {
            return false;
        };
        let expected_trend = match delta_sum.cmp(&0) {
            std::cmp::Ordering::Less => AffectTrendV1::Falling,
            std::cmp::Ordering::Equal => AffectTrendV1::Stable,
            std::cmp::Ordering::Greater => AffectTrendV1::Rising,
        };
        self.semantic_revision > 0
            && self.personality_revision > 0
            && self.confidence_fxp6 <= AFFECT_FXP6_SCALE_V1 as u32
            && self
                .region_mean_fxp6
                .iter()
                .all(|value| (0..=AFFECT_FXP6_SCALE_V1).contains(value))
            && self
                .region_delta_fxp6
                .iter()
                .all(|value| (-AFFECT_FXP6_SCALE_V1..=AFFECT_FXP6_SCALE_V1).contains(value))
            && self.trend == expected_trend
    }
}

pub const SEMANTIC_APPRAISAL_DAILY_TOKEN_MAX_V1: u32 = 1_000_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticAppraisalBeginRequestV1 {
    pub schema_version: u16,
    pub interaction: InteractionFactBatchV1,
    pub daily_token_limit: u32,
    pub reserved_tokens: u32,
    #[serde(with = "crate::hex::d32")]
    pub provider_digest: Digest,
}

impl SemanticAppraisalBeginRequestV1 {
    pub const SCHEMA_VERSION: u16 = 1;

    pub fn validate_v1(&self) -> bool {
        self.schema_version == Self::SCHEMA_VERSION
            && self.daily_token_limit <= SEMANTIC_APPRAISAL_DAILY_TOKEN_MAX_V1
            && self.reserved_tokens >= 768
            && self.reserved_tokens <= SEMANTIC_APPRAISAL_DAILY_TOKEN_MAX_V1
            && self.provider_digest.iter().any(|byte| *byte != 0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticAppraisalBeginStatusV1 {
    Claimed,
    BudgetExhausted,
    CapacityDeferred,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticAppraisalCapacityReasonV1 {
    RetentionCapacityUnavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticAppraisalBudgetReceiptV1 {
    pub utc_day: u64,
    pub daily_token_limit: u32,
    pub reserved_tokens: u32,
    pub charged_tokens: u64,
    pub remaining_tokens: u64,
    pub blocked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticAppraisalBeginResultV1 {
    pub status: SemanticAppraisalBeginStatusV1,
    pub interaction: ApplyInteractionResultV1,
    /// Independent terminal handle for a claim whose challenge projection is
    /// rejected by the Host.  Native still authenticates and consumes this
    /// nonce; the Host cannot mint or substitute it.
    #[serde(with = "crate::hex::d32_opt")]
    pub settlement_nonce_digest: Option<Digest>,
    pub challenge: Option<PerceptionChallengeV1>,
    pub capacity_reason: Option<SemanticAppraisalCapacityReasonV1>,
    pub budget: Option<SemanticAppraisalBudgetReceiptV1>,
    pub reply_affect: Option<ReplyAffectV1>,
}

impl SemanticAppraisalBeginResultV1 {
    pub fn validate_v1(&self) -> bool {
        match self.status {
            SemanticAppraisalBeginStatusV1::Claimed => {
                self.settlement_nonce_digest.is_some()
                    && self.challenge.is_some()
                    && self.budget.is_some()
                    && self.capacity_reason.is_none()
            }
            SemanticAppraisalBeginStatusV1::BudgetExhausted => {
                self.settlement_nonce_digest.is_none()
                    && self.challenge.is_none()
                    && self.budget.is_some()
                    && self.capacity_reason.is_none()
            }
            SemanticAppraisalBeginStatusV1::CapacityDeferred => {
                self.settlement_nonce_digest.is_none()
                    && self.challenge.is_none()
                    && self.capacity_reason
                        == Some(SemanticAppraisalCapacityReasonV1::RetentionCapacityUnavailable)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticAppraisalOutcomeV1 {
    Success,
    ProviderError,
    Timeout,
    Malformed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticAppraisalProviderUsageV1 {
    pub known: bool,
    pub used_tokens: Option<u32>,
}

impl SemanticAppraisalProviderUsageV1 {
    pub fn validate_v1(&self) -> bool {
        matches!(
            (self.known, self.used_tokens),
            (true, Some(_)) | (false, None)
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticAppraisalSettleRequestV1 {
    pub schema_version: u16,
    pub scope: ScopeRef,
    #[serde(with = "crate::hex::d32")]
    pub request_nonce_digest: Digest,
    pub outcome: SemanticAppraisalOutcomeV1,
    pub provider_usage: SemanticAppraisalProviderUsageV1,
    pub proposal: Option<PerceptionProposalV1>,
}

impl SemanticAppraisalSettleRequestV1 {
    pub const SCHEMA_VERSION: u16 = 1;

    pub fn validate_v1(&self) -> bool {
        self.schema_version == Self::SCHEMA_VERSION
            && self.request_nonce_digest.iter().any(|byte| *byte != 0)
            && self.provider_usage.validate_v1()
            && matches!(
                (&self.outcome, &self.proposal),
                (SemanticAppraisalOutcomeV1::Success, Some(_))
                    | (SemanticAppraisalOutcomeV1::ProviderError, None)
                    | (SemanticAppraisalOutcomeV1::Timeout, None)
                    | (SemanticAppraisalOutcomeV1::Malformed, None)
            )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticAppraisalSettleStatusV1 {
    Committed,
    ZeroMutation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticAppraisalSettleResultV1 {
    pub status: SemanticAppraisalSettleStatusV1,
    pub canonical_revision: u64,
    pub semantic_revision: Option<u64>,
    pub charged_tokens: u64,
    pub contract: Option<ActionContract>,
    pub reply_affect: Option<ReplyAffectV1>,
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

/// Reconstruct the fixed, named fifteen-dimensional JSON layout without
/// allowing a caller to choose a positional interpretation.
pub fn evidence_vector_from_values(values: [Fixed; 15]) -> EvidenceVector {
    EvidenceVector {
        positive: values[0],
        affiliation: values[1],
        harm: values[2],
        boundary: values[3],
        repair: values[4],
        repetition: values[5],
        new_information: values[6],
        constraint_instability: values[7],
        epistemic_conflict: values[8],
        self_responsibility: values[9],
        other_responsibility: values[10],
        hostility: values[11],
        publicness: values[12],
        engagement: values[13],
        rejection: values[14],
    }
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

pub const NATIVE_TELEMETRY_RECEIPT_SCHEMA_V1: &str = "native-telemetry-receipt.v1";
pub const NATIVE_TELEMETRY_RECEIPT_DOMAIN_V1: &[u8] =
    b"astr-embodiment/native-telemetry-receipt-v1";
pub const LEGACY_CHECKPOINT_DOMAIN_V1: &[u8] = b"astr-embodiment/phase0-learning-checkpoint-v1";
pub const LEGACY_RESERVED_VECTOR_DIGEST_DOMAIN_V1: &[u8] =
    b"astr-embodiment/phase0-compensation-vector-v1";

pub fn legacy_reserved_zero_digest_v1() -> Digest {
    let mut bytes = Vec::with_capacity(9 * 8);
    for _ in 0..9 {
        bytes.extend_from_slice(&Fixed::ZERO.encode());
    }
    wire::domain_hash(LEGACY_RESERVED_VECTOR_DIGEST_DOMAIN_V1, &[&bytes])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NativeTelemetryFormulaV1 {
    #[serde(rename = "phase0-native-propagation-fxp6-v1")]
    Phase0NativePropagationFxp6V1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NativeTelemetryPhaseV1 {
    #[serde(rename = "PREPARE")]
    Prepare,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnergyTelemetryV1 {
    pub reserve_before: Fixed,
    pub reserve_after: Fixed,
    pub recovered: Fixed,
    pub spent: Fixed,
    pub headroom: Fixed,
    pub residual: Fixed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapacityTelemetryV1 {
    pub upper_saturated_nodes: u32,
    pub node_limit: u32,
    pub node_headroom: Fixed,
    pub edge_used: u32,
    pub edge_limit: u32,
    pub edge_headroom: Fixed,
    pub headroom: Fixed,
    pub residual: Fixed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTelemetryReceiptV1 {
    pub schema: String,
    pub formula: NativeTelemetryFormulaV1,
    #[serde(with = "crate::hex::d32")]
    pub formula_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub scope_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub event_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub source_digest: Digest,
    pub base_revision: u64,
    pub next_revision: u64,
    pub phase: NativeTelemetryPhaseV1,
    #[serde(with = "crate::hex::d32")]
    pub state_before: Digest,
    #[serde(with = "crate::hex::d32")]
    pub state_after: Digest,
    #[serde(with = "crate::hex::d32")]
    pub graph_before: Digest,
    #[serde(with = "crate::hex::d32")]
    pub graph_after: Digest,
    #[serde(with = "crate::hex::d32")]
    pub local_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub compensation_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub effective_digest: Digest,
    pub energy: EnergyTelemetryV1,
    pub capacity: CapacityTelemetryV1,
    pub residuals: InvariantResiduals,
    pub residual_health: Fixed,
    pub native_gate: Fixed,
    #[serde(with = "crate::hex::d32")]
    pub checkpoint_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub telemetry_digest: Digest,
}

impl NativeTelemetryReceiptV1 {
    pub fn canonical_bytes_without_digests(&self) -> Vec<u8> {
        fn push_fixed(out: &mut Vec<u8>, value: Fixed) {
            out.extend_from_slice(&value.encode());
        }
        fn push_digest(out: &mut Vec<u8>, value: &Digest) {
            out.extend_from_slice(value);
        }

        let mut out = Vec::with_capacity(32 * 12 + 8 * 20 + 32);
        out.push(match self.formula {
            NativeTelemetryFormulaV1::Phase0NativePropagationFxp6V1 => 1,
        });
        out.push(match self.phase {
            NativeTelemetryPhaseV1::Prepare => 1,
        });
        for digest in [
            &self.formula_digest,
            &self.scope_digest,
            &self.event_digest,
            &self.source_digest,
        ] {
            push_digest(&mut out, digest);
        }
        out.extend_from_slice(&self.base_revision.to_le_bytes());
        out.extend_from_slice(&self.next_revision.to_le_bytes());
        for digest in [
            &self.state_before,
            &self.state_after,
            &self.graph_before,
            &self.graph_after,
            &self.local_digest,
            &self.compensation_digest,
            &self.effective_digest,
        ] {
            push_digest(&mut out, digest);
        }
        for value in [
            self.energy.reserve_before,
            self.energy.reserve_after,
            self.energy.recovered,
            self.energy.spent,
            self.energy.headroom,
            self.energy.residual,
        ] {
            push_fixed(&mut out, value);
        }
        out.extend_from_slice(&self.capacity.upper_saturated_nodes.to_le_bytes());
        out.extend_from_slice(&self.capacity.node_limit.to_le_bytes());
        push_fixed(&mut out, self.capacity.node_headroom);
        out.extend_from_slice(&self.capacity.edge_used.to_le_bytes());
        out.extend_from_slice(&self.capacity.edge_limit.to_le_bytes());
        for value in [
            self.capacity.edge_headroom,
            self.capacity.headroom,
            self.capacity.residual,
            self.residuals.authority,
            self.residuals.continuity,
            self.residuals.energy,
            self.residuals.renormalization,
            self.residuals.capacity,
            self.residual_health,
            self.native_gate,
        ] {
            push_fixed(&mut out, value);
        }
        out
    }

    pub fn telemetry_digest_for_body(&self) -> Digest {
        wire::domain_hash(
            NATIVE_TELEMETRY_RECEIPT_DOMAIN_V1,
            &[&self.canonical_bytes_without_digests()],
        )
    }

    pub fn checkpoint_digest_for_body(&self) -> Digest {
        wire::domain_hash(
            LEGACY_CHECKPOINT_DOMAIN_V1,
            &[&self.canonical_bytes_without_digests()],
        )
    }

    pub fn seal(mut self) -> Self {
        self.checkpoint_digest = self.checkpoint_digest_for_body();
        self.telemetry_digest = self.telemetry_digest_for_body();
        self
    }

    pub fn validate(&self) -> bool {
        let unit = |value: Fixed| (Fixed::ZERO..=Fixed::ONE).contains(&value);
        let nonzero = |digest: &Digest| digest.iter().any(|byte| *byte != 0);
        let headroom = |used: u32, limit: u32| {
            if limit == 0 || used > limit {
                return None;
            }
            let denominator = i128::from(limit);
            let ratio = i128::from(used)
                .checked_mul(i128::from(Fixed::ONE.raw()))?
                .checked_add(denominator / 2)?
                / denominator;
            let raw = i128::from(Fixed::ONE.raw()).checked_sub(ratio)?;
            i64::try_from(raw).ok().map(Fixed::from_raw)
        };
        if self.schema != NATIVE_TELEMETRY_RECEIPT_SCHEMA_V1
            || self.formula != NativeTelemetryFormulaV1::Phase0NativePropagationFxp6V1
            || self.phase != NativeTelemetryPhaseV1::Prepare
            || self.base_revision.checked_add(1) != Some(self.next_revision)
            || self.capacity.node_limit == 0
            || self.capacity.edge_limit == 0
            || self.capacity.upper_saturated_nodes > self.capacity.node_limit
            || self.capacity.edge_used > self.capacity.edge_limit
            || self.compensation_digest != legacy_reserved_zero_digest_v1()
            || ![
                &self.formula_digest,
                &self.scope_digest,
                &self.event_digest,
                &self.source_digest,
                &self.state_before,
                &self.state_after,
                &self.graph_before,
                &self.graph_after,
                &self.local_digest,
                &self.compensation_digest,
                &self.effective_digest,
            ]
            .into_iter()
            .all(nonzero)
        {
            return false;
        }
        let values = [
            self.energy.reserve_before,
            self.energy.reserve_after,
            self.energy.recovered,
            self.energy.spent,
            self.energy.headroom,
            self.energy.residual,
            self.capacity.node_headroom,
            self.capacity.edge_headroom,
            self.capacity.headroom,
            self.capacity.residual,
            self.residuals.authority,
            self.residuals.continuity,
            self.residuals.energy,
            self.residuals.renormalization,
            self.residuals.capacity,
            self.residual_health,
            self.native_gate,
        ];
        if !values.into_iter().all(unit)
            || self.energy.headroom != self.energy.reserve_after
            || self.energy.residual != self.residuals.energy
            || self.capacity.residual != self.residuals.capacity
            || self.capacity.residual != Fixed::ZERO
            || headroom(
                self.capacity.upper_saturated_nodes,
                self.capacity.node_limit,
            ) != Some(self.capacity.node_headroom)
            || headroom(self.capacity.edge_used, self.capacity.edge_limit)
                != Some(self.capacity.edge_headroom)
            || self.capacity.headroom
                != self.capacity.node_headroom.min(self.capacity.edge_headroom)
        {
            return false;
        }
        let worst_residual = [
            self.residuals.authority,
            self.residuals.continuity,
            self.residuals.energy,
            self.residuals.renormalization,
            self.residuals.capacity,
        ]
        .into_iter()
        .max()
        .unwrap_or(Fixed::ONE);
        self.residual_health == Fixed::ONE.saturating_sub(worst_residual)
            && self.native_gate
                == self
                    .energy
                    .headroom
                    .min(self.capacity.headroom)
                    .min(self.residual_health)
            && self.checkpoint_digest == self.checkpoint_digest_for_body()
            && self.telemetry_digest == self.telemetry_digest_for_body()
    }
}

pub const NODE_OBSERVABILITY_CONTRACT_INFO_SCHEMA_V1: &str =
    "astr-embodiment.node-observability-contract-info.v1";
pub const NODE_OBSERVABILITY_SCHEMA_V2: &str = "astr-embodiment.node-observability.v2";
pub const NODE_OBSERVABILITY_FORMULA_V1: &str = "spc1-node-observability-v1";
pub const NODE_OBSERVABILITY_REGION_LAYOUT_V1: &str = "regions-v1";
pub const NODE_OBSERVABILITY_CONTRACT_ID_V2: &str =
    "astr-embodiment.node-observability-contract.v2";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeObservabilityContractInfoV1 {
    pub schema: String,
    pub contract_id: String,
    pub node_observability_schema: String,
}

impl NodeObservabilityContractInfoV1 {
    pub fn native_v1() -> Self {
        Self {
            schema: NODE_OBSERVABILITY_CONTRACT_INFO_SCHEMA_V1.to_owned(),
            contract_id: NODE_OBSERVABILITY_CONTRACT_ID_V2.to_owned(),
            node_observability_schema: NODE_OBSERVABILITY_SCHEMA_V2.to_owned(),
        }
    }

    pub fn validate(&self) -> bool {
        self.schema == NODE_OBSERVABILITY_CONTRACT_INFO_SCHEMA_V1
            && self.contract_id == NODE_OBSERVABILITY_CONTRACT_ID_V2
            && self.node_observability_schema == NODE_OBSERVABILITY_SCHEMA_V2
    }
}

pub fn node_observability_contract_info_v1() -> NodeObservabilityContractInfoV1 {
    NodeObservabilityContractInfoV1::native_v1()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NodeObservabilityResidualStateV1 {
    NotComputed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeObservabilityResidualsV1 {
    pub state: NodeObservabilityResidualStateV1,
    pub formula: Option<String>,
    pub values_fxp6: Option<[u32; 5]>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeObservabilityCountsV1 {
    pub selected_node_count: u32,
    pub activated_node_count: u32,
    pub changed_node_count: u32,
    pub potential_nonzero_after_count: u32,
    pub excitation_nonzero_after_count: u32,
    pub signal_nonzero_after_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeObservabilityComponentV1 {
    pub before_mean_fxp6: i64,
    pub after_mean_fxp6: i64,
    pub delta_mean_fxp6: i64,
    pub changed_node_count: u32,
    pub nonzero_after_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeObservabilityRegionV1 {
    pub region_id: u8,
    pub region_name: String,
    pub node_capacity: u32,
    pub selected_node_count: u32,
    pub activated_node_count: u32,
    pub changed_node_count: u32,
    pub potential: NodeObservabilityComponentV1,
    pub excitation: NodeObservabilityComponentV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeObservabilityProjectionWireV2 {
    pub schema: String,
    pub contract_id: String,
    pub formula: String,
    pub revision: u64,
    pub field_node_capacity: u32,
    pub region_layout: String,
    pub counts: NodeObservabilityCountsV1,
    pub residuals: NodeObservabilityResidualsV1,
    pub regions: Vec<NodeObservabilityRegionV1>,
}

impl NodeObservabilityProjectionWireV2 {
    pub fn new(
        revision: u64,
        field_node_capacity: u32,
        counts: NodeObservabilityCountsV1,
        residuals: NodeObservabilityResidualsV1,
        regions: Vec<NodeObservabilityRegionV1>,
    ) -> Self {
        Self {
            schema: NODE_OBSERVABILITY_SCHEMA_V2.to_owned(),
            contract_id: NODE_OBSERVABILITY_CONTRACT_ID_V2.to_owned(),
            formula: NODE_OBSERVABILITY_FORMULA_V1.to_owned(),
            revision,
            field_node_capacity,
            region_layout: NODE_OBSERVABILITY_REGION_LAYOUT_V1.to_owned(),
            counts,
            residuals,
            regions,
        }
    }

    pub fn validate(&self) -> bool {
        if self.schema != NODE_OBSERVABILITY_SCHEMA_V2
            || self.formula != NODE_OBSERVABILITY_FORMULA_V1
            || self.contract_id != NODE_OBSERVABILITY_CONTRACT_ID_V2
            || self.region_layout != NODE_OBSERVABILITY_REGION_LAYOUT_V1
            || self.field_node_capacity == 0
            || self.regions.is_empty()
            || self.residuals.state != NodeObservabilityResidualStateV1::NotComputed
            || self.residuals.formula.is_some()
            || self.residuals.values_fxp6.is_some()
        {
            return false;
        }

        let capacity = self.field_node_capacity;
        let counts = &self.counts;
        if [
            counts.selected_node_count,
            counts.activated_node_count,
            counts.changed_node_count,
            counts.potential_nonzero_after_count,
            counts.excitation_nonzero_after_count,
            counts.signal_nonzero_after_count,
        ]
        .into_iter()
        .any(|value| value > capacity)
            || counts.selected_node_count != counts.changed_node_count
            || counts.activated_node_count > counts.changed_node_count
            || counts.signal_nonzero_after_count
                < counts
                    .potential_nonzero_after_count
                    .max(counts.excitation_nonzero_after_count)
        {
            return false;
        }

        let mut selected_total = 0_u64;
        let mut activated_total = 0_u64;
        let mut changed_total = 0_u64;
        let mut potential_nonzero_after_total = 0_u64;
        let mut excitation_nonzero_after_total = 0_u64;
        let mut region_capacity_total = 0_u64;

        for (index, region) in self.regions.iter().enumerate() {
            if u8::try_from(index).ok() != Some(region.region_id)
                || region.region_name.is_empty()
                || self.regions[..index]
                    .iter()
                    .any(|previous| previous.region_name == region.region_name)
                || region.node_capacity == 0
                || [
                    region.selected_node_count,
                    region.activated_node_count,
                    region.changed_node_count,
                    region.potential.changed_node_count,
                    region.potential.nonzero_after_count,
                    region.excitation.changed_node_count,
                    region.excitation.nonzero_after_count,
                ]
                .into_iter()
                .any(|value| value > region.node_capacity)
                || region.selected_node_count != region.changed_node_count
                || region.activated_node_count > region.changed_node_count
                || region.potential.changed_node_count > region.activated_node_count
                || region.excitation.changed_node_count > region.activated_node_count
            {
                return false;
            }

            let Some(next_selected_total) =
                selected_total.checked_add(u64::from(region.selected_node_count))
            else {
                return false;
            };
            let Some(next_activated_total) =
                activated_total.checked_add(u64::from(region.activated_node_count))
            else {
                return false;
            };
            let Some(next_changed_total) =
                changed_total.checked_add(u64::from(region.changed_node_count))
            else {
                return false;
            };
            let Some(next_potential_nonzero_after_total) = potential_nonzero_after_total
                .checked_add(u64::from(region.potential.nonzero_after_count))
            else {
                return false;
            };
            let Some(next_excitation_nonzero_after_total) = excitation_nonzero_after_total
                .checked_add(u64::from(region.excitation.nonzero_after_count))
            else {
                return false;
            };
            let Some(next_region_capacity_total) =
                region_capacity_total.checked_add(u64::from(region.node_capacity))
            else {
                return false;
            };
            selected_total = next_selected_total;
            activated_total = next_activated_total;
            changed_total = next_changed_total;
            potential_nonzero_after_total = next_potential_nonzero_after_total;
            excitation_nonzero_after_total = next_excitation_nonzero_after_total;
            region_capacity_total = next_region_capacity_total;
        }

        region_capacity_total == u64::from(capacity)
            && selected_total == u64::from(counts.selected_node_count)
            && activated_total == u64::from(counts.activated_node_count)
            && changed_total == u64::from(counts.changed_node_count)
            && potential_nonzero_after_total == u64::from(counts.potential_nonzero_after_count)
            && excitation_nonzero_after_total == u64::from(counts.excitation_nonzero_after_count)
    }
}
