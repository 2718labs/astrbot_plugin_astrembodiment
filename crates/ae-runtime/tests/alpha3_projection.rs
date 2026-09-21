use ae_contracts::*;
use ae_fixed::Fixed;
use ae_runtime::{frozen_time_input_digest, outbound_target_binding_digest, AstrRuntime};
use rusqlite::{params, Connection};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_db() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!(
            "alpha3-body-projection-db-{}-{nonce}",
            std::process::id()
        ));
    std::fs::create_dir_all(&root).unwrap();
    root.join("store.db")
}

fn legacy_dispatch_claim_token(claim: &DispatchClaimV2, caller_incarnation: &Digest) -> Digest {
    ae_contracts::wire::domain_hash(
        b"ae.alpha3.dispatch-claim.v2",
        &[
            &claim.outbound_public_ref,
            &claim.consent_epoch.to_le_bytes(),
            &claim.consent_revision.to_le_bytes(),
            &claim.policy_revision.to_le_bytes(),
            &claim.target_binding_digest,
            &claim.capability_snapshot_digest,
            &claim.lease_deadline_utc_ms.to_le_bytes(),
            caller_incarnation,
        ],
    )
}

fn genesis_request() -> PersonaGenesisRequest {
    let scope = PersonaScopeRef {
        bot_token: [11; 16],
        persona_token: [12; 16],
    };
    let source = PersonaSourceRef {
        scope,
        source_digest: [13; 32],
        capability_digest: [14; 32],
        selection: PersonaSelectionKind::Conversation,
        prompt_chars: 10,
        begin_dialog_count: 1,
        mood_dialog_count: 0,
    };
    PersonaGenesisRequest {
        source: source.clone(),
        proposal: GenesisManifestProposal {
            schema_version: 1,
            source,
            traits: PersonalityVector::default(),
            trait_confidence: PersonalityVector::default(),
            expression: ExpressionPhenotype::default(),
            allostasis: AllostaticSetpoints::default(),
            epistemic: EpistemicPriors::default(),
            social: SocialPriors::default(),
            compiler_protocol_digest: [15; 32],
            compiler_model_digest: [16; 32],
        },
        formula_digest: [17; 32],
        incarnation_nonce: [18; 32],
        parent_incarnation_id: None,
        observed_at_ms: 1_700_000_000_000,
    }
}

fn profile(persona_scope: Digest) -> PersonaTemporalProfileV1 {
    PersonaTemporalProfileV1 {
        schema_version: 1,
        persona_scope,
        home_timezone: "Asia/Shanghai".into(),
        current_timezone: "Asia/Shanghai".into(),
        chronotype: ChronotypeV1::Intermediate,
        preferred_sleep_local_minute: 1_380,
        preferred_wake_local_minute: 420,
        sleep_flex_minutes: 90,
        entrainment_rate_minutes_per_day: 60,
        revision: 1,
    }
}

fn relation_policy(relation_scope: Digest) -> RelationTemporalPolicyV1 {
    RelationTemporalPolicyV1 {
        schema_version: 1,
        relation_scope,
        user_timezone: "Asia/Shanghai".into(),
        timezone_source: TimezoneSourceV1::Explicit,
        quiet_hours_start_minute: 0,
        quiet_hours_end_minute: 0,
        quiet_hours_emergency_bypass: false,
        proactive_enabled: true,
        proactive_daily_max: 20,
        min_proactive_cooldown_ms: 0,
        intention_ttl_ms: 86_400_000,
        unanswered_backoff_base_ms: 0,
        unanswered_hard_stop: 20,
        emergency_threshold: Fixed::ONE,
        daily_submitted: 0,
        consecutive_unanswered: 0,
        last_inbound_utc_ms: None,
        last_proactive_submitted_utc_ms: None,
        revision: 1,
        auto_policy_version: 0,
        next_claim_reservation_tokens: 256,
    }
}

fn frozen(now_utc_ms: u64, day_start_utc_ms: u64) -> FrozenTimeInputV1 {
    FrozenTimeInputV1 {
        schema_version: 1,
        observed_now_utc_ms: now_utc_ms,
        effective_now_utc_ms: now_utc_ms,
        persona_tzid: "Asia/Shanghai".into(),
        persona_utc_offset_seconds: 28_800,
        persona_local_minute: 720,
        persona_day_ordinal: 20_000,
        relation_tzid: "Asia/Shanghai".into(),
        relation_utc_offset_seconds: 28_800,
        relation_local_minute: 720,
        relation_day_ordinal: 20_000,
        budget_day_start_utc_ms: day_start_utc_ms,
        budget_next_day_start_utc_ms: day_start_utc_ms + 86_400_000,
        next_timezone_transition_utc_ms: None,
        tzdb_fingerprint: [19; 32],
    }
}

fn fact(id: u8, kind: InteractionFactKindV1, observed_at_utc_ms: u64) -> InteractionFactV1 {
    InteractionFactV1 {
        fact_id: [id; 16],
        kind,
        observed_at_utc_ms,
        source_authority: InteractionSourceAuthorityV1::ExplicitControl,
        source_digest: [id.wrapping_add(40); 32],
        extractor_digest: [id.wrapping_add(80); 32],
        confidence: Fixed::ONE,
        value_code: None,
        subject_public_ref: None,
        consent_terms: None,
        scheduled_at_utc_ms: None,
        expires_at_utc_ms: None,
    }
}

fn apply_follow_up(
    runtime: &mut AstrRuntime,
    persona: &ScopeRef,
    relation: &ScopeRef,
    seed: u8,
    now: u64,
    grant: bool,
) {
    let mut follow_up = fact(seed, InteractionFactKindV1::FollowUpRequested, now);
    follow_up.value_code = Some(InteractionValueCodeV1::FollowUp);
    follow_up.scheduled_at_utc_ms = Some(now);
    follow_up.expires_at_utc_ms = Some(now + 80_000_000);
    let mut facts = vec![follow_up];
    if grant {
        let mut consent = fact(seed + 1, InteractionFactKindV1::ContactGranted, now);
        consent.value_code = Some(InteractionValueCodeV1::Grant);
        consent.consent_terms = Some(ConsentTermsV1 {
            purposes: vec![ContactPurposeV1::ExplicitFollowUp],
            channels: vec![ContactChannelV1::AstrbotSession],
            valid_until_utc_ms: Some(now + 200_000_000),
            pause_until_utc_ms: None,
        });
        facts.push(consent);
    }
    let base_revision = runtime.current_revision(persona).unwrap();
    runtime
        .apply_interaction_fact_batch_v1(&InteractionFactBatchV1 {
            schema_version: ALPHA3_SCHEMA_VERSION,
            event_id: [seed + 2; 16],
            scope: relation.clone(),
            causal: CausalRef {
                turn_id: [seed + 3; 16],
                action_id: None,
                delivery_id: None,
                claim_id: None,
                base_revision,
            },
            facts,
        })
        .unwrap();
}

fn materialize_intention(
    runtime: &mut AstrRuntime,
    persona: &ScopeRef,
    generation: &mut u64,
    seed: u8,
    now: u64,
    day_start: u64,
) -> DurableIntentionV1 {
    let frozen = frozen(now, day_start);
    let event = TimeAdvanceV1 {
        event_id: [seed; 16],
        scope: persona.clone(),
        expected_generation: *generation,
        frozen_input_digest: frozen_time_input_digest(&frozen),
        frozen,
        stimulus: AutonomousStimulusV1 {
            arousal: Fixed::ONE,
            urgency: Fixed::ONE,
            emergency_authorized: false,
            source_digest: [seed + 1; 32],
        },
    };
    let claim = runtime.claim_wake_v2(persona, &event).unwrap();
    let intention = claim.proposal.legacy.intentions[0].clone();
    *generation = claim.proposal.legacy.state.generation;
    runtime.settle_wake_v2(&claim.claim_token).unwrap();
    intention
}

