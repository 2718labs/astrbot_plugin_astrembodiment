//! Closed, versioned contracts for the autonomous CyberHuman runtime.

use crate::{hex, Digest, Id128, ScopeRef};
use ae_fixed::Fixed;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const AUTONOMY_SCHEMA_VERSION: u16 = 1;

/// Schema and cardinality fences for persona-global cognition that remains
/// private to this plugin. These records carry no relation, transport, model,
/// or content authority.
pub const LOCAL_COGNITION_SCHEMA_VERSION: u16 = 1;
pub const MAX_LOCAL_COGNITION_SOURCE_EVENT_IDS_V1: usize = 8;
pub const MAX_LOCAL_DREAM_TAGS_V1: usize = 8;
pub const MAX_ENDOGENOUS_INTENT_TTL_MS_V1: u64 = 604_800_000;
pub const MAX_LOCAL_DREAM_TTL_MS_V1: u64 = 604_800_000;

mod local_hex_vec16 {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndogenousIntentPhaseV1 {
    Dormant,
    Incubating,
    Salient,
    Inhibited,
    Dissipating,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalDreamTagV1 {
    Rest,
    Continuity,
    Change,
    Curiosity,
    Completion,
    Uncertainty,
    Motion,
    Threshold,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LocalCognitionContractErrorV1 {
    #[error("local cognition schema version is unsupported")]
    SchemaUnsupported,
    #[error("local cognition identity or commitment is zero")]
    ZeroIdentity,
    #[error("local cognition revision must be nonzero")]
    InvalidRevision,
    #[error("local cognition fixed-point value is outside its closed range")]
    FixedOutOfRange,
    #[error("local cognition time range is invalid")]
    InvalidTimeRange,
    #[error("local cognition time-to-live exceeds its bound")]
    TtlExceeded,
    #[error("local cognition vector must not be empty")]
    EmptyVector,
    #[error("local cognition vector exceeds its bound")]
    VectorBound,
    #[error("local cognition vector contains duplicate values")]
    DuplicateValue,
    #[error("local dream residue must remain non-fact")]
    DreamMustBeNonFact,
}

/// Persona-global endogenous pressure. It is deliberately incapable of
/// identifying another party, holding prose, or authorizing external work.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndogenousIntentStateV1 {
    pub schema_version: u16,
    #[serde(with = "hex::d16")]
    pub intent_id: Id128,
    #[serde(with = "hex::d32")]
    pub persona_scope: Digest,
    pub state: EndogenousIntentPhaseV1,
    pub salience: Fixed,
    pub inhibition: Fixed,
    pub urgency: Fixed,
    pub revision: u64,
    pub source_semantic_revision: u64,
    pub updated_at_utc_ms: u64,
    pub expires_at_utc_ms: u64,
    #[serde(with = "hex::d16")]
    pub source_event_id: Id128,
    #[serde(with = "hex::d32")]
    pub commitment_digest: Digest,
}

impl EndogenousIntentStateV1 {
    pub const SCHEMA_VERSION: u16 = LOCAL_COGNITION_SCHEMA_VERSION;

    pub fn validate_v1(&self) -> Result<(), LocalCognitionContractErrorV1> {
        if self.schema_version != Self::SCHEMA_VERSION {
            return Err(LocalCognitionContractErrorV1::SchemaUnsupported);
        }
        if self.intent_id.iter().all(|byte| *byte == 0)
            || self.persona_scope.iter().all(|byte| *byte == 0)
            || self.source_event_id.iter().all(|byte| *byte == 0)
            || self.commitment_digest.iter().all(|byte| *byte == 0)
        {
            return Err(LocalCognitionContractErrorV1::ZeroIdentity);
        }
        if self.revision == 0 {
            return Err(LocalCognitionContractErrorV1::InvalidRevision);
        }
        if [self.salience, self.inhibition, self.urgency]
            .into_iter()
            .any(|value| !(Fixed::ZERO..=Fixed::ONE).contains(&value))
        {
            return Err(LocalCognitionContractErrorV1::FixedOutOfRange);
        }
        if self.updated_at_utc_ms == 0 || self.expires_at_utc_ms <= self.updated_at_utc_ms {
            return Err(LocalCognitionContractErrorV1::InvalidTimeRange);
        }
        if self.expires_at_utc_ms - self.updated_at_utc_ms > MAX_ENDOGENOUS_INTENT_TTL_MS_V1 {
            return Err(LocalCognitionContractErrorV1::TtlExceeded);
        }
        Ok(())
    }
}

/// Bounded affective residue produced locally during sleep. The record has a
/// closed vocabulary and cannot store dream prose or claim an observed fact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalDreamResidueV1 {
    pub schema_version: u16,
    #[serde(with = "hex::d16")]
    pub residue_id: Id128,
    #[serde(with = "hex::d32")]
    pub persona_scope: Digest,
    pub tags: Vec<LocalDreamTagV1>,
    pub affect_valence: Fixed,
    pub affect_arousal: Fixed,
    #[serde(with = "local_hex_vec16")]
    pub source_event_ids: Vec<Id128>,
    pub created_at_utc_ms: u64,
    pub expires_at_utc_ms: u64,
    pub non_fact: bool,
    #[serde(with = "hex::d32")]
    pub commitment_digest: Digest,
}

impl LocalDreamResidueV1 {
    pub const SCHEMA_VERSION: u16 = LOCAL_COGNITION_SCHEMA_VERSION;

