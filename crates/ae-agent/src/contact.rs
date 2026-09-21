use ae_contracts::{
    wire, ContactChannelV1, ContactIntentionBasisV1, ContactPurposeV1, Digest, DurableIntentionV1,
    EffectiveFrequencyBandV1, EffectiveFrequencyV1, FrequencySelectionReasonV1, Id128,
    IntentionStateV1, InteractionFactKindV1, InteractionFactV1, InteractionSourceAuthorityV1,
    RelationBudgetLedgerV1, RelationConsentStateV1, RelationConsentV1, RelationContactProcessV1,
    RelationTemporalPolicyV1,
};
use ae_fixed::{Fixed, SCALE};

pub const CONTACT_DUE_THRESHOLD_V1: Fixed = Fixed::from_raw(500_000);
const MIN_DUE_WINDOW_MS: u64 = 3_600_000;
const REPETITION_LIMIT_RAW: i64 = 750_000;
const HOUR_MS: u64 = 3_600_000;
const DAY_MS: u64 = 24 * HOUR_MS;
const AUTO_POLICY_VERSION_V1: u16 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrequencyEvaluationErrorV1 {
    InvalidPolicy,
    ScopeMismatch,
    FutureContact,
    InvalidLedger,
    ArithmeticOverflow,
}

#[derive(Clone, Copy, Debug)]
pub struct FrequencyEvidenceV1<'a> {
    pub policy: &'a RelationTemporalPolicyV1,
    pub contact: &'a RelationContactProcessV1,
    pub ledger: &'a RelationBudgetLedgerV1,
    pub effective_now_utc_ms: u64,
    pub authoritative_daily_submitted: u16,
}

fn append_evidence_field(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&(value.len() as u64).to_le_bytes());
    output.extend_from_slice(value);
}

fn append_optional_u64(output: &mut Vec<u8>, value: Option<u64>) {
    match value {
        Some(value) => {
            append_evidence_field(output, &[1]);
            append_evidence_field(output, &value.to_le_bytes());
        }
        None => append_evidence_field(output, &[0]),
    }
}