fn target(scope: &ScopeRef, now: u64) -> OutboundTargetEnvelopeV1 {
    let mut value = OutboundTargetEnvelopeV1 {
        schema_version: 1,
        target_kind: TargetKindV1::Private,
        umo_ciphertext: b"raw-umo-must-never-project".to_vec(),
        umo_nonce: vec![2; 12],
        key_id: "alpha3-super-secret-key".into(),
        umo_digest: [3; 32],
        platform_token: [4; 16],
        bot_token: scope.bot_token,
        persona_token: scope.persona_token,
        relation_token: scope.relation_token.unwrap(),
        session_token: scope.session_token,
        bound_at_utc_ms: now,
        binding_generation: 1,
        binding_digest: [0; 32],
    };
    value.binding_digest = outbound_target_binding_digest(&value);
    value
}

fn ready_witness() -> HostReadinessWitnessV1 {
    let items = [
        ReadinessItemKindV1::GlobalSwitch,
        ReadinessItemKindV1::RelationConsent,
        ReadinessItemKindV1::TrustedTimezone,
        ReadinessItemKindV1::TargetEnvelope,
        ReadinessItemKindV1::SecretStore,
        ReadinessItemKindV1::Provider,
        ReadinessItemKindV1::AstrbotSend,
        ReadinessItemKindV1::Budget,
        ReadinessItemKindV1::SleepQuietHours,
        ReadinessItemKindV1::PolicyRevision,
    ]
    .into_iter()
    .map(|kind| ReadinessItemV1 {
        kind,
        status: ReadinessItemStatusV1::Ready,
        witness_revision: 1,
    })
    .collect::<Vec<_>>();
    HostReadinessWitnessV1 {
        schema_version: ALPHA3_SCHEMA_VERSION,
        witness_digest: AstrRuntime::host_readiness_witness_digest_v1(&items),
        items,
    }
}

fn source_rows(path: &Path) -> BTreeMap<&'static str, Vec<String>> {
    let connection = Connection::open(path).unwrap();
    let queries = [
        ("binding", "SELECT hex(work_scope)||':'||hex(persona_scope)||':'||COALESCE(hex(relation_scope),'')||':'||hex(CAST(scope_json AS BLOB)) FROM autonomy_scope_binding ORDER BY work_scope"),
        ("body", "SELECT hex(persona_scope)||':'||generation||':'||state_revision||':'||hex(CAST(body_json AS BLOB)) FROM autonomous_runtime_state ORDER BY persona_scope"),
        ("wake", "SELECT hex(persona_scope)||':'||generation||':'||next_wake_at_utc_ms FROM wake_schedule ORDER BY persona_scope"),
        ("consent", "SELECT hex(relation_scope)||':'||consent_epoch||':'||revision||':'||state||':'||hex(CAST(body_json AS BLOB)) FROM relation_consent ORDER BY relation_scope,consent_epoch,revision"),
        ("consent_head", "SELECT hex(relation_scope)||':'||consent_epoch||':'||revision FROM relation_consent_head ORDER BY relation_scope"),
        ("contact", "SELECT hex(relation_scope)||':'||revision||':'||hex(CAST(body_json AS BLOB)) FROM relation_contact_process ORDER BY relation_scope"),
        ("budget", "SELECT hex(relation_scope)||':'||budget_day_start_utc_ms||':'||reserved_tokens||':'||limit_tokens||':'||charged_tokens||':'||used_tokens||':'||usage_known||':'||revision FROM externalization_budget ORDER BY relation_scope,budget_day_start_utc_ms"),
        ("budget_claim", "SELECT hex(claim_token)||':'||hex(relation_scope)||':'||budget_day_start_utc_ms||':'||reserved_tokens||':'||migrated_unknown_full_charge FROM externalization_budget_claim ORDER BY claim_token"),
        ("gate", "SELECT hex(relation_scope)||':'||gate_phase||':'||revision||':'||hex(CAST(body_json AS BLOB)) FROM gate_decision_latest ORDER BY relation_scope,gate_phase"),
        ("intention", "SELECT hex(intention_id)||':'||hex(persona_scope)||':'||hex(relation_scope)||':'||state||':'||revision||':'||hex(CAST(body_json AS BLOB)) FROM durable_intention ORDER BY intention_id"),
        ("outbound", "SELECT hex(outbound_id)||':'||hex(intention_id)||':'||state||':'||hex(target_digest)||':'||hex(CAST(body_json AS BLOB)) FROM outbound_attempt ORDER BY outbound_id"),
        ("claim", "SELECT hex(claim_token)||':'||claim_kind||':'||hex(record_id)||':'||lease_deadline_utc_ms||':'||hex(CAST(body_json AS BLOB)) FROM autonomy_claim ORDER BY claim_token"),
        ("operational", "SELECT sequence||':'||hex(persona_scope)||':'||persona_ordinal||':'||hex(relation_scope)||':'||hex(intention_id)||':'||event_kind||':'||hex(previous_digest)||':'||hex(chain_digest) FROM autonomy_operational_authority ORDER BY sequence"),
        ("operational_head", "SELECT hex(persona_scope)||':'||entry_count||':'||hex(head_digest)||':'||ever_seen FROM autonomy_operational_authority_head ORDER BY persona_scope"),
    ];
    queries
        .into_iter()
        .map(|(name, sql)| {
            let mut statement = connection.prepare(sql).unwrap();
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            (name, rows)
        })
        .collect()
}

fn seed_historical_grant(path: &Path, relation_scope: Digest, now: u64) {
    let connection = Connection::open(path).unwrap();
    let (epoch, revision, body): (u64, u64, String) = connection
        .query_row(
            "SELECT h.consent_epoch,h.revision,c.body_json
             FROM relation_consent_head AS h
             JOIN relation_consent AS c
               ON c.relation_scope=h.relation_scope
              AND c.consent_epoch=h.consent_epoch AND c.revision=h.revision
             WHERE h.relation_scope=?1",
            [relation_scope.to_vec()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    let mut consent: RelationConsentV1 = serde_json::from_str(&body).unwrap();
    assert_eq!(consent.consent_epoch, epoch);
    assert_eq!(consent.revision, revision);
    consent.state = RelationConsentStateV1::Granted;
    consent.purposes = vec![ContactPurposeV1::ExplicitFollowUp];
    consent.channels = vec![ContactChannelV1::AstrbotSession];
    consent.valid_from_utc_ms = now;
    consent.valid_until_utc_ms = Some(now + 200_000_000);
    consent.pause_until_utc_ms = None;
    consent.validate().unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE relation_consent SET state='granted',body_json=?4
                 WHERE relation_scope=?1 AND consent_epoch=?2 AND revision=?3",
                params![
                    relation_scope.to_vec(),
                    epoch,
                    revision,
                    serde_json::to_string(&consent).unwrap(),
                ],
            )
            .unwrap(),
        1
    );
}

fn assert_commitment(domain: &[u8], actual: Digest, zeroed_body: &[u8]) {
    assert_eq!(actual, wire::domain_hash(domain, &[zeroed_body]));
    assert_ne!(actual, [0; 32]);
}

