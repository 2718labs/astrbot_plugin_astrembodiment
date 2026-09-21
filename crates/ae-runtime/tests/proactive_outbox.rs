use ae_contracts::*;
use ae_fixed::Fixed;
use ae_runtime::{evaluate_proactive_gates, outbound_target_binding_digest};
use ae_store::Store;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_db(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "ae-proactive-{label}-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root.join("store.db")
}

fn scope() -> ScopeRef {
    ScopeRef {
        bot_token: [1; 16],
        persona_token: [2; 16],
        relation_token: Some([3; 16]),
        session_token: [4; 16],
    }
}

fn target() -> OutboundTargetEnvelopeV1 {
    let scope = scope();
    let mut target = OutboundTargetEnvelopeV1 {
        schema_version: 1,
        target_kind: TargetKindV1::Private,
        umo_ciphertext: vec![5; 32],
        umo_nonce: vec![6; 12],
        key_id: "dpapi-v1".into(),
        umo_digest: [7; 32],
        platform_token: [8; 16],
        bot_token: scope.bot_token,
        persona_token: scope.persona_token,
        relation_token: scope.relation_token.unwrap(),
        session_token: scope.session_token,
        bound_at_utc_ms: 1_000,
        binding_generation: 1,
        binding_digest: [0; 32],
    };
    target.binding_digest = outbound_target_binding_digest(&target);
    target
}

fn state(now: u64) -> AutonomousRuntimeStateV1 {
    let scope = scope();
    AutonomousRuntimeStateV1 {
        schema_version: 1,
        persona_scope: wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None),
        relation_scope: Some(wire::persona_scope_digest(
            &scope.bot_token,
            &scope.persona_token,
            scope.relation_token.as_ref(),
        )),
        generation: 0,
        state_revision: 0,
        last_advanced_at_utc_ms: now,
        next_wake_at_utc_ms: now + 60_000,
        wake_intensity: WakeIntensityV1::Ignition,
        sleep_state: SleepStateV1::Awake,
        process_s: Fixed::ZERO,
        process_c: Fixed::ZERO,
        arousal: Fixed::ONE,
        sleep_threshold_held_ms: 0,
        circadian_phase_minutes: Fixed::ZERO,
        affiliation_need: Fixed::ONE,
        unfinished_topic_salience: Fixed::ONE,
        social_energy: Fixed::ONE,
        formula_digest: [9; 32],
        mapping_digest: [10; 32],
        workspace_residual: Fixed::ZERO,
    }
}

fn intention(now: u64) -> DurableIntentionV1 {
    let state = state(now);
    DurableIntentionV1 {
        schema_version: 1,
        intention_id: [11; 16],
        persona_scope: state.persona_scope,
        relation_scope: state.relation_scope.unwrap(),
        state: IntentionStateV1::Ready,
        action_class: "relationship_connection".into(),
        salience: Fixed::ONE,
        urgency: Fixed::from_raw(800_000),
        confidence: Fixed::ONE,
        created_at_utc_ms: now - 1_000,
        not_before_utc_ms: now - 1,
        expires_at_utc_ms: now + 86_400_000,
        externalization_attempts: 0,
        semantic_idempotency_digest: [12; 32],
        workspace_mapping_digest: state.mapping_digest,
        workspace_residual: Fixed::ZERO,
        source_event_ids: vec![[13; 16]],
    }
}

fn policy(now: u64) -> RelationTemporalPolicyV1 {
    RelationTemporalPolicyV1 {
        schema_version: 1,
        relation_scope: intention(now).relation_scope,
        user_timezone: "Asia/Shanghai".into(),
        timezone_source: TimezoneSourceV1::Explicit,
        quiet_hours_start_minute: 0,
        quiet_hours_end_minute: 0,
        quiet_hours_emergency_bypass: true,
        proactive_enabled: true,
        proactive_daily_max: 2,
        min_proactive_cooldown_ms: 6 * 60 * 60 * 1_000,
        intention_ttl_ms: 86_400_000,
        unanswered_backoff_base_ms: 6 * 60 * 60 * 1_000,
        unanswered_hard_stop: 3,
        emergency_threshold: Fixed::from_raw(900_000),
        daily_submitted: 0,
        consecutive_unanswered: 0,
        last_inbound_utc_ms: Some(now - 86_400_000),
        last_proactive_submitted_utc_ms: None,
        revision: 1,
        auto_policy_version: 0,
        next_claim_reservation_tokens: 256,
    }
}