fn frequency_evidence_digest_v1(
    input: &FrequencyEvidenceV1<'_>,
    result: &EffectiveFrequencyV1,
) -> Digest {
    let policy = input.policy;
    let contact = input.contact;
    let ledger = input.ledger;
    let mut encoded = Vec::with_capacity(512);
    append_evidence_field(&mut encoded, &policy.schema_version.to_le_bytes());
    append_evidence_field(&mut encoded, &policy.relation_scope);
    append_evidence_field(&mut encoded, policy.user_timezone.as_bytes());
    append_evidence_field(&mut encoded, &[policy.timezone_source as u8]);
    append_evidence_field(&mut encoded, &policy.quiet_hours_start_minute.to_le_bytes());
    append_evidence_field(&mut encoded, &policy.quiet_hours_end_minute.to_le_bytes());
    append_evidence_field(&mut encoded, &[policy.quiet_hours_emergency_bypass as u8]);
    append_evidence_field(&mut encoded, &[policy.proactive_enabled as u8]);
    append_evidence_field(&mut encoded, &policy.proactive_daily_max.to_le_bytes());
    append_evidence_field(
        &mut encoded,
        &policy.min_proactive_cooldown_ms.to_le_bytes(),
    );
    append_evidence_field(&mut encoded, &policy.intention_ttl_ms.to_le_bytes());
    append_evidence_field(
        &mut encoded,
        &policy.unanswered_backoff_base_ms.to_le_bytes(),
    );
    append_evidence_field(&mut encoded, &policy.unanswered_hard_stop.to_le_bytes());
    append_evidence_field(
        &mut encoded,
        &policy.emergency_threshold.raw().to_le_bytes(),
    );
    append_evidence_field(&mut encoded, &policy.daily_submitted.to_le_bytes());
    append_evidence_field(&mut encoded, &policy.consecutive_unanswered.to_le_bytes());
    append_optional_u64(&mut encoded, policy.last_inbound_utc_ms);
    append_optional_u64(&mut encoded, policy.last_proactive_submitted_utc_ms);
    append_evidence_field(&mut encoded, &policy.revision.to_le_bytes());
    append_evidence_field(&mut encoded, &policy.auto_policy_version.to_le_bytes());
    append_evidence_field(
        &mut encoded,
        &policy.next_claim_reservation_tokens.to_le_bytes(),
    );
    append_evidence_field(&mut encoded, &contact.schema_version.to_le_bytes());
    append_evidence_field(&mut encoded, &contact.relation_scope);
    append_evidence_field(&mut encoded, &contact.revision.to_le_bytes());
    append_optional_u64(&mut encoded, contact.last_inbound_utc_ms);
    append_optional_u64(&mut encoded, contact.last_outbound_submitted_utc_ms);
    append_optional_u64(&mut encoded, contact.response_cadence_ema_ms);
    for value in [
        contact.response_cadence_variation,
        contact.contact_due_score,
        contact.unfinished_follow_up_salience,
        contact.repetition_penalty,
    ] {
        append_evidence_field(&mut encoded, &value.raw().to_le_bytes());
    }
    append_evidence_field(&mut encoded, &contact.consecutive_unanswered.to_le_bytes());
    append_optional_u64(&mut encoded, contact.next_contact_eligible_utc_ms);
    match contact.active_cause_digest {
        Some(value) => {
            append_evidence_field(&mut encoded, &[1]);
            append_evidence_field(&mut encoded, &value);
        }
        None => append_evidence_field(&mut encoded, &[0]),
    }
    append_evidence_field(
        &mut encoded,
        &(contact.active_source_event_ids.len() as u64).to_le_bytes(),
    );
    for event_id in &contact.active_source_event_ids {
        append_evidence_field(&mut encoded, event_id);
    }
    append_evidence_field(&mut encoded, &contact.formula_digest);
    append_evidence_field(&mut encoded, &ledger.relation_scope);
    for value in [
        ledger.day_start_utc_ms,
        ledger.limit_tokens,
        ledger.reserved_tokens,
        ledger.charged_tokens,
        ledger.used_tokens,
    ] {
        append_evidence_field(&mut encoded, &value.to_le_bytes());
    }
    append_evidence_field(&mut encoded, &[ledger.usage_known as u8]);
    append_evidence_field(&mut encoded, &ledger.revision.to_le_bytes());
    append_evidence_field(&mut encoded, &input.effective_now_utc_ms.to_le_bytes());
    append_evidence_field(
        &mut encoded,
        &input.authoritative_daily_submitted.to_le_bytes(),
    );
    append_evidence_field(&mut encoded, &[result.band as u8]);
    append_evidence_field(&mut encoded, &result.daily_max.to_le_bytes());
    append_evidence_field(&mut encoded, &result.cooldown_ms.to_le_bytes());
    append_evidence_field(&mut encoded, &[result.selection_reason as u8]);
    wire::domain_hash(b"ae.alpha4.effective-frequency-evidence.v1", &[&encoded])
}

fn fixed_frequency_evidence_digest_v1(
    policy: &RelationTemporalPolicyV1,
    result: &EffectiveFrequencyV1,
) -> Digest {
    let mut encoded = Vec::with_capacity(256);
    append_evidence_field(&mut encoded, &policy.schema_version.to_le_bytes());
    append_evidence_field(&mut encoded, &policy.relation_scope);
    append_evidence_field(&mut encoded, policy.user_timezone.as_bytes());
    append_evidence_field(&mut encoded, &[policy.timezone_source as u8]);
    append_evidence_field(&mut encoded, &policy.quiet_hours_start_minute.to_le_bytes());
    append_evidence_field(&mut encoded, &policy.quiet_hours_end_minute.to_le_bytes());
    append_evidence_field(&mut encoded, &[policy.quiet_hours_emergency_bypass as u8]);
    append_evidence_field(&mut encoded, &[policy.proactive_enabled as u8]);
    append_evidence_field(&mut encoded, &policy.proactive_daily_max.to_le_bytes());
    append_evidence_field(
        &mut encoded,
        &policy.min_proactive_cooldown_ms.to_le_bytes(),
    );
    append_evidence_field(&mut encoded, &policy.intention_ttl_ms.to_le_bytes());
    append_evidence_field(
        &mut encoded,
        &policy.unanswered_backoff_base_ms.to_le_bytes(),
    );
    append_evidence_field(&mut encoded, &policy.unanswered_hard_stop.to_le_bytes());
    append_evidence_field(
        &mut encoded,
        &policy.emergency_threshold.raw().to_le_bytes(),
    );
    append_evidence_field(&mut encoded, &policy.revision.to_le_bytes());
    append_evidence_field(&mut encoded, &policy.auto_policy_version.to_le_bytes());
    append_evidence_field(
        &mut encoded,
        &policy.next_claim_reservation_tokens.to_le_bytes(),
    );
    append_evidence_field(&mut encoded, &[result.band as u8]);
    append_evidence_field(&mut encoded, &result.daily_max.to_le_bytes());
    append_evidence_field(&mut encoded, &result.cooldown_ms.to_le_bytes());
    append_evidence_field(&mut encoded, &[result.selection_reason as u8]);
    wire::domain_hash(b"ae.alpha4.fixed-frequency-evidence.v1", &[&encoded])
}

