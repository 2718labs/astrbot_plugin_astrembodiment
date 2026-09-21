//! Persona-local time contracts. All time samples are supplied once by Host.
use crate::*;
use serde::{Deserialize, Serialize};
pub const KIND_EMBODIMENT_TIME_ADVANCE: u8 = 11;
pub const EMBODIMENT_TIME_ADVANCE_WIRE_VERSION_V1: u16 = 1;
pub const EMBODIMENT_MAX_HORIZON_MS: u64 = 604_800_000;
pub const EMBODIMENT_TZDB_RELEASE: &str = "2026c";
pub const EMBODIMENT_TZDB_SHA256: &str =
    "762e0caecd4eb713fcf2f24b0e38f4eb819da4973c9d69d67edb81c119204206";
const TZDB_BYTES: &[u8] = include_bytes!("../../../astr_embodiment/assets/tzdb/tzdb-2026c.bin");
const TZDB_MANIFEST: &str = include_str!("../../../astr_embodiment/assets/tzdb/manifest.json");

pub fn embodiment_fixed_offset(tzid: &str) -> Option<i32> {
    if tzid == "UTC" {
        return Some(0);
    }
    let b = tzid.as_bytes();
    if b.len() != 9
        || &b[..3] != b"UTC"
        || ![b'+', b'-'].contains(&b[3])
        || b[6] != b':'
        || ![b[4], b[5], b[7], b[8]].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let hours = i32::from(b[4] - b'0') * 10 + i32::from(b[5] - b'0');
    let minutes = i32::from(b[7] - b'0') * 10 + i32::from(b[8] - b'0');
    if hours > 14 || minutes > 59 || hours == 14 && minutes != 0 || hours == 0 && minutes == 0 {
        return None;
    }
    Some((hours * 3600 + minutes * 60) * if b[3] == b'+' { 1 } else { -1 })
}
fn tzdb_zones() -> Result<&'static std::collections::BTreeMap<String, tz::TimeZone>, String> {
    use sha2::{Digest as _, Sha256};
    static ZONES: std::sync::OnceLock<
        Result<std::collections::BTreeMap<String, tz::TimeZone>, String>,
    > = std::sync::OnceLock::new();
    ZONES
        .get_or_init(|| {
            let digest: Digest = Sha256::digest(TZDB_BYTES).into();
            if hex::encode32(&digest) != EMBODIMENT_TZDB_SHA256 || !TZDB_BYTES.starts_with(b"AETZ1")
            {
                return Err("TZDB_DIGEST_MISMATCH".into());
            }
            let mut cursor = 5usize;
            fn take<'a>(data: &'a [u8], cursor: &mut usize, n: usize) -> Result<&'a [u8], String> {
                let end = cursor.checked_add(n).ok_or("TZDB_FORMAT_INVALID")?;
                let v = data.get(*cursor..end).ok_or("TZDB_FORMAT_INVALID")?;
                *cursor = end;
                Ok(v)
            }
            let count = u32::from_le_bytes(take(TZDB_BYTES, &mut cursor, 4)?.try_into().unwrap());
            if count > 2048 {
                return Err("TZDB_FORMAT_INVALID".into());
            }
            let mut zones = std::collections::BTreeMap::new();
            for _ in 0..count {
                let len = u16::from_le_bytes(take(TZDB_BYTES, &mut cursor, 2)?.try_into().unwrap())
                    as usize;
                let name = std::str::from_utf8(take(TZDB_BYTES, &mut cursor, len)?)
                    .map_err(|_| "TZDB_FORMAT_INVALID")?
                    .to_owned();
                let len = u32::from_le_bytes(take(TZDB_BYTES, &mut cursor, 4)?.try_into().unwrap())
                    as usize;
                let zone = tz::TimeZone::from_tz_data(take(TZDB_BYTES, &mut cursor, len)?)
                    .map_err(|_| "TZDB_FORMAT_INVALID")?;
                if zones.insert(name, zone).is_some() {
                    return Err("TZDB_FORMAT_INVALID".into());
                }
            }
            if cursor != TZDB_BYTES.len() {
                return Err("TZDB_FORMAT_INVALID".into());
            }
            Ok(zones)
        })
        .as_ref()
        .map_err(Clone::clone)
}
pub fn canonical_embodiment_timezone(tzid: &str) -> Result<String, String> {
    if embodiment_fixed_offset(tzid).is_some() {
        return Ok(tzid.into());
    }
    let manifest: serde_json::Value =
        serde_json::from_str(TZDB_MANIFEST).map_err(|_| "TZDB_MANIFEST_INVALID")?;
    let name = manifest["aliases"][tzid].as_str().unwrap_or(tzid);
    if !tzdb_zones()?.contains_key(name) {
        return Err("UNKNOWN_PERSONA_TIMEZONE".into());
    }
    Ok(name.into())
}
fn rule_time(day: &tz::timezone::RuleDay, year: i32, seconds: i64) -> Result<i64, String> {
    use tz::timezone::RuleDay;
    let january = tz::UtcDateTime::new(year, 1, 1, 0, 0, 0, 0)
        .map_err(|_| "FROZEN_TIME_RANGE")?
        .unix_time();
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let start = match day {
        RuleDay::Julian0WithLeap(d) => january + i64::from(d.get()) * 86400,
        RuleDay::Julian1WithoutLeap(d) => {
            january + (i64::from(d.get()) - 1 + i64::from(leap && d.get() >= 60)) * 86400
        }
        RuleDay::MonthWeekDay(d) => {
            let month = tz::UtcDateTime::new(year, d.month(), 1, 0, 0, 0, 0)
                .map_err(|_| "FROZEN_TIME_RANGE")?
                .unix_time();
            let first_weekday = (month.div_euclid(86400) + 4).rem_euclid(7);
            let mut ordinal = (i64::from(d.week_day()) - first_weekday).rem_euclid(7)
                + 7 * (i64::from(d.week()) - 1);
            let days = match d.month() {
                2 => {
                    if leap {
                        29
                    } else {
                        28
                    }
                }
                4 | 6 | 9 | 11 => 30,
                _ => 31,
            };
            if ordinal >= days {
                ordinal -= 7;
            }
            month + ordinal * 86400
        }
    };
    start
        .checked_add(seconds)
        .ok_or_else(|| "FROZEN_TIME_RANGE".into())
}
pub fn freeze_embodiment_time_v1(
    tzid: &str,
    now_utc_ms: u64,
) -> Result<FrozenPersonaTimeV1, String> {
    if now_utc_ms == 0 || now_utc_ms > i64::MAX as u64 {
        return Err("INVALID_FROZEN_TIME".into());
    }
    let tzid = canonical_embodiment_timezone(tzid)?;
    let now = (now_utc_ms / 1000) as i64;
    let mut next = None;
    let offset = if let Some(v) = embodiment_fixed_offset(&tzid) {
        v
    } else {
        let zone = &tzdb_zones()?[&tzid];
        let zref = zone.as_ref();
        let offset = zone
            .find_local_time_type(now)
            .map_err(|_| "FROZEN_TIME_RANGE")?
            .ut_offset();
        let mut candidates: Vec<i64> = zref
            .transitions()
            .iter()
            .map(|t| t.unix_leap_time())
            .filter(|t| *t > now)
            .collect();
        if let Some(tz::timezone::TransitionRule::Alternate(rule)) = zref.extra_rule() {
            let year = tz::UtcDateTime::from_timespec(now, 0)
                .map_err(|_| "FROZEN_TIME_RANGE")?
                .year();
            for y in [year - 1, year, year + 1, year + 2] {
                for t in [
                    rule_time(
                        rule.dst_start(),
                        y,
                        i64::from(rule.dst_start_time()) - i64::from(rule.std().ut_offset()),
                    )?,
                    rule_time(
                        rule.dst_end(),
                        y,
                        i64::from(rule.dst_end_time()) - i64::from(rule.dst().ut_offset()),
                    )?,
                ] {
                    if t > now
                        && zref
                            .transitions()
                            .last()
                            .is_none_or(|last| t > last.unix_leap_time())
                    {
                        candidates.push(t);
                    }
                }
            }
        }
        next = candidates.into_iter().min().map(|t| (t as u64) * 1000);
        offset
    };
    let local = now
        .checked_add(i64::from(offset))
        .ok_or("FROZEN_TIME_RANGE")?;
    Ok(FrozenPersonaTimeV1 {
        schema_version: 1,
        now_utc_ms,
        persona_tzid: tzid,
        persona_utc_offset_seconds: offset,
        persona_local_minute: (local.rem_euclid(86400) / 60) as u16,
        persona_day_ordinal: local
            .div_euclid(86400)
            .try_into()
            .map_err(|_| "FROZEN_TIME_RANGE")?,
        next_timezone_transition_utc_ms: next,
        tzdb_release: EMBODIMENT_TZDB_RELEASE.into(),
        tzdb_content_sha256: hex::decode32(EMBODIMENT_TZDB_SHA256)?,
    })
}
impl FrozenPersonaTimeV1 {
    pub fn validate_v1(&self) -> bool {
        freeze_embodiment_time_v1(&self.persona_tzid, self.now_utc_ms).is_ok_and(|v| v == *self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbodimentTemporalProfileV1 {
    pub schema_version: u16,
    pub persona_tzid: String,
    pub tzdb_release: String,
    #[serde(with = "crate::hex::d32")]
    pub tzdb_content_sha256: Digest,
    pub circadian_period_millis: u64,
    pub homeostatic_awake_gain_per_hour: Fixed,
    pub homeostatic_asleep_decay_per_hour: Fixed,
    pub drowsy_enter_threshold: Fixed,
    pub drowsy_exit_threshold: Fixed,
    pub endogenous_phase_hysteresis: Fixed,
    pub maximum_analytic_horizon_ms: u64,
}
impl EmbodimentTemporalProfileV1 {
    pub fn validate_v1(&self) -> bool {
        self.schema_version == 1
            && !self.persona_tzid.is_empty()
            && self.persona_tzid.len() <= 128
            && !self.tzdb_release.is_empty()
            && self.tzdb_release.len() <= 32
            && self.tzdb_content_sha256 != [0; 32]
            && self.tzdb_release == EMBODIMENT_TZDB_RELEASE
            && hex::encode32(&self.tzdb_content_sha256) == EMBODIMENT_TZDB_SHA256
            && freeze_embodiment_time_v1(&self.persona_tzid, 1).is_ok_and(|f| f.persona_tzid == self.persona_tzid)
            && (72_000_000..=100_800_000).contains(&self.circadian_period_millis)
            && self.maximum_analytic_horizon_ms == EMBODIMENT_MAX_HORIZON_MS
            && [
                self.homeostatic_awake_gain_per_hour,
                self.homeostatic_asleep_decay_per_hour,
                self.drowsy_enter_threshold,
                self.drowsy_exit_threshold,
                self.endogenous_phase_hysteresis,
            ]
            .iter()
            .all(|v| (0..=1_000_000).contains(&v.raw()))
            && self.homeostatic_awake_gain_per_hour > Fixed::ZERO
            && self.homeostatic_asleep_decay_per_hour > Fixed::ZERO
            && self.drowsy_exit_threshold < self.drowsy_enter_threshold
            && self.endogenous_phase_hysteresis > Fixed::ZERO
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SleepScheduleModeV1 {
    Auto,
    Fixed,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbodimentSleepScheduleV1 {
    pub schema_version: u16,
    pub mode: SleepScheduleModeV1,
    pub chronotype: ChronotypeV1,
    pub preferred_sleep_local_minute: u16,
    pub preferred_wake_local_minute: u16,
    pub sleep_flex_minutes: u16,
    pub entrainment_rate_minutes_per_day: u16,
}
impl EmbodimentSleepScheduleV1 {
    pub fn validate_v1(&self) -> bool {
        self.schema_version == 1
            && self.preferred_sleep_local_minute < 1440
            && self.preferred_wake_local_minute < 1440
            && self.preferred_sleep_local_minute != self.preferred_wake_local_minute
            && self.sleep_flex_minutes <= 720
            && self.entrainment_rate_minutes_per_day <= 720
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenPersonaTimeV1 {
    pub schema_version: u16,
    pub now_utc_ms: u64,
    pub persona_tzid: String,
    pub persona_utc_offset_seconds: i32,
    pub persona_local_minute: u16,
    pub persona_day_ordinal: i32,
    pub next_timezone_transition_utc_ms: Option<u64>,
    pub tzdb_release: String,
    #[serde(with = "crate::hex::d32")]
    pub tzdb_content_sha256: Digest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateEmbodimentPersonaIfMissingV1 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d16")]
    pub operation_id: Id128,
    pub scope: PersonaScopeRef,
    #[serde(with = "crate::hex::d32")]
    pub incarnation_digest: Digest,
    pub profile_template: EmbodimentTemporalProfileV1,
    pub sleep_schedule: EmbodimentSleepScheduleV1,
    pub frozen_creation: FrozenPersonaTimeV1,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareAndSwapEmbodimentProfileV1 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d16")]
    pub operation_id: Id128,
    pub scope: PersonaScopeRef,
    pub expected_profile_revision: u64,
    pub expected_schedule_revision: u64,
    pub replacement_profile: EmbodimentTemporalProfileV1,
    pub replacement_schedule: EmbodimentSleepScheduleV1,
    pub frozen: FrozenPersonaTimeV1,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbodimentClockStatusRequestV1 {
    pub schema_version: u16,
    pub scope: PersonaScopeRef,
    pub profile_revision: u64,
    pub schedule_revision: u64,
    pub frozen: FrozenPersonaTimeV1,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbodimentTimeAdvanceRequestV1 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d16")]
    pub operation_id: Id128,
    pub scope: PersonaScopeRef,
    pub profile_revision: u64,
    pub schedule_revision: u64,
    pub frozen: FrozenPersonaTimeV1,
}
impl EmbodimentTimeAdvanceRequestV1 {
    pub fn expected_operation_id(&self) -> Id128 {
        core_id(
            b"ae.embodiment-time.operation-id.v1",
            &[
                &core_persona_digest(&self.scope),
                &self.frozen.now_utc_ms.to_le_bytes(),
                &self.profile_revision.to_le_bytes(),
                &self.schedule_revision.to_le_bytes(),
                &self.frozen.tzdb_content_sha256,
            ],
        )
    }
    pub fn encode_wire_v1(&self) -> Result<Vec<u8>, String> {
        let mut bytes = wire::WIRE_SCHEMA_VERSION.to_le_bytes().to_vec();
        bytes.push(KIND_EMBODIMENT_TIME_ADVANCE);
        bytes.extend(EMBODIMENT_TIME_ADVANCE_WIRE_VERSION_V1.to_le_bytes());
        bytes.extend(core_encode(self)?);
        Ok(bytes)
    }
    pub fn decode_wire_v1(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 65536 || !bytes.starts_with(&[5, 0, 11, 1, 0]) {
            return Err("EMBODIMENT_WIRE_VERSION".into());
        }
        core_decode(&bytes[5..])
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListEmbodimentPersonasV1 {
    pub schema_version: u16,
    pub limit: u16,
    #[serde(with = "crate::hex::d32_opt")]
    pub snapshot_token: Option<Digest>,
    pub after: Option<PersonaScopeRef>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbodimentPersonaInventoryEntryV1 {
    pub scope: PersonaScopeRef,
    pub binding_revision: u64,
    #[serde(with = "crate::hex::d32")]
    pub persona_scope: Digest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbodimentPersonaInventoryPageV1 {
    pub entries: Vec<EmbodimentPersonaInventoryEntryV1>,
    #[serde(with = "crate::hex::d32")]
    pub snapshot_token: Digest,
    pub epoch: u64,
    pub entry_count: u64,
    pub next_after: Option<PersonaScopeRef>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbodimentProfileReadV1 {
    pub profile: EmbodimentTemporalProfileV1,
    pub schedule: EmbodimentSleepScheduleV1,
    pub profile_revision: u64,
    pub schedule_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbodimentClockStateV1 {
    pub schema_version: u16,
    pub process_s: Fixed,
    pub process_c: Fixed,
    pub arousal: Fixed,
    pub circadian_phase_minutes: Fixed,
    pub sleep_started_at_utc_ms: u64,
    pub entrainment_shift_milliminutes: i64,
    pub homeostatic_remainder: i64,
    #[serde(with = "crate::hex::d32")]
    pub matrix_state_digest: Digest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbodimentClockHeadV1 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d32")]
    pub persona_scope: Digest,
    pub sequence: u64,
    pub compacted_count: u64,
    pub recent_count: u16,
    pub time_revision: u64,
    pub state_revision: u64,
    pub profile_revision: u64,
    pub schedule_revision: u64,
    pub tzdb_release: String,
    #[serde(with = "crate::hex::d32")]
    pub tzdb_content_sha256: Digest,
    pub last_now_utc_ms: u64,
    pub next_due_at_utc_ms: u64,
    pub compacted_through_now_utc_ms: u64,
    pub sleep_state: SleepStateV1,
    pub sleep_episode_ordinal: u64,
    pub dream_consolidated_episode_ordinal: u64,
    pub endogenous_phase_code: u8,
    pub matrix_epoch: MatrixTimeEpochV1,
    #[serde(with = "crate::hex::d32")]
    pub matrix_anchor_graph_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub formula_digest: Digest,
    pub state: EmbodimentClockStateV1,
    #[serde(with = "crate::hex::d32")]
    pub compacted_chain_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub recent_chain_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub head_digest: Digest,
}
impl EmbodimentClockHeadV1 {
    pub fn digest_v1(&self) -> Result<Digest, String> {
        let mut fields = serde_json::to_value(self).map_err(|e| e.to_string())?;
        let fields = fields.as_object_mut().ok_or("CLOCK_HEAD_WIRE_INVALID")?;
        fields.remove("head_digest");
        fields.remove("compacted_chain_digest");
        fields.remove("recent_chain_digest");
        Ok(wire::domain_hash(
            b"ae.embodiment.clock-head.v1",
            &[
                &core_encode(fields)?,
                &self.compacted_chain_digest,
                &self.recent_chain_digest,
            ],
        ))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbodimentPersonaAnchorReceiptV1 {
    pub schema_version: u16,
    pub scope: PersonaScopeRef,
    #[serde(with = "crate::hex::d16")]
    pub operation_id: Id128,
    #[serde(with = "crate::hex::d32")]
    pub incarnation_digest: Digest,
    pub binding_revision: u64,
    pub initial_semantic_revision: u64,
    pub anchored_at_utc_ms: u64,
    #[serde(with = "crate::hex::d32")]
    pub legacy_sleep_anchor_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub profile_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub schedule_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub initial_state_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub request_digest: Digest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum EmbodimentPersonaLookupV1 {
    Missing {
        #[serde(with = "crate::hex::d32")]
        incarnation_digest: Digest,
    },
    Present {
        first_creation_receipt: EmbodimentPersonaAnchorReceiptV1,
        #[serde(with = "crate::hex::d32")]
        incarnation_digest: Digest,
        profile_revision: u64,
        schedule_revision: u64,
        clock_head: EmbodimentClockHeadV1,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbodimentPersonaCreateOutcomeV1 {
    pub commit_status: CoreCommitStatusV1,
    pub first_creation_receipt: EmbodimentPersonaAnchorReceiptV1,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbodimentClockMutationKindV1 {
    Advance,
    ProfileCas,
    SemanticAnchor,
}
impl EmbodimentClockMutationKindV1 {
    pub fn code(self) -> u8 {
        match self {
            Self::Advance => 1,
            Self::ProfileCas => 2,
            Self::SemanticAnchor => 3,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbodimentCadenceV1 {
    Active,
    Asleep,
    Calm,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbodimentClockResultCoreV1 {
    pub schema_version: u16,
    pub mutation_kind: EmbodimentClockMutationKindV1,
    pub scope: PersonaScopeRef,
    #[serde(with = "crate::hex::d16")]
    pub operation_id: Id128,
    #[serde(with = "crate::hex::d16")]
    pub event_id: Id128,
    #[serde(with = "crate::hex::d32")]
    pub request_digest: Digest,
    pub sequence: u64,
    pub time_revision: u64,
    pub state_revision: u64,
    pub profile_revision: u64,
    pub schedule_revision: u64,
    pub matrix_semantic_revision: u64,
    pub frozen_now_utc_ms: u64,
    pub raw_elapsed_ms: u64,
    pub applied_elapsed_ms: u64,
    pub capped_gap: bool,
    pub discrete_event_mask: u32,
    pub sleep_state: SleepStateV1,
    pub next_due_at_utc_ms: u64,
    pub cadence_class: EmbodimentCadenceV1,
    #[serde(with = "crate::hex::d32")]
    pub resulting_state_digest: Digest,
    #[serde(with = "crate::hex::d32_opt")]
    pub semantic_proof_digest: Option<Digest>,
    #[serde(with = "crate::hex::d32_opt")]
    pub semantic_commitment_digest: Option<Digest>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbodimentClockCommitReceiptV1 {
    pub result: EmbodimentClockResultCoreV1,
    #[serde(with = "crate::hex::d32")]
    pub result_core_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub leaf_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub chain_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub head_digest: Digest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbodimentClockCommitOutcomeV1 {
    pub commit_status: CoreCommitStatusV1,
    pub receipt: EmbodimentClockCommitReceiptV1,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum EmbodimentClockStatusV1 {
    NotDue {
        next_due_at_utc_ms: u64,
        cadence_class: EmbodimentCadenceV1,
        #[serde(with = "crate::hex::d32")]
        clock_head_digest: Digest,
    },
    Due {
        exact_advance_request_bytes: Vec<u8>,
        #[serde(with = "crate::hex::d32")]
        request_digest: Digest,
        next_due_at_utc_ms: u64,
        #[serde(with = "crate::hex::d32")]
        clock_head_digest: Digest,
    },
}

/// Opaque historical verifier. It never returns an active CanonicalEvent.
pub mod legacy_audit {
    use crate::{wire, CanonicalEvent, Digest};
    pub struct VerifiedLegacyTimeV1 {
        bytes: Vec<u8>,
        digest: Digest,
    }
    impl VerifiedLegacyTimeV1 {
        pub fn bytes(&self) -> &[u8] {
            &self.bytes
        }
        pub fn digest(&self) -> Digest {
            self.digest
        }
    }
    pub fn verify_time_v1(bytes: &[u8]) -> Result<VerifiedLegacyTimeV1, String> {
        if bytes.len() > 65536 {
            return Err("LEGACY_TIME_LIMIT".into());
        }
        let event = wire::decode_event(bytes).map_err(|e| e.to_string())?;
        if !matches!(event, CanonicalEvent::TimeAdvance(_)) {
            return Err("LEGACY_TIME_KIND".into());
        }
        Ok(VerifiedLegacyTimeV1 {
            bytes: bytes.to_vec(),
            digest: wire::event_digest(&event),
        })
    }
}