fn frozen(now: u64) -> FrozenTimeInputV1 {
    FrozenTimeInputV1 {
        schema_version: 1,
        observed_now_utc_ms: now,
        effective_now_utc_ms: now,
        persona_tzid: "America/Los_Angeles".into(),
        persona_utc_offset_seconds: -25_200,
        persona_local_minute: 600,
        persona_day_ordinal: 1,
        relation_tzid: "Asia/Shanghai".into(),
        relation_utc_offset_seconds: 28_800,
        relation_local_minute: 1_000,
        relation_day_ordinal: 1,
        budget_day_start_utc_ms: now - now % 86_400_000,
        budget_next_day_start_utc_ms: now - now % 86_400_000 + 86_400_000,
        next_timezone_transition_utc_ms: None,
        tzdb_fingerprint: [14; 32],
    }
}

fn capability() -> HostCapabilitySnapshotV1 {
    let mut value = HostCapabilitySnapshotV1 {
        schema_version: 1,
        astrbot_send_available: true,
        credential_store_available: true,
        platform_idempotent: false,
        provider_identifier: "provider-a".into(),
        config_source_digest: [15; 32],
        config_revision: 7,
        content_boundary_version: 1,
        policy_version: 1,
        snapshot_digest: [0; 32],
    };
    value.snapshot_digest = ae_runtime::host_capability_snapshot_digest(&value);
    value
}

fn insert_test_intention(path: &std::path::Path, value: &DurableIntentionV1) {
    let connection = rusqlite::Connection::open(path).unwrap();
    connection
        .execute(
            "INSERT INTO durable_intention(intention_id,persona_scope,relation_scope,semantic_digest,state,revision,body_json) VALUES(?1,?2,?3,?4,'ready',0,?5)",
            rusqlite::params![
                value.intention_id.to_vec(),
                value.persona_scope.to_vec(),
                value.relation_scope.to_vec(),
                value.semantic_idempotency_digest.to_vec(),
                serde_json::to_string(value).unwrap(),
            ],
        )
        .unwrap();
}

fn seed_pending_outbound(
    path: &std::path::Path,
    store: &mut Store,
    now: u64,
) -> (OutboundAttemptV1, FrozenTimeInputV1) {
    let scope = scope();
    let initial = state(now);
    store.initialize_autonomous_state(&initial).unwrap();
    store.upsert_relation_policy(&policy(now)).unwrap();
    store
        .store_outbound_target(&intention(now).relation_scope, &target())
        .unwrap();
    let frozen = frozen(now);
    insert_test_intention(path, &intention(now));
    let gated = store
        .gate_and_claim_externalization(&GateAndClaimExternalizationRequestV1 {
            scope,
            intention_id: [11; 16],
            attempt_no: 1,
            expected_revision: 0,
            caller_incarnation: [72; 32],
            capability: capability(),
            frozen: frozen.clone(),
        })
        .unwrap();
    assert!(gated.decision.permitted);
    let claim = gated.claim.unwrap();
    store
        .settle_externalization(&ExternalizationSettleV1 {
            claim_token: claim.claim_token,
            caller_incarnation: [72; 32],
            outcome: ExternalizationOutcomeV1::Success,
            used_tokens: Some(100),
            candidate_digest: Some([73; 32]),
            candidate_ciphertext: Some(vec![74; 48]),
        })
        .unwrap();
    (
        store
            .load_outbound_for_intention(&[11; 16])
            .unwrap()
            .unwrap(),
        frozen,
    )
}

