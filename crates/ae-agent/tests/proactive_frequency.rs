use ae_agent::{evaluate_effective_frequency_v1, FrequencyEvidenceV1};
use ae_contracts::{
    EffectiveFrequencyBandV1, EffectiveFrequencyV1, FrequencySelectionReasonV1,
    RelationBudgetLedgerV1, RelationContactProcessV1, RelationTemporalPolicyV1,
};
use ae_fixed::Fixed;

const HOUR_MS: u64 = 3_600_000;
const NOW: u64 = 30 * 24 * HOUR_MS;

fn policy(auto_policy_version: u16, daily_max: u16, cooldown_ms: u64) -> RelationTemporalPolicyV1 {
    RelationTemporalPolicyV1 {
        schema_version: 1,
        relation_scope: [7; 32],
        user_timezone: "UTC".into(),
        timezone_source: ae_contracts::TimezoneSourceV1::Explicit,
        quiet_hours_start_minute: 0,
        quiet_hours_end_minute: 0,
        quiet_hours_emergency_bypass: false,
        proactive_enabled: true,
        proactive_daily_max: daily_max,
        min_proactive_cooldown_ms: cooldown_ms,
        intention_ttl_ms: HOUR_MS,
        unanswered_backoff_base_ms: 6 * HOUR_MS,
        unanswered_hard_stop: 3,
        emergency_threshold: Fixed::from_raw(900_000),
        daily_submitted: 0,
        consecutive_unanswered: 0,
        last_inbound_utc_ms: None,
        last_proactive_submitted_utc_ms: None,
        revision: 1,
        auto_policy_version,
        next_claim_reservation_tokens: 256,
    }
}

fn contact(
    inbound: Option<u64>,
    outbound: Option<u64>,
    unanswered: u16,
) -> RelationContactProcessV1 {
    RelationContactProcessV1 {
        schema_version: 1,
        relation_scope: [7; 32],
        revision: 1,
        last_inbound_utc_ms: inbound,
        last_outbound_submitted_utc_ms: outbound,
        response_cadence_ema_ms: None,
        response_cadence_variation: Fixed::ZERO,
        contact_due_score: Fixed::ZERO,
        unfinished_follow_up_salience: Fixed::ZERO,
        repetition_penalty: Fixed::ZERO,
        consecutive_unanswered: unanswered,
        next_contact_eligible_utc_ms: None,
        active_cause_digest: None,
        active_source_event_ids: Vec::new(),
        formula_digest: [9; 32],
    }
}

fn ledger(limit: u64, charged: u64, reserved: u64) -> RelationBudgetLedgerV1 {
    RelationBudgetLedgerV1 {
        relation_scope: [7; 32],
        day_start_utc_ms: NOW - HOUR_MS,
        limit_tokens: limit,
        reserved_tokens: reserved,
        charged_tokens: charged,
        used_tokens: charged,
        usage_known: true,
        revision: 1,
    }
}

fn selected(
    policy: &RelationTemporalPolicyV1,
    contact: &RelationContactProcessV1,
    ledger: &RelationBudgetLedgerV1,
    now: u64,
    daily_submitted: u16,
) -> ae_contracts::EffectiveFrequencyV1 {
    evaluate_effective_frequency_v1(&FrequencyEvidenceV1 {
        policy,
        contact,
        ledger,
        effective_now_utc_ms: now,
        authoritative_daily_submitted: daily_submitted,
    })
    .expect("valid evidence")
    .expect("frequency selected")
}