    pub fn validate_v1(&self) -> Result<(), LocalCognitionContractErrorV1> {
        if self.schema_version != Self::SCHEMA_VERSION {
            return Err(LocalCognitionContractErrorV1::SchemaUnsupported);
        }
        if self.residue_id.iter().all(|byte| *byte == 0)
            || self.persona_scope.iter().all(|byte| *byte == 0)
            || self.commitment_digest.iter().all(|byte| *byte == 0)
            || self
                .source_event_ids
                .iter()
                .any(|event_id| event_id.iter().all(|byte| *byte == 0))
        {
            return Err(LocalCognitionContractErrorV1::ZeroIdentity);
        }
        if self.tags.is_empty() || self.source_event_ids.is_empty() {
            return Err(LocalCognitionContractErrorV1::EmptyVector);
        }
        if self.tags.len() > MAX_LOCAL_DREAM_TAGS_V1
            || self.source_event_ids.len() > MAX_LOCAL_COGNITION_SOURCE_EVENT_IDS_V1
        {
            return Err(LocalCognitionContractErrorV1::VectorBound);
        }
        if self.tags.iter().copied().collect::<BTreeSet<_>>().len() != self.tags.len()
            || self
                .source_event_ids
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
                != self.source_event_ids.len()
        {
            return Err(LocalCognitionContractErrorV1::DuplicateValue);
        }
        if !self.non_fact {
            return Err(LocalCognitionContractErrorV1::DreamMustBeNonFact);
        }
        if !(Fixed::from_raw(-1_000_000)..=Fixed::ONE).contains(&self.affect_valence)
            || !(Fixed::ZERO..=Fixed::ONE).contains(&self.affect_arousal)
        {
            return Err(LocalCognitionContractErrorV1::FixedOutOfRange);
        }
        if self.created_at_utc_ms == 0 || self.expires_at_utc_ms <= self.created_at_utc_ms {
            return Err(LocalCognitionContractErrorV1::InvalidTimeRange);
        }
        if self.expires_at_utc_ms - self.created_at_utc_ms > MAX_LOCAL_DREAM_TTL_MS_V1 {
            return Err(LocalCognitionContractErrorV1::TtlExceeded);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DerivedMutationClass {
    TemporalState,
    SleepState,
    WorkspaceProjection,
    OperationalIntention,
    WakeSchedule,
    PermanentMemory,
    RelationCommitment,
    PersonaGenesis,
    DeliveryFact,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenTimeInputV1 {
    pub schema_version: u16,
    pub observed_now_utc_ms: u64,
    pub effective_now_utc_ms: u64,
    pub persona_tzid: String,
    pub persona_utc_offset_seconds: i32,
    pub persona_local_minute: u16,
    pub persona_day_ordinal: i32,
    pub relation_tzid: String,
    pub relation_utc_offset_seconds: i32,
    pub relation_local_minute: u16,
    pub relation_day_ordinal: i32,
    pub budget_day_start_utc_ms: u64,
    pub budget_next_day_start_utc_ms: u64,
    pub next_timezone_transition_utc_ms: Option<u64>,
    #[serde(with = "hex::d32")]
    pub tzdb_fingerprint: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeAdvanceV1 {
    #[serde(with = "hex::d16")]
    pub event_id: Id128,
    pub scope: ScopeRef,
    pub expected_generation: u64,
    pub frozen: FrozenTimeInputV1,
    #[serde(with = "hex::d32")]
    pub frozen_input_digest: Digest,
    pub stimulus: AutonomousStimulusV1,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutonomousStimulusV1 {
    pub arousal: Fixed,
    pub urgency: Fixed,
    pub emergency_authorized: bool,
    #[serde(with = "hex::d32")]
    pub source_digest: Digest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChronotypeV1 {
    Morning,
    Intermediate,
    NightOwl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimezoneSourceV1 {
    Explicit,
    Platform,
    HostFallback,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SleepStateV1 {
    Awake,
    Drowsy,
    Asleep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeIntensityV1 {
    Micro,
    Associative,
    Ignition,
    Emergency,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentionStateV1 {
    Forming,
    Ready,
    Deferred,
    Externalizing,
    DispatchPending,
    AdapterCallStarted,
    AdapterSubmitted,
    PlatformAccepted,
    DeliveryConfirmed,
    DispatchUnknown,
    Suppressed,
    Expired,
    Terminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchOutcomeV1 {
    AdapterRejectedTerminal,
    AdapterSubmitted,
    PlatformAccepted,
    DeliveryConfirmed,
    DispatchUnknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKindV1 {
    Private,
    Group,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InnerEventKindV1 {
    HomeostasisChanged,
    SleepTransition,
    MemoryReferenceSurfaced,
    WorkspaceIgnited,
    WorkspaceSuppressed,
    WorkspaceResidualRejected,
    IntentionFormed,
    IntentionDeferred,
    IntentionSuppressed,
    ActionArbitrated,
    MemoryConsolidationCandidate,
    TravelPhaseAdjusted,
    OfflineGap,
    OutboxStage,
    ClockRollback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateSuppressionReasonV1 {
    ProactiveDisabled,
    TargetUnavailable,
    IntentionUnavailable,
    ResidualRejected,
    PersonaAsleep,
    QuietHours,
    DailyLimit,
    Cooldown,
    UnansweredHardStop,
    TimezoneUnreliable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaTemporalProfileV1 {
    pub schema_version: u16,
    #[serde(with = "hex::d32")]
    pub persona_scope: Digest,
    pub home_timezone: String,
    pub current_timezone: String,
    pub chronotype: ChronotypeV1,
    pub preferred_sleep_local_minute: u16,
    pub preferred_wake_local_minute: u16,
    pub sleep_flex_minutes: u16,
    pub entrainment_rate_minutes_per_day: u16,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationTemporalPolicyV1 {
    pub schema_version: u16,
    #[serde(with = "hex::d32")]
    pub relation_scope: Digest,
    pub user_timezone: String,
    pub timezone_source: TimezoneSourceV1,
    pub quiet_hours_start_minute: u16,
    pub quiet_hours_end_minute: u16,
    pub quiet_hours_emergency_bypass: bool,
    pub proactive_enabled: bool,
    pub proactive_daily_max: u16,
    pub min_proactive_cooldown_ms: u64,
    pub intention_ttl_ms: u64,
    pub unanswered_backoff_base_ms: u64,
    pub unanswered_hard_stop: u16,
    pub emergency_threshold: Fixed,
    pub daily_submitted: u16,
    pub consecutive_unanswered: u16,
    pub last_inbound_utc_ms: Option<u64>,
    pub last_proactive_submitted_utc_ms: Option<u64>,
    pub revision: u64,
    /// `0` preserves the stored fixed pair; `1` enables the closed adaptive policy.
    #[serde(default)]
    pub auto_policy_version: u16,
    /// Tokens reserved by the next externalization claim.
    #[serde(default)]
    pub next_claim_reservation_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutonomousRuntimeStateV1 {
    pub schema_version: u16,
    #[serde(with = "hex::d32")]
    pub persona_scope: Digest,
    #[serde(with = "hex::d32_opt")]
    pub relation_scope: Option<Digest>,
    pub generation: u64,
    pub state_revision: u64,
    pub last_advanced_at_utc_ms: u64,
    pub next_wake_at_utc_ms: u64,
    pub wake_intensity: WakeIntensityV1,
    pub sleep_state: SleepStateV1,
    pub process_s: Fixed,
    pub process_c: Fixed,
    pub arousal: Fixed,
    pub sleep_threshold_held_ms: u64,
    pub circadian_phase_minutes: Fixed,
    pub affiliation_need: Fixed,
    pub unfinished_topic_salience: Fixed,
    pub social_energy: Fixed,
    #[serde(with = "hex::d32")]
    pub formula_digest: Digest,
    #[serde(with = "hex::d32")]
    pub mapping_digest: Digest,
    pub workspace_residual: Fixed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InnerEventV1 {
    pub schema_version: u16,
    #[serde(with = "hex::d16")]
    pub event_id: Id128,
    #[serde(with = "hex::d32")]
    pub persona_scope: Digest,
    pub kind: InnerEventKindV1,
    pub committed_at_utc_ms: u64,
    pub summary_code: String,
    pub value_before: Option<Fixed>,
    pub value_after: Option<Fixed>,
    pub source_event_ids: Vec<Id128>,
    pub tombstoned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObserveModeV1 {
    CommittedOnly,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveSnapshotRequestV1 {
    pub schema_version: u16,
    #[serde(with = "hex::d32")]
    pub persona_scope: Digest,
    #[serde(with = "hex::d32_opt")]
    pub relation_scope: Option<Digest>,
    pub mode: ObserveModeV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveCursorV1 {
    pub schema_version: u16,
    #[serde(with = "hex::d32")]
    pub persona_scope: Digest,
    pub journal_revision: u64,
    /// Event position within `journal_revision`. `None` means that revision is
    /// fully consumed (including a canonical TimeAdvance with zero events).
    #[serde(with = "hex::d16_opt")]
    pub event_id: Option<Id128>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveEventsRequestV1 {
    pub schema_version: u16,
    #[serde(with = "hex::d32")]
    pub persona_scope: Digest,
    #[serde(with = "hex::d32_opt")]
    pub relation_scope: Option<Digest>,
    /// Pinned canonical watermark for one pagination run. Omit to advance to
    /// the current head while retaining `after` as the resume position.
    pub through_revision: Option<u64>,
    pub after: Option<ObserveCursorV1>,
    pub limit: u16,
    pub mode: ObserveModeV1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObserveEventSourceV1 {
    Inner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObserveAuthorityStatusV1 {
    Committed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveEventCausalV1 {
    #[serde(with = "hex::d16_opt")]
    pub turn_id: Option<Id128>,
    #[serde(with = "hex::d16_opt")]
    pub action_id: Option<Id128>,
    #[serde(with = "hex::d16_opt")]
    pub delivery_id: Option<Id128>,
    #[serde(with = "hex::d16_opt")]
    pub claim_id: Option<Id128>,
    pub parent_event_refs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveEventV1 {
    pub public_event_ref: String,
    pub source: ObserveEventSourceV1,
    pub kind: InnerEventKindV1,
    pub committed_at_utc_ms: u64,
    #[serde(with = "hex::d32_opt")]
    pub relation_scope: Option<Digest>,
    pub authority_status: ObserveAuthorityStatusV1,
    pub causal: ObserveEventCausalV1,
    pub summary_code: String,
    pub value_before: Option<Fixed>,
    pub value_after: Option<Fixed>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveHighWaterV1 {
    pub through_revision: u64,
    pub operational_ordinal: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveEventsV1 {
    pub schema_version: u16,
    pub items: Vec<ObserveEventV1>,
    pub next: Option<ObserveCursorV1>,
    pub high_water: ObserveHighWaterV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveProjectionStatusV1 {
    /// Local agreement between the latest state-bearing canonical delta and
    /// its current runtime/snapshot projection; not a genesis-rooted replay.
    pub head_consistent: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservePendingV1 {
    pub intentions: Option<u64>,
    pub outbounds: Option<u64>,
    pub active_claims: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveBudgetV1 {
    pub day_start_utc_ms: Option<u64>,
    pub reserved_tokens: Option<u64>,
    pub used_tokens: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveSnapshotV1 {
    pub schema_version: u16,
    pub generated_at_utc_ms: u64,
    #[serde(with = "hex::d32")]
    pub persona_scope: Digest,
    #[serde(with = "hex::d32_opt")]
    pub relation_scope: Option<Digest>,
    pub canonical_revision: u64,
    pub operational_ordinal: Option<u64>,
    pub actor_epoch: Option<u64>,
    pub actor_sequence: Option<u64>,
    pub projection: ObserveProjectionStatusV1,
    pub runtime: AutonomousRuntimeStateV1,
    pub mood: Option<MoodCardV1>,
    pub pending: ObservePendingV1,
    pub budget: ObserveBudgetV1,
    pub latest_gate: Option<GateDecisionV1>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurableIntentionV1 {
    pub schema_version: u16,
    #[serde(with = "hex::d16")]
    pub intention_id: Id128,
    #[serde(with = "hex::d32")]
    pub persona_scope: Digest,
    #[serde(with = "hex::d32")]
    pub relation_scope: Digest,
    pub state: IntentionStateV1,
    pub action_class: String,
    pub salience: Fixed,
    pub urgency: Fixed,
    pub confidence: Fixed,
    pub created_at_utc_ms: u64,
    pub not_before_utc_ms: u64,
    pub expires_at_utc_ms: u64,
    pub externalization_attempts: u8,
    #[serde(with = "hex::d32")]
    pub semantic_idempotency_digest: Digest,
    #[serde(with = "hex::d32")]
    pub workspace_mapping_digest: Digest,
    pub workspace_residual: Fixed,
    pub source_event_ids: Vec<Id128>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutboundTargetEnvelopeV1 {
    pub schema_version: u16,
    pub target_kind: TargetKindV1,
    pub umo_ciphertext: Vec<u8>,
    pub umo_nonce: Vec<u8>,
    pub key_id: String,
    #[serde(with = "hex::d32")]
    pub umo_digest: Digest,
    #[serde(with = "hex::d16")]
    pub platform_token: Id128,
    #[serde(with = "hex::d16")]
    pub bot_token: Id128,
    #[serde(with = "hex::d16")]
    pub persona_token: Id128,
    #[serde(with = "hex::d16")]
    pub relation_token: Id128,
    #[serde(with = "hex::d16")]
    pub session_token: Id128,
    pub bound_at_utc_ms: u64,
    pub binding_generation: u64,
    #[serde(with = "hex::d32")]
    pub binding_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WakeClaimRequestV1 {
    pub event: TimeAdvanceV1,
    #[serde(with = "hex::d32")]
    pub caller_incarnation: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WakeProposalV1 {
    pub state: AutonomousRuntimeStateV1,
    pub inner_events: Vec<InnerEventV1>,
    pub intentions: Vec<DurableIntentionV1>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WakeClaimV1 {
    #[serde(with = "hex::d32")]
    pub claim_token: Digest,
    pub event: TimeAdvanceV1,
    pub proposal: WakeProposalV1,
    pub lease_deadline_utc_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalizationClaimV1 {
    #[serde(with = "hex::d32")]
    pub claim_token: Digest,
    #[serde(with = "hex::d16")]
    pub intention_id: Id128,
    pub attempt_no: u8,
    pub max_tokens: u16,
    pub prompt_contract: String,
    #[serde(with = "hex::d32")]
    pub prompt_contract_digest: Digest,
    pub relation_policy_revision: u64,
    #[serde(with = "hex::d32")]
    pub target_binding_digest: Digest,
    #[serde(with = "hex::d32")]
    pub capability_snapshot_digest: Digest,
    #[serde(with = "hex::d32")]
    pub frozen_input_digest: Digest,
    #[serde(with = "hex::d32")]
    pub caller_incarnation: Digest,
    pub lease_deadline_utc_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateAndClaimExternalizationRequestV1 {
    pub scope: ScopeRef,
    #[serde(with = "hex::d16")]
    pub intention_id: Id128,
    pub attempt_no: u8,
    pub expected_revision: u64,
    #[serde(with = "hex::d32")]
    pub caller_incarnation: Digest,
    pub capability: HostCapabilitySnapshotV1,
    pub frozen: FrozenTimeInputV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateAndClaimExternalizationV1 {
    pub decision: GateDecisionV1,
    pub claim: Option<ExternalizationClaimV1>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalizationOutcomeV1 {
    Success,
    TransientProviderFailure,
    RejectedTerminal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalizationSettleV1 {
    #[serde(with = "hex::d32")]
    pub claim_token: Digest,
    pub outcome: ExternalizationOutcomeV1,
    pub used_tokens: Option<u16>,
    #[serde(with = "hex::d32_opt")]
    pub candidate_digest: Option<Digest>,
    pub candidate_ciphertext: Option<Vec<u8>>,
    #[serde(with = "hex::d32")]
    pub caller_incarnation: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostCapabilitySnapshotV1 {
    pub schema_version: u16,
    pub astrbot_send_available: bool,
    pub credential_store_available: bool,
    pub platform_idempotent: bool,
    pub provider_identifier: String,
    #[serde(with = "hex::d32")]
    pub config_source_digest: Digest,
    pub config_revision: u64,
    pub content_boundary_version: u16,
    pub policy_version: u16,
    #[serde(with = "hex::d32")]
    pub snapshot_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateAndClaimDispatchRequestV1 {
    pub scope: ScopeRef,
    #[serde(with = "hex::d16")]
    pub outbound_id: Id128,
    #[serde(with = "hex::d32")]
    pub expected_target_digest: Digest,
    #[serde(with = "hex::d32")]
    pub caller_incarnation: Digest,
    pub capability: HostCapabilitySnapshotV1,
    pub frozen: FrozenTimeInputV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateAndClaimDispatchV1 {
    pub decision: GateDecisionV1,
    pub claim: Option<DispatchClaimV1>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutonomousPendingWorkV1 {
    pub intentions: Vec<PendingIntentionV1>,
    pub outbounds: Vec<OutboundAttemptV1>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingIntentionV1 {
    pub intention: DurableIntentionV1,
    pub current_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutonomyJournalDeltaV1 {
    pub state: Option<AutonomousRuntimeStateV1>,
    pub inner_events: Vec<InnerEventV1>,
    pub intention: Option<DurableIntentionV1>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutonomyProjectionReportV1 {
    pub checked_rows: u64,
    pub ok: bool,
    pub rebuilt_generation: Option<u64>,
    pub first_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchClaimV1 {
    #[serde(with = "hex::d32")]
    pub claim_token: Digest,
    #[serde(with = "hex::d16")]
    pub outbound_id: Id128,
    pub target: OutboundTargetEnvelopeV1,
    pub candidate_ciphertext: Vec<u8>,
    #[serde(with = "hex::d32")]
    pub preflight_digest: Digest,
    pub relation_policy_revision: u64,
    #[serde(with = "hex::d32")]
    pub capability_snapshot_digest: Digest,
    #[serde(with = "hex::d32")]
    pub frozen_input_digest: Digest,
    #[serde(with = "hex::d32")]
    pub caller_incarnation: Digest,
    pub lease_deadline_utc_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchSettleV1 {
    #[serde(with = "hex::d32")]
    pub claim_token: Digest,
    pub outcome: DispatchOutcomeV1,
    pub settled_at_utc_ms: u64,
    #[serde(with = "hex::d32_opt")]
    pub receipt_digest: Option<Digest>,
    #[serde(with = "hex::d32")]
    pub caller_incarnation: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutboundAttemptV1 {
    #[serde(with = "hex::d16")]
    pub outbound_id: Id128,
    #[serde(with = "hex::d16")]
    pub intention_id: Id128,
    pub state: IntentionStateV1,
    pub target: OutboundTargetEnvelopeV1,
    pub candidate_ciphertext: Vec<u8>,
    #[serde(with = "hex::d32")]
    pub candidate_digest: Digest,
    pub created_at_utc_ms: u64,
    pub settled_at_utc_ms: Option<u64>,
    pub counted_budget_day_start_utc_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InnerEventQueryV1 {
    #[serde(with = "hex::d32")]
    pub persona_scope: Digest,
    #[serde(with = "hex::d16_opt")]
    pub after_event_id: Option<Id128>,
    pub limit: u16,
    pub display_timezone: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InnerEventPageV1 {
    pub items: Vec<InnerEventV1>,
    #[serde(with = "hex::d16_opt")]
    pub next_after_event_id: Option<Id128>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoodCardV1 {
    pub mood: String,
    pub sleep: SleepStateV1,
    #[serde(with = "hex::d16")]
    pub event_id: Id128,
    pub contact_tendency: Fixed,
    pub as_of_utc_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryReportV1 {
    pub state: AutonomousRuntimeStateV1,
    pub orphaned_dispatches_marked_unknown: u32,
    pub orphaned_externalizations_recovered: u32,
    pub offline_gap_recorded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateDecisionV1 {
    pub permitted: bool,
    pub suppression: Option<GateSuppressionReasonV1>,
    pub retry_at_utc_ms: Option<u64>,
}
