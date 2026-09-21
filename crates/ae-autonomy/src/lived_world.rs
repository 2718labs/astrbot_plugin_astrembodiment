//! Pure deterministic lived-world authority.
//!
//! This module deliberately has no clock, random, network, Provider, Host, or
//! callback dependency. Every decision is a function of committed Native
//! inputs and the caller-supplied frozen time witness.

use ae_contracts::{
    wire, Digest, DreamResidueStateV1, FrozenTimeInputV1, Id128, InnerEventKindV1, InnerEventV1,
    LivedActivityClassV1, LivedActivitySegmentV1, LivedDayStateV1, LivedGoalClassV1,
    LivedGoalOriginV1, LivedGoalStateV1, LivedGoalV1, LivedNodeImportanceV1, LivedSegmentStateV1,
    PersonaTemporalProfileV1, ReflectionThemeCodeV1, SleepStateV1, WorldAnchorV1, WorldLayerV1,
    WorldModeV1, ALPHA3_SCHEMA_VERSION,
};
use ae_fixed::Fixed;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

pub const MAX_EXPLICIT_LIVED_TRANSITIONS: usize = 64;
pub const LIVED_DAY_MS: u64 = 86_400_000;
const MINUTE_MS: u64 = 60_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveExplicitFollowUpV1 {
    pub source_event_id: Id128,
    pub due_at_utc_ms: u64,
    pub expires_at_utc_ms: Option<u64>,
    pub source_attested: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedActivityOfferV1 {
    pub proposal_id: Id128,
    pub world_anchor_id: Id128,
    pub world_layer: WorldLayerV1,
    pub activity_class: LivedActivityClassV1,
    pub goal_class: Option<LivedGoalClassV1>,
    pub valid_from_utc_ms: u64,
    pub expires_at_utc_ms: u64,
    pub duration_minutes: u16,
    pub source_attested: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedDreamThemeV1 {
    pub residue_id: Id128,
    pub state: DreamResidueStateV1,
    pub theme: ReflectionThemeCodeV1,
    pub reviewed_at_utc_ms: Option<u64>,
    pub non_fact: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LivedCatchUpV1 {
    pub starts_at_utc_ms: u64,
    pub ends_at_utc_ms: u64,
    pub explicit_transitions: u16,
    pub skipped_transitions: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LivedDayAdvanceV1 {
    pub lived_day: LivedDayStateV1,
    pub inner_events: Vec<InnerEventV1>,
    pub catch_up: Option<LivedCatchUpV1>,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum LivedWorldError {
    #[error("lived-world persona scope mismatch")]
    ScopeMismatch,
    #[error("lived-world input has an invalid world anchor")]
    InvalidWorldAnchor,
    #[error("lived-world temporal profile is invalid")]
    InvalidTemporalProfile,
    #[error("lived-world source is not attested and unexpired")]
    SourceUntrusted,
    #[error("lived-world activity offer is invalid")]
    InvalidActivityOffer,
    #[error("dream residue is not retained non-fact authority")]
    DreamNotReviewed,
}

#[derive(Clone, Copy)]
struct RoutineSlot {
    slot_index: u8,
    starts_after_wake_minute: u16,
    ends_after_wake_minute: u16,
    activity_class: LivedActivityClassV1,
}

fn id128(domain: &[u8], parts: &[&[u8]]) -> Id128 {
    wire::domain_hash(domain, parts)[..16]
        .try_into()
        .expect("digest prefix")
}

fn activity_code(value: LivedActivityClassV1) -> &'static [u8] {
    match value {
        LivedActivityClassV1::Sleep => b"sleep",
        LivedActivityClassV1::PersonalCare => b"personal_care",
        LivedActivityClassV1::Maintenance => b"maintenance",
        LivedActivityClassV1::FocusedProject => b"focused_project",
        LivedActivityClassV1::Learning => b"learning",
        LivedActivityClassV1::Leisure => b"leisure",
        LivedActivityClassV1::SocialAvailability => b"social_availability",
        LivedActivityClassV1::Reflection => b"reflection",
        LivedActivityClassV1::Transition => b"transition",
    }
}

fn goal_code(value: LivedGoalClassV1) -> &'static [u8] {
    match value {
        LivedGoalClassV1::MaintainRoutine => b"maintain_routine",
        LivedGoalClassV1::AdvanceProject => b"advance_project",
        LivedGoalClassV1::ExploreInterest => b"explore_interest",
        LivedGoalClassV1::RestoreCapacity => b"restore_capacity",
        LivedGoalClassV1::CompleteUserFollowUp => b"complete_user_follow_up",
        LivedGoalClassV1::ReflectOnTheme => b"reflect_on_theme",
    }
}

fn origin_code(value: LivedGoalOriginV1) -> &'static [u8] {
    match value {
        LivedGoalOriginV1::Routine => b"routine",
        LivedGoalOriginV1::ExplicitFollowUp => b"explicit_follow_up",
        LivedGoalOriginV1::AcceptedEcosystem => b"accepted_ecosystem",
        LivedGoalOriginV1::DreamNonFactReview => b"dream_non_fact_review",
    }
}

fn layer_code(value: WorldLayerV1) -> &'static [u8] {
    match value {
        WorldLayerV1::ExternalObserved => b"external_observed",
        WorldLayerV1::PersonaNearReal => b"persona_near_real",
        WorldLayerV1::DeclaredFantasy => b"declared_fantasy",
    }
}

/// Stable alpha3 anchor for a persona that did not exist during v6 -> v7
/// migration. Its identity is independent of first-wake time and process ID.
pub fn fixed_world_anchor_v1(persona_scope: Digest) -> WorldAnchorV1 {
    WorldAnchorV1 {
        schema_version: ALPHA3_SCHEMA_VERSION,
        world_anchor_id: id128(b"ae.world-anchor.id.v1", &[&persona_scope]),
        persona_scope,
        mode: WorldModeV1::MixedMainWorld,
        home_context_ref: wire::domain_hash(b"ae.world-anchor.home-context.v1", &[&persona_scope]),
        lore_manifest_digest: wire::domain_hash(b"ae.world-anchor.empty-lore.v1", &[]),
        reality_policy_digest: wire::domain_hash(
            b"ae.world-anchor.reality-policy.v1",
            &[&persona_scope, b"mixed_main_world", b"alpha3-v1"],
        ),
        allowed_layers: vec![
            WorldLayerV1::ExternalObserved,
            WorldLayerV1::PersonaNearReal,
            WorldLayerV1::DeclaredFantasy,
        ],
        created_from_event_id: id128(b"ae.world-anchor.bootstrap-event.v1", &[&persona_scope]),
        revision: 1,
    }
}

pub fn lived_day_formula_digest_v1() -> Digest {
    wire::domain_hash(
        b"ae.lived-day.formula.v1",
        &[
            b"wake-relative",
            b"60:care",
            b"180:focused_project|learning",
            b"60:maintenance",
            b"180:learning|leisure",
            b"60:social_availability",
            b"remainder:leisure|transition",
            b"priority:sleep>follow_up>accepted>routine>fallback",
            b"catch_up:max_explicit=64",
        ],
    )
}

fn stable_choice(
    persona_scope: &Digest,
    world_anchor_id: &Id128,
    day_ordinal: i32,
    slot_index: u8,
    seed_digest: &Digest,
    variants: u8,
) -> u8 {
    let day = day_ordinal.to_le_bytes();
    let slot = [slot_index];
    let digest = wire::domain_hash(
        b"ae.lived-day.choice.v1",
        &[persona_scope, world_anchor_id, &day, &slot, seed_digest],
    );
    digest[0] % variants
}

fn awake_window_minutes(profile: &PersonaTemporalProfileV1) -> u16 {
    (i32::from(profile.preferred_sleep_local_minute)
        - i32::from(profile.preferred_wake_local_minute))
    .rem_euclid(1_440) as u16
}

fn routine_slots(
    profile: &PersonaTemporalProfileV1,
    anchor: &WorldAnchorV1,
    day_ordinal: i32,
    seed_digest: &Digest,
) -> Vec<RoutineSlot> {
    let awake = awake_window_minutes(profile);
    if awake == 0 {
        return Vec::new();
    }
    let selected = [
        LivedActivityClassV1::PersonalCare,
        if stable_choice(
            &profile.persona_scope,
            &anchor.world_anchor_id,
            day_ordinal,
            1,
            seed_digest,
            2,
        ) == 0
        {
            LivedActivityClassV1::FocusedProject
        } else {
            LivedActivityClassV1::Learning
        },
        LivedActivityClassV1::Maintenance,
        if stable_choice(
            &profile.persona_scope,
            &anchor.world_anchor_id,
            day_ordinal,
            3,
            seed_digest,
            2,
        ) == 0
        {
            LivedActivityClassV1::Learning
        } else {
            LivedActivityClassV1::Leisure
        },
        LivedActivityClassV1::SocialAvailability,
        if stable_choice(
            &profile.persona_scope,
            &anchor.world_anchor_id,
            day_ordinal,
            5,
            seed_digest,
            2,
        ) == 0
        {
            LivedActivityClassV1::Leisure
        } else {
            LivedActivityClassV1::Transition
        },
    ];
    let durations = [60u16, 180, 60, 180, 60, u16::MAX];
    let mut slots = Vec::with_capacity(selected.len());
    let mut cursor = 0u16;
    for (index, (activity_class, duration)) in selected.into_iter().zip(durations).enumerate() {
        if cursor >= awake {
            break;
        }
        let remaining = awake - cursor;
        let actual = remaining.min(duration);
        slots.push(RoutineSlot {
            slot_index: index as u8,
            starts_after_wake_minute: cursor,
            ends_after_wake_minute: cursor + actual,
            activity_class,
        });
        cursor += actual;
    }
    slots
}

fn local_day_start_utc_ms(frozen: &FrozenTimeInputV1) -> u64 {
    frozen
        .effective_now_utc_ms
        .saturating_sub(u64::from(frozen.persona_local_minute) * MINUTE_MS)
}

fn wake_start_utc_ms(profile: &PersonaTemporalProfileV1, frozen: &FrozenTimeInputV1) -> u64 {
    let local_day_start = local_day_start_utc_ms(frozen);
    if frozen.persona_local_minute >= profile.preferred_wake_local_minute {
        local_day_start.saturating_add(u64::from(profile.preferred_wake_local_minute) * MINUTE_MS)
    } else {
        local_day_start
            .saturating_sub(LIVED_DAY_MS)
            .saturating_add(u64::from(profile.preferred_wake_local_minute) * MINUTE_MS)
    }
}

fn base_segment(
    profile: &PersonaTemporalProfileV1,
    anchor: &WorldAnchorV1,
    frozen: &FrozenTimeInputV1,
    seed_digest: &Digest,
) -> (u8, LivedActivityClassV1, u64, u64) {
    let wake_start = wake_start_utc_ms(profile, frozen);
    let awake = awake_window_minutes(profile);
    let elapsed_minutes = frozen.effective_now_utc_ms.saturating_sub(wake_start) / MINUTE_MS;
    if awake == 0 || elapsed_minutes >= u64::from(awake) {
        let sleep_start = wake_start.saturating_add(u64::from(awake) * MINUTE_MS);
        return (
            u8::MAX,
            LivedActivityClassV1::Sleep,
            sleep_start,
            wake_start.saturating_add(LIVED_DAY_MS),
        );
    }
    let slots = routine_slots(profile, anchor, frozen.persona_day_ordinal, seed_digest);
    let slot = slots
        .iter()
        .find(|slot| elapsed_minutes < u64::from(slot.ends_after_wake_minute))
        .expect("awake routine covers the complete awake window");
    (
        slot.slot_index,
        slot.activity_class,
        wake_start.saturating_add(u64::from(slot.starts_after_wake_minute) * MINUTE_MS),
        wake_start.saturating_add(u64::from(slot.ends_after_wake_minute) * MINUTE_MS),
    )
}

fn routine_goal_class(activity: LivedActivityClassV1) -> LivedGoalClassV1 {
    match activity {
        LivedActivityClassV1::FocusedProject => LivedGoalClassV1::AdvanceProject,
        LivedActivityClassV1::Learning => LivedGoalClassV1::ExploreInterest,
        LivedActivityClassV1::Leisure | LivedActivityClassV1::Sleep => {
            LivedGoalClassV1::RestoreCapacity
        }
        _ => LivedGoalClassV1::MaintainRoutine,
    }
}

fn segment_id(
    persona_scope: &Digest,
    anchor_id: &Id128,
    layer: WorldLayerV1,
    activity: LivedActivityClassV1,
    starts_at_utc_ms: u64,
    ends_at_utc_ms: u64,
    source_event_ids: &[Id128],
) -> Id128 {
    let start = starts_at_utc_ms.to_le_bytes();
    let end = ends_at_utc_ms.to_le_bytes();
    let mut sources = Vec::with_capacity(source_event_ids.len() * 16);
    for source in source_event_ids {
        sources.extend_from_slice(source);
    }
    id128(
        b"ae.lived-day.segment-id.v1",
        &[
            persona_scope,
            anchor_id,
            layer_code(layer),
            activity_code(activity),
            &start,
            &end,
            &sources,
        ],
    )
}

fn goal_id(
    persona_scope: &Digest,
    anchor_id: &Id128,
    class: LivedGoalClassV1,
    origin: LivedGoalOriginV1,
    layer: WorldLayerV1,
    due_at_utc_ms: Option<u64>,
    source_event_ids: &[Id128],
) -> Id128 {
    let due = due_at_utc_ms.unwrap_or(0).to_le_bytes();
    let mut sources = Vec::with_capacity(source_event_ids.len() * 16);
    for source in source_event_ids {
        sources.extend_from_slice(source);
    }
    id128(
        b"ae.lived-day.goal-id.v1",
        &[
            persona_scope,
            anchor_id,
            goal_code(class),
            origin_code(origin),
            layer_code(layer),
            &due,
            &sources,
        ],
    )
}

fn make_goal(
    persona_scope: &Digest,
    anchor_id: &Id128,
    class: LivedGoalClassV1,
    origin: LivedGoalOriginV1,
    layer: WorldLayerV1,
    due_at_utc_ms: Option<u64>,
    source_event_ids: Vec<Id128>,
) -> LivedGoalV1 {
    LivedGoalV1 {
        goal_id: goal_id(
            persona_scope,
            anchor_id,
            class,
            origin,
            layer,
            due_at_utc_ms,
            &source_event_ids,
        ),
        goal_class: class,
        state: LivedGoalStateV1::Active,
        progress: Fixed::ZERO,
        due_at_utc_ms,
        world_layer: layer,
        origin,
        source_event_ids,
    }
}

fn validate_inputs(
    profile: &PersonaTemporalProfileV1,
    anchor: &WorldAnchorV1,
    prior: Option<&LivedDayStateV1>,
    frozen: &FrozenTimeInputV1,
) -> Result<(), LivedWorldError> {
    anchor
        .validate()
        .map_err(|_| LivedWorldError::InvalidWorldAnchor)?;
    if profile.persona_scope != anchor.persona_scope
        || prior.is_some_and(|state| {
            state.persona_scope != profile.persona_scope
                || state.world_anchor_id != anchor.world_anchor_id
        })
    {
        return Err(LivedWorldError::ScopeMismatch);
    }
    if profile.preferred_wake_local_minute >= 1_440
        || profile.preferred_sleep_local_minute >= 1_440
        || frozen.persona_local_minute >= 1_440
    {
        return Err(LivedWorldError::InvalidTemporalProfile);
    }
    Ok(())
}

fn select_offer<'a>(
    offers: &'a [AcceptedActivityOfferV1],
    anchor: &WorldAnchorV1,
    now: u64,
) -> Result<Option<&'a AcceptedActivityOfferV1>, LivedWorldError> {
    let mut eligible = Vec::new();
    let mut identities = BTreeSet::new();
    for offer in offers {
        if !identities.insert(offer.proposal_id) {
            return Err(LivedWorldError::InvalidActivityOffer);
        }
        if offer.valid_from_utc_ms > now || now >= offer.expires_at_utc_ms {
            continue;
        }
        if offer.world_anchor_id != anchor.world_anchor_id
            || offer.duration_minutes == 0
            || !anchor.allowed_layers.contains(&offer.world_layer)
        {
            return Err(LivedWorldError::InvalidActivityOffer);
        }
        if offer.world_layer == WorldLayerV1::ExternalObserved && !offer.source_attested {
            return Err(LivedWorldError::SourceUntrusted);
        }
        eligible.push(offer);
    }
    eligible.sort_by_key(|offer| {
        let layer_priority = match offer.world_layer {
            WorldLayerV1::ExternalObserved => 0,
            WorldLayerV1::PersonaNearReal => 1,
            WorldLayerV1::DeclaredFantasy => 2,
        };
        (layer_priority, offer.proposal_id)
    });
    Ok(eligible.into_iter().next())
}