fn selected_frequency(
    input: &FrequencyEvidenceV1<'_>,
    band: EffectiveFrequencyBandV1,
    daily_max: u16,
    cooldown_ms: u64,
    selection_reason: FrequencySelectionReasonV1,
) -> EffectiveFrequencyV1 {
    let mut result = EffectiveFrequencyV1 {
        band,
        daily_max,
        cooldown_ms,
        selection_reason,
        evidence_digest: [0; 32],
    };
    result.evidence_digest = frequency_evidence_digest_v1(input, &result);
    result
}

/// Select a closed proactive frequency from a frozen, relation-local evidence set.
/// Invalid or arithmetically inconsistent authority is rejected; the unanswered
/// hard stop is the only valid no-frequency result.
pub fn evaluate_effective_frequency_v1(
    input: &FrequencyEvidenceV1<'_>,
) -> Result<Option<EffectiveFrequencyV1>, FrequencyEvaluationErrorV1> {
    let policy = input.policy;
    let contact = input.contact;
    let ledger = input.ledger;
    if policy.relation_scope != contact.relation_scope
        || policy.relation_scope != ledger.relation_scope
    {
        return Err(FrequencyEvaluationErrorV1::ScopeMismatch);
    }
    if !policy.proactive_enabled
        || policy.schema_version != 1
        || policy.revision == 0
        || policy.user_timezone.is_empty()
        || policy.quiet_hours_start_minute >= 24 * 60
        || policy.quiet_hours_end_minute >= 24 * 60
        || policy.intention_ttl_ms == 0
        || policy.unanswered_backoff_base_ms == 0
        || policy.unanswered_hard_stop == 0
        || !(Fixed::ZERO..=Fixed::ONE).contains(&policy.emergency_threshold)
        || policy.auto_policy_version > AUTO_POLICY_VERSION_V1
    {
        return Err(FrequencyEvaluationErrorV1::InvalidPolicy);
    }
    // A historical fixed custom policy may use zero as a deliberate daily
    // disable. It cannot yield allowed frequency authority, and needs no
    // budget/contact evidence to suppress safely.
    if policy.auto_policy_version == 0 && policy.proactive_daily_max == 0 {
        return Ok(None);
    }
    if policy.next_claim_reservation_tokens == 0
        || (policy.auto_policy_version == AUTO_POLICY_VERSION_V1
            && (policy.proactive_daily_max == 0 || policy.min_proactive_cooldown_ms == 0))
    {
        return Err(FrequencyEvaluationErrorV1::InvalidPolicy);
    }
    if contact.validate().is_err() {
        return Err(FrequencyEvaluationErrorV1::InvalidPolicy);
    }
    if contact
        .last_inbound_utc_ms
        .is_some_and(|value| value > input.effective_now_utc_ms)
        || contact
            .last_outbound_submitted_utc_ms
            .is_some_and(|value| value > input.effective_now_utc_ms)
    {
        return Err(FrequencyEvaluationErrorV1::FutureContact);
    }
    let consumed = ledger
        .charged_tokens
        .checked_add(ledger.reserved_tokens)
        .ok_or(FrequencyEvaluationErrorV1::ArithmeticOverflow)?;
    let remaining = ledger
        .limit_tokens
        .checked_sub(consumed)
        .ok_or(FrequencyEvaluationErrorV1::InvalidLedger)?;
    if ledger.revision == 0
        || !ledger.usage_known
        || ledger.used_tokens > ledger.charged_tokens
        || ledger.day_start_utc_ms > input.effective_now_utc_ms
    {
        return Err(FrequencyEvaluationErrorV1::InvalidLedger);
    }
    if contact.consecutive_unanswered >= policy.unanswered_hard_stop {
        return Ok(None);
    }

    if policy.auto_policy_version == 0 {
        let band = match (policy.proactive_daily_max, policy.min_proactive_cooldown_ms) {
            (2, value) if value == 6 * HOUR_MS => EffectiveFrequencyBandV1::Restrained,
            (4, value) if value == 3 * HOUR_MS => EffectiveFrequencyBandV1::Moderate,
            _ => EffectiveFrequencyBandV1::Custom,
        };
        let mut result = selected_frequency(
            input,
            band,
            policy.proactive_daily_max,
            policy.min_proactive_cooldown_ms,
            FrequencySelectionReasonV1::FixedPolicy,
        );
        result.evidence_digest = fixed_frequency_evidence_digest_v1(policy, &result);
        return Ok(Some(result));
    }

    if contact.consecutive_unanswered > 0 {
        let shift = u32::from(contact.consecutive_unanswered - 1);
        let multiplier = 1_u64
            .checked_shl(shift)
            .ok_or(FrequencyEvaluationErrorV1::ArithmeticOverflow)?;
        let cooldown = (6 * HOUR_MS)
            .checked_mul(multiplier)
            .ok_or(FrequencyEvaluationErrorV1::ArithmeticOverflow)?
            .max(6 * HOUR_MS);
        return Ok(Some(selected_frequency(
            input,
            EffectiveFrequencyBandV1::Restrained,
            2,
            cooldown,
            FrequencySelectionReasonV1::Unanswered,
        )));
    }

    let activity_window = match contact.response_cadence_ema_ms {
        Some(ema) => ema
            .checked_mul(2)
            .ok_or(FrequencyEvaluationErrorV1::ArithmeticOverflow)?
            .clamp(DAY_MS, 14 * DAY_MS),
        None => 7 * DAY_MS,
    };
    let active = contact
        .last_inbound_utc_ms
        .is_some_and(|inbound| input.effective_now_utc_ms - inbound <= activity_window);
    let remaining_claims = remaining / policy.next_claim_reservation_tokens;
    let available_submissions = u64::from(input.authoritative_daily_submitted)
        .checked_add(remaining_claims)
        .ok_or(FrequencyEvaluationErrorV1::ArithmeticOverflow)?;
    let token_ceiling = available_submissions.min(4);

    let (band, daily_max, cooldown_ms, reason) = if !active {
        (
            EffectiveFrequencyBandV1::Restrained,
            2,
            6 * HOUR_MS,
            FrequencySelectionReasonV1::InactiveRelation,
        )
    } else if token_ceiling <= 2 {
        (
            EffectiveFrequencyBandV1::Restrained,
            2,
            6 * HOUR_MS,
            FrequencySelectionReasonV1::BudgetCeiling,
        )
    } else if contact
        .last_outbound_submitted_utc_ms
        .is_some_and(|outbound| {
            contact
                .last_inbound_utc_ms
                .is_some_and(|inbound| inbound > outbound)
        })
        && token_ceiling >= 4
    {
        (
            EffectiveFrequencyBandV1::Moderate,
            4,
            3 * HOUR_MS,
            FrequencySelectionReasonV1::RecentReply,
        )
    } else if token_ceiling >= 3 {
        (
            EffectiveFrequencyBandV1::Balanced,
            3,
            4 * HOUR_MS,
            FrequencySelectionReasonV1::ActiveRelation,
        )
    } else {
        (
            EffectiveFrequencyBandV1::Restrained,
            2,
            6 * HOUR_MS,
            FrequencySelectionReasonV1::ConservativeDefault,
        )
    };
    Ok(Some(selected_frequency(
        input,
        band,
        daily_max,
        cooldown_ms,
        reason,
    )))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContactReducerErrorV1 {
    RelationScopeMismatch,
    TimeRegression,
    SourceUntrusted,
    SourceVectorOverflow,
}

#[derive(Clone, Debug)]
pub struct ContactCandidateInputV1<'a> {
    pub persona_scope: Digest,
    pub contact: &'a RelationContactProcessV1,
    pub consent: &'a RelationConsentV1,
    pub policy: &'a RelationTemporalPolicyV1,
    pub purpose: ContactPurposeV1,
    pub cause_created_at_utc_ms: u64,
    pub cause_expires_at_utc_ms: Option<u64>,
    pub cause_public_refs: Vec<Digest>,
    pub cause_confidence: Fixed,
    pub now_utc_ms: u64,
    pub workspace_mapping_digest: Digest,
    pub workspace_residual: Fixed,
}