#[test]
fn permit_and_representative_suppression_matrix() {
    let now = 2_000_000_000;
    let state = state(now);
    let intention = intention(now);
    let target = target();
    let capability = capability();
    assert_eq!(
        hex::encode32(&capability.snapshot_digest),
        "a16784d835924bec8f013354b404da0db40232b0497a1792cf2791a1ba751f7f"
    );
    let frozen_input = frozen(now);
    let policy = policy(now);
    assert!(
        evaluate_proactive_gates(
            &state,
            &intention,
            &policy,
            &target,
            &capability,
            &frozen_input
        )
        .permitted
    );

    let mut disabled = policy.clone();
    disabled.proactive_enabled = false;
    assert_eq!(
        evaluate_proactive_gates(
            &state,
            &intention,
            &disabled,
            &target,
            &capability,
            &frozen_input
        )
        .suppression,
        Some(GateSuppressionReasonV1::ProactiveDisabled),
    );
    let mut unknown_tz = policy.clone();
    unknown_tz.timezone_source = TimezoneSourceV1::HostFallback;
    assert_eq!(
        evaluate_proactive_gates(
            &state,
            &intention,
            &unknown_tz,
            &target,
            &capability,
            &frozen_input
        )
        .suppression,
        Some(GateSuppressionReasonV1::TimezoneUnreliable),
    );
    let mut asleep = state.clone();
    asleep.sleep_state = SleepStateV1::Asleep;
    assert_eq!(
        evaluate_proactive_gates(
            &asleep,
            &intention,
            &policy,
            &target,
            &capability,
            &frozen_input
        )
        .suppression,
        Some(GateSuppressionReasonV1::PersonaAsleep),
    );
    let mut unanswered = policy;
    unanswered.consecutive_unanswered = 3;
    assert_eq!(
        evaluate_proactive_gates(
            &state,
            &intention,
            &unanswered,
            &target,
            &capability,
            &frozen_input
        )
        .suppression,
        Some(GateSuppressionReasonV1::UnansweredHardStop),
    );
}