fn transition_boundaries_minutes(profile: &PersonaTemporalProfileV1) -> Vec<u16> {
    let awake = awake_window_minutes(profile);
    if awake == 0 {
        return Vec::new();
    }
    let mut relative = vec![0u16, 60, 240, 300, 480, 540, awake];
    relative.retain(|minute| *minute <= awake);
    relative.sort_unstable();
    relative.dedup();
    let wake = u32::from(profile.preferred_wake_local_minute);
    let mut absolute = relative
        .into_iter()
        .map(|minute| ((wake + u32::from(minute)) % 1_440) as u16)
        .collect::<Vec<_>>();
    absolute.sort_unstable();
    absolute.dedup();
    absolute
}

fn div_floor(value: i128, divisor: i128) -> i128 {
    value.div_euclid(divisor)
}

fn count_boundary_occurrences(
    boundaries: &[u16],
    reference_day_start: u64,
    starts_at_utc_ms: u64,
    ends_at_utc_ms: u64,
) -> u64 {
    if starts_at_utc_ms > ends_at_utc_ms {
        return 0;
    }
    let day = i128::from(LIVED_DAY_MS);
    let reference = i128::from(reference_day_start);
    let start = i128::from(starts_at_utc_ms);
    let end = i128::from(ends_at_utc_ms);
    boundaries
        .iter()
        .map(|minute| {
            let base = reference + i128::from(*minute) * i128::from(MINUTE_MS);
            let first_k = div_floor(start - base + day - 1, day);
            let last_k = div_floor(end - base, day);
            if last_k < first_k {
                0
            } else {
                u64::try_from(last_k - first_k + 1).unwrap_or(u64::MAX)
            }
        })
        .sum()
}

