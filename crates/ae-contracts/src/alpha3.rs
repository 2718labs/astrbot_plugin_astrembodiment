//! Closed authority contracts introduced by the 1.1.0-alpha3 lived-world runtime.
//!
//! These DTOs intentionally live beside, rather than inside, the frozen v1
//! contracts. Native state uses closed codes, digests and fxp6 values; prose,
//! prompts, URLs and arbitrary JSON do not cross this boundary.

use crate::{
    CausalRef, Digest, DispatchOutcomeV1, ExternalizationOutcomeV1, FrozenTimeInputV1, Id128,
    ScopeRef, SleepStateV1, TimeAdvanceV1, WakeProposalV1,
};
use ae_fixed::Fixed;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

pub const ALPHA3_SCHEMA_VERSION: u16 = 1;
pub const MAX_INTERACTION_FACTS: usize = 16;
pub const MAX_ALPHA3_SOURCE_REFS: usize = 8;
pub const MAX_ALPHA3_CODE_BYTES: usize = 64;
pub const MAX_ALPHA3_NOTABLE_NODES: usize = 8;
pub const MAX_ALPHA3_PUBLIC_RECORDS: usize = 16;
pub const MAX_ALPHA3_ECOSYSTEM_PAYLOAD_BYTES: usize = 16 * 1024;
pub const AFFECT_REGION_COUNT_V1: usize = 9;
pub const AFFECT_FXP6_SCALE_V1: i64 = 1_000_000;
pub const AFFECT_NODE_CAPACITY_V1: u32 = 16_384;
pub const AFFECT_EDGE_CAPACITY_V1: u32 = 524_288;

const INTENTION_SCOPED_LOCATOR_DOMAIN_V1: &[u8] = b"ae.alpha3.intention-scoped-locator.v1";
const EXECUTION_SCOPED_LOCATOR_DOMAIN_V1: &[u8] = b"ae.alpha3.execution-scoped-locator.v1";
const SCOPED_LOCATOR_CODEC_DOMAIN_V1: &[u8] = b"ae.alpha3.scoped-locator.v1";
const SCOPED_LOCATOR_CODEC_VERSION_V1: u8 = 1;
const LEGACY_INTENTION_PUBLIC_REF_DOMAIN_V2: &[u8] = b"ae.alpha3.intention-public-ref.v1";
const LEGACY_EXECUTION_PUBLIC_REF_DOMAIN_V2: &[u8] = b"ae.alpha3.execution-public-ref.v1";
const LEGACY_PUBLIC_REF_CODEC_DOMAIN_V2: &[u8] = b"ae.alpha3.authenticated-public-ref.v1";