fn bounded_u64(value: u128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn ema_u64(previous: u64, observed: u64) -> u64 {
    bounded_u64((u128::from(previous) * 3 + u128::from(observed)) / 4)
}

fn normalized_error_raw(observed: u64, ema: u64) -> i64 {
    let difference = observed.abs_diff(ema);
    let denominator = ema.max(1);
    let scaled = u128::from(difference).saturating_mul(SCALE as u128) / u128::from(denominator);
    i64::try_from(scaled.min(SCALE as u128)).unwrap_or(SCALE)
}

fn ema_fixed(previous: Fixed, observed_raw: i64) -> Fixed {
    let raw = (i128::from(previous.raw()) * 3 + i128::from(observed_raw)) / 4;
    Fixed::from_raw(i64::try_from(raw).unwrap_or(i64::MAX)).clamp(Fixed::ZERO, Fixed::ONE)
}

fn cadence_or_backoff_ms(
    contact: &RelationContactProcessV1,
    policy: &RelationTemporalPolicyV1,
) -> u64 {
    contact
        .response_cadence_ema_ms
        .unwrap_or(policy.unanswered_backoff_base_ms)
}

fn purpose_code(purpose: ContactPurposeV1) -> &'static [u8] {
    match purpose {
        ContactPurposeV1::ScheduledCheckIn => b"scheduled_check_in",
        ContactPurposeV1::ExplicitFollowUp => b"explicit_follow_up",
        ContactPurposeV1::RepairInvitation => b"repair_invitation",
    }
}