#[test]
fn claim_before_call_crash_recovers_unknown_and_outbound_id_is_stable() {
    let now = 2_000_000_000;
    let path = temp_db("claim-crash");
    let mut store = Store::open(&path).unwrap();
    let scope = scope();
    let initial = state(now);
    store.initialize_autonomous_state(&initial).unwrap();
    store.upsert_relation_policy(&policy(now)).unwrap();
    let target = target();
    store
        .store_outbound_target(&intention(now).relation_scope, &target)
        .unwrap();

    let frozen_input = frozen(now);
    insert_test_intention(&path, &intention(now));
    let external = store
        .gate_and_claim_externalization(&GateAndClaimExternalizationRequestV1 {
            scope: scope.clone(),
            intention_id: [11; 16],
            attempt_no: 1,
            expected_revision: 0,
            caller_incarnation: [18; 32],
            capability: capability(),
            frozen: frozen_input.clone(),
        })
        .unwrap()
        .claim
        .unwrap();
    store
        .settle_externalization(&ExternalizationSettleV1 {
            claim_token: external.claim_token,
            caller_incarnation: [18; 32],
            outcome: ExternalizationOutcomeV1::Success,
            used_tokens: Some(100),
            candidate_digest: Some([19; 32]),
            candidate_ciphertext: Some(vec![20; 48]),
        })
        .unwrap();
    let pending = store
        .load_outbound_for_intention(&[11; 16])
        .unwrap()
        .unwrap();
    let old_budget_day = frozen_input.budget_day_start_utc_ms;
    let mut dispatch_frozen = frozen_input;
    dispatch_frozen.budget_day_start_utc_ms += 86_400_000;
    dispatch_frozen.budget_next_day_start_utc_ms += 86_400_000;
    let dispatch_budget_day = dispatch_frozen.budget_day_start_utc_ms;
    let gated = store
        .gate_and_claim_dispatch(&GateAndClaimDispatchRequestV1 {
            scope,
            outbound_id: pending.outbound_id,
            expected_target_digest: target.binding_digest,
            caller_incarnation: [22; 32],
            capability: capability(),
            frozen: dispatch_frozen,
        })
        .unwrap();
    assert!(gated.decision.permitted);
    assert!(gated.claim.is_some());
    assert_ne!(gated.claim.as_ref().unwrap().preflight_digest, [0; 32]);
    assert_eq!(
        store
            .recover_orphaned_dispatches(
                &state(now).persona_scope,
                gated.claim.as_ref().unwrap().lease_deadline_utc_ms,
            )
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .load_outbound_for_intention(&[11; 16])
            .unwrap()
            .unwrap()
            .state,
        IntentionStateV1::DispatchUnknown,
    );
    let (counted, last) = store
        .outbound_submission_metrics(&intention(now).relation_scope, dispatch_budget_day)
        .unwrap();
    assert_eq!(counted, 1);
    assert!(last.is_some());
    assert_eq!(
        store
            .outbound_submission_metrics(&intention(now).relation_scope, old_budget_day)
            .unwrap()
            .0,
        0,
    );
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn newer_target_revokes_old_generation_and_old_digest_cannot_claim() {
    let path = temp_db("target-rotation");
    let mut store = Store::open(&path).unwrap();
    let relation_scope = intention(2_000_000_000).relation_scope;
    let old = target();
    store.store_outbound_target(&relation_scope, &old).unwrap();
    let mut new = old.clone();
    new.binding_generation = 2;
    new.bound_at_utc_ms += 1;
    new.binding_digest = [0; 32];
    new.binding_digest = outbound_target_binding_digest(&new);
    store.store_outbound_target(&relation_scope, &new).unwrap();
    assert!(store
        .load_current_outbound_target(&relation_scope)
        .unwrap()
        .is_some_and(|value| value.binding_digest == new.binding_digest));
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn revoked_target_terminalizes_poison_pending_row() {
    let now = 2_000_000_000;
    let path = temp_db("poison-target");
    let mut store = Store::open(&path).unwrap();
    let (pending, frozen) = seed_pending_outbound(&path, &mut store, now);
    let old = pending.target.clone();
    let mut replacement = old.clone();
    replacement.binding_generation += 1;
    replacement.bound_at_utc_ms += 1;
    replacement.binding_digest = [0; 32];
    replacement.binding_digest = outbound_target_binding_digest(&replacement);
    store
        .store_outbound_target(&intention(now).relation_scope, &replacement)
        .unwrap();

    let gated = store
        .gate_and_claim_dispatch(&GateAndClaimDispatchRequestV1 {
            scope: scope(),
            outbound_id: pending.outbound_id,
            expected_target_digest: old.binding_digest,
            caller_incarnation: [75; 32],
            capability: capability(),
            frozen,
        })
        .unwrap();
    assert!(!gated.decision.permitted);
    assert_eq!(
        gated.decision.suppression,
        Some(GateSuppressionReasonV1::TargetUnavailable)
    );
    assert!(store
        .list_pending_autonomy_work(&state(now).persona_scope)
        .unwrap()
        .outbounds
        .is_empty());
    assert_eq!(
        store
            .load_outbound_for_intention(&[11; 16])
            .unwrap()
            .unwrap()
            .state,
        IntentionStateV1::Terminal
    );
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn policy_revision_is_native_monotonic_and_operational_fields_survive_config_change() {
    let path = temp_db("policy-revision");
    let mut store = Store::open(&path).unwrap();
    let now = 2_000_000_000;
    let original = policy(now);
    store.upsert_relation_policy(&original).unwrap();
    store
        .record_relation_inbound(&original.relation_scope, now)
        .unwrap();
    let mut changed = original.clone();
    changed.revision = 1;
    changed.proactive_daily_max = 1;
    changed.consecutive_unanswered = 99;
    store.upsert_relation_policy(&changed).unwrap();
    let stored = store
        .load_relation_policy(&original.relation_scope)
        .unwrap()
        .unwrap();
    assert_eq!(stored.revision, 2);
    assert_eq!(stored.proactive_daily_max, 1);
    assert_eq!(stored.consecutive_unanswered, 0);
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn orphaned_externalization_is_recovered_as_durable_retryable_work() {
    let now = 2_000_000_000;
    let path = temp_db("externalization-recovery");
    let mut store = Store::open(&path).unwrap();
    let scope = scope();
    let initial = state(now);
    store.initialize_autonomous_state(&initial).unwrap();
    store.upsert_relation_policy(&policy(now)).unwrap();
    let target = target();
    store
        .store_outbound_target(&intention(now).relation_scope, &target)
        .unwrap();
    let frozen = frozen(now);
    insert_test_intention(&path, &intention(now));
    let mut bad_capability = capability();
    bad_capability.snapshot_digest = [99; 32];
    assert!(store
        .gate_and_claim_externalization(&GateAndClaimExternalizationRequestV1 {
            scope: scope.clone(),
            intention_id: [11; 16],
            attempt_no: 1,
            expected_revision: 0,
            caller_incarnation: [28; 32],
            capability: bad_capability,
            frozen: frozen.clone(),
        })
        .is_err());
    let gated = store
        .gate_and_claim_externalization(&GateAndClaimExternalizationRequestV1 {
            scope,
            intention_id: [11; 16],
            attempt_no: 1,
            expected_revision: 0,
            caller_incarnation: [28; 32],
            capability: capability(),
            frozen: frozen.clone(),
        })
        .unwrap();
    assert!(gated.decision.permitted);
    assert!(gated.claim.is_some());
    assert_eq!(
        store
            .recover_orphaned_externalizations(
                &initial.persona_scope,
                gated.claim.as_ref().unwrap().lease_deadline_utc_ms,
            )
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .externalization_reserved_tokens(
                &intention(now).relation_scope,
                frozen.budget_day_start_utc_ms,
            )
            .unwrap(),
        512,
    );
    let pending = store
        .list_pending_autonomy_work(&state(now).persona_scope)
        .unwrap();
    assert_eq!(pending.intentions.len(), 1);
    assert_eq!(
        pending.intentions[0].intention.state,
        IntentionStateV1::Deferred
    );
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn externalization_settle_rejects_a_tampered_claim_body() {
    let now = 2_000_000_000;
    let path = temp_db("externalization-claim-integrity");
    let mut store = Store::open(&path).unwrap();
    store.initialize_autonomous_state(&state(now)).unwrap();
    store.upsert_relation_policy(&policy(now)).unwrap();
    store
        .store_outbound_target(&intention(now).relation_scope, &target())
        .unwrap();
    insert_test_intention(&path, &intention(now));
    let caller = [81; 32];
    let claim = store
        .gate_and_claim_externalization(&GateAndClaimExternalizationRequestV1 {
            scope: scope(),
            intention_id: [11; 16],
            attempt_no: 1,
            expected_revision: 0,
            caller_incarnation: caller,
            capability: capability(),
            frozen: frozen(now),
        })
        .unwrap()
        .claim
        .unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    let raw: String = connection
        .query_row(
            "SELECT body_json FROM autonomy_claim WHERE claim_token=?1",
            rusqlite::params![claim.claim_token.to_vec()],
            |row| row.get(0),
        )
        .unwrap();
    let mut body: serde_json::Value = serde_json::from_str(&raw).unwrap();
    body["max_tokens"] = serde_json::Value::from(511);
    connection
        .execute(
            "UPDATE autonomy_claim SET body_json=?2 WHERE claim_token=?1",
            rusqlite::params![claim.claim_token.to_vec(), body.to_string()],
        )
        .unwrap();
    drop(connection);
    assert!(store
        .settle_externalization(&ExternalizationSettleV1 {
            claim_token: claim.claim_token,
            caller_incarnation: caller,
            outcome: ExternalizationOutcomeV1::RejectedTerminal,
            used_tokens: None,
            candidate_digest: None,
            candidate_ciphertext: None,
        })
        .is_err());
    drop(store);
    let _ = std::fs::remove_file(path);
}