fn next_boundary_at_or_after(
    boundaries: &[u16],
    reference_day_start: u64,
    cursor: u64,
) -> Option<u64> {
    let day = i128::from(LIVED_DAY_MS);
    let reference = i128::from(reference_day_start);
    let cursor = i128::from(cursor);
    boundaries
        .iter()
        .filter_map(|minute| {
            let base = reference + i128::from(*minute) * i128::from(MINUTE_MS);
            let k = div_floor(cursor - base + day - 1, day);
            u64::try_from(base + k * day).ok()
        })
        .min()
}

fn transition_event(
    persona_scope: &Digest,
    anchor_id: &Id128,
    formula_digest: &Digest,
    transition_at_utc_ms: u64,
) -> InnerEventV1 {
    let at = transition_at_utc_ms.to_le_bytes();
    InnerEventV1 {
        schema_version: 1,
        event_id: id128(
            b"ae.lived-day.transition-event.v1",
            &[persona_scope, anchor_id, formula_digest, &at],
        ),
        persona_scope: *persona_scope,
        kind: InnerEventKindV1::HomeostasisChanged,
        committed_at_utc_ms: transition_at_utc_ms,
        summary_code: "lived_day_transition".into(),
        value_before: None,
        value_after: None,
        source_event_ids: Vec::new(),
        tombstoned: false,
    }
}