fn action_class(purpose: ContactPurposeV1) -> &'static str {
    match purpose {
        ContactPurposeV1::ScheduledCheckIn => "scheduled_check_in",
        ContactPurposeV1::ExplicitFollowUp => "explicit_follow_up",
        ContactPurposeV1::RepairInvitation => "repair_invitation",
    }
}

pub fn interaction_fact_public_ref_v1(relation_scope: &Digest, fact: &InteractionFactV1) -> Digest {
    wire::domain_hash(
        b"ae.interaction-fact.public-ref.v1",
        &[relation_scope, &fact.fact_id, &fact.source_digest],
    )
}

pub fn contact_cause_digest_v1(
    relation_scope: &Digest,
    purpose: ContactPurposeV1,
    fact: &InteractionFactV1,
) -> Digest {
    wire::domain_hash(
        b"ae.contact-cause.v1",
        &[
            relation_scope,
            purpose_code(purpose),
            &fact.fact_id,
            &fact.source_digest,
        ],
    )
}

/// Apply a canonical fact batch to exactly one relation-local contact process.
///
/// The caller validates the closed wire contract and persists the returned
/// revision with a compare-and-swap. This function performs no I/O and uses
/// integer/fxp6 arithmetic only. `subject_causes` binds public follow-up
/// references to relation-local causes so resolution cannot clear another cause.
pub fn reduce_interaction_v1(
    current: &RelationContactProcessV1,
    policy: &RelationTemporalPolicyV1,
    facts: &[InteractionFactV1],
    subject_causes: &[(Digest, Digest)],
) -> Result<RelationContactProcessV1, ContactReducerErrorV1> {
    if current.relation_scope != policy.relation_scope {
        return Err(ContactReducerErrorV1::RelationScopeMismatch);
    }
    if facts.len() > 16 {
        return Err(ContactReducerErrorV1::SourceVectorOverflow);
    }

    let mut ordered = facts.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|fact| (fact.observed_at_utc_ms, fact.fact_id));
    let mut next = current.clone();

    for fact in ordered {
        if fact.source_authority == InteractionSourceAuthorityV1::ModelCandidate
            && fact.kind != InteractionFactKindV1::InboundObserved
        {
            return Err(ContactReducerErrorV1::SourceUntrusted);
        }
        match fact.kind {
            InteractionFactKindV1::InboundObserved => {
                if next
                    .last_inbound_utc_ms
                    .is_some_and(|previous| fact.observed_at_utc_ms < previous)
                {
                    return Err(ContactReducerErrorV1::TimeRegression);
                }
                if let Some(previous) = next.last_inbound_utc_ms {
                    let interval = fact.observed_at_utc_ms - previous;
                    let cadence = next
                        .response_cadence_ema_ms
                        .map_or(interval, |old| ema_u64(old, interval));
                    let error = normalized_error_raw(interval, cadence);
                    next.response_cadence_ema_ms = Some(cadence);
                    next.response_cadence_variation =
                        ema_fixed(next.response_cadence_variation, error);
                }
                next.last_inbound_utc_ms = Some(fact.observed_at_utc_ms);
                next.contact_due_score = Fixed::ZERO;
                next.consecutive_unanswered = 0;
            }
            InteractionFactKindV1::FollowUpRequested => {
                let purpose = ContactPurposeV1::ExplicitFollowUp;
                next.active_cause_digest =
                    Some(contact_cause_digest_v1(&next.relation_scope, purpose, fact));
                next.active_source_event_ids = vec![fact.fact_id];
                next.unfinished_follow_up_salience = fact.confidence.clamp(Fixed::ZERO, Fixed::ONE);
                let automatic = fact.observed_at_utc_ms.saturating_add(
                    policy
                        .min_proactive_cooldown_ms
                        .max(cadence_or_backoff_ms(&next, policy)),
                );
                next.next_contact_eligible_utc_ms =
                    Some(fact.scheduled_at_utc_ms.unwrap_or(automatic));
                next.contact_due_score = Fixed::ZERO;
            }
            InteractionFactKindV1::FollowUpResolved => {
                let resolves_active = match fact.subject_public_ref {
                    Some(subject) => subject_causes.iter().any(|(public_ref, cause)| {
                        *public_ref == subject && next.active_cause_digest == Some(*cause)
                    }),
                    None => false,
                };
                if resolves_active {
                    next.active_cause_digest = None;
                    next.active_source_event_ids.clear();
                    next.next_contact_eligible_utc_ms = None;
                    next.contact_due_score = Fixed::ZERO;
                    next.unfinished_follow_up_salience = Fixed::ZERO;
                }
            }
            InteractionFactKindV1::RelationEnded => {
                next.active_cause_digest = None;
                next.active_source_event_ids.clear();
                next.next_contact_eligible_utc_ms = None;
                next.contact_due_score = Fixed::ZERO;
                next.unfinished_follow_up_salience = Fixed::ZERO;
            }
            InteractionFactKindV1::BoundarySet
            | InteractionFactKindV1::ContactGranted
            | InteractionFactKindV1::ContactPaused
            | InteractionFactKindV1::ContactResumed
            | InteractionFactKindV1::ExplicitOutcomeReported => {}
        }
    }

    if next != *current {
        next.revision = current.revision.saturating_add(1);
    }
    Ok(next)
}