#[test]
fn body_projections_are_bounded_read_only_and_integration_is_unavailable() {
    let path = temp_db();
    let mut runtime = AstrRuntime::open(&path).unwrap();
    let genesis = genesis_request();
    runtime.ensure_genesis(&genesis).unwrap();
    let persona = ScopeRef {
        bot_token: genesis.source.scope.bot_token,
        persona_token: genesis.source.scope.persona_token,
        relation_token: None,
        session_token: [20; 16],
    };
    let persona_scope =
        wire::persona_scope_digest(&persona.bot_token, &persona.persona_token, None);
    let mut relation = persona.clone();
    relation.relation_token = Some([21; 16]);
    relation.session_token = [22; 16];
    let relation_scope = wire::persona_scope_digest(
        &relation.bot_token,
        &relation.persona_token,
        relation.relation_token.as_ref(),
    );
    let initial = runtime
        .bootstrap_autonomy(&persona, &profile(persona_scope), None)
        .unwrap();
    runtime
        .bootstrap_autonomy(
            &relation,
            &profile(persona_scope),
            Some(&relation_policy(relation_scope)),
        )
        .unwrap();
    let mut foreign_relation = persona.clone();
    foreign_relation.relation_token = Some([71; 16]);
    foreign_relation.session_token = [72; 16];
    let foreign_relation_scope = wire::persona_scope_digest(
        &foreign_relation.bot_token,
        &foreign_relation.persona_token,
        foreign_relation.relation_token.as_ref(),
    );
    runtime
        .bootstrap_autonomy(
            &foreign_relation,
            &profile(persona_scope),
            Some(&relation_policy(foreign_relation_scope)),
        )
        .unwrap();
    let wall_now = u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    let day_start = wall_now - wall_now % 86_400_000;
    let mut now = day_start + 3_600_000;
    let mut generation = initial.generation;
    let reopen_probe_ref = runtime
        .execution_scoped_locator_v1(&relation, &[88; 16])
        .unwrap();
    let reopened = AstrRuntime::open(&path).unwrap();
    assert_eq!(
        reopened
            .execution_scoped_locator_v1(&relation, &[88; 16])
            .unwrap(),
        reopen_probe_ref
    );
    drop(reopened);

    seed_historical_grant(&path, relation_scope, now);
    apply_follow_up(&mut runtime, &persona, &relation, 30, now, false);
    now += 2_000_000;
    let first = materialize_intention(&mut runtime, &persona, &mut generation, 33, now, day_start);
    let outbound_target = target(&relation, now);
    runtime
        .bind_outbound_target(&relation, &outbound_target)
        .unwrap();
    runtime
        .upsert_relation_budget_policy_v1(&RelationBudgetPolicyV1 {
            schema_version: 1,
            relation_scope,
            timezone_id: "Asia/Shanghai".into(),
            daily_token_limit: 100,
            revision: 1,
            source_digest: [23; 32],
        })
        .unwrap();
    let first_externalization_request = GateAndClaimExternalizationRequestV2 {
        scope: relation.clone(),
        intention_id: first.intention_id,
        attempt_no: 1,
        expected_revision: 0,
        max_tokens: 40,
        caller_incarnation: [24; 32],
        readiness: ready_witness(),
        frozen: frozen(now + 2, day_start),
    };
    let first_gate = runtime
        .gate_and_claim_externalization_v2(&first_externalization_request)
        .unwrap();
    let first_claim = first_gate.claim.clone().unwrap();
    assert_eq!(
        Connection::open(&path)
            .unwrap()
            .execute(
                "UPDATE gate_decision_latest
                 SET body_json=json_set(body_json,'$.intention_public_ref',?2)
                 WHERE relation_scope=?1 AND gate_phase='externalization'",
                params![
                    relation_scope.to_vec(),
                    ae_contracts::hex::encode32(&[99; 32])
                ],
            )
            .unwrap(),
        1
    );
    let retried_first_gate = runtime
        .gate_and_claim_externalization_v2(&first_externalization_request)
        .unwrap();
    assert_eq!(retried_first_gate.decision, first_gate.decision);
    assert_eq!(retried_first_gate.claim, first_gate.claim);
    assert_eq!(
        relation_scope,
        [
            193, 205, 114, 72, 14, 117, 244, 198, 72, 47, 212, 75, 151, 151, 244, 154, 144, 125,
            184, 110, 246, 173, 133, 42, 242, 186, 97, 249, 45, 62, 233, 238,
        ]
    );
    assert_eq!(
        first.intention_id,
        [226, 155, 22, 255, 65, 2, 240, 182, 58, 2, 82, 188, 50, 129, 18, 130,]
    );
    let legacy_first_intention_ref = [
        208, 180, 145, 56, 44, 225, 139, 53, 79, 40, 119, 168, 112, 192, 110, 53, 68, 72, 152, 82,
        133, 125, 78, 20, 122, 91, 1, 237, 78, 140, 204, 27,
    ];
    assert_eq!(
        Connection::open(&path)
            .unwrap()
            .execute(
                "UPDATE autonomy_claim
                 SET body_json=json_remove(
                     json_set(body_json,'$.intention_public_ref',?2),
                     '$.gate_decision_snapshot'
                 )
                 WHERE claim_token=?1 AND claim_kind='externalization'",
                params![
                    first_claim.claim_token.to_vec(),
                    ae_contracts::hex::encode32(&legacy_first_intention_ref),
                ],
            )
            .unwrap(),
        1
    );
    let historical_claim_with_overwritten_latest = runtime
        .gate_and_claim_externalization_v2(&first_externalization_request)
        .unwrap_err();
    assert!(matches!(
        historical_claim_with_overwritten_latest,
        ae_runtime::RuntimeError::Alpha3(Alpha3ErrorCodeV1::ProposalStale)
    ));
    runtime
        .settle_externalization_v2(&ExternalizationSettleV2 {
            claim_token: first_claim.claim_token,
            outcome: ExternalizationOutcomeV1::Success,
            provider_usage: ProviderUsageV1 {
                known: true,
                used_tokens: Some(17),
            },
            candidate_digest: Some([25; 32]),
            candidate_ciphertext: Some(b"provider-content-must-never-project".to_vec()),
            caller_incarnation: [24; 32],
        })
        .unwrap();

    now += 1_000;
    apply_follow_up(&mut runtime, &persona, &relation, 40, now, false);
    now += 2_000_000;
    let second = materialize_intention(&mut runtime, &persona, &mut generation, 43, now, day_start);
    let second_gate = runtime
        .gate_and_claim_externalization_v2(&GateAndClaimExternalizationRequestV2 {
            scope: relation.clone(),
            intention_id: second.intention_id,
            attempt_no: 1,
            expected_revision: 0,
            max_tokens: 40,
            caller_incarnation: [26; 32],
            readiness: ready_witness(),
            frozen: frozen(now + 2, day_start),
        })
        .unwrap();
    runtime
        .settle_externalization_v2(&ExternalizationSettleV2 {
            claim_token: second_gate.claim.unwrap().claim_token,
            outcome: ExternalizationOutcomeV1::Success,
            provider_usage: ProviderUsageV1 {
                known: false,
                used_tokens: None,
            },
            candidate_digest: Some([27; 32]),
            candidate_ciphertext: Some(b"unknown-provider-content-must-never-project".to_vec()),
            caller_incarnation: [26; 32],
        })
        .unwrap();

    let raw_outbound_id: Vec<u8> = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT outbound_id FROM outbound_attempt WHERE intention_id=?1",
            [second.intention_id.to_vec()],
            |row| row.get(0),
        )
        .unwrap();
    let outbound_id: Id128 = raw_outbound_id.try_into().unwrap();
    let execution_public_ref = runtime
        .execution_scoped_locator_v1(&relation, &outbound_id)
        .unwrap();
    let foreign_execution_locator = runtime
        .execution_scoped_locator_v1(&foreign_relation, &outbound_id)
        .unwrap();
    let connection = Connection::open(&path).unwrap();
    let original_foreign_bodies: (String, String) = connection
        .query_row(
            "SELECT o.body_json,i.body_json
             FROM outbound_attempt AS o
             JOIN durable_intention AS i ON i.intention_id=o.intention_id
             WHERE o.outbound_id=?1",
            [outbound_id.to_vec()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE outbound_attempt SET body_json='{foreign-malformed'
                 WHERE outbound_id=?1",
                [outbound_id.to_vec()],
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .execute(
                "UPDATE durable_intention SET body_json='{foreign-malformed'
                 WHERE intention_id=?1",
                [second.intention_id.to_vec()],
            )
            .unwrap(),
        1
    );
    drop(connection);
    let foreign_observation = runtime
        .observe_execution_receipt_v1(ObserveExecutionReceiptRequestV1 {
            schema_version: 1,
            scope: foreign_relation.clone(),
            execution_public_ref: foreign_execution_locator,
        })
        .unwrap_err();
    assert!(matches!(
        foreign_observation,
        ae_runtime::RuntimeError::Alpha3(Alpha3ErrorCodeV1::ProjectionIncomplete)
    ));
    let foreign_dispatch = runtime
        .gate_and_claim_dispatch_v2(&GateAndClaimDispatchRequestV2 {
            scope: foreign_relation.clone(),
            outbound_public_ref: foreign_execution_locator,
            expected_target_digest: outbound_target.binding_digest,
            caller_incarnation: [73; 32],
            readiness: ready_witness(),
            frozen: frozen(now + 3, day_start),
        })
        .unwrap_err();
    assert!(matches!(
        foreign_dispatch,
        ae_runtime::RuntimeError::Alpha3(Alpha3ErrorCodeV1::ProposalStale)
    ));
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE outbound_attempt SET body_json=?2 WHERE outbound_id=?1",
                params![outbound_id.to_vec(), original_foreign_bodies.0],
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .execute(
                "UPDATE durable_intention SET body_json=?2 WHERE intention_id=?1",
                params![second.intention_id.to_vec(), original_foreign_bodies.1],
            )
            .unwrap(),
        1
    );
    drop(connection);
    let mut dispatch_claim = runtime
        .gate_and_claim_dispatch_v2(&GateAndClaimDispatchRequestV2 {
            scope: relation.clone(),
            outbound_public_ref: execution_public_ref,
            expected_target_digest: outbound_target.binding_digest,
            caller_incarnation: [28; 32],
            readiness: ready_witness(),
            frozen: frozen(now + 3, day_start),
        })
        .unwrap()
        .claim
        .unwrap();
    let current_dispatch_claim_token = dispatch_claim.claim_token;
    dispatch_claim.gate_decision_snapshot = None;
    dispatch_claim.claim_token = legacy_dispatch_claim_token(&dispatch_claim, &[28; 32]);
    assert_eq!(
        Connection::open(&path)
            .unwrap()
            .execute(
                "UPDATE autonomy_claim SET claim_token=?2,body_json=?3
                 WHERE claim_token=?1 AND claim_kind='dispatch'",
                params![
                    current_dispatch_claim_token.to_vec(),
                    dispatch_claim.claim_token.to_vec(),
                    serde_json::to_string(&dispatch_claim).unwrap(),
                ],
            )
            .unwrap(),
        1
    );
    runtime
        .settle_dispatch_v2(&DispatchSettleV2 {
            claim_token: dispatch_claim.claim_token,
            outcome: DispatchOutcomeV1::AdapterSubmitted,
            settled_at_utc_ms: now + 4,
            receipt_digest: None,
            caller_incarnation: [28; 32],
        })
        .unwrap();
    let execution_request = ObserveExecutionReceiptRequestV1 {
        schema_version: 1,
        scope: relation.clone(),
        execution_public_ref,
    };
    let execution_before_unrelated_writes = runtime
        .observe_execution_receipt_v1(execution_request.clone())
        .unwrap();
    let expected_execution_body_revision: u64 = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT persona_ordinal
             FROM autonomy_operational_authority
             WHERE persona_scope=?1 AND relation_scope=?2 AND intention_id=?3
               AND event_kind='dispatch_v2_settled'
               AND json_extract(delta_json,'$.outbound.outbound_id')=?4
               AND json_extract(delta_json,'$.outbound.state')='adapter_submitted'",
            params![
                persona_scope.to_vec(),
                relation_scope.to_vec(),
                second.intention_id.to_vec(),
                ae_contracts::hex::encode16(&outbound_id),
            ],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(
        execution_before_unrelated_writes.body_revision,
        expected_execution_body_revision
    );

    let current_dispatch_gate = ObserveGateReasonsRequestV1 {
        schema_version: 1,
        scope: relation.clone(),
        phase: GatePhaseV1::Dispatch,
        observed_at_utc_ms: now + 4,
    };
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE durable_intention
                 SET state='terminal',body_json=json_set(body_json,'$.state','terminal')
                 WHERE intention_id=?1",
                [second.intention_id.to_vec()],
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .execute(
                "UPDATE outbound_attempt
                 SET state='terminal',body_json=json_set(body_json,'$.state','terminal')
                 WHERE outbound_id=?1",
                [outbound_id.to_vec()],
            )
            .unwrap(),
        1
    );
    drop(connection);
    let terminal_gate = runtime
        .observe_gate_reasons_v1(current_dispatch_gate.clone())
        .unwrap_err();
    assert!(
        matches!(
            terminal_gate,
            ae_runtime::RuntimeError::Alpha3(Alpha3ErrorCodeV1::ProjectionIncomplete)
        ),
        "unexpected terminal gate error: {terminal_gate:?}"
    );

    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE durable_intention
                 SET state='dispatch_pending',
                     body_json=json_set(body_json,'$.state','dispatch_pending')
                 WHERE intention_id=?1",
                [second.intention_id.to_vec()],
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .execute(
                "UPDATE outbound_attempt
                 SET state='dispatch_pending',
                     body_json=json_set(body_json,'$.state','dispatch_pending')
                 WHERE outbound_id=?1",
                [outbound_id.to_vec()],
            )
            .unwrap(),
        1
    );
    drop(connection);
    let refreshed_dispatch_request = GateAndClaimDispatchRequestV2 {
        scope: relation.clone(),
        outbound_public_ref: execution_public_ref,
        expected_target_digest: outbound_target.binding_digest,
        caller_incarnation: [31; 32],
        readiness: ready_witness(),
        frozen: frozen(now + 5, day_start),
    };
    let refreshed_dispatch = runtime
        .gate_and_claim_dispatch_v2(&refreshed_dispatch_request)
        .unwrap();
    assert_eq!(
        refreshed_dispatch.decision.decision,
        GateDecisionKindV2::Allowed
    );
    assert!(refreshed_dispatch
        .decision
        .authority_snapshot_digest
        .is_some());
    assert!(refreshed_dispatch
        .decision
        .authority_expires_at_utc_ms
        .is_some());
    assert_eq!(
        serde_json::to_value(refreshed_dispatch.claim.as_ref().unwrap()).unwrap()
            ["gate_decision_snapshot"],
        serde_json::to_value(&refreshed_dispatch.decision).unwrap()
    );
    let connection = Connection::open(&path).unwrap();
    let (raw_dispatch_claim_token, immutable_dispatch_claim_body): (Vec<u8>, String) = connection
        .query_row(
            "SELECT claim_token,body_json FROM autonomy_claim
             WHERE claim_kind='dispatch' AND record_id=?1",
            [outbound_id.to_vec()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        raw_dispatch_claim_token,
        refreshed_dispatch
            .claim
            .as_ref()
            .unwrap()
            .claim_token
            .to_vec()
    );
    let dispatch_gate_body: String = connection
        .query_row(
            "SELECT body_json FROM gate_decision_latest
             WHERE relation_scope=?1 AND gate_phase='dispatch'",
            [relation_scope.to_vec()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE gate_decision_latest
                 SET body_json=json_set(body_json,'$.intention_public_ref',?2)
                 WHERE relation_scope=?1 AND gate_phase='dispatch'",
                params![
                    relation_scope.to_vec(),
                    ae_contracts::hex::encode32(&[99; 32])
                ],
            )
            .unwrap(),
        1
    );
    drop(connection);
    let retry_after_latest_overwrite = runtime
        .gate_and_claim_dispatch_v2(&refreshed_dispatch_request)
        .unwrap();
    assert_eq!(retry_after_latest_overwrite, refreshed_dispatch);
    assert_eq!(
        Connection::open(&path)
            .unwrap()
            .execute(
                "UPDATE gate_decision_latest SET body_json=?2
                 WHERE relation_scope=?1 AND gate_phase='dispatch'",
                params![relation_scope.to_vec(), dispatch_gate_body.clone()],
            )
            .unwrap(),
        1
    );

    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE autonomy_claim
                 SET body_json=json_set(
                     body_json,
                     '$.gate_decision_snapshot.authority_snapshot_digest',
                     ?2
                 )
                 WHERE claim_kind='dispatch' AND record_id=?1",
                params![outbound_id.to_vec(), ae_contracts::hex::encode32(&[97; 32]),],
            )
            .unwrap(),
        1
    );
    drop(connection);
    let mutated_authority_digest_retry =
        runtime.gate_and_claim_dispatch_v2(&refreshed_dispatch_request);
    let connection = Connection::open(&path).unwrap();
    let token_after_digest_mutation: Vec<u8> = connection
        .query_row(
            "SELECT claim_token FROM autonomy_claim
             WHERE claim_kind='dispatch' AND record_id=?1",
            [outbound_id.to_vec()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(token_after_digest_mutation, raw_dispatch_claim_token);
    assert_eq!(
        connection
            .execute(
                "UPDATE autonomy_claim SET body_json=?2
                 WHERE claim_kind='dispatch' AND record_id=?1",
                params![outbound_id.to_vec(), immutable_dispatch_claim_body.clone()],
            )
            .unwrap(),
        1
    );
    let extended_authority_expiry = refreshed_dispatch
        .decision
        .authority_expires_at_utc_ms
        .unwrap()
        .saturating_add(10_000);
    assert_eq!(
        connection
            .execute(
                "UPDATE autonomy_claim
                 SET body_json=json_set(
                     body_json,
                     '$.gate_decision_snapshot.authority_expires_at_utc_ms',
                     ?2
                 )
                 WHERE claim_kind='dispatch' AND record_id=?1",
                params![outbound_id.to_vec(), extended_authority_expiry],
            )
            .unwrap(),
        1
    );
    drop(connection);
    let extended_authority_expiry_retry =
        runtime.gate_and_claim_dispatch_v2(&refreshed_dispatch_request);
    let connection = Connection::open(&path).unwrap();
    let token_after_expiry_mutation: Vec<u8> = connection
        .query_row(
            "SELECT claim_token FROM autonomy_claim
             WHERE claim_kind='dispatch' AND record_id=?1",
            [outbound_id.to_vec()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(token_after_expiry_mutation, raw_dispatch_claim_token);
    assert_eq!(
        connection
            .execute(
                "UPDATE autonomy_claim SET body_json=?2
                 WHERE claim_kind='dispatch' AND record_id=?1",
                params![outbound_id.to_vec(), immutable_dispatch_claim_body],
            )
            .unwrap(),
        1
    );
    drop(connection);
    let unchanged_snapshot_retry = runtime
        .gate_and_claim_dispatch_v2(&refreshed_dispatch_request)
        .unwrap();
    assert!(matches!(
        mutated_authority_digest_retry,
        Err(ae_runtime::RuntimeError::Alpha3(
            Alpha3ErrorCodeV1::ProposalStale
        ))
    ));
    assert!(matches!(
        extended_authority_expiry_retry,
        Err(ae_runtime::RuntimeError::Alpha3(
            Alpha3ErrorCodeV1::ProposalStale
        ))
    ));
    assert_eq!(unchanged_snapshot_retry, refreshed_dispatch);

    let mut changed_frozen_retry = refreshed_dispatch_request.clone();
    changed_frozen_retry.frozen.observed_now_utc_ms += 1;
    changed_frozen_retry.frozen.effective_now_utc_ms += 1;
    let changed_frozen = runtime
        .gate_and_claim_dispatch_v2(&changed_frozen_retry)
        .unwrap_err();
    assert!(matches!(
        changed_frozen,
        ae_runtime::RuntimeError::Alpha3(Alpha3ErrorCodeV1::ProposalStale)
    ));
    let mut changed_readiness_retry = refreshed_dispatch_request.clone();
    changed_readiness_retry.readiness.items[0].witness_revision += 1;
    changed_readiness_retry.readiness.witness_digest =
        AstrRuntime::host_readiness_witness_digest_v1(&changed_readiness_retry.readiness.items);
    let changed_readiness = runtime
        .gate_and_claim_dispatch_v2(&changed_readiness_retry)
        .unwrap_err();
    assert!(matches!(
        changed_readiness,
        ae_runtime::RuntimeError::Alpha3(Alpha3ErrorCodeV1::ProposalStale)
    ));

    let connection = Connection::open(&path).unwrap();
    let (dispatch_claim_deadline, dispatch_claim_body): (i64, String) = connection
        .query_row(
            "SELECT lease_deadline_utc_ms,body_json FROM autonomy_claim
             WHERE claim_kind='dispatch' AND record_id=?1",
            [outbound_id.to_vec()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE autonomy_claim
                 SET lease_deadline_utc_ms=1,
                     body_json=json_set(body_json,'$.lease_deadline_utc_ms',1)
                 WHERE claim_kind='dispatch' AND record_id=?1",
                [outbound_id.to_vec()],
            )
            .unwrap(),
        1
    );
    drop(connection);
    let expired_retry = runtime
        .gate_and_claim_dispatch_v2(&refreshed_dispatch_request)
        .unwrap_err();
    assert!(matches!(
        expired_retry,
        ae_runtime::RuntimeError::Alpha3(Alpha3ErrorCodeV1::ProposalStale)
    ));
    let mut historical_dispatch_claim: DispatchClaimV2 =
        serde_json::from_str(&dispatch_claim_body).unwrap();
    let current_dispatch_claim_token = historical_dispatch_claim.claim_token;
    historical_dispatch_claim.gate_decision_snapshot = None;
    historical_dispatch_claim.claim_token =
        legacy_dispatch_claim_token(&historical_dispatch_claim, &[31; 32]);
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE autonomy_claim
                 SET claim_token=?2,lease_deadline_utc_ms=?3,body_json=?4
                 WHERE claim_kind='dispatch' AND record_id=?1",
                params![
                    outbound_id.to_vec(),
                    historical_dispatch_claim.claim_token.to_vec(),
                    dispatch_claim_deadline,
                    serde_json::to_string(&historical_dispatch_claim).unwrap(),
                ],
            )
            .unwrap(),
        1
    );
    drop(connection);
    let historical_matching_retry = runtime
        .gate_and_claim_dispatch_v2(&refreshed_dispatch_request)
        .unwrap();
    assert_eq!(
        historical_matching_retry.decision,
        refreshed_dispatch.decision
    );
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE gate_decision_latest
                 SET body_json=json_set(body_json,'$.intention_public_ref',?2)
                 WHERE relation_scope=?1 AND gate_phase='dispatch'",
                params![
                    relation_scope.to_vec(),
                    ae_contracts::hex::encode32(&[98; 32])
                ],
            )
            .unwrap(),
        1
    );
    drop(connection);
    let historical_overwritten_retry = runtime
        .gate_and_claim_dispatch_v2(&refreshed_dispatch_request)
        .unwrap_err();
    assert!(matches!(
        historical_overwritten_retry,
        ae_runtime::RuntimeError::Alpha3(Alpha3ErrorCodeV1::ProposalStale)
    ));
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE gate_decision_latest SET body_json=?2
                 WHERE relation_scope=?1 AND gate_phase='dispatch'",
                params![relation_scope.to_vec(), dispatch_gate_body],
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .execute(
                "UPDATE autonomy_claim SET claim_token=?2,body_json=?3
                  WHERE claim_kind='dispatch' AND record_id=?1",
                params![
                    outbound_id.to_vec(),
                    current_dispatch_claim_token.to_vec(),
                    dispatch_claim_body,
                ],
            )
            .unwrap(),
        1
    );
    drop(connection);
    let retried_refreshed_dispatch = runtime
        .gate_and_claim_dispatch_v2(&refreshed_dispatch_request)
        .unwrap();
    assert_eq!(retried_refreshed_dispatch, refreshed_dispatch);
    let active_dispatch_observation = ObserveGateReasonsRequestV1 {
        schema_version: 1,
        scope: relation.clone(),
        phase: GatePhaseV1::Dispatch,
        observed_at_utc_ms: now + 6,
    };
    let active_dispatch_gate = runtime
        .observe_gate_reasons_v1(active_dispatch_observation.clone())
        .unwrap_err();
    assert!(matches!(
        active_dispatch_gate,
        ae_runtime::RuntimeError::Alpha3(Alpha3ErrorCodeV1::ProjectionIncomplete)
    ));
    let refreshed_claim = refreshed_dispatch.claim.unwrap();
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .execute(
                "DELETE FROM autonomy_claim WHERE claim_token=?1 AND claim_kind='dispatch'",
                [refreshed_claim.claim_token.to_vec()],
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .execute(
                "UPDATE outbound_attempt
                 SET state='dispatch_pending',
                     body_json=json_set(body_json,'$.state','dispatch_pending')
                 WHERE outbound_id=?1 AND state='adapter_call_started'",
                [outbound_id.to_vec()],
            )
            .unwrap(),
        1
    );
    drop(connection);
    let current_dispatch_gate = active_dispatch_observation;
    let dispatch_before_sleep = runtime
        .observe_gate_reasons_v1(current_dispatch_gate.clone())
        .unwrap();
    assert_eq!(dispatch_before_sleep.decision, GateDecisionKindV2::Allowed);
    let body_before_sleep_observed_at = now + 6;
    let body_before_sleep = runtime
        .observe_body_snapshot_v1(ObserveBodySnapshotRequestV1 {
            schema_version: 1,
            scope: relation.clone(),
            observed_at_utc_ms: body_before_sleep_observed_at,
        })
        .unwrap();
    assert!(!body_before_sleep.outreach_available);
    assert_eq!(
        body_before_sleep.expires_at_utc_ms,
        body_before_sleep_observed_at
            .saturating_add(60_000)
            .min(dispatch_before_sleep.expires_at_utc_ms)
    );
    assert!(
        body_before_sleep.expires_at_utc_ms < body_before_sleep_observed_at.saturating_add(60_000)
    );
    let (intention_deadline, basis_deadline): (u64, u64) = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT json_extract(i.body_json,'$.expires_at_utc_ms'),
                    json_extract(b.body_json,'$.expires_at_utc_ms')
             FROM durable_intention AS i
             JOIN contact_intention_basis AS b ON b.intention_id=i.intention_id
             WHERE i.intention_id=?1",
            [second.intention_id.to_vec()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let fixed_gate_expiry = dispatch_before_sleep
        .evaluated_at_utc_ms
        .saturating_add(60_000)
        .min(intention_deadline)
        .min(basis_deadline)
        .min(dispatch_before_sleep.budget_day_start_utc_ms + 86_400_000);
    assert_eq!(dispatch_before_sleep.expires_at_utc_ms, fixed_gate_expiry);
    let expired_gate = runtime
        .observe_gate_reasons_v1(ObserveGateReasonsRequestV1 {
            observed_at_utc_ms: fixed_gate_expiry,
            ..current_dispatch_gate.clone()
        })
        .unwrap_err();
    assert!(matches!(
        expired_gate,
        ae_runtime::RuntimeError::Alpha3(Alpha3ErrorCodeV1::ProjectionIncomplete)
    ));

    let connection = Connection::open(&path).unwrap();
    let current_gate_body: String = connection
        .query_row(
            "SELECT body_json FROM gate_decision_latest
             WHERE relation_scope=?1 AND gate_phase='dispatch'",
            [relation_scope.to_vec()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE gate_decision_latest
                 SET body_json=json_set(body_json,'$.authority_expires_at_utc_ms',?2)
                 WHERE relation_scope=?1 AND gate_phase='dispatch'",
                params![
                    relation_scope.to_vec(),
                    i64::try_from(fixed_gate_expiry + 60_000).unwrap(),
                ],
            )
            .unwrap(),
        1
    );
    drop(connection);
    let mutable_expiry = runtime
        .observe_gate_reasons_v1(current_dispatch_gate.clone())
        .unwrap_err();
    assert!(matches!(
        mutable_expiry,
        ae_runtime::RuntimeError::Alpha3(Alpha3ErrorCodeV1::ProjectionIncomplete)
    ));
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE gate_decision_latest SET body_json=?2
                 WHERE relation_scope=?1 AND gate_phase='dispatch'",
                params![relation_scope.to_vec(), current_gate_body.clone()],
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .execute(
                "UPDATE gate_decision_latest
                 SET body_json=json_remove(
                     body_json,
                     '$.authority_snapshot_digest',
                     '$.authority_expires_at_utc_ms'
                 )
                 WHERE relation_scope=?1 AND gate_phase='dispatch'",
                [relation_scope.to_vec()],
            )
            .unwrap(),
        1
    );
    drop(connection);
    let historical_gate = runtime
        .observe_gate_reasons_v1(current_dispatch_gate.clone())
        .unwrap_err();
    assert!(matches!(
        historical_gate,
        ae_runtime::RuntimeError::Alpha3(Alpha3ErrorCodeV1::ProjectionIncomplete)
    ));
    assert_eq!(
        Connection::open(&path)
            .unwrap()
            .execute(
                "UPDATE gate_decision_latest SET body_json=?2
                 WHERE relation_scope=?1 AND gate_phase='dispatch'",
                params![relation_scope.to_vec(), current_gate_body],
            )
            .unwrap(),
        1
    );

    let connection = Connection::open(&path).unwrap();
    let original_basis_body: String = connection
        .query_row(
            "SELECT body_json FROM contact_intention_basis WHERE intention_id=?1",
            [second.intention_id.to_vec()],
            |row| row.get(0),
        )
        .unwrap();
    connection
        .execute(
            "UPDATE contact_intention_basis
             SET body_json=json_set(body_json,'$.cause_digest',?2)
             WHERE intention_id=?1",
            params![
                second.intention_id.to_vec(),
                ae_contracts::hex::encode32(&[91; 32]),
            ],
        )
        .unwrap();
    drop(connection);
    let cause_mismatch = runtime
        .observe_gate_reasons_v1(current_dispatch_gate.clone())
        .unwrap_err();
    assert!(matches!(
        cause_mismatch,
        ae_runtime::RuntimeError::Alpha3(Alpha3ErrorCodeV1::ProjectionIncomplete)
    ));
    assert_eq!(
        Connection::open(&path)
            .unwrap()
            .execute(
                "UPDATE contact_intention_basis SET body_json=?2 WHERE intention_id=?1",
                params![second.intention_id.to_vec(), original_basis_body],
            )
            .unwrap(),
        1
    );

    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE proactive_readiness
                 SET revision=revision+1,
                     body_json=json_set(body_json,'$.revision',revision+1)
                 WHERE relation_scope=?1",
                [relation_scope.to_vec()],
            )
            .unwrap(),
        1
    );
    drop(connection);
    let stale_readiness = runtime
        .observe_gate_reasons_v1(current_dispatch_gate.clone())
        .unwrap_err();
    assert!(matches!(
        stale_readiness,
        ae_runtime::RuntimeError::Alpha3(Alpha3ErrorCodeV1::ProjectionIncomplete)
    ));
    assert_eq!(
        Connection::open(&path)
            .unwrap()
            .execute(
                "UPDATE proactive_readiness
             SET body_json=json_set(body_json,'$.revision',revision-1),revision=revision-1
             WHERE relation_scope=?1",
                [relation_scope.to_vec()],
            )
            .unwrap(),
        1
    );

    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE autonomous_runtime_state
                 SET body_json=json_set(body_json,'$.sleep_state','asleep')
                 WHERE persona_scope=?1",
                [persona_scope.to_vec()],
            )
            .unwrap(),
        1
    );
    drop(connection);
    let sleeping_gate = runtime
        .observe_gate_reasons_v1(current_dispatch_gate.clone())
        .unwrap_err();
    assert!(matches!(
        sleeping_gate,
        ae_runtime::RuntimeError::Alpha3(Alpha3ErrorCodeV1::ProjectionIncomplete)
    ));
    let sleeping_body = runtime
        .observe_body_snapshot_v1(ObserveBodySnapshotRequestV1 {
            schema_version: 1,
            scope: relation.clone(),
            observed_at_utc_ms: now + 4,
        })
        .unwrap();
    assert!(!sleeping_body.outreach_available);
    assert!(sleeping_body
        .blocking_reasons
        .contains(&GateReasonV2::PersonaAsleep));
    assert!(sleeping_body
        .blocking_reasons
        .contains(&GateReasonV2::CauseUnavailable));

    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE autonomous_runtime_state
                 SET body_json=json_set(body_json,'$.sleep_state','awake')
                 WHERE persona_scope=?1",
                [persona_scope.to_vec()],
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .execute(
                "UPDATE durable_intention
                 SET state='adapter_submitted',
                     body_json=json_set(body_json,'$.state','adapter_submitted')
                 WHERE intention_id=?1",
                [second.intention_id.to_vec()],
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .execute(
                "UPDATE outbound_attempt
                 SET state='adapter_submitted',
                     body_json=json_set(body_json,'$.state','adapter_submitted')
                 WHERE outbound_id=?1",
                [outbound_id.to_vec()],
            )
            .unwrap(),
        1
    );
    drop(connection);

    now += 1_000;
    apply_follow_up(&mut runtime, &persona, &relation, 50, now, false);
    now += 2_000_000;
    let third = materialize_intention(&mut runtime, &persona, &mut generation, 53, now, day_start);
    assert!(runtime
        .gate_and_claim_externalization_v2(&GateAndClaimExternalizationRequestV2 {
            scope: relation.clone(),
            intention_id: third.intention_id,
            attempt_no: 1,
            expected_revision: 0,
            max_tokens: 40,
            caller_incarnation: [29; 32],
            readiness: ready_witness(),
            frozen: frozen(now + 2, day_start)
        })
        .unwrap()
        .claim
        .is_some());
    now += 1_000;
    apply_follow_up(&mut runtime, &persona, &relation, 60, now, false);
    now += 2_000_000;
    let fourth = materialize_intention(&mut runtime, &persona, &mut generation, 63, now, day_start);
    let blocked = runtime
        .gate_and_claim_externalization_v2(&GateAndClaimExternalizationRequestV2 {
            scope: relation.clone(),
            intention_id: fourth.intention_id,
            attempt_no: 1,
            expected_revision: 0,
            max_tokens: 40,
            caller_incarnation: [30; 32],
            readiness: ready_witness(),
            frozen: frozen(now + 2, day_start),
        })
        .unwrap();
    assert_eq!(
        blocked.decision.reason,
        GateReasonV2::ProviderUsageUnsettled
    );

    let body_request = ObserveBodySnapshotRequestV1 {
        schema_version: 1,
        scope: relation.clone(),
        observed_at_utc_ms: now + 2,
    };
    let budget_request = ObserveBudgetSummaryRequestV1 {
        schema_version: 1,
        scope: relation.clone(),
        observed_at_utc_ms: now + 2,
    };
    let externalization_request = ObserveGateReasonsRequestV1 {
        schema_version: 1,
        scope: relation.clone(),
        phase: GatePhaseV1::Externalization,
        observed_at_utc_ms: now + 2,
    };
    let dispatch_request = ObserveGateReasonsRequestV1 {
        phase: GatePhaseV1::Dispatch,
        ..externalization_request.clone()
    };

    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE autonomous_runtime_state
                 SET body_json=json_set(body_json,'$.process_s','do-not-read')
                 WHERE persona_scope=?1",
                [persona_scope.to_vec()],
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .execute(
                "UPDATE outbound_attempt
                 SET body_json=json_set(body_json,'$.candidate_ciphertext','do-not-read')
                 WHERE outbound_id=?1",
                [outbound_id.to_vec()],
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .execute(
                "UPDATE durable_intention
                 SET body_json=json_set(body_json,'$.workspace_residual','do-not-read')
                 WHERE intention_id=?1",
                [second.intention_id.to_vec()],
            )
            .unwrap(),
        1
    );
    drop(connection);

    let source_before = source_rows(&path);
    let fingerprint_before = runtime
        .alpha3_authoritative_fingerprint_v1(&persona_scope)
        .unwrap();
    let body = runtime
        .observe_body_snapshot_v1(body_request.clone())
        .unwrap();
    assert_eq!(
        body,
        runtime.observe_body_snapshot_v1(body_request).unwrap()
    );
    let execution = runtime
        .observe_execution_receipt_v1(execution_request.clone())
        .unwrap();
    assert_eq!(execution_before_unrelated_writes, execution);
    assert_eq!(
        execution,
        runtime
            .observe_execution_receipt_v1(execution_request.clone())
            .unwrap()
    );
    let budget = runtime
        .observe_budget_summary_v1(budget_request.clone())
        .unwrap();
    assert_eq!(
        budget,
        runtime.observe_budget_summary_v1(budget_request).unwrap()
    );
    let externalization = runtime
        .observe_gate_reasons_v1(externalization_request.clone())
        .unwrap();
    assert_eq!(
        externalization,
        runtime
            .observe_gate_reasons_v1(externalization_request.clone())
            .unwrap()
    );
    for request in [dispatch_request.clone(), dispatch_request] {
        let stale_dispatch = runtime.observe_gate_reasons_v1(request).unwrap_err();
        assert!(matches!(
            stale_dispatch,
            ae_runtime::RuntimeError::Alpha3(Alpha3ErrorCodeV1::ProjectionIncomplete)
        ));
    }
    let integration = integration_availability_v1();
    assert_eq!(integration, integration_availability_v1());
    assert_eq!(source_before, source_rows(&path));
    assert_eq!(
        fingerprint_before,
        runtime
            .alpha3_authoritative_fingerprint_v1(&persona_scope)
            .unwrap()
    );

    assert_eq!(body.persona_token, relation.persona_token);
    assert_eq!(body.wake_state, BodyWakeStateV1::Awake);
    assert_eq!(body.execution_capacity, ExecutionCapacityV1::Available);
    assert!(!body.outreach_available);
    assert!(body
        .blocking_reasons
        .contains(&GateReasonV2::CauseUnavailable));
    assert!(body.blocking_reasons.len() <= 16);
    let wake: u64 = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT next_wake_at_utc_ms FROM wake_schedule WHERE persona_scope=?1",
            [persona_scope.to_vec()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        body.next_wake_window,
        Some(WakeWindowV1 {
            earliest_utc_ms: wake,
            latest_utc_ms: wake
        })
    );
    assert_eq!(
        execution.outcome,
        ExecutionReceiptOutcomeV1::AdapterSubmitted
    );
    assert_ne!(
        execution.outcome,
        ExecutionReceiptOutcomeV1::PlatformAccepted
    );
    assert_ne!(execution.outcome, ExecutionReceiptOutcomeV1::Delivered);
    assert!(!execution.provider_usage_known);
    assert_eq!(execution.provider_used_tokens, None);
    assert_eq!(execution.adapter_receipt_ref, None);
    assert!(execution.reason_codes.len() <= 16);
    assert_eq!(
        (
            budget.limit_tokens,
            budget.charged_tokens,
            budget.reserved_tokens,
            budget.remaining_tokens
        ),
        (100, 57, 40, 3)
    );
    assert!(!budget.usage_known);
    assert_eq!(budget.used_tokens, None);
    assert_eq!(externalization.reason, GateReasonV2::ProviderUsageUnsettled);
    assert_eq!(
        (dispatch_before_sleep.decision, dispatch_before_sleep.reason),
        (
            GateDecisionKindV2::Allowed,
            GateReasonV2::AllRequirementsSatisfied
        )
    );

    let mut zeroed_body = body.clone();
    zeroed_body.body_commitment = [0; 32];
    assert_commitment(
        b"ae.body-snapshot.v1",
        body.body_commitment,
        &serde_json::to_vec(&zeroed_body).unwrap(),
    );
    let mut zeroed_execution = execution.clone();
    zeroed_execution.body_commitment = [0; 32];
    assert_commitment(
        b"ae.execution-receipt.v1",
        execution.body_commitment,
        &serde_json::to_vec(&zeroed_execution).unwrap(),
    );
    let mut zeroed_budget = budget.clone();
    zeroed_budget.body_commitment = [0; 32];
    assert_commitment(
        b"ae.budget-summary.v1",
        budget.body_commitment,
        &serde_json::to_vec(&zeroed_budget).unwrap(),
    );
    let mut zeroed_gate = externalization.clone();
    zeroed_gate.body_commitment = [0; 32];
    assert_commitment(
        b"ae.gate-reasons.v1",
        externalization.body_commitment,
        &serde_json::to_vec(&zeroed_gate).unwrap(),
    );

    assert_eq!(
        integration.state,
        IntegrationAvailabilityStateV1::UnavailableHostAttestation
    );
    assert_eq!(
        integration.required_capabilities,
        vec![
            HostAttestationRequirementV1::ServiceInstanceProof,
            HostAttestationRequirementV1::CallerIdentityProof,
            HostAttestationRequirementV1::InstallationManifestBinding,
            HostAttestationRequirementV1::BoundedCall,
            HostAttestationRequirementV1::LifecycleRevocation
        ]
    );
    let integration_json = serde_json::to_string(&integration).unwrap();
    assert!(integration_json.contains("UNAVAILABLE_HOST_ATTESTATION"));
    assert!(!integration_json.contains("route"));

    let serialized = serde_json::to_string(&(
        body,
        execution,
        budget,
        externalization,
        dispatch_before_sleep,
        integration,
    ))
    .unwrap();
    for forbidden in [
        "world_mode",
        "world_layer",
        "notable_nodes",
        "dreams",
        "privacy_controls",
        "intentions",
        "outbounds",
        "claims",
        "umo_ciphertext",
        "raw-umo-must-never-project",
        "candidate_ciphertext",
        "provider-content-must-never-project",
        "unknown-provider-content-must-never-project",
        "alpha3-super-secret-key",
        "claim_token",
        "process_s",
        "process_c",
        "workspace_residual",
        "affiliation_need",
        "unfinished_topic_salience",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "projection leaked {forbidden}"
        );
    }
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE gate_decision_latest
                 SET body_json=json_set(
                     body_json,
                     '$.consent_revision',
                     json_extract(body_json,'$.consent_revision') + 1
                 )
                 WHERE relation_scope=?1 AND gate_phase='externalization'",
                [relation_scope.to_vec()],
            )
            .unwrap(),
        1
    );
    drop(connection);
    let stale_authority = runtime
        .observe_gate_reasons_v1(externalization_request)
        .unwrap_err();
    assert!(matches!(
        stale_authority,
        ae_runtime::RuntimeError::Alpha3(Alpha3ErrorCodeV1::ProjectionIncomplete)
    ));

    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE outbound_attempt
                 SET body_json=json_set(body_json,'$.state','dispatch_pending')
                 WHERE outbound_id=?1",
                [outbound_id.to_vec()],
            )
            .unwrap(),
        1
    );
    let mismatched_state = runtime
        .observe_execution_receipt_v1(execution_request.clone())
        .unwrap_err();
    assert!(matches!(
        mismatched_state,
        ae_runtime::RuntimeError::Alpha3(Alpha3ErrorCodeV1::ProjectionIncomplete)
    ));
    assert_eq!(
        connection
            .execute(
                "UPDATE outbound_attempt
                 SET body_json=json_set(body_json,'$.state','adapter_submitted')
                 WHERE outbound_id=?1",
                [outbound_id.to_vec()],
            )
            .unwrap(),
        1
    );

    let existing_candidates: usize = connection
        .query_row(
            "SELECT COUNT(*)
             FROM outbound_attempt AS o
             JOIN durable_intention AS i ON i.intention_id=o.intention_id
             WHERE i.relation_scope=?1",
            [relation_scope.to_vec()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(existing_candidates < MAX_ALPHA3_PUBLIC_RECORDS + 1);
    for offset in 0..(MAX_ALPHA3_PUBLIC_RECORDS + 1 - existing_candidates) {
        let intention_id = [u8::try_from(180 + offset).unwrap(); 16];
        let synthetic_outbound_id = [u8::try_from(210 + offset).unwrap(); 16];
        let semantic_digest = [u8::try_from(150 + offset).unwrap(); 32];
        assert_eq!(
            connection
                .execute(
                    "INSERT INTO durable_intention(
                         intention_id,persona_scope,relation_scope,semantic_digest,
                         state,revision,body_json
                     )
                     SELECT ?1,persona_scope,relation_scope,?2,state,revision,
                            json_set(body_json,'$.intention_id',?3)
                     FROM durable_intention WHERE intention_id=?4",
                    params![
                        intention_id.to_vec(),
                        semantic_digest.to_vec(),
                        ae_contracts::hex::encode16(&intention_id),
                        second.intention_id.to_vec(),
                    ],
                )
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .execute(
                    "INSERT INTO outbound_attempt(
                         outbound_id,intention_id,state,target_digest,body_json
                     )
                     SELECT ?1,?2,state,target_digest,
                            json_set(
                                body_json,
                                '$.outbound_id',?3,
                                '$.intention_id',?4
                            )
                     FROM outbound_attempt WHERE outbound_id=?5",
                    params![
                        synthetic_outbound_id.to_vec(),
                        intention_id.to_vec(),
                        ae_contracts::hex::encode16(&synthetic_outbound_id),
                        ae_contracts::hex::encode16(&intention_id),
                        outbound_id.to_vec(),
                    ],
                )
                .unwrap(),
            1
        );
    }
    drop(connection);
    assert_eq!(
        runtime
            .observe_execution_receipt_v1(execution_request.clone())
            .unwrap(),
        execution_before_unrelated_writes
    );
    let mut tampered_execution_ref = execution_public_ref;
    tampered_execution_ref[0] ^= 1;
    let tampered_lookup = runtime
        .observe_execution_receipt_v1(ObserveExecutionReceiptRequestV1 {
            execution_public_ref: tampered_execution_ref,
            ..execution_request.clone()
        })
        .unwrap_err();
    assert!(matches!(
        tampered_lookup,
        ae_runtime::RuntimeError::Alpha3(Alpha3ErrorCodeV1::ProjectionIncomplete)
    ));
    let wrong_domain_lookup = runtime
        .observe_execution_receipt_v1(ObserveExecutionReceiptRequestV1 {
            execution_public_ref: runtime
                .intention_scoped_locator_v1(&relation, &outbound_id)
                .unwrap(),
            ..execution_request.clone()
        })
        .unwrap_err();
    assert!(matches!(
        wrong_domain_lookup,
        ae_runtime::RuntimeError::Alpha3(Alpha3ErrorCodeV1::ProjectionIncomplete)
    ));
    let mut wrong_relation = relation.clone();
    wrong_relation.relation_token = Some([29; 16]);
    wrong_relation.session_token = [30; 16];
    let locator_without_scope_authority = runtime
        .observe_execution_receipt_v1(ObserveExecutionReceiptRequestV1 {
            scope: wrong_relation,
            ..execution_request.clone()
        })
        .unwrap_err();
    assert!(matches!(
        locator_without_scope_authority,
        ae_runtime::RuntimeError::Alpha3(Alpha3ErrorCodeV1::ScopeMismatch)
    ));
    assert_eq!(
        Connection::open(&path)
            .unwrap()
            .execute(
                "DELETE FROM autonomy_operational_authority
                 WHERE persona_scope=?1 AND persona_ordinal=?2",
                params![
                    persona_scope.to_vec(),
                    i64::try_from(expected_execution_body_revision).unwrap(),
                ],
            )
            .unwrap(),
        1
    );
    let missing_execution_authority = runtime
        .observe_execution_receipt_v1(execution_request)
        .unwrap_err();
    assert!(matches!(
        missing_execution_authority,
        ae_runtime::RuntimeError::Alpha3(Alpha3ErrorCodeV1::ProjectionIncomplete)
    ));
    runtime.flush_and_close().unwrap();
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