fn changed_event(
    prior: Option<&LivedDayStateV1>,
    day: &LivedDayStateV1,
    now: u64,
    sources: Vec<Id128>,
) -> InnerEventV1 {
    let summary_code = match prior {
        None => "lived_day_changed",
        Some(previous) if previous.persona_day_ordinal != day.persona_day_ordinal => {
            "lived_day_changed"
        }
        Some(previous)
            if previous.current_segment.activity_class == LivedActivityClassV1::Sleep
                || day.current_segment.activity_class == LivedActivityClassV1::Sleep =>
        {
            "lived_sleep_changed"
        }
        Some(previous)
            if previous
                .active_goal
                .as_ref()
                .is_some_and(|goal| goal.origin == LivedGoalOriginV1::AcceptedEcosystem)
                || day
                    .active_goal
                    .as_ref()
                    .is_some_and(|goal| goal.origin == LivedGoalOriginV1::AcceptedEcosystem) =>
        {
            "lived_proposal_changed"
        }
        Some(previous)
            if previous
                .active_goal
                .as_ref()
                .is_some_and(|goal| goal.origin != LivedGoalOriginV1::Routine)
                || day
                    .active_goal
                    .as_ref()
                    .is_some_and(|goal| goal.origin != LivedGoalOriginV1::Routine) =>
        {
            "lived_goal_changed"
        }
        Some(_) => "lived_routine_transition",
    };
    let day_ordinal = day.persona_day_ordinal.to_le_bytes();
    let prior_revision = prior.map_or(0, |state| state.revision).to_le_bytes();
    let prior_segment = prior
        .map(|state| state.current_segment.segment_id)
        .unwrap_or([0; 16]);
    let prior_goal = prior
        .and_then(|state| state.active_goal.as_ref())
        .map(|goal| goal.goal_id)
        .unwrap_or([0; 16]);
    let goal = day
        .active_goal
        .as_ref()
        .map(|goal| goal.goal_id)
        .unwrap_or([0; 16]);
    InnerEventV1 {
        schema_version: 1,
        event_id: id128(
            b"ae.lived-day.changed-event.v1",
            &[
                &day.persona_scope,
                &day.world_anchor_id,
                &day_ordinal,
                &prior_revision,
                &prior_segment,
                &prior_goal,
                summary_code.as_bytes(),
                &day.current_segment.segment_id,
                &goal,
            ],
        ),
        persona_scope: day.persona_scope,
        kind: InnerEventKindV1::HomeostasisChanged,
        committed_at_utc_ms: now,
        summary_code: summary_code.into(),
        value_before: None,
        value_after: None,
        source_event_ids: sources,
        tombstoned: false,
    }
}