/// Advance timing cost for an already explicit cause. Silence without a cause
/// remains a deterministic no-op.
pub fn advance_contact_due_v1(
    current: &RelationContactProcessV1,
    policy: &RelationTemporalPolicyV1,
    now_utc_ms: u64,
    cause_expires_at_utc_ms: Option<u64>,
) -> RelationContactProcessV1 {
    let mut next = current.clone();
    if next.active_cause_digest.is_none()
        || cause_expires_at_utc_ms.is_some_and(|expires| now_utc_ms >= expires)
    {
        next.active_cause_digest = None;
        next.active_source_event_ids.clear();
        next.next_contact_eligible_utc_ms = None;
        next.contact_due_score = Fixed::ZERO;
        next.unfinished_follow_up_salience = Fixed::ZERO;
    } else if let Some(eligible) = next.next_contact_eligible_utc_ms {
        if now_utc_ms < eligible {
            next.contact_due_score = Fixed::ZERO;
        } else {
            let denominator = cadence_or_backoff_ms(&next, policy).max(MIN_DUE_WINDOW_MS);
            let overdue = now_utc_ms.saturating_sub(eligible);
            let raw = u128::from(overdue).saturating_mul(SCALE as u128) / u128::from(denominator);
            next.contact_due_score =
                Fixed::from_raw(i64::try_from(raw.min(SCALE as u128)).unwrap_or(SCALE));
        }
    } else {
        next.contact_due_score = Fixed::ZERO;
    }
    if next != *current {
        next.revision = current.revision.saturating_add(1);
    }
    next
}