mod hex_vec16 {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(values: &[Id128], serializer: S) -> Result<S::Ok, S::Error> {
        values
            .iter()
            .map(crate::hex::encode16)
            .collect::<Vec<_>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Id128>, D::Error> {
        Vec::<String>::deserialize(deserializer)?
            .into_iter()
            .map(|value| crate::hex::decode16(&value).map_err(serde::de::Error::custom))
            .collect()
    }
}

mod hex_vec32 {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(values: &[Digest], serializer: S) -> Result<S::Ok, S::Error> {
        values
            .iter()
            .map(crate::hex::encode32)
            .collect::<Vec<_>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<Digest>, D::Error> {
        Vec::<String>::deserialize(deserializer)?
            .into_iter()
            .map(|value| crate::hex::decode32(&value).map_err(serde::de::Error::custom))
            .collect()
    }
}

macro_rules! closed_enum {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case", deny_unknown_fields)]
        pub enum $name { $($variant),+ }
    };
}

closed_enum!(WorldModeV1 { MixedMainWorld });
closed_enum!(WorldLayerV1 {
    ExternalObserved,
    PersonaNearReal,
    DeclaredFantasy,
});
closed_enum!(LivedActivityClassV1 {
    Sleep,
    PersonalCare,
    Maintenance,
    FocusedProject,
    Learning,
    Leisure,
    SocialAvailability,
    Reflection,
    Transition,
});
closed_enum!(LivedSegmentStateV1 {
    Scheduled,
    Active,
    Completed,
    Superseded,
});
closed_enum!(LivedGoalClassV1 {
    MaintainRoutine,
    AdvanceProject,
    ExploreInterest,
    RestoreCapacity,
    CompleteUserFollowUp,
    ReflectOnTheme,
});
closed_enum!(LivedGoalStateV1 {
    Active,
    Completed,
    Blocked,
    Cancelled,
});
closed_enum!(LivedGoalOriginV1 {
    Routine,
    ExplicitFollowUp,
    AcceptedEcosystem,
    DreamNonFactReview,
});
closed_enum!(LivedNodeImportanceV1 {
    Routine,
    Notable,
    SafetyCritical,
});
closed_enum!(InteractionFactKindV1 {
    InboundObserved,
    FollowUpRequested,
    FollowUpResolved,
    BoundarySet,
    ContactGranted,
    ContactPaused,
    ContactResumed,
    RelationEnded,
    ExplicitOutcomeReported,
});
closed_enum!(InteractionSourceAuthorityV1 {
    ExplicitControl,
    AstrbotMetadata,
    DeterministicRule,
    ModelCandidate,
});
closed_enum!(InteractionValueCodeV1 {
    FollowUp,
    FollowUpResolved,
    NoContactBoundary,
    Grant,
    Pause,
    Resume,
    End,
    OutcomePositive,
    OutcomeNeutral,
    OutcomeNegative,
});
closed_enum!(RelationConsentStateV1 {
    Disabled,
    PendingReconfirmation,
    Granted,
    Paused,
    Ended,
});
closed_enum!(ContactPurposeV1 {
    ScheduledCheckIn,
    ExplicitFollowUp,
    RepairInvitation,
});
closed_enum!(ContactChannelV1 { AstrbotSession });
closed_enum!(DreamResidueStateV1 {
    PendingWakingReview,
    Rejected,
    RetainedNonFact,
    Expired,
});
closed_enum!(DreamImageryCodeV1 {
    Place,
    Journey,
    Conversation,
    Object,
    Weather,
    Light,
    Threshold,
    Motion,
});
closed_enum!(ReflectionThemeCodeV1 {
    Rest,
    Continuity,
    Change,
    Connection,
    Boundary,
    Curiosity,
    Completion,
    Uncertainty,
});
closed_enum!(DreamReviewActionV1 {
    Reject,
    RetainNonFact,
    Expire,
});
closed_enum!(EcosystemCapabilityV1 {
    ObserveLivedState,
    ProposeExternalObservation,
    ProposeActivity,
    ProposeGoalProgress,
});
closed_enum!(EcosystemProposalKindV1 {
    ExternalObservation,
    ActivityOffer,
    GoalProgressEvidence,
});
closed_enum!(EcosystemDecisionV1 {
    Accepted,
    Rejected,
    Deferred,
});
closed_enum!(CapabilityGrantStateV1 {
    Active,
    Revoked,
    Expired,
});
closed_enum!(ExternalObservationClassV1 {
    Weather,
    Calendar,
    Home,
    Work,
});
closed_enum!(ExternalObservationValueV1 {
    Clear,
    Cloudy,
    Rain,
    Snow,
    EventUpcoming,
    EventStarted,
    EventEnded,
    HomeOccupied,
    HomeQuiet,
    WorkAvailable,
    WorkBusy,
});
closed_enum!(MeasurementUnitV1 {
    Celsius,
    Percent,
    Millimeters,
    Minutes,
    Count,
});
closed_enum!(GateDecisionKindV2 {
    Allowed,
    Suppressed,
    Deferred,
});
closed_enum!(EffectiveFrequencyBandV1 {
    Restrained,
    Balanced,
    Moderate,
    Custom,
});
closed_enum!(FrequencySelectionReasonV1 {
    FixedPolicy,
    Unanswered,
    InactiveRelation,
    BudgetCeiling,
    RecentReply,
    ActiveRelation,
    ConservativeDefault,
});
closed_enum!(GateReasonV2 {
    AllRequirementsSatisfied,
    ProactiveDisabled,
    ConsentRequired,
    ConsentPaused,
    RelationEnded,
    CauseUnavailable,
    BudgetUnavailable,
    ProviderUsageUnsettled,
    EcosystemSourceRevoked,
    PersonaAsleep,
    QuietHours,
    Cooldown,
    DailyLimit,
    UnansweredHardStop,
    TimezoneUnreliable,
    TargetUnavailable,
    ProviderUnavailable,
    SendCapabilityUnavailable,
    ResidualRejected,
});
closed_enum!(BodyWakeStateV1 { Awake, Sleeping });
closed_enum!(BodyBandV1 { Low, Nominal, High });
closed_enum!(ExecutionCapacityV1 {
    Unavailable,
    Reduced,
    Available,
});
closed_enum!(ExecutionReceiptOutcomeV1 {
    Denied,
    Deferred,
    ProviderFailed,
    CandidateRejected,
    AdapterSubmitted,
    PlatformAccepted,
    Delivered,
    DispatchUnknown,
});
closed_enum!(GatePhaseV1 {
    Externalization,
    Dispatch,
});
closed_enum!(HostAttestationRequirementV1 {
    ServiceInstanceProof,
    CallerIdentityProof,
    InstallationManifestBinding,
    BoundedCall,
    LifecycleRevocation,
});
closed_enum!(ProjectionUnavailableV1 {
    UnavailableOnHost,
    NotInitialized,
    Redacted,
    Inconsistent,
});
closed_enum!(ReadinessItemKindV1 {
    GlobalSwitch,
    RelationConsent,
    TrustedTimezone,
    TargetEnvelope,
    SecretStore,
    Provider,
    AstrbotSend,
    Budget,
    SleepQuietHours,
    PolicyRevision,
});
closed_enum!(ReadinessItemStatusV1 {
    Ready,
    UnavailableOnHost,
    NotInitialized,
    Stale,
    Revoked,
    Blocked,
});
closed_enum!(ObserveLayerV2 {
    Experience,
    Private,
    Developer,
});
closed_enum!(ContactExplanationCodeV1 {
    ExplicitFollowUpDue,
    ScheduledCheckInDue,
    RepairInvitationDue,
    NoContactDue,
    Unavailable,
});
closed_enum!(DisclosureStatusV1 {
    ComputedPersonaWorld,
    HiddenByUser,
});
closed_enum!(PrivacyControlStateV1 {
    Available,
    Pending,
    Completed,
    UnavailableOnHost,
});
closed_enum!(ProjectionHealthV1 {
    Healthy,
    Incomplete,
    Inconsistent,
});
closed_enum!(AffectTrendV1 {
    Falling,
    Stable,
    Rising,
});
closed_enum!(PublicIntentionStateV2 {
    Pending,
    Ready,
    Deferred,
    Suppressed,
    Expired,
    DispatchPending,
    Settled,
});
closed_enum!(PublicOutboundStateV2 {
    CandidateReady,
    AdapterCallStarted,
    AdapterSubmitted,
    PlatformAccepted,
    DeliveryConfirmed,
    DispatchUnknown,
    Rejected,
});
closed_enum!(PublicClaimKindV2 {
    Wake,
    Externalization,
    Dispatch,
});
closed_enum!(PublicClaimStateV2 {
    Active,
    Settled,
    Expired,
});
closed_enum!(InnerActivityModeV1 {
    Economy,
    Balanced,
    Rich,
});
closed_enum!(ConsentControlActionV1 {
    Grant,
    Pause,
    Resume,
    End,
});

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum Alpha3ContractError {
    #[error("alpha3 schema version is unsupported")]
    SchemaUnsupported,
    #[error("interaction fact batch must not be empty")]
    EmptyInteractionFacts,
    #[error("interaction fact batch exceeds sixteen facts")]
    TooManyInteractionFacts,
    #[error("alpha3 source vector exceeds eight references")]
    TooManySourceRefs,
    #[error("alpha3 vector exceeds its closed vocabulary")]
    VectorBound,
    #[error("alpha3 vector contains duplicate closed values")]
    DuplicateValue,
    #[error("alpha3 fixed-point value is outside its closed range")]
    FixedOutOfRange,
    #[error("interaction fact field combination is invalid")]
    InvalidInteractionFact,
    #[error("closed payload kind and variant disagree")]
    PayloadKindMismatch,
    #[error("alpha3 time range is invalid")]
    InvalidTimeRange,
    #[error("world anchor is not the immutable alpha3 shape")]
    InvalidWorldAnchor,
    #[error("alpha3 code exceeds sixty-four UTF-8 bytes")]
    CodeTooLong,
    #[error("provider usage presence and value disagree")]
    InvalidProviderUsage,
    #[error("dream residue must remain non-fact")]
    DreamMustBeNonFact,
    #[error("alpha3 scoped locator checksum failed")]
    ScopedLocatorChecksumFailed,
    #[error("effective proactive frequency is outside the closed policy")]
    InvalidEffectiveFrequency,
    #[error("affect projection is outside the closed bounded shape")]
    InvalidAffectProjection,
}

fn scoped_locator_checksum(
    relation_scope: &Digest,
    record_domain: &[u8],
    record_id: &Id128,
) -> Digest {
    let version = [SCOPED_LOCATOR_CODEC_VERSION_V1];
    let mut hasher = blake3::Hasher::new();
    for field in [
        SCOPED_LOCATOR_CODEC_DOMAIN_V1,
        version.as_slice(),
        record_domain,
        relation_scope.as_slice(),
        record_id.as_slice(),
    ] {
        hasher.update(&(field.len() as u64).to_le_bytes());
        hasher.update(field);
    }
    *hasher.finalize().as_bytes()
}

fn encode_scoped_locator(
    relation_scope: &Digest,
    record_domain: &[u8],
    record_id: &Id128,
) -> Digest {
    let mut locator = [0; 32];
    locator[..record_id.len()].copy_from_slice(record_id);
    let checksum = scoped_locator_checksum(relation_scope, record_domain, record_id);
    locator[record_id.len()..].copy_from_slice(&checksum[..record_id.len()]);
    locator
}

fn decode_scoped_locator(
    relation_scope: &Digest,
    record_domain: &[u8],
    locator: &Digest,
) -> Result<Id128, Alpha3ContractError> {
    let record_id: Id128 = locator[..16]
        .try_into()
        .expect("fixed scoped locator prefix");
    let expected_checksum = scoped_locator_checksum(relation_scope, record_domain, &record_id);
    let mut checksum_difference = 0_u8;
    for index in 0..16 {
        checksum_difference |= expected_checksum[index] ^ locator[index + 16];
    }
    if checksum_difference != 0 {
        return Err(Alpha3ContractError::ScopedLocatorChecksumFailed);
    }
    Ok(record_id)
}

pub fn encode_intention_scoped_locator_v1(relation_scope: &Digest, intention_id: &Id128) -> Digest {
    encode_scoped_locator(
        relation_scope,
        INTENTION_SCOPED_LOCATOR_DOMAIN_V1,
        intention_id,
    )
}

pub fn decode_intention_scoped_locator_v1(
    relation_scope: &Digest,
    locator: &Digest,
) -> Result<Id128, Alpha3ContractError> {
    decode_scoped_locator(relation_scope, INTENTION_SCOPED_LOCATOR_DOMAIN_V1, locator)
}

pub fn encode_execution_scoped_locator_v1(relation_scope: &Digest, outbound_id: &Id128) -> Digest {
    encode_scoped_locator(
        relation_scope,
        EXECUTION_SCOPED_LOCATOR_DOMAIN_V1,
        outbound_id,
    )
}

pub fn decode_execution_scoped_locator_v1(
    relation_scope: &Digest,
    locator: &Digest,
) -> Result<Id128, Alpha3ContractError> {
    decode_scoped_locator(relation_scope, EXECUTION_SCOPED_LOCATOR_DOMAIN_V1, locator)
}

fn legacy_public_ref_hash_v2(
    relation_scope: &Digest,
    purpose: &[u8],
    record_domain: &[u8],
    payload: &[u8],
) -> Digest {
    let mut hasher = blake3::Hasher::new_keyed(relation_scope);
    for field in [
        LEGACY_PUBLIC_REF_CODEC_DOMAIN_V2,
        purpose,
        record_domain,
        payload,
    ] {
        hasher.update(&(field.len() as u64).to_le_bytes());
        hasher.update(field);
    }
    *hasher.finalize().as_bytes()
}

fn legacy_public_ref_matches_v2(
    relation_scope: &Digest,
    record_domain: &[u8],
    record_id: &Id128,
    candidate: &Digest,
) -> bool {
    let mask = legacy_public_ref_hash_v2(relation_scope, b"mask", record_domain, &[]);
    let mut expected = [0; 32];
    for index in 0..record_id.len() {
        expected[index] = record_id[index] ^ mask[index];
    }
    let tag = legacy_public_ref_hash_v2(
        relation_scope,
        b"auth",
        record_domain,
        &expected[..record_id.len()],
    );
    expected[record_id.len()..].copy_from_slice(&tag[..record_id.len()]);
    expected
        .iter()
        .zip(candidate)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[doc(hidden)]
pub fn legacy_v2_intention_public_ref_matches(
    relation_scope: &Digest,
    intention_id: &Id128,
    candidate: &Digest,
) -> bool {
    legacy_public_ref_matches_v2(
        relation_scope,
        LEGACY_INTENTION_PUBLIC_REF_DOMAIN_V2,
        intention_id,
        candidate,
    )
}

#[doc(hidden)]
pub fn legacy_v2_execution_public_ref_matches(
    relation_scope: &Digest,
    outbound_id: &Id128,
    candidate: &Digest,
) -> bool {
    legacy_public_ref_matches_v2(
        relation_scope,
        LEGACY_EXECUTION_PUBLIC_REF_DOMAIN_V2,
        outbound_id,
        candidate,
    )
}

pub fn validate_source_ref_count<T>(source_refs: &[T]) -> Result<(), Alpha3ContractError> {
    if source_refs.len() > MAX_ALPHA3_SOURCE_REFS {
        return Err(Alpha3ContractError::TooManySourceRefs);
    }
    Ok(())
}

fn ensure_schema(schema_version: u16) -> Result<(), Alpha3ContractError> {
    if schema_version != ALPHA3_SCHEMA_VERSION {
        return Err(Alpha3ContractError::SchemaUnsupported);
    }
    Ok(())
}

fn ensure_fxp01(value: Fixed) -> Result<(), Alpha3ContractError> {
    if !(Fixed::ZERO..=Fixed::ONE).contains(&value) {
        return Err(Alpha3ContractError::FixedOutOfRange);
    }
    Ok(())
}

fn ensure_unique<T: Ord + Copy>(values: &[T]) -> Result<(), Alpha3ContractError> {
    let unique = values.iter().copied().collect::<BTreeSet<_>>();
    if unique.len() != values.len() {
        return Err(Alpha3ContractError::DuplicateValue);
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteractionFactBatchV1 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d16")]
    pub event_id: Id128,
    pub scope: ScopeRef,
    pub causal: CausalRef,
    pub facts: Vec<InteractionFactV1>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteractionFactV1 {
    #[serde(with = "crate::hex::d16")]
    pub fact_id: Id128,
    pub kind: InteractionFactKindV1,
    pub observed_at_utc_ms: u64,
    pub source_authority: InteractionSourceAuthorityV1,
    #[serde(with = "crate::hex::d32")]
    pub source_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub extractor_digest: Digest,
    pub confidence: Fixed,
    pub value_code: Option<InteractionValueCodeV1>,
    #[serde(with = "crate::hex::d32_opt")]
    pub subject_public_ref: Option<Digest>,
    pub consent_terms: Option<ConsentTermsV1>,
    pub scheduled_at_utc_ms: Option<u64>,
    pub expires_at_utc_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentTermsV1 {
    pub purposes: Vec<ContactPurposeV1>,
    pub channels: Vec<ContactChannelV1>,
    pub valid_until_utc_ms: Option<u64>,
    pub pause_until_utc_ms: Option<u64>,
}

pub fn validate_interaction_fact_batch(
    batch: &InteractionFactBatchV1,
) -> Result<(), Alpha3ContractError> {
    ensure_schema(batch.schema_version)?;
    if batch.facts.is_empty() {
        return Err(Alpha3ContractError::EmptyInteractionFacts);
    }
    if batch.facts.len() > MAX_INTERACTION_FACTS {
        return Err(Alpha3ContractError::TooManyInteractionFacts);
    }
    let mut fact_ids = BTreeSet::new();
    for fact in &batch.facts {
        if !fact_ids.insert(fact.fact_id) {
            return Err(Alpha3ContractError::DuplicateValue);
        }
        validate_interaction_fact(fact)?;
    }
    Ok(())
}

pub fn validate_interaction_fact(fact: &InteractionFactV1) -> Result<(), Alpha3ContractError> {
    ensure_fxp01(fact.confidence)?;
    if fact
        .expires_at_utc_ms
        .is_some_and(|expires| expires < fact.observed_at_utc_ms)
        || matches!(
            (fact.scheduled_at_utc_ms, fact.expires_at_utc_ms),
            (Some(scheduled), Some(expires)) if expires < scheduled
        )
    {
        return Err(Alpha3ContractError::InvalidTimeRange);
    }
    if let Some(terms) = &fact.consent_terms {
        if terms.purposes.is_empty()
            || terms.purposes.len() > 3
            || terms.channels.is_empty()
            || terms.channels.len() > 1
        {
            return Err(Alpha3ContractError::VectorBound);
        }
        ensure_unique(&terms.purposes)?;
        ensure_unique(&terms.channels)?;
    }
    let value_matches = match fact.kind {
        InteractionFactKindV1::InboundObserved => fact.value_code.is_none(),
        InteractionFactKindV1::FollowUpRequested => {
            fact.value_code == Some(InteractionValueCodeV1::FollowUp)
        }
        InteractionFactKindV1::FollowUpResolved => {
            fact.value_code == Some(InteractionValueCodeV1::FollowUpResolved)
        }
        InteractionFactKindV1::BoundarySet => {
            fact.value_code == Some(InteractionValueCodeV1::NoContactBoundary)
        }
        InteractionFactKindV1::ContactGranted => {
            fact.value_code == Some(InteractionValueCodeV1::Grant)
        }
        InteractionFactKindV1::ContactPaused => {
            fact.value_code == Some(InteractionValueCodeV1::Pause)
        }
        InteractionFactKindV1::ContactResumed => {
            fact.value_code == Some(InteractionValueCodeV1::Resume)
        }
        InteractionFactKindV1::RelationEnded => {
            fact.value_code == Some(InteractionValueCodeV1::End)
        }
        InteractionFactKindV1::ExplicitOutcomeReported => matches!(
            fact.value_code,
            Some(
                InteractionValueCodeV1::OutcomePositive
                    | InteractionValueCodeV1::OutcomeNeutral
                    | InteractionValueCodeV1::OutcomeNegative
            )
        ),
    };
    let grant_terms_match =
        matches!(fact.kind, InteractionFactKindV1::ContactGranted) == fact.consent_terms.is_some();
    let follow_up_timing = matches!(fact.kind, InteractionFactKindV1::FollowUpRequested)
        || (fact.scheduled_at_utc_ms.is_none() && fact.expires_at_utc_ms.is_none());
    let model_candidate_safe = fact.source_authority
        != InteractionSourceAuthorityV1::ModelCandidate
        || (fact.kind == InteractionFactKindV1::InboundObserved
            && fact.value_code.is_none()
            && fact.consent_terms.is_none()
            && fact.scheduled_at_utc_ms.is_none()
            && fact.expires_at_utc_ms.is_none());
    let authority_matches = match fact.kind {
        InteractionFactKindV1::InboundObserved => true,
        InteractionFactKindV1::FollowUpResolved => matches!(
            fact.source_authority,
            InteractionSourceAuthorityV1::ExplicitControl
                | InteractionSourceAuthorityV1::DeterministicRule
        ),
        _ => fact.source_authority == InteractionSourceAuthorityV1::ExplicitControl,
    };
    if !value_matches
        || !grant_terms_match
        || !follow_up_timing
        || !model_candidate_safe
        || !authority_matches
    {
        return Err(Alpha3ContractError::InvalidInteractionFact);
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldAnchorV1 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d16")]
    pub world_anchor_id: Id128,
    #[serde(with = "crate::hex::d32")]
    pub persona_scope: Digest,
    pub mode: WorldModeV1,
    #[serde(with = "crate::hex::d32")]
    pub home_context_ref: Digest,
    #[serde(with = "crate::hex::d32")]
    pub lore_manifest_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub reality_policy_digest: Digest,
    pub allowed_layers: Vec<WorldLayerV1>,
    #[serde(with = "crate::hex::d16")]
    pub created_from_event_id: Id128,
    pub revision: u64,
}

impl WorldAnchorV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        ensure_schema(self.schema_version)?;
        let layers = self.allowed_layers.iter().copied().collect::<BTreeSet<_>>();
        if self.mode != WorldModeV1::MixedMainWorld
            || self.revision != 1
            || layers
                != BTreeSet::from([
                    WorldLayerV1::ExternalObserved,
                    WorldLayerV1::PersonaNearReal,
                    WorldLayerV1::DeclaredFantasy,
                ])
            || self.allowed_layers.len() != 3
        {
            return Err(Alpha3ContractError::InvalidWorldAnchor);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LivedActivitySegmentV1 {
    #[serde(with = "crate::hex::d16")]
    pub segment_id: Id128,
    pub world_layer: WorldLayerV1,
    pub activity_class: LivedActivityClassV1,
    pub starts_at_utc_ms: u64,
    pub ends_at_utc_ms: u64,
    pub state: LivedSegmentStateV1,
    pub importance: LivedNodeImportanceV1,
    #[serde(with = "hex_vec16")]
    pub source_event_ids: Vec<Id128>,
}

impl LivedActivitySegmentV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        validate_source_ref_count(&self.source_event_ids)?;
        if self.ends_at_utc_ms <= self.starts_at_utc_ms {
            return Err(Alpha3ContractError::InvalidTimeRange);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LivedGoalV1 {
    #[serde(with = "crate::hex::d16")]
    pub goal_id: Id128,
    pub goal_class: LivedGoalClassV1,
    pub state: LivedGoalStateV1,
    pub progress: Fixed,
    pub due_at_utc_ms: Option<u64>,
    pub world_layer: WorldLayerV1,
    pub origin: LivedGoalOriginV1,
    #[serde(with = "hex_vec16")]
    pub source_event_ids: Vec<Id128>,
}

impl LivedGoalV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        validate_source_ref_count(&self.source_event_ids)?;
        ensure_fxp01(self.progress)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LivedDayStateV1 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d32")]
    pub persona_scope: Digest,
    #[serde(with = "crate::hex::d16")]
    pub world_anchor_id: Id128,
    pub persona_day_ordinal: i32,
    pub revision: u64,
    pub current_segment: LivedActivitySegmentV1,
    pub active_goal: Option<LivedGoalV1>,
    pub next_transition_utc_ms: u64,
    #[serde(with = "crate::hex::d32")]
    pub routine_formula_digest: Digest,
    #[serde(with = "hex_vec16")]
    pub source_event_ids: Vec<Id128>,
}

impl LivedDayStateV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        ensure_schema(self.schema_version)?;
        validate_source_ref_count(&self.source_event_ids)?;
        self.current_segment.validate()?;
        if let Some(goal) = &self.active_goal {
            goal.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationConsentV1 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d32")]
    pub relation_scope: Digest,
    pub consent_epoch: u64,
    pub revision: u64,
    pub state: RelationConsentStateV1,
    pub purposes: Vec<ContactPurposeV1>,
    pub channels: Vec<ContactChannelV1>,
    pub valid_from_utc_ms: u64,
    pub valid_until_utc_ms: Option<u64>,
    pub pause_until_utc_ms: Option<u64>,
    #[serde(with = "crate::hex::d16")]
    pub source_event_id: Id128,
    #[serde(with = "crate::hex::d32")]
    pub policy_digest: Digest,
}

impl RelationConsentV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        ensure_schema(self.schema_version)?;
        if self.purposes.len() > 3 || self.channels.len() > 1 {
            return Err(Alpha3ContractError::VectorBound);
        }
        ensure_unique(&self.purposes)?;
        ensure_unique(&self.channels)?;
        if self
            .valid_until_utc_ms
            .is_some_and(|until| until < self.valid_from_utc_ms)
        {
            return Err(Alpha3ContractError::InvalidTimeRange);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationContactProcessV1 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d32")]
    pub relation_scope: Digest,
    pub revision: u64,
    pub last_inbound_utc_ms: Option<u64>,
    pub last_outbound_submitted_utc_ms: Option<u64>,
    pub response_cadence_ema_ms: Option<u64>,
    pub response_cadence_variation: Fixed,
    pub contact_due_score: Fixed,
    pub unfinished_follow_up_salience: Fixed,
    pub repetition_penalty: Fixed,
    pub consecutive_unanswered: u16,
    pub next_contact_eligible_utc_ms: Option<u64>,
    #[serde(with = "crate::hex::d32_opt")]
    pub active_cause_digest: Option<Digest>,
    #[serde(with = "hex_vec16")]
    pub active_source_event_ids: Vec<Id128>,
    #[serde(with = "crate::hex::d32")]
    pub formula_digest: Digest,
}

impl RelationContactProcessV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        ensure_schema(self.schema_version)?;
        validate_source_ref_count(&self.active_source_event_ids)?;
        ensure_fxp01(self.response_cadence_variation)?;
        ensure_fxp01(self.contact_due_score)?;
        ensure_fxp01(self.unfinished_follow_up_salience)?;
        ensure_fxp01(self.repetition_penalty)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DreamAffectAfterglowV1 {
    pub valence: Fixed,
    pub arousal: Fixed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DreamResidueV1 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d16")]
    pub residue_id: Id128,
    #[serde(with = "crate::hex::d32")]
    pub persona_scope: Digest,
    pub state: DreamResidueStateV1,
    pub imagery_tags: Vec<DreamImageryCodeV1>,
    pub affect_afterglow: DreamAffectAfterglowV1,
    pub reflection_theme_codes: Vec<ReflectionThemeCodeV1>,
    #[serde(with = "hex_vec16")]
    pub source_event_ids: Vec<Id128>,
    pub created_at_utc_ms: u64,
    pub expires_at_utc_ms: u64,
    pub reviewed_at_utc_ms: Option<u64>,
    #[serde(with = "crate::hex::d16_opt")]
    pub review_event_id: Option<Id128>,
    pub non_fact: bool,
}

impl DreamResidueV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        ensure_schema(self.schema_version)?;
        validate_source_ref_count(&self.imagery_tags)?;
        validate_source_ref_count(&self.reflection_theme_codes)?;
        validate_source_ref_count(&self.source_event_ids)?;
        ensure_unique(&self.imagery_tags)?;
        ensure_unique(&self.reflection_theme_codes)?;
        if !self.non_fact {
            return Err(Alpha3ContractError::DreamMustBeNonFact);
        }
        if self.affect_afterglow.valence < Fixed::from_raw(-1_000_000)
            || self.affect_afterglow.valence > Fixed::ONE
        {
            return Err(Alpha3ContractError::FixedOutOfRange);
        }
        ensure_fxp01(self.affect_afterglow.arousal)?;
        if self.expires_at_utc_ms < self.created_at_utc_ms {
            return Err(Alpha3ContractError::InvalidTimeRange);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DreamReviewRequestV1 {
    #[serde(with = "crate::hex::d16")]
    pub residue_id: Id128,
    pub action: DreamReviewActionV1,
    pub reviewed_at_utc_ms: u64,
    #[serde(with = "crate::hex::d16")]
    pub review_event_id: Id128,
    pub awake: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EcosystemCapabilityGrantV1 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d32")]
    pub plugin_instance_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub plugin_name_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub plugin_version_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub manifest_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub capability_digest: Digest,
    pub capabilities: Vec<EcosystemCapabilityV1>,
    pub state: CapabilityGrantStateV1,
    pub valid_until_utc_ms: Option<u64>,
    pub revision: u64,
    #[serde(with = "crate::hex::d16")]
    pub source_event_id: Id128,
}

impl EcosystemCapabilityGrantV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        ensure_schema(self.schema_version)?;
        if self.capabilities.is_empty() || self.capabilities.len() > 4 {
            return Err(Alpha3ContractError::VectorBound);
        }
        ensure_unique(&self.capabilities)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EcosystemObservationV1 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d16")]
    pub observation_id: Id128,
    #[serde(with = "crate::hex::d32")]
    pub persona_public_handle: Digest,
    pub as_of_canonical_revision: u64,
    #[serde(with = "crate::hex::d32")]
    pub world_anchor_public_ref: Digest,
    pub current_world_layer: WorldLayerV1,
    pub persona_local_minute: u16,
    pub sleep_state: SleepStateV1,
    pub lived_activity_class: LivedActivityClassV1,
    pub active_goal_class: Option<LivedGoalClassV1>,
    pub allowed_proposal_kinds: Vec<EcosystemProposalKindV1>,
    #[serde(with = "crate::hex::d32")]
    pub capability_digest: Digest,
    pub expires_at_utc_ms: u64,
}

impl EcosystemObservationV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        ensure_schema(self.schema_version)?;
        if self.persona_local_minute >= 1_440
            || self.allowed_proposal_kinds.len() > 3
            || self.allowed_proposal_kinds.is_empty()
        {
            return Err(Alpha3ContractError::VectorBound);
        }
        ensure_unique(&self.allowed_proposal_kinds)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundedMeasurementV1 {
    pub value: Fixed,
    pub unit: MeasurementUnitV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "payload_kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EcosystemProposalPayloadV1 {
    ExternalObservation {
        class: ExternalObservationClassV1,
        value: ExternalObservationValueV1,
        measurement: Option<BoundedMeasurementV1>,
        observed_at_utc_ms: u64,
    },
    ActivityOffer {
        activity_class: LivedActivityClassV1,
        goal_class: Option<LivedGoalClassV1>,
        duration_minutes: u16,
    },
    GoalProgressEvidence {
        #[serde(with = "crate::hex::d32")]
        goal_public_ref: Digest,
        progress_delta: Fixed,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EcosystemProposalV1 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d16")]
    pub proposal_id: Id128,
    #[serde(with = "crate::hex::d32")]
    pub plugin_instance_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub capability_digest: Digest,
    #[serde(with = "crate::hex::d16")]
    pub observation_id: Id128,
    pub expected_canonical_revision: u64,
    pub kind: EcosystemProposalKindV1,
    pub world_layer: WorldLayerV1,
    pub valid_from_utc_ms: u64,
    pub expires_at_utc_ms: u64,
    pub payload: EcosystemProposalPayloadV1,
    #[serde(with = "crate::hex::d32")]
    pub source_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub semantic_idempotency_digest: Digest,
}

impl EcosystemProposalV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        ensure_schema(self.schema_version)?;
        if self.expires_at_utc_ms < self.valid_from_utc_ms {
            return Err(Alpha3ContractError::InvalidTimeRange);
        }
        let shape_matches = matches!(
            (&self.kind, &self.payload),
            (
                EcosystemProposalKindV1::ExternalObservation,
                EcosystemProposalPayloadV1::ExternalObservation { .. }
            ) | (
                EcosystemProposalKindV1::ActivityOffer,
                EcosystemProposalPayloadV1::ActivityOffer { .. }
            ) | (
                EcosystemProposalKindV1::GoalProgressEvidence,
                EcosystemProposalPayloadV1::GoalProgressEvidence { .. }
            )
        );
        if !shape_matches
            || (self.kind == EcosystemProposalKindV1::ExternalObservation
                && self.world_layer != WorldLayerV1::ExternalObserved)
        {
            return Err(Alpha3ContractError::PayloadKindMismatch);
        }
        match &self.payload {
            EcosystemProposalPayloadV1::ActivityOffer {
                duration_minutes, ..
            } if *duration_minutes == 0 => return Err(Alpha3ContractError::InvalidTimeRange),
            EcosystemProposalPayloadV1::GoalProgressEvidence { progress_delta, .. }
                if *progress_delta < Fixed::from_raw(-1_000_000)
                    || *progress_delta > Fixed::ONE =>
            {
                return Err(Alpha3ContractError::FixedOutOfRange);
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EcosystemProposalDecisionV1 {
    #[serde(with = "crate::hex::d16")]
    pub proposal_id: Id128,
    pub decision: EcosystemDecisionV1,
    pub canonical_revision: u64,
    #[serde(with = "crate::hex::d16_opt")]
    pub committed_event_id: Option<Id128>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EcosystemObserveRequestV1 {
    #[serde(with = "crate::hex::d32")]
    pub persona_scope: Digest,
    #[serde(with = "crate::hex::d32")]
    pub plugin_instance_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub capability_digest: Digest,
    pub observed_at_utc_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContactIntentionBasisV1 {
    #[serde(with = "crate::hex::d16")]
    pub intention_id: Id128,
    #[serde(with = "crate::hex::d32")]
    pub relation_scope: Digest,
    pub purpose: ContactPurposeV1,
    #[serde(with = "crate::hex::d32")]
    pub cause_digest: Digest,
    #[serde(with = "hex_vec32")]
    pub cause_public_refs: Vec<Digest>,
    pub consent_epoch: u64,
    pub consent_revision: u64,
    pub created_at_utc_ms: u64,
    pub expires_at_utc_ms: u64,
    pub live: bool,
}

impl ContactIntentionBasisV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        validate_source_ref_count(&self.cause_public_refs)?;
        if self.expires_at_utc_ms < self.created_at_utc_ms {
            return Err(Alpha3ContractError::InvalidTimeRange);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Alpha3WakeProposalV1 {
    pub legacy: WakeProposalV1,
    pub contact_updates: Vec<RelationContactProcessV1>,
    pub contact_bases: Vec<ContactIntentionBasisV1>,
    pub lived_day: Option<LivedDayStateV1>,
    pub world_anchor: Option<WorldAnchorV1>,
    pub dream_updates: Vec<DreamResidueV1>,
}

impl Alpha3WakeProposalV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        if self.lived_day.is_some() || self.world_anchor.is_some() || !self.dream_updates.is_empty()
        {
            return Err(Alpha3ContractError::PayloadKindMismatch);
        }
        if self.contact_updates.len() > MAX_INTERACTION_FACTS
            || self.contact_bases.len() > MAX_INTERACTION_FACTS
        {
            return Err(Alpha3ContractError::VectorBound);
        }
        for contact in &self.contact_updates {
            contact.validate()?;
        }
        for basis in &self.contact_bases {
            basis.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WakeClaimV2 {
    #[serde(with = "crate::hex::d32")]
    pub claim_token: Digest,
    pub event: TimeAdvanceV1,
    pub proposal: Alpha3WakeProposalV1,
    pub lease_deadline_utc_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WakeSettleRequestV2 {
    #[serde(with = "crate::hex::d32")]
    pub claim_token: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationBudgetPolicyV1 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d32")]
    pub relation_scope: Digest,
    pub timezone_id: String,
    pub daily_token_limit: u64,
    pub revision: u64,
    #[serde(with = "crate::hex::d32")]
    pub source_digest: Digest,
}

impl RelationBudgetPolicyV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        ensure_schema(self.schema_version)?;
        if self.timezone_id.is_empty() || self.timezone_id.len() > MAX_ALPHA3_CODE_BYTES {
            return Err(Alpha3ContractError::CodeTooLong);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationBudgetLedgerV1 {
    #[serde(with = "crate::hex::d32")]
    pub relation_scope: Digest,
    pub day_start_utc_ms: u64,
    pub limit_tokens: u64,
    pub reserved_tokens: u64,
    pub charged_tokens: u64,
    pub used_tokens: u64,
    pub usage_known: bool,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderUsageV1 {
    pub known: bool,
    pub used_tokens: Option<u64>,
}

impl ProviderUsageV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        if self.known != self.used_tokens.is_some() {
            return Err(Alpha3ContractError::InvalidProviderUsage);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalizationSettleV2 {
    #[serde(with = "crate::hex::d32")]
    pub claim_token: Digest,
    pub outcome: ExternalizationOutcomeV1,
    pub provider_usage: ProviderUsageV1,
    #[serde(with = "crate::hex::d32_opt")]
    pub candidate_digest: Option<Digest>,
    pub candidate_ciphertext: Option<Vec<u8>>,
    #[serde(with = "crate::hex::d32")]
    pub caller_incarnation: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessItemV1 {
    pub kind: ReadinessItemKindV1,
    pub status: ReadinessItemStatusV1,
    pub witness_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProactiveReadinessV1 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d32")]
    pub relation_scope: Digest,
    pub revision: u64,
    pub evaluated_at_utc_ms: u64,
    pub items: Vec<ReadinessItemV1>,
    #[serde(with = "crate::hex::d32")]
    pub witness_digest: Digest,
}

impl ProactiveReadinessV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        ensure_schema(self.schema_version)?;
        if self.items.len() > 10 {
            return Err(Alpha3ContractError::VectorBound);
        }
        let kinds = self.items.iter().map(|item| item.kind).collect::<Vec<_>>();
        ensure_unique(&kinds)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostReadinessWitnessV1 {
    pub schema_version: u16,
    pub items: Vec<ReadinessItemV1>,
    #[serde(with = "crate::hex::d32")]
    pub witness_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveFrequencyV1 {
    pub band: EffectiveFrequencyBandV1,
    pub daily_max: u16,
    pub cooldown_ms: u64,
    pub selection_reason: FrequencySelectionReasonV1,
    #[serde(with = "crate::hex::d32")]
    pub evidence_digest: Digest,
}

impl EffectiveFrequencyV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        if self.evidence_digest == [0; 32] || self.daily_max == 0 {
            return Err(Alpha3ContractError::InvalidEffectiveFrequency);
        }
        const RESTRAINED_MS: u64 = 6 * 60 * 60 * 1_000;
        const BALANCED_MS: u64 = 4 * 60 * 60 * 1_000;
        const MODERATE_MS: u64 = 3 * 60 * 60 * 1_000;
        let restrained_preset = self.daily_max == 2 && self.cooldown_ms == RESTRAINED_MS;
        let unanswered_backoff = self.daily_max == 2
            && self.cooldown_ms % RESTRAINED_MS == 0
            && (self.cooldown_ms / RESTRAINED_MS).is_power_of_two();
        let balanced_preset = self.daily_max == 3 && self.cooldown_ms == BALANCED_MS;
        let moderate_preset = self.daily_max == 4 && self.cooldown_ms == MODERATE_MS;
        let shape_is_valid = match self.selection_reason {
            FrequencySelectionReasonV1::FixedPolicy => match self.band {
                EffectiveFrequencyBandV1::Restrained => restrained_preset,
                EffectiveFrequencyBandV1::Moderate => moderate_preset,
                EffectiveFrequencyBandV1::Custom => true,
                EffectiveFrequencyBandV1::Balanced => false,
            },
            FrequencySelectionReasonV1::Unanswered => {
                self.band == EffectiveFrequencyBandV1::Restrained && unanswered_backoff
            }
            FrequencySelectionReasonV1::InactiveRelation
            | FrequencySelectionReasonV1::BudgetCeiling
            | FrequencySelectionReasonV1::ConservativeDefault => {
                self.band == EffectiveFrequencyBandV1::Restrained && restrained_preset
            }
            FrequencySelectionReasonV1::RecentReply => {
                self.band == EffectiveFrequencyBandV1::Moderate && moderate_preset
            }
            FrequencySelectionReasonV1::ActiveRelation => {
                self.band == EffectiveFrequencyBandV1::Balanced && balanced_preset
            }
        };
        if !shape_is_valid {
            return Err(Alpha3ContractError::InvalidEffectiveFrequency);
        }
        Ok(())
    }
}

impl HostReadinessWitnessV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        ensure_schema(self.schema_version)?;
        if self.items.len() > 10 {
            return Err(Alpha3ContractError::VectorBound);
        }
        let kinds = self.items.iter().map(|item| item.kind).collect::<Vec<_>>();
        ensure_unique(&kinds)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateDecisionV2 {
    pub decision: GateDecisionKindV2,
    pub reason: GateReasonV2,
    pub evaluated_at_utc_ms: u64,
    pub retry_at_utc_ms: Option<u64>,
    pub consent_epoch: u64,
    pub consent_revision: u64,
    pub policy_revision: u64,
    pub budget_day_start_utc_ms: u64,
    #[serde(with = "crate::hex::d32")]
    pub intention_public_ref: Digest,
    #[serde(with = "hex_vec32")]
    pub cause_public_refs: Vec<Digest>,
    #[serde(with = "crate::hex::d32")]
    pub capability_snapshot_digest: Digest,
    #[serde(default, with = "crate::hex::d32_opt")]
    pub authority_snapshot_digest: Option<Digest>,
    #[serde(default)]
    pub authority_expires_at_utc_ms: Option<u64>,
    #[serde(default)]
    pub authority_budget_expires_at_utc_ms: Option<u64>,
    #[serde(default)]
    pub effective_frequency: Option<EffectiveFrequencyV1>,
}

impl GateDecisionV2 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        validate_source_ref_count(&self.cause_public_refs)?;
        if let Some(frequency) = &self.effective_frequency {
            frequency.validate()?;
        }
        let authority_shape = match (
            self.authority_snapshot_digest,
            self.authority_expires_at_utc_ms,
        ) {
            (Some(digest), Some(expires_at)) if digest != [0; 32] && expires_at != 0 => true,
            (None, None) => true,
            _ => false,
        };
        if !authority_shape
            || self
                .authority_budget_expires_at_utc_ms
                .is_some_and(|expires_at| expires_at <= self.budget_day_start_utc_ms)
        {
            return Err(Alpha3ContractError::InvalidTimeRange);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateAndClaimExternalizationRequestV2 {
    pub scope: ScopeRef,
    #[serde(with = "crate::hex::d16")]
    pub intention_id: Id128,
    pub attempt_no: u8,
    pub expected_revision: u64,
    pub max_tokens: u64,
    #[serde(with = "crate::hex::d32")]
    pub caller_incarnation: Digest,
    pub readiness: HostReadinessWitnessV1,
    pub frozen: FrozenTimeInputV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalizationClaimV2 {
    #[serde(with = "crate::hex::d32")]
    pub claim_token: Digest,
    #[serde(with = "crate::hex::d32")]
    pub intention_public_ref: Digest,
    pub attempt_no: u8,
    pub reserved_tokens: u64,
    pub budget_day_start_utc_ms: u64,
    pub consent_epoch: u64,
    pub consent_revision: u64,
    pub policy_revision: u64,
    #[serde(with = "crate::hex::d32")]
    pub capability_snapshot_digest: Digest,
    pub lease_deadline_utc_ms: u64,
    #[serde(default)]
    pub gate_decision_snapshot: Option<GateDecisionV2>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateAndClaimExternalizationV2 {
    pub decision: GateDecisionV2,
    pub claim: Option<ExternalizationClaimV2>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateAndClaimDispatchRequestV2 {
    pub scope: ScopeRef,
    #[serde(with = "crate::hex::d32")]
    pub outbound_public_ref: Digest,
    #[serde(with = "crate::hex::d32")]
    pub expected_target_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub caller_incarnation: Digest,
    pub readiness: HostReadinessWitnessV1,
    pub frozen: FrozenTimeInputV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchClaimV2 {
    #[serde(with = "crate::hex::d32")]
    pub claim_token: Digest,
    #[serde(with = "crate::hex::d32")]
    pub outbound_public_ref: Digest,
    pub consent_epoch: u64,
    pub consent_revision: u64,
    pub policy_revision: u64,
    #[serde(with = "crate::hex::d32")]
    pub target_binding_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub capability_snapshot_digest: Digest,
    pub lease_deadline_utc_ms: u64,
    #[serde(default)]
    pub gate_decision_snapshot: Option<GateDecisionV2>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateAndClaimDispatchV2 {
    pub decision: GateDecisionV2,
    pub claim: Option<DispatchClaimV2>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchSettleV2 {
    #[serde(with = "crate::hex::d32")]
    pub claim_token: Digest,
    pub outcome: DispatchOutcomeV1,
    pub settled_at_utc_ms: u64,
    #[serde(with = "crate::hex::d32_opt")]
    pub receipt_digest: Option<Digest>,
    #[serde(with = "crate::hex::d32")]
    pub caller_incarnation: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WakeWindowV1 {
    pub earliest_utc_ms: u64,
    pub latest_utc_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveBodySnapshotRequestV1 {
    pub schema_version: u16,
    pub scope: ScopeRef,
    pub observed_at_utc_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbodimentSnapshotV1 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d16")]
    pub persona_token: Id128,
    pub body_revision: u64,
    pub observed_at_utc_ms: u64,
    pub wake_state: BodyWakeStateV1,
    pub sleep_stage_public: SleepStateV1,
    pub energy_band: BodyBandV1,
    pub arousal_band: BodyBandV1,
    pub next_wake_window: Option<WakeWindowV1>,
    pub execution_capacity: ExecutionCapacityV1,
    pub outreach_available: bool,
    pub blocking_reasons: Vec<GateReasonV2>,
    pub expires_at_utc_ms: u64,
    #[serde(with = "crate::hex::d32")]
    pub body_commitment: Digest,
}

impl EmbodimentSnapshotV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        ensure_schema(self.schema_version)?;
        if self.blocking_reasons.len() > MAX_ALPHA3_PUBLIC_RECORDS {
            return Err(Alpha3ContractError::VectorBound);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveExecutionReceiptRequestV1 {
    pub schema_version: u16,
    pub scope: ScopeRef,
    #[serde(with = "crate::hex::d32")]
    pub execution_public_ref: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionReceiptV1 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d32")]
    pub intention_public_ref: Digest,
    #[serde(with = "crate::hex::d32")]
    pub execution_public_ref: Digest,
    pub body_revision: u64,
    pub outcome: ExecutionReceiptOutcomeV1,
    pub reason_codes: Vec<GateReasonV2>,
    pub provider_usage_known: bool,
    pub provider_used_tokens: Option<u64>,
    #[serde(with = "crate::hex::d32_opt")]
    pub adapter_receipt_ref: Option<Digest>,
    pub occurred_at_utc_ms: u64,
    #[serde(with = "crate::hex::d32")]
    pub body_commitment: Digest,
}

impl ExecutionReceiptV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        ensure_schema(self.schema_version)?;
        if self.reason_codes.len() > MAX_ALPHA3_PUBLIC_RECORDS {
            return Err(Alpha3ContractError::VectorBound);
        }
        if self.provider_usage_known != self.provider_used_tokens.is_some() {
            return Err(Alpha3ContractError::InvalidProviderUsage);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveBudgetSummaryRequestV1 {
    pub schema_version: u16,
    pub scope: ScopeRef,
    pub observed_at_utc_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetSummaryV1 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d16")]
    pub relation_token: Id128,
    pub budget_day_start_utc_ms: u64,
    pub limit_tokens: u64,
    pub reserved_tokens: u64,
    pub charged_tokens: u64,
    pub usage_known: bool,
    pub used_tokens: Option<u64>,
    pub remaining_tokens: u64,
    pub expires_at_utc_ms: u64,
    #[serde(with = "crate::hex::d32")]
    pub body_commitment: Digest,
}

impl BudgetSummaryV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        ensure_schema(self.schema_version)?;
        if self.usage_known != self.used_tokens.is_some() {
            return Err(Alpha3ContractError::InvalidProviderUsage);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveGateReasonsRequestV1 {
    pub schema_version: u16,
    pub scope: ScopeRef,
    pub phase: GatePhaseV1,
    pub observed_at_utc_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateReasonsV1 {
    pub schema_version: u16,
    pub body_revision: u64,
    pub phase: GatePhaseV1,
    pub decision: GateDecisionKindV2,
    pub reason: GateReasonV2,
    pub evaluated_at_utc_ms: u64,
    pub retry_at_utc_ms: Option<u64>,
    pub consent_epoch: u64,
    pub consent_revision: u64,
    pub policy_revision: u64,
    pub budget_day_start_utc_ms: u64,
    #[serde(with = "crate::hex::d32")]
    pub capability_snapshot_digest: Digest,
    pub expires_at_utc_ms: u64,
    #[serde(with = "crate::hex::d32")]
    pub body_commitment: Digest,
}

impl GateReasonsV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        ensure_schema(self.schema_version)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntegrationAvailabilityStateV1 {
    #[serde(rename = "UNAVAILABLE_HOST_ATTESTATION")]
    UnavailableHostAttestation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationAvailabilityV1 {
    pub schema_version: u16,
    pub state: IntegrationAvailabilityStateV1,
    pub required_capabilities: Vec<HostAttestationRequirementV1>,
}

pub fn integration_availability_v1() -> IntegrationAvailabilityV1 {
    IntegrationAvailabilityV1 {
        schema_version: ALPHA3_SCHEMA_VERSION,
        state: IntegrationAvailabilityStateV1::UnavailableHostAttestation,
        required_capabilities: vec![
            HostAttestationRequirementV1::ServiceInstanceProof,
            HostAttestationRequirementV1::CallerIdentityProof,
            HostAttestationRequirementV1::InstallationManifestBinding,
            HostAttestationRequirementV1::BoundedCall,
            HostAttestationRequirementV1::LifecycleRevocation,
        ],
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "availability",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ProjectionFieldV1<T> {
    Available(T),
    UnavailableOnHost,
    NotInitialized,
    Redacted,
    Inconsistent,
}

impl<T> Default for ProjectionFieldV1<T> {
    fn default() -> Self {
        Self::NotInitialized
    }
}

impl<T> ProjectionFieldV1<T> {
    pub fn value(&self) -> &T {
        match self {
            Self::Available(value) => value,
            _ => panic!("projection field is not available"),
        }
    }

    pub fn available(&self) -> Option<&T> {
        match self {
            Self::Available(value) => Some(value),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicNotableNodeV2 {
    #[serde(with = "crate::hex::d32")]
    pub public_ref: Digest,
    pub importance: LivedNodeImportanceV1,
    pub occurred_at_utc_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AffectProjectionV1 {
    pub semantic_revision: u64,
    pub personality_revision: u64,
    #[serde(with = "crate::hex::d32")]
    pub state_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub formula_digest: Digest,
    pub confidence_fxp6: u32,
    pub region_mean_fxp6: [i64; AFFECT_REGION_COUNT_V1],
    pub region_delta_fxp6: [i64; AFFECT_REGION_COUNT_V1],
    pub trend: AffectTrendV1,
}

impl AffectProjectionV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        let delta_sum = self
            .region_delta_fxp6
            .iter()
            .try_fold(0_i128, |sum, value| sum.checked_add(i128::from(*value)))
            .ok_or(Alpha3ContractError::InvalidAffectProjection)?;
        let expected_trend = match delta_sum.cmp(&0) {
            std::cmp::Ordering::Less => AffectTrendV1::Falling,
            std::cmp::Ordering::Equal => AffectTrendV1::Stable,
            std::cmp::Ordering::Greater => AffectTrendV1::Rising,
        };
        if self.semantic_revision == 0
            || self.personality_revision == 0
            || self.state_digest == [0; 32]
            || self.formula_digest == [0; 32]
            || self.confidence_fxp6 > AFFECT_FXP6_SCALE_V1 as u32
            || self
                .region_mean_fxp6
                .iter()
                .any(|value| !(0..=AFFECT_FXP6_SCALE_V1).contains(value))
            || self
                .region_delta_fxp6
                .iter()
                .any(|value| !(-AFFECT_FXP6_SCALE_V1..=AFFECT_FXP6_SCALE_V1).contains(value))
            || self.trend != expected_trend
        {
            return Err(Alpha3ContractError::InvalidAffectProjection);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticProjectionHealthV1 {
    pub semantic_revision: u64,
    pub active_node_count: u32,
    pub active_edge_count: u32,
    #[serde(with = "crate::hex::d32")]
    pub graph_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub renorm_mapping_digest: Digest,
    pub semantic_dynamics_renormalization_residual_fxp6: i64,
    pub observer_pyramid_consistency_residual_fxp6: i64,
}

impl SemanticProjectionHealthV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        if self.semantic_revision == 0
            || self.active_node_count > AFFECT_NODE_CAPACITY_V1
            || self.active_edge_count > AFFECT_EDGE_CAPACITY_V1
            || self.graph_digest == [0; 32]
            || self.renorm_mapping_digest == [0; 32]
            || !(0..=AFFECT_FXP6_SCALE_V1)
                .contains(&self.semantic_dynamics_renormalization_residual_fxp6)
            || !(0..=AFFECT_FXP6_SCALE_V1)
                .contains(&self.observer_pyramid_consistency_residual_fxp6)
        {
            return Err(Alpha3ContractError::InvalidAffectProjection);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExperienceProjectionV2 {
    pub world_mode: ProjectionFieldV1<WorldModeV1>,
    pub world_layer: ProjectionFieldV1<WorldLayerV1>,
    pub activity: ProjectionFieldV1<LivedActivityClassV1>,
    pub goal: ProjectionFieldV1<LivedGoalClassV1>,
    pub sleep: ProjectionFieldV1<SleepStateV1>,
    #[serde(default)]
    pub affect: ProjectionFieldV1<AffectProjectionV1>,
    pub notable_nodes: ProjectionFieldV1<Vec<PublicNotableNodeV2>>,
    pub contact_explanation: ProjectionFieldV1<ContactExplanationCodeV1>,
    pub disclosure: DisclosureStatusV1,
}

impl ExperienceProjectionV2 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        if let Some(affect) = self.affect.available() {
            affect.validate()?;
        }
        if let Some(nodes) = self.notable_nodes.available() {
            if nodes.len() > MAX_ALPHA3_NOTABLE_NODES {
                return Err(Alpha3ContractError::VectorBound);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WhyContactedV2 {
    #[serde(with = "crate::hex::d32")]
    pub intention_public_ref: Digest,
    pub purpose: ContactPurposeV1,
    #[serde(with = "hex_vec32")]
    pub cause_public_refs: Vec<Digest>,
    pub consent_epoch: u64,
    pub consent_revision: u64,
    pub latest_gate: GateDecisionV2,
}

impl WhyContactedV2 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        validate_source_ref_count(&self.cause_public_refs)?;
        self.latest_gate.validate()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivacyControlStatusV1 {
    pub correction: PrivacyControlStateV1,
    pub export: PrivacyControlStateV1,
    pub delete: PrivacyControlStateV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateProjectionV2 {
    pub consent: ProjectionFieldV1<RelationConsentV1>,
    pub contact: ProjectionFieldV1<RelationContactProcessV1>,
    pub budget: ProjectionFieldV1<RelationBudgetLedgerV1>,
    pub why_contacted: ProjectionFieldV1<WhyContactedV2>,
    pub dreams: ProjectionFieldV1<Vec<DreamResidueV1>>,
    pub privacy_controls: ProjectionFieldV1<PrivacyControlStatusV1>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicIntentionV2 {
    #[serde(with = "crate::hex::d32")]
    pub public_ref: Digest,
    pub state: PublicIntentionStateV2,
    pub purpose: ContactPurposeV1,
    #[serde(with = "hex_vec32")]
    pub cause_public_refs: Vec<Digest>,
    pub created_at_utc_ms: u64,
    pub not_before_utc_ms: u64,
    pub expires_at_utc_ms: u64,
    pub revision: u64,
}

impl PublicIntentionV2 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        validate_source_ref_count(&self.cause_public_refs)?;
        if self.not_before_utc_ms < self.created_at_utc_ms
            || self.expires_at_utc_ms < self.not_before_utc_ms
        {
            return Err(Alpha3ContractError::InvalidTimeRange);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicOutboundV2 {
    #[serde(with = "crate::hex::d32")]
    pub public_ref: Digest,
    #[serde(with = "crate::hex::d32")]
    pub intention_public_ref: Digest,
    pub state: PublicOutboundStateV2,
    pub created_at_utc_ms: u64,
    pub settled_at_utc_ms: Option<u64>,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicClaimV2 {
    #[serde(with = "crate::hex::d32")]
    pub public_ref: Digest,
    pub kind: PublicClaimKindV2,
    pub state: PublicClaimStateV2,
    pub lease_deadline_utc_ms: u64,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeveloperProjectionV2 {
    #[serde(with = "crate::hex::d32")]
    pub build_digest: Digest,
    pub wire_schema_version: u16,
    pub autonomy_schema_version: u32,
    #[serde(with = "hex_vec32")]
    pub formula_digests: Vec<Digest>,
    pub projection_health: ProjectionHealthV1,
    #[serde(default)]
    pub semantic_health: ProjectionFieldV1<SemanticProjectionHealthV1>,
    pub readiness: ProjectionFieldV1<ProactiveReadinessV1>,
    pub latest_gate: ProjectionFieldV1<GateDecisionV2>,
    pub intentions: Vec<PublicIntentionV2>,
    pub outbounds: Vec<PublicOutboundV2>,
    pub claims: Vec<PublicClaimV2>,
    pub canonical_revision: u64,
    pub migration_revision: u32,
}

impl DeveloperProjectionV2 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        validate_source_ref_count(&self.formula_digests)?;
        if let Some(health) = self.semantic_health.available() {
            health.validate()?;
        }
        if self.intentions.len() > MAX_ALPHA3_PUBLIC_RECORDS
            || self.outbounds.len() > MAX_ALPHA3_PUBLIC_RECORDS
            || self.claims.len() > MAX_ALPHA3_PUBLIC_RECORDS
        {
            return Err(Alpha3ContractError::VectorBound);
        }
        for intention in &self.intentions {
            intention.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "layer",
    content = "projection",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ObserveProjectionV2 {
    Experience(ExperienceProjectionV2),
    Private(PrivateProjectionV2),
    Developer(DeveloperProjectionV2),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveSnapshotRequestV2 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d32")]
    pub persona_scope: Digest,
    #[serde(with = "crate::hex::d32_opt")]
    pub relation_scope: Option<Digest>,
    pub committed_only: bool,
    pub layer: ObserveLayerV2,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveSnapshotV2 {
    pub schema_version: u16,
    pub generated_at_utc_ms: u64,
    pub canonical_high_water: u64,
    pub projection: ObserveProjectionV2,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationContextRequestV1 {
    #[serde(with = "crate::hex::d32")]
    pub persona_scope: Digest,
    pub mode: InnerActivityModeV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationContextV1 {
    pub canonical_high_water: u64,
    pub current_segment: ProjectionFieldV1<LivedActivitySegmentV1>,
    pub active_goal: ProjectionFieldV1<LivedGoalV1>,
    pub notable_nodes: Vec<PublicNotableNodeV2>,
}

impl ConversationContextV1 {
    pub fn validate(&self) -> Result<(), Alpha3ContractError> {
        if self.notable_nodes.len() > MAX_ALPHA3_NOTABLE_NODES {
            return Err(Alpha3ContractError::VectorBound);
        }
        if let Some(segment) = self.current_segment.available() {
            segment.validate()?;
        }
        if let Some(goal) = self.active_goal.available() {
            goal.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentControlV1 {
    #[serde(with = "crate::hex::d32")]
    pub relation_scope: Digest,
    pub action: ConsentControlActionV1,
    pub terms: Option<ConsentTermsV1>,
    pub observed_at_utc_ms: u64,
    #[serde(with = "crate::hex::d32")]
    pub source_digest: Digest,
    #[serde(with = "crate::hex::d16")]
    pub source_event_id: Id128,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Alpha3CommitReceiptV1 {
    pub canonical_revision: u64,
    #[serde(with = "crate::hex::d32")]
    pub event_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyInteractionResultV1 {
    pub receipt: Alpha3CommitReceiptV1,
    pub applied_fact_count: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WakeSettleResultV2 {
    pub receipt: Alpha3CommitReceiptV1,
    pub state_revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Alpha3ErrorCodeV1 {
    SchemaUnsupported,
    ScopeMismatch,
    SourceUntrusted,
    CapabilityDenied,
    WorldAnchorImmutable,
    WorldLayerForbidden,
    ConsentRequired,
    ConsentRevoked,
    ProposalExpired,
    ProposalStale,
    DuplicateProposal,
    BudgetExhausted,
    ReadinessUnavailable,
    DreamNotReviewed,
    ProjectionIncomplete,
    MigrationFailed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Alpha3ErrorV1 {
    pub code: Alpha3ErrorCodeV1,
    pub retry_at_utc_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Alpha3ResultV1<T> {
    Ok(T),
    Error(Alpha3ErrorV1),
}
