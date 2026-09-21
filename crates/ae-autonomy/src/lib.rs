#![forbid(unsafe_code)]

//! Deterministic, platform-independent temporal and sleep dynamics.

use ae_contracts::*;
use ae_fixed::{Fixed, SCALE};
use thiserror::Error;

mod lived_world;

pub use lived_world::{
    advance_lived_day_v1, fixed_world_anchor_v1, lived_day_formula_digest_v1,
    AcceptedActivityOfferV1, ActiveExplicitFollowUpV1, LivedCatchUpV1, LivedDayAdvanceV1,
    LivedWorldError, RetainedDreamThemeV1, LIVED_DAY_MS, MAX_EXPLICIT_LIVED_TRANSITIONS,
};

pub const MAX_OFFLINE_INTEGRATION_MS: u64 = 168 * 60 * 60 * 1_000;
pub const FORMULA_DOMAIN: &[u8] = b"ae.autonomy.two-process.v1";
const MIN_WAKE_DELAY_MS: u64 = 60_000;
const ACTIVE_RECHECK_MS: u64 = 5 * 60_000;
const ASLEEP_RECHECK_MS: u64 = 60 * 60_000;
const CALM_RECHECK_MS: u64 = 6 * 60 * 60_000;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AutonomyError {
    #[error("unsupported autonomy schema")]
    UnsupportedSchema,
    #[error("invalid frozen time input: {0}")]
    InvalidFrozenTime(&'static str),
    #[error("persona scope mismatch")]
    ScopeMismatch,
}

pub fn validate_frozen_time(input: &FrozenTimeInputV1) -> Result<(), AutonomyError> {
    if input.schema_version != AUTONOMY_SCHEMA_VERSION {
        return Err(AutonomyError::UnsupportedSchema);
    }
    if input.persona_tzid.is_empty() || input.relation_tzid.is_empty() {
        return Err(AutonomyError::InvalidFrozenTime("empty tzid"));
    }
    if input.persona_local_minute >= 1_440 || input.relation_local_minute >= 1_440 {
        return Err(AutonomyError::InvalidFrozenTime("local minute"));
    }
    if input.effective_now_utc_ms < input.observed_now_utc_ms {
        return Err(AutonomyError::InvalidFrozenTime("effective clock"));
    }
    if input.budget_day_start_utc_ms >= input.budget_next_day_start_utc_ms
        || input.effective_now_utc_ms < input.budget_day_start_utc_ms
        || input.effective_now_utc_ms >= input.budget_next_day_start_utc_ms
    {
        return Err(AutonomyError::InvalidFrozenTime("budget day"));
    }
    if input.tzdb_fingerprint == [0; 32] {
        return Err(AutonomyError::InvalidFrozenTime("tzdb fingerprint"));
    }
    Ok(())
}

fn rational_approach(value: Fixed, target: Fixed, dt_ms: u64, tau_ms: u64) -> Fixed {
    let dt = i128::from(dt_ms);
    let tau = i128::from(tau_ms);
    let fraction = (dt * i128::from(SCALE)) / (tau + dt);
    let delta = (i128::from(target.raw() - value.raw()) * fraction) / i128::from(SCALE);
    Fixed::from_raw(value.raw().saturating_add(delta as i64)).clamp(Fixed::ZERO, Fixed::ONE)
}

fn wrap_minutes(raw: i64) -> i64 {
    raw.rem_euclid(1_440 * SCALE)
}
fn signed_phase_error(target: i64, current: i64) -> i64 {
    (target - current + 720 * SCALE).rem_euclid(1_440 * SCALE) - 720 * SCALE
}

/// Integer cosine surrogate with the correct extrema and symmetry.  Its
/// coefficients are schema-v1 constants, avoiding platform libm drift.
fn circadian_drive(phase: Fixed) -> Fixed {
    let minute = wrap_minutes(phase.raw()) / SCALE;
    let distance = if minute <= 720 {
        minute
    } else {
        1_440 - minute
    };
    Fixed::from_raw(SCALE - (2 * SCALE * distance / 720)).clamp(Fixed::from_raw(-SCALE), Fixed::ONE)
}

fn inner_id(state: &AutonomousRuntimeStateV1, frozen: &FrozenTimeInputV1, label: &[u8]) -> Id128 {
    let digest = wire::domain_hash(
        b"ae.inner-event.v1",
        &[
            &state.persona_scope,
            &frozen.effective_now_utc_ms.to_le_bytes(),
            label,
        ],
    );
    digest[..16].try_into().unwrap()
}

pub fn advance_temporal_state(
    old: &AutonomousRuntimeStateV1,
    profile: &PersonaTemporalProfileV1,
    frozen: &FrozenTimeInputV1,
    stimulus_arousal: Fixed,
) -> Result<WakeProposalV1, AutonomyError> {
    advance_temporal_state_with_stimulus(
        old,
        profile,
        frozen,
        &AutonomousStimulusV1 {
            arousal: stimulus_arousal,
            urgency: stimulus_arousal,
            emergency_authorized: false,
            source_digest: [0; 32],
        },
    )
}

pub fn advance_temporal_state_with_stimulus(
    old: &AutonomousRuntimeStateV1,
    profile: &PersonaTemporalProfileV1,
    frozen: &FrozenTimeInputV1,
    stimulus: &AutonomousStimulusV1,
) -> Result<WakeProposalV1, AutonomyError> {
    validate_frozen_time(frozen)?;
    if old.persona_scope != profile.persona_scope {
        return Err(AutonomyError::ScopeMismatch);
    }
    let elapsed_ms = frozen
        .effective_now_utc_ms
        .saturating_sub(old.last_advanced_at_utc_ms)
        .min(MAX_OFFLINE_INTEGRATION_MS);
    let was_asleep = old.sleep_state == SleepStateV1::Asleep;
    let process_s = if was_asleep {
        rational_approach(old.process_s, Fixed::ZERO, elapsed_ms, 4 * 60 * 60 * 1_000)
    } else {
        rational_approach(old.process_s, Fixed::ONE, elapsed_ms, 18 * 60 * 60 * 1_000)
    };
    let decayed_arousal =
        rational_approach(old.arousal, Fixed::ZERO, elapsed_ms, 2 * 60 * 60 * 1_000);
    let stimulus_arousal = stimulus
        .arousal
        .clamp(Fixed::ZERO, Fixed::from_raw(200_000));
    let stimulus_urgency = stimulus.urgency.clamp(Fixed::ZERO, Fixed::ONE);
    let emergency = stimulus.emergency_authorized
        && stimulus_arousal.max(stimulus_urgency) >= Fixed::from_raw(900_000);
    let arousal = decayed_arousal
        .saturating_add(stimulus_arousal)
        .clamp(Fixed::ZERO, Fixed::ONE);
    let elapsed_minutes_raw = i64::try_from(elapsed_ms / 60_000)
        .unwrap_or(i64::MAX)
        .saturating_mul(SCALE);
    let target_raw = i64::from(frozen.persona_local_minute)
        .saturating_sub(i64::from(profile.preferred_wake_local_minute))
        .rem_euclid(1_440)
        .saturating_mul(SCALE);
    let current_raw = wrap_minutes(
        old.circadian_phase_minutes
            .raw()
            .saturating_add(elapsed_minutes_raw),
    );
    let error = signed_phase_error(target_raw, current_raw);
    let max_adjust = i64::try_from(
        (i128::from(profile.entrainment_rate_minutes_per_day)
            * i128::from(elapsed_ms)
            * i128::from(SCALE))
            / i128::from(86_400_000u64),
    )
    .unwrap_or(i64::MAX);
    let phase = Fixed::from_raw(wrap_minutes(
        current_raw.saturating_add(error.clamp(-max_adjust, max_adjust)),
    ));
    let process_c = circadian_drive(phase);
    let q = process_s
        .saturating_sub(
            process_c
                .checked_mul(Fixed::from_raw(250_000))
                .unwrap_or(Fixed::ZERO),
        )
        .saturating_sub(arousal)
        .clamp(Fixed::ZERO, Fixed::ONE);
    let (sleep_state, held) = match old.sleep_state {
        SleepStateV1::Awake if q >= Fixed::from_raw(620_000) => {
            let held = old.sleep_threshold_held_ms.saturating_add(elapsed_ms);
            if held >= 600_000 {
                (SleepStateV1::Drowsy, 0)
            } else {
                (SleepStateV1::Awake, held)
            }
        }
        SleepStateV1::Drowsy if q >= Fixed::from_raw(720_000) => {
            let held = old.sleep_threshold_held_ms.saturating_add(elapsed_ms);
            if held >= 900_000 {
                (SleepStateV1::Asleep, 0)
            } else {
                (SleepStateV1::Drowsy, held)
            }
        }
        SleepStateV1::Drowsy if q < Fixed::from_raw(550_000) => (SleepStateV1::Awake, 0),
        SleepStateV1::Asleep if q <= Fixed::from_raw(380_000) || emergency => {
            (SleepStateV1::Awake, 0)
        }
        state => (state, 0),
    };
    let intensity = if emergency {
        WakeIntensityV1::Emergency
    } else if old.unfinished_topic_salience >= Fixed::from_raw(650_000)
        && sleep_state != SleepStateV1::Asleep
    {
        WakeIntensityV1::Ignition
    } else if old.affiliation_need >= Fixed::from_raw(450_000) {
        WakeIntensityV1::Associative
    } else {
        WakeIntensityV1::Micro
    };
    let mut state = old.clone();
    state.generation = old.generation.saturating_add(1);
    state.state_revision = old.state_revision.saturating_add(1);
    state.last_advanced_at_utc_ms = frozen.effective_now_utc_ms;
    state.process_s = process_s;
    state.process_c = process_c;
    state.arousal = arousal;
    state.sleep_state = sleep_state;
    state.sleep_threshold_held_ms = held;
    state.circadian_phase_minutes = phase;
    state.wake_intensity = intensity;
    state.next_wake_at_utc_ms = next_wake_utc_ms(&state, intensity);
    let mut events = vec![InnerEventV1 {
        schema_version: 1,
        event_id: inner_id(&state, frozen, b"homeostasis"),
        persona_scope: state.persona_scope,
        kind: InnerEventKindV1::HomeostasisChanged,
        committed_at_utc_ms: frozen.effective_now_utc_ms,
        summary_code: "two_process_advanced".into(),
        value_before: Some(old.process_s),
        value_after: Some(process_s),
        source_event_ids: vec![],
        tombstoned: false,
    }];
    if sleep_state != old.sleep_state {
        events.push(InnerEventV1 {
            schema_version: 1,
            event_id: inner_id(&state, frozen, b"sleep"),
            persona_scope: state.persona_scope,
            kind: InnerEventKindV1::SleepTransition,
            committed_at_utc_ms: frozen.effective_now_utc_ms,
            summary_code: format!("{:?}_to_{:?}", old.sleep_state, sleep_state).to_lowercase(),
            value_before: None,
            value_after: None,
            source_event_ids: vec![],
            tombstoned: false,
        });
    }
    if frozen.effective_now_utc_ms < old.last_advanced_at_utc_ms {
        events.push(InnerEventV1 {
            schema_version: 1,
            event_id: inner_id(&state, frozen, b"rollback"),
            persona_scope: state.persona_scope,
            kind: InnerEventKindV1::ClockRollback,
            committed_at_utc_ms: frozen.effective_now_utc_ms,
            summary_code: "clock_rollback_clamped".into(),
            value_before: None,
            value_after: None,
            source_event_ids: vec![],
            tombstoned: false,
        });
    }
    if frozen
        .effective_now_utc_ms
        .saturating_sub(old.last_advanced_at_utc_ms)
        > MAX_OFFLINE_INTEGRATION_MS
    {
        events.push(InnerEventV1 {
            schema_version: 1,
            event_id: inner_id(&state, frozen, b"offline"),
            persona_scope: state.persona_scope,
            kind: InnerEventKindV1::OfflineGap,
            committed_at_utc_ms: frozen.effective_now_utc_ms,
            summary_code: "offline_gap_bounded_168h".into(),
            value_before: None,
            value_after: None,
            source_event_ids: vec![],
            tombstoned: false,
        });
    }
    Ok(WakeProposalV1 {
        state,
        inner_events: events,
        intentions: vec![],
    })
}

pub fn next_wake_utc_ms(state: &AutonomousRuntimeStateV1, intensity: WakeIntensityV1) -> u64 {
    let interval = match (state.sleep_state, intensity) {
        (SleepStateV1::Asleep, _) => ASLEEP_RECHECK_MS,
        (
            _,
            WakeIntensityV1::Ignition | WakeIntensityV1::Emergency | WakeIntensityV1::Associative,
        ) => ACTIVE_RECHECK_MS,
        _ => CALM_RECHECK_MS,
    };
    state
        .last_advanced_at_utc_ms
        .saturating_add(interval.max(MIN_WAKE_DELAY_MS))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> PersonaTemporalProfileV1 {
        PersonaTemporalProfileV1 {
            schema_version: 1,
            persona_scope: [1; 32],
            home_timezone: "America/Los_Angeles".into(),
            current_timezone: "America/Los_Angeles".into(),
            chronotype: ChronotypeV1::NightOwl,
            preferred_sleep_local_minute: 90,
            preferred_wake_local_minute: 570,
            sleep_flex_minutes: 120,
            entrainment_rate_minutes_per_day: 90,
            revision: 1,
        }
    }
    fn state(now: u64) -> AutonomousRuntimeStateV1 {
        AutonomousRuntimeStateV1 {
            schema_version: 1,
            persona_scope: [1; 32],
            relation_scope: Some([2; 32]),
            generation: 0,
            state_revision: 0,
            last_advanced_at_utc_ms: now,
            next_wake_at_utc_ms: now,
            wake_intensity: WakeIntensityV1::Micro,
            sleep_state: SleepStateV1::Awake,
            process_s: Fixed::from_raw(400_000),
            process_c: Fixed::ONE,
            arousal: Fixed::ZERO,
            sleep_threshold_held_ms: 0,
            circadian_phase_minutes: Fixed::ZERO,
            affiliation_need: Fixed::from_raw(100_000),
            unfinished_topic_salience: Fixed::ZERO,
            social_energy: Fixed::ONE,
            formula_digest: [3; 32],
            mapping_digest: [4; 32],
            workspace_residual: Fixed::ZERO,
        }
    }
    fn frozen(now: u64, minute: u16) -> FrozenTimeInputV1 {
        FrozenTimeInputV1 {
            schema_version: 1,
            observed_now_utc_ms: now,
            effective_now_utc_ms: now,
            persona_tzid: "America/Los_Angeles".into(),
            persona_utc_offset_seconds: -25_200,
            persona_local_minute: minute,
            persona_day_ordinal: 1,
            relation_tzid: "Asia/Shanghai".into(),
            relation_utc_offset_seconds: 28_800,
            relation_local_minute: (minute + 900) % 1440,
            relation_day_ordinal: 2,
            budget_day_start_utc_ms: now - (now % 86_400_000),
            budget_next_day_start_utc_ms: now - (now % 86_400_000) + 86_400_000,
            next_timezone_transition_utc_ms: None,
            tzdb_fingerprint: [5; 32],
        }
    }
    #[test]
    fn deterministic() {
        let s = state(1_000);
        let f = frozen(3_601_000, 100);
        assert_eq!(
            advance_temporal_state(&s, &profile(), &f, Fixed::ZERO).unwrap(),
            advance_temporal_state(&s, &profile(), &f, Fixed::ZERO).unwrap()
        );
    }
    #[test]
    fn rollback_is_zero() {
        let s = state(5_000);
        let mut f = frozen(5_000, 0);
        f.observed_now_utc_ms = 4_000;
        assert_eq!(
            advance_temporal_state(&s, &profile(), &f, Fixed::ZERO)
                .unwrap()
                .state
                .process_s,
            s.process_s
        );
    }
    #[test]
    fn long_gap_is_bounded() {
        let s = state(1);
        let f = frozen(30 * 86_400_000 + 1, 0);
        let p = advance_temporal_state(&s, &profile(), &f, Fixed::ZERO).unwrap();
        assert!(p
            .inner_events
            .iter()
            .any(|e| e.kind == InnerEventKindV1::OfflineGap));
        assert_eq!(p.state.last_advanced_at_utc_ms, f.effective_now_utc_ms);
    }
    #[test]
    fn travel_phase_is_bounded() {
        let s = state(1);
        let f = frozen(86_400_001, 1_000);
        let p = advance_temporal_state(&s, &profile(), &f, Fixed::ZERO).unwrap();
        let ordinary_day = 1_440 * SCALE;
        let adjustment =
            (p.state.circadian_phase_minutes.raw() - ordinary_day).rem_euclid(1_440 * SCALE);
        assert!(adjustment <= 90 * SCALE || adjustment >= 1_350 * SCALE);
    }
    #[test]
    fn process_a_stays_bounded_while_authorized_urgency_wakes_sleep() {
        let mut sleeping = state(1);
        sleeping.sleep_state = SleepStateV1::Asleep;
        sleeping.process_s = Fixed::ONE;
        let proposal = advance_temporal_state_with_stimulus(
            &sleeping,
            &profile(),
            &frozen(60_001, 120),
            &AutonomousStimulusV1 {
                arousal: Fixed::ONE,
                urgency: Fixed::ONE,
                emergency_authorized: true,
                source_digest: [9; 32],
            },
        )
        .unwrap();
        assert_eq!(proposal.state.arousal, Fixed::from_raw(200_000));
        assert_eq!(proposal.state.sleep_state, SleepStateV1::Awake);
        assert_eq!(proposal.state.wake_intensity, WakeIntensityV1::Emergency);
    }
}