/// Build the one stable outbox candidate authorized by an explicit,
/// relation-bound cause and a live consent epoch.
pub fn form_contact_candidate_v1(
    input: &ContactCandidateInputV1<'_>,
) -> Option<(DurableIntentionV1, ContactIntentionBasisV1)> {
    let contact = input.contact;
    let consent = input.consent;
    let policy = input.policy;
    let now = input.now_utc_ms;
    let cause = contact.active_cause_digest?;
    let eligible = contact.next_contact_eligible_utc_ms?;
    if contact.relation_scope != consent.relation_scope
        || contact.relation_scope != policy.relation_scope
        || !policy.proactive_enabled
        || consent.state != RelationConsentStateV1::Granted
        || now < consent.valid_from_utc_ms
        || consent.valid_until_utc_ms.is_some_and(|until| now >= until)
        || input
            .cause_expires_at_utc_ms
            .is_some_and(|expires| now >= expires)
        || !consent.purposes.contains(&input.purpose)
        || !consent.channels.contains(&ContactChannelV1::AstrbotSession)
        || now < eligible
        || contact.contact_due_score < CONTACT_DUE_THRESHOLD_V1
        || contact.repetition_penalty.raw() >= REPETITION_LIMIT_RAW
        || contact.consecutive_unanswered >= policy.unanswered_hard_stop
        || contact.active_source_event_ids.is_empty()
        || contact.active_source_event_ids.len() > 8
        || input.cause_public_refs.is_empty()
        || input.cause_public_refs.len() > 8
    {
        return None;
    }

    let semantic = wire::domain_hash(
        b"ae.contact-intention.semantic.v1",
        &[
            &contact.relation_scope,
            purpose_code(input.purpose),
            &cause,
            &consent.consent_epoch.to_le_bytes(),
        ],
    );
    let intention_id: Id128 = semantic[..16].try_into().expect("digest prefix");
    let policy_expiry = input
        .cause_created_at_utc_ms
        .saturating_add(policy.intention_ttl_ms);
    let expires = [
        Some(policy_expiry),
        consent.valid_until_utc_ms,
        input.cause_expires_at_utc_ms,
    ]
    .into_iter()
    .flatten()
    .min()
    .unwrap_or(policy_expiry);
    if expires <= now || expires < eligible {
        return None;
    }

    let intention = DurableIntentionV1 {
        schema_version: 1,
        intention_id,
        persona_scope: input.persona_scope,
        relation_scope: contact.relation_scope,
        state: IntentionStateV1::Ready,
        action_class: action_class(input.purpose).into(),
        salience: contact.contact_due_score,
        urgency: contact.unfinished_follow_up_salience,
        confidence: input.cause_confidence.clamp(Fixed::ZERO, Fixed::ONE),
        created_at_utc_ms: input.cause_created_at_utc_ms,
        not_before_utc_ms: eligible,
        expires_at_utc_ms: expires,
        externalization_attempts: 0,
        semantic_idempotency_digest: semantic,
        workspace_mapping_digest: input.workspace_mapping_digest,
        workspace_residual: input.workspace_residual,
        source_event_ids: contact.active_source_event_ids.clone(),
    };
    let basis = ContactIntentionBasisV1 {
        intention_id,
        relation_scope: contact.relation_scope,
        purpose: input.purpose,
        cause_digest: cause,
        cause_public_refs: input.cause_public_refs.clone(),
        consent_epoch: consent.consent_epoch,
        consent_revision: consent.revision,
        created_at_utc_ms: input.cause_created_at_utc_ms,
        expires_at_utc_ms: expires,
        live: true,
    };
    Some((intention, basis))
}