/// Advance the fixed mixed-world lived day using only committed inputs.
///
/// Activity reducer precedence is: sleep, due explicit follow-up, accepted
/// unexpired activity, routine template, deterministic leisure/transition
/// fallback. A retained non-fact dream can only replace the routine goal with
/// a reflection goal; it never changes the selected activity segment.
#[allow(clippy::too_many_arguments)]
pub fn advance_lived_day_v1(
    profile: &PersonaTemporalProfileV1,
    anchor: &WorldAnchorV1,
    prior: Option<&LivedDayStateV1>,
    accepted_activity_offers: &[AcceptedActivityOfferV1],
    explicit_follow_up: Option<&ActiveExplicitFollowUpV1>,
    retained_dream: Option<&RetainedDreamThemeV1>,
    sleep_state: SleepStateV1,
    frozen: &FrozenTimeInputV1,
    seed_digest: &Digest,
) -> Result<LivedDayAdvanceV1, LivedWorldError> {
    validate_inputs(profile, anchor, prior, frozen)?;
    let now = frozen.effective_now_utc_ms;
    let (_slot_index, routine_activity, base_start, base_end) =
        base_segment(profile, anchor, frozen, seed_digest);
    let mut layer = WorldLayerV1::PersonaNearReal;
    let mut activity = routine_activity;
    let mut starts_at = base_start;
    let mut ends_at = base_end;
    let mut importance = if routine_activity == LivedActivityClassV1::Sleep {
        LivedNodeImportanceV1::Notable
    } else {
        LivedNodeImportanceV1::Routine
    };
    let mut segment_sources = Vec::new();
    let mut goal = None;

    if sleep_state == SleepStateV1::Asleep || routine_activity == LivedActivityClassV1::Sleep {
        activity = LivedActivityClassV1::Sleep;
        importance = LivedNodeImportanceV1::Notable;
        goal = Some(make_goal(
            &profile.persona_scope,
            &anchor.world_anchor_id,
            LivedGoalClassV1::RestoreCapacity,
            LivedGoalOriginV1::Routine,
            WorldLayerV1::PersonaNearReal,
            None,
            Vec::new(),
        ));
    } else if explicit_follow_up.is_some_and(|follow_up| {
        follow_up.due_at_utc_ms <= now && follow_up.expires_at_utc_ms.is_none()
    }) {
        return Err(LivedWorldError::SourceUntrusted);
    } else if let Some(follow_up) = explicit_follow_up.filter(|follow_up| {
        follow_up.due_at_utc_ms <= now
            && follow_up
                .expires_at_utc_ms
                .is_some_and(|expires| now < expires)
    }) {
        if !follow_up.source_attested {
            return Err(LivedWorldError::SourceUntrusted);
        }
        layer = WorldLayerV1::ExternalObserved;
        activity = LivedActivityClassV1::Reflection;
        starts_at = base_start.max(follow_up.due_at_utc_ms);
        ends_at = follow_up
            .expires_at_utc_ms
            .expect("eligible external follow-up has an expiry")
            .min(base_end)
            .max(now.saturating_add(1));
        importance = LivedNodeImportanceV1::Notable;
        segment_sources.push(follow_up.source_event_id);
        goal = Some(make_goal(
            &profile.persona_scope,
            &anchor.world_anchor_id,
            LivedGoalClassV1::CompleteUserFollowUp,
            LivedGoalOriginV1::ExplicitFollowUp,
            layer,
            follow_up.expires_at_utc_ms,
            segment_sources.clone(),
        ));
    } else if let Some(offer) = select_offer(accepted_activity_offers, anchor, now)? {
        layer = offer.world_layer;
        activity = offer.activity_class;
        starts_at = base_start.max(offer.valid_from_utc_ms);
        ends_at = starts_at
            .saturating_add(u64::from(offer.duration_minutes) * MINUTE_MS)
            .min(base_end)
            .min(offer.expires_at_utc_ms)
            .max(now.saturating_add(1));
        importance = LivedNodeImportanceV1::Notable;
        segment_sources.push(offer.proposal_id);
        let class = offer
            .goal_class
            .unwrap_or_else(|| routine_goal_class(activity));
        goal = Some(make_goal(
            &profile.persona_scope,
            &anchor.world_anchor_id,
            class,
            LivedGoalOriginV1::AcceptedEcosystem,
            layer,
            Some(offer.expires_at_utc_ms),
            segment_sources.clone(),
        ));
    }

    if goal.is_none() {
        if let Some(dream) = retained_dream {
            if dream.state != DreamResidueStateV1::RetainedNonFact
                || !dream.non_fact
                || dream.reviewed_at_utc_ms.is_none()
            {
                return Err(LivedWorldError::DreamNotReviewed);
            }
            goal = Some(make_goal(
                &profile.persona_scope,
                &anchor.world_anchor_id,
                LivedGoalClassV1::ReflectOnTheme,
                LivedGoalOriginV1::DreamNonFactReview,
                WorldLayerV1::PersonaNearReal,
                None,
                vec![dream.residue_id],
            ));
        } else {
            goal = Some(make_goal(
                &profile.persona_scope,
                &anchor.world_anchor_id,
                routine_goal_class(activity),
                LivedGoalOriginV1::Routine,
                layer,
                None,
                segment_sources.clone(),
            ));
        }
    }

    let current_segment = LivedActivitySegmentV1 {
        segment_id: segment_id(
            &profile.persona_scope,
            &anchor.world_anchor_id,
            layer,
            activity,
            starts_at,
            ends_at,
            &segment_sources,
        ),
        world_layer: layer,
        activity_class: activity,
        starts_at_utc_ms: starts_at,
        ends_at_utc_ms: ends_at,
        state: LivedSegmentStateV1::Active,
        importance,
        source_event_ids: segment_sources.clone(),
    };
    let mut day_sources = segment_sources;
    if let Some(active_goal) = &goal {
        for source in &active_goal.source_event_ids {
            if !day_sources.contains(source) && day_sources.len() < 8 {
                day_sources.push(*source);
            }
        }
    }
    let lived_day = LivedDayStateV1 {
        schema_version: ALPHA3_SCHEMA_VERSION,
        persona_scope: profile.persona_scope,
        world_anchor_id: anchor.world_anchor_id,
        persona_day_ordinal: frozen.persona_day_ordinal,
        revision: prior.map_or(1, |state| state.revision.saturating_add(1)),
        current_segment,
        active_goal: goal,
        next_transition_utc_ms: ends_at,
        routine_formula_digest: lived_day_formula_digest_v1(),
        source_event_ids: day_sources.clone(),
    };

    let boundaries = transition_boundaries_minutes(profile);
    let catch_up_start = prior.map(|state| state.next_transition_utc_ms);
    let transition_count = catch_up_start.map_or(0, |start| {
        count_boundary_occurrences(&boundaries, local_day_start_utc_ms(frozen), start, now)
    });
    let explicit_count = transition_count.min(MAX_EXPLICIT_LIVED_TRANSITIONS as u64);
    let mut inner_events = Vec::with_capacity(
        usize::try_from(explicit_count).unwrap_or(MAX_EXPLICIT_LIVED_TRANSITIONS) + 2,
    );
    if let Some(mut cursor) = catch_up_start {
        for _ in 0..explicit_count {
            let Some(at) =
                next_boundary_at_or_after(&boundaries, local_day_start_utc_ms(frozen), cursor)
            else {
                break;
            };
            if at > now {
                break;
            }
            inner_events.push(transition_event(
                &profile.persona_scope,
                &anchor.world_anchor_id,
                &lived_day.routine_formula_digest,
                at,
            ));
            cursor = at.saturating_add(1);
        }
    }
    let catch_up = if transition_count > explicit_count {
        let start = catch_up_start.unwrap_or(now);
        let skipped = transition_count - explicit_count;
        let end_bytes = now.to_le_bytes();
        let skipped_bytes = skipped.to_le_bytes();
        inner_events.push(InnerEventV1 {
            schema_version: 1,
            event_id: id128(
                b"ae.lived-day.catch-up-event.v1",
                &[
                    &profile.persona_scope,
                    &anchor.world_anchor_id,
                    &start.to_le_bytes(),
                    &end_bytes,
                    &skipped_bytes,
                ],
            ),
            persona_scope: profile.persona_scope,
            kind: InnerEventKindV1::OfflineGap,
            committed_at_utc_ms: now,
            summary_code: "lived_day_catch_up".into(),
            // committed_at carries the end; these fixed raw values carry the
            // start and skipped count without introducing dynamic prose.
            value_before: Some(Fixed::from_raw(i64::try_from(start).unwrap_or(i64::MAX))),
            value_after: Some(Fixed::from_raw(i64::try_from(skipped).unwrap_or(i64::MAX))),
            source_event_ids: Vec::new(),
            tombstoned: false,
        });
        Some(LivedCatchUpV1 {
            starts_at_utc_ms: start,
            ends_at_utc_ms: now,
            explicit_transitions: explicit_count as u16,
            skipped_transitions: skipped,
        })
    } else {
        None
    };

    let materially_changed = prior.is_none_or(|previous| {
        previous.persona_day_ordinal != lived_day.persona_day_ordinal
            || previous.current_segment.segment_id != lived_day.current_segment.segment_id
            || previous.active_goal.as_ref().map(|goal| goal.goal_id)
                != lived_day.active_goal.as_ref().map(|goal| goal.goal_id)
    });
    if materially_changed {
        inner_events.push(changed_event(prior, &lived_day, now, day_sources));
    }
    Ok(LivedDayAdvanceV1 {
        lived_day,
        inner_events,
        catch_up,
    })
}