#[test]
fn frequency_policy_is_closed_and_unanswered_never_widens() {
    let inactive = contact(Some(NOW - 8 * 24 * HOUR_MS), None, 0);
    let active = contact(Some(NOW - HOUR_MS), None, 0);
    let recent_reply = contact(Some(NOW - HOUR_MS), Some(NOW - 2 * HOUR_MS), 0);

    for (daily, cooldown, expected_band) in [
        (2, 6 * HOUR_MS, EffectiveFrequencyBandV1::Restrained),
        (4, 3 * HOUR_MS, EffectiveFrequencyBandV1::Moderate),
        (1, HOUR_MS, EffectiveFrequencyBandV1::Custom),
    ] {
        let fixed = policy(0, daily, cooldown);
        let result = selected(&fixed, &active, &ledger(4096, 0, 0), NOW, 0);
        assert_eq!(
            (result.daily_max, result.cooldown_ms, result.band),
            (daily, cooldown, expected_band)
        );
        assert_eq!(
            result.selection_reason,
            FrequencySelectionReasonV1::FixedPolicy
        );
        assert_ne!(result.evidence_digest, [0; 32]);
    }

    let auto = policy(1, 2, 6 * HOUR_MS);
    let rows = [
        (
            &inactive,
            ledger(1024, 0, 0),
            0,
            EffectiveFrequencyBandV1::Restrained,
            2,
            6 * HOUR_MS,
        ),
        (
            &active,
            ledger(512, 0, 0),
            0,
            EffectiveFrequencyBandV1::Restrained,
            2,
            6 * HOUR_MS,
        ),
        (
            &active,
            ledger(1024, 0, 0),
            0,
            EffectiveFrequencyBandV1::Balanced,
            3,
            4 * HOUR_MS,
        ),
        (
            &recent_reply,
            ledger(1024, 0, 0),
            0,
            EffectiveFrequencyBandV1::Moderate,
            4,
            3 * HOUR_MS,
        ),
        (
            &active,
            ledger(768, 0, 0),
            0,
            EffectiveFrequencyBandV1::Balanced,
            3,
            4 * HOUR_MS,
        ),
    ];
    for (state, budget, submitted, band, daily, cooldown) in rows {
        let result = selected(&auto, state, &budget, NOW, submitted);
        assert_eq!(
            (result.band, result.daily_max, result.cooldown_ms),
            (band, daily, cooldown)
        );
    }

    for (unanswered, cooldown) in [(1, 6 * HOUR_MS), (2, 12 * HOUR_MS)] {
        let state = contact(Some(NOW - HOUR_MS), Some(NOW - 2 * HOUR_MS), unanswered);
        let result = selected(&auto, &state, &ledger(1024, 0, 0), NOW, 0);
        assert_eq!(result.band, EffectiveFrequencyBandV1::Restrained);
        assert_eq!(result.cooldown_ms, cooldown);
        assert_eq!(
            result.selection_reason,
            FrequencySelectionReasonV1::Unanswered
        );
    }

    let hard_stop = contact(Some(NOW - HOUR_MS), Some(NOW - 2 * HOUR_MS), 3);
    assert_eq!(
        evaluate_effective_frequency_v1(&FrequencyEvidenceV1 {
            policy: &auto,
            contact: &hard_stop,
            ledger: &ledger(1024, 0, 0),
            effective_now_utc_ms: NOW,
            authoritative_daily_submitted: 0,
        })
        .expect("hard stop is a valid suppression"),
        None
    );

    let invalid_cases = [
        (contact(Some(NOW + 1), None, 0), ledger(1024, 0, 0), 256),
        (contact(None, Some(NOW + 1), 0), ledger(1024, 0, 0), 256),
        (active.clone(), ledger(255, 128, 128), 256),
    ];
    for (state, budget, reservation) in invalid_cases {
        let mut bad = auto.clone();
        bad.next_claim_reservation_tokens = reservation;
        assert!(evaluate_effective_frequency_v1(&FrequencyEvidenceV1 {
            policy: &bad,
            contact: &state,
            ledger: &budget,
            effective_now_utc_ms: NOW,
            authoritative_daily_submitted: 0,
        })
        .is_err());
    }
    let mut zero_reservation = auto.clone();
    zero_reservation.next_claim_reservation_tokens = 0;
    assert!(evaluate_effective_frequency_v1(&FrequencyEvidenceV1 {
        policy: &zero_reservation,
        contact: &active,
        ledger: &ledger(1024, 0, 0),
        effective_now_utc_ms: NOW,
        authoritative_daily_submitted: 0,
    })
    .is_err());

    let unanswered = contact(Some(NOW - HOUR_MS), Some(NOW - 2 * HOUR_MS), 1);
    let first = selected(&auto, &unanswered, &ledger(1024, 0, 0), NOW, 0);
    let later = selected(
        &auto,
        &unanswered,
        &ledger(1024, 0, 0),
        NOW + 2 * HOUR_MS,
        0,
    );
    assert!(later.daily_max <= first.daily_max);
    assert!(later.cooldown_ms >= first.cooldown_ms);

    let frequency = |band, cooldown_ms, reason| EffectiveFrequencyV1 {
        band,
        daily_max: match band {
            EffectiveFrequencyBandV1::Restrained => 2,
            EffectiveFrequencyBandV1::Balanced => 3,
            EffectiveFrequencyBandV1::Moderate => 4,
            EffectiveFrequencyBandV1::Custom => 1,
        },
        cooldown_ms,
        selection_reason: reason,
        evidence_digest: [1; 32],
    };
    for cooldown in [6 * HOUR_MS, 12 * HOUR_MS, 24 * HOUR_MS] {
        frequency(
            EffectiveFrequencyBandV1::Restrained,
            cooldown,
            FrequencySelectionReasonV1::Unanswered,
        )
        .validate()
        .expect("closed unanswered power-of-two backoff");
    }
    for cooldown in [7 * HOUR_MS, 13 * HOUR_MS, u64::MAX] {
        assert!(frequency(
            EffectiveFrequencyBandV1::Restrained,
            cooldown,
            FrequencySelectionReasonV1::Unanswered,
        )
        .validate()
        .is_err());
    }
    assert!(frequency(
        EffectiveFrequencyBandV1::Restrained,
        12 * HOUR_MS,
        FrequencySelectionReasonV1::FixedPolicy,
    )
    .validate()
    .is_err());
    assert!(frequency(
        EffectiveFrequencyBandV1::Balanced,
        4 * HOUR_MS,
        FrequencySelectionReasonV1::RecentReply,
    )
    .validate()
    .is_err());
    assert!(frequency(
        EffectiveFrequencyBandV1::Moderate,
        3 * HOUR_MS,
        FrequencySelectionReasonV1::ActiveRelation,
    )
    .validate()
    .is_err());

    let fixed = policy(0, 2, 6 * HOUR_MS);
    let fixed_first = selected(&fixed, &active, &ledger(4096, 0, 0), NOW, 0);
    let fixed_other_runtime = selected(
        &fixed,
        &contact(Some(NOW - 20 * HOUR_MS), Some(NOW - 21 * HOUR_MS), 0),
        &ledger(512, 256, 0),
        NOW + 20 * HOUR_MS,
        1,
    );
    assert_eq!(
        fixed_first.evidence_digest,
        fixed_other_runtime.evidence_digest
    );

    let fixed_zero_cooldown = policy(0, 5, 0);
    let zero_cooldown = selected(&fixed_zero_cooldown, &active, &ledger(4096, 0, 0), NOW, 0);
    assert_eq!(zero_cooldown.band, EffectiveFrequencyBandV1::Custom);
    assert_eq!((zero_cooldown.daily_max, zero_cooldown.cooldown_ms), (5, 0));
    zero_cooldown
        .validate()
        .expect("custom zero cooldown is valid");

    let fixed_zero_daily = policy(0, 0, 6 * HOUR_MS);
    assert_eq!(
        evaluate_effective_frequency_v1(&FrequencyEvidenceV1 {
            policy: &fixed_zero_daily,
            contact: &active,
            ledger: &ledger(4096, 0, 0),
            effective_now_utc_ms: NOW,
            authoritative_daily_submitted: 0,
        })
        .expect("fixed zero daily is a valid compatibility suppression"),
        None
    );
}
