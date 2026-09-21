#![cfg(feature = "legacy-semantic-test-api")]

use ae_contracts::*;
use ae_fixed::Fixed;
use ae_runtime::AstrRuntime;
use rusqlite::Connection;

fn genesis(seed: u8) -> PersonaGenesisRequest {
    let scope = PersonaScopeRef {
        bot_token: [seed; 16],
        persona_token: [seed.wrapping_add(1); 16],
    };
    let source = PersonaSourceRef {
        scope,
        source_digest: [seed.wrapping_add(2); 32],
        capability_digest: [seed.wrapping_add(3); 32],
        selection: PersonaSelectionKind::Conversation,
        prompt_chars: 8,
        begin_dialog_count: 1,
        mood_dialog_count: 0,
    };
    PersonaGenesisRequest {
        source: source.clone(),
        proposal: GenesisManifestProposal {
            schema_version: 1,
            source,
            traits: PersonalityVector {
                baseline_warmth: Fixed::from_raw(500_000 + i64::from(seed) * 1_000),
                ..PersonalityVector::default()
            },
            trait_confidence: PersonalityVector {
                baseline_warmth: Fixed::ONE,
                ..PersonalityVector::default()
            },
            expression: ExpressionPhenotype::default(),
            allostasis: AllostaticSetpoints::default(),
            epistemic: EpistemicPriors::default(),
            social: SocialPriors::default(),
            compiler_protocol_digest: [seed.wrapping_add(4); 32],
            compiler_model_digest: [seed.wrapping_add(5); 32],
        },
        formula_digest: [seed.wrapping_add(6); 32],
        incarnation_nonce: [seed.wrapping_add(7); 32],
        parent_incarnation_id: None,
        observed_at_ms: 1_700_000_000_000,
    }
}

fn stimulus_proposal(
    runtime: &mut AstrRuntime,
    request: &PersonaGenesisRequest,
    relation: u8,
    event: u8,
    positive: i64,
) -> (ScopeRef, PerceptionProposalV1) {
    let scope = ScopeRef {
        bot_token: request.source.scope.bot_token,
        persona_token: request.source.scope.persona_token,
        relation_token: Some([relation; 16]),
        session_token: [relation.wrapping_add(1); 16],
    };
    let persona_scope = wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
    let relation_scope = wire::persona_scope_digest(
        &scope.bot_token,
        &scope.persona_token,
        scope.relation_token.as_ref(),
    );
    runtime
        .bootstrap_autonomy(
            &scope,
            &PersonaTemporalProfileV1 {
                schema_version: 1,
                persona_scope,
                home_timezone: "UTC".into(),
                current_timezone: "UTC".into(),
                chronotype: ChronotypeV1::Intermediate,
                preferred_sleep_local_minute: 1_380,
                preferred_wake_local_minute: 420,
                sleep_flex_minutes: 90,
                entrainment_rate_minutes_per_day: 60,
                revision: 1,
            },
            Some(&RelationTemporalPolicyV1 {
                schema_version: 1,
                relation_scope,
                user_timezone: "UTC".into(),
                timezone_source: TimezoneSourceV1::Explicit,
                quiet_hours_start_minute: 0,
                quiet_hours_end_minute: 0,
                quiet_hours_emergency_bypass: false,
                proactive_enabled: false,
                proactive_daily_max: 0,
                min_proactive_cooldown_ms: 0,
                intention_ttl_ms: 86_400_000,
                unanswered_backoff_base_ms: 0,
                unanswered_hard_stop: 3,
                emergency_threshold: Fixed::ONE,
                daily_submitted: 0,
                consecutive_unanswered: 0,
                last_inbound_utc_ms: None,
                last_proactive_submitted_utc_ms: None,
                revision: 1,
                auto_policy_version: 0,
                next_claim_reservation_tokens: 256,
            }),
        )
        .unwrap();
    let base_revision = runtime.current_revision(&scope).unwrap();
    let observed_at_ms = 1_700_000_000_100 + u64::from(event);
    let interaction_id = |prefix: u8| {
        let mut id = [prefix; 16];
        id[12] = scope.bot_token[0];
        id[13] = prefix;
        id[14] = relation;
        id[15] = event;
        id
    };
    let inbound = InteractionFactBatchV1 {
        schema_version: ALPHA3_SCHEMA_VERSION,
        event_id: interaction_id(0xA1),
        scope: scope.clone(),
        causal: CausalRef {
            turn_id: [event; 16],
            action_id: None,
            delivery_id: None,
            claim_id: None,
            base_revision,
        },
        facts: vec![InteractionFactV1 {
            fact_id: interaction_id(0xA2),
            kind: InteractionFactKindV1::InboundObserved,
            observed_at_utc_ms: observed_at_ms,
            source_authority: InteractionSourceAuthorityV1::AstrbotMetadata,
            source_digest: [event.wrapping_add(2); 32],
            extractor_digest: [event.wrapping_add(3); 32],
            confidence: Fixed::ONE,
            value_code: None,
            subject_public_ref: None,
            consent_terms: None,
            scheduled_at_utc_ms: None,
            expires_at_utc_ms: None,
        }],
    };
    let receipt = runtime.apply_interaction_fact_batch_v1(&inbound).unwrap();
    let challenge = runtime
        .mint_perception_challenge_from_committed_inbound_v1(receipt.receipt.event_digest)
        .unwrap();
    (
        scope,
        PerceptionProposalV1 {
            schema_version: PerceptionProposalV1::SCHEMA_VERSION,
            origin_digest: challenge.origin.origin_digest,
            dimensions: EvidenceVector {
                positive: Fixed::from_raw(positive),
                affiliation: Fixed::from_raw(300_000),
                engagement: Fixed::from_raw(600_000),
                ..EvidenceVector::default()
            },
            estimator_confidence: Fixed::from_raw(800_000),
            protocol_version: PerceptionProposalV1::PROTOCOL_VERSION,
            request_nonce_digest: challenge.request_nonce_digest,
        },
    )
}

fn delivery(
    request: &PersonaGenesisRequest,
    relation: u8,
    event: u8,
    journal_base: u64,
) -> CanonicalEvent {
    CanonicalEvent::DeliveryOutcome(DeliveryOutcome {
        event_id: [event; 16],
        scope: ScopeRef {
            bot_token: request.source.scope.bot_token,
            persona_token: request.source.scope.persona_token,
            relation_token: Some([relation; 16]),
            session_token: [relation.wrapping_add(1); 16],
        },
        causal: CausalRef {
            turn_id: [event.wrapping_add(1); 16],
            action_id: None,
            delivery_id: None,
            claim_id: None,
            base_revision: journal_base,
        },
        delivered: true,
        visible_action_digest: [event.wrapping_add(2); 32],
        delivered_at_ms: 1_700_000_100_000 + u64::from(event),
    })
}

fn temp_database(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "ae-runtime-task7-{label}-{}-{:?}.db",
        std::process::id(),
        std::thread::current().id()
    ))
}

fn persona_scope(request: &PersonaGenesisRequest) -> ScopeRef {
    ScopeRef {
        bot_token: request.source.scope.bot_token,
        persona_token: request.source.scope.persona_token,
        relation_token: None,
        session_token: [0; 16],
    }
}

fn sidecar_bytes(path: &std::path::Path) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
    Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT snapshot.snapshot_bytes,graph.graph_bytes,receipt.receipt_bytes,
                    telemetry.telemetry_bytes,cursor.commitment_digest
             FROM semantic_cursor AS cursor
             JOIN semantic_snapshots AS snapshot USING(persona_scope,semantic_revision)
             JOIN semantic_graphs AS graph USING(persona_scope,semantic_revision)
             JOIN semantic_receipts AS receipt USING(persona_scope,semantic_revision)
             JOIN semantic_telemetry AS telemetry USING(persona_scope,semantic_revision)",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap()
}

#[test]
fn authenticated_user_stimulus_changes_and_reopens_persona_emotion_state() {
    let database = temp_database("evolves");
    let _ = std::fs::remove_file(&database);
    let request = genesis(0x31);
    let mut runtime = AstrRuntime::open(&database).unwrap();
    runtime.ensure_genesis(&request).unwrap();
    let (first_scope, first_event) = stimulus_proposal(&mut runtime, &request, 0x41, 0x51, 700_000);
    let first = runtime
        .apply_perception_proposal_v1(&first_scope, &first_event)
        .unwrap();
    assert_eq!(first.revision, 2);
    assert_ne!(first.receipt.state_before, first.receipt.state_after);
    assert_eq!(
        runtime
            .semantic_revision_v1(&persona_scope(&request))
            .unwrap(),
        1
    );
    let delivered = runtime
        .apply_event(&persona_scope(&request), &delivery(&request, 0x41, 0x61, 2))
        .unwrap();
    assert_eq!(delivered.revision, 3);
    assert_eq!(
        runtime
            .semantic_revision_v1(&persona_scope(&request))
            .unwrap(),
        1
    );

    let before_reopen = sidecar_bytes(&database);
    drop(runtime);
    let mut reopened = AstrRuntime::open(&database).unwrap();
    assert_eq!(sidecar_bytes(&database), before_reopen);
    let (second_scope, second_event) =
        stimulus_proposal(&mut reopened, &request, 0x41, 0x52, 200_000);
    let second = reopened
        .apply_perception_proposal_v1(&second_scope, &second_event)
        .unwrap();
    assert_eq!(second.revision, 5);
    assert_eq!(second.receipt.state_before, first.receipt.state_after);
    assert_eq!(
        reopened
            .semantic_revision_v1(&persona_scope(&request))
            .unwrap(),
        2
    );

    drop(reopened);
    let _ = std::fs::remove_file(&database);
}

#[test]
fn relation_evidence_is_private_while_persona_mood_is_continuous() {
    let database = temp_database("relations");
    let _ = std::fs::remove_file(&database);
    let first_persona = genesis(0x61);
    let second_persona = genesis(0x71);
    let mut runtime = AstrRuntime::open(&database).unwrap();
    runtime.ensure_genesis(&first_persona).unwrap();

    let (relation_a_scope, relation_a_proposal) =
        stimulus_proposal(&mut runtime, &first_persona, 0x11, 0x21, 750_000);
    let relation_a = runtime
        .apply_perception_proposal_v1(&relation_a_scope, &relation_a_proposal)
        .unwrap();
    let (relation_b_scope, relation_b_proposal) =
        stimulus_proposal(&mut runtime, &first_persona, 0x12, 0x21, 150_000);
    let relation_b = runtime
        // The same event ID is valid in a different private relation.
        .apply_perception_proposal_v1(&relation_b_scope, &relation_b_proposal)
        .unwrap();
    assert_eq!(
        relation_b.receipt.state_before,
        relation_a.receipt.state_after
    );
    assert_ne!(
        relation_b.receipt.state_before,
        relation_b.receipt.state_after
    );

    runtime.ensure_genesis(&second_persona).unwrap();
    let (isolated_scope, isolated_proposal) =
        stimulus_proposal(&mut runtime, &second_persona, 0x11, 0x22, 900_000);
    let isolated = runtime
        .apply_perception_proposal_v1(&isolated_scope, &isolated_proposal)
        .unwrap();
    assert_ne!(
        isolated.receipt.state_before,
        relation_b.receipt.state_after
    );

    let (continuation_scope, continuation_proposal) =
        stimulus_proposal(&mut runtime, &first_persona, 0x11, 0x23, 300_000);
    let continuation = runtime
        .apply_perception_proposal_v1(&continuation_scope, &continuation_proposal)
        .unwrap();
    assert_eq!(
        continuation.receipt.state_before,
        relation_b.receipt.state_after
    );

    drop(runtime);
    let _ = std::fs::remove_file(&database);
}

#[test]
fn invalid_evidence_and_commit_fault_leave_hot_and_store_unchanged() {
    let database = temp_database("invalid");
    let _ = std::fs::remove_file(&database);
    let request = genesis(0x81);
    let mut runtime = AstrRuntime::open(&database).unwrap();
    runtime.ensure_genesis(&request).unwrap();

    let (invalid_scope, mut invalid) =
        stimulus_proposal(&mut runtime, &request, 0x31, 0x41, 500_000);
    invalid.estimator_confidence = Fixed::ZERO;
    assert!(runtime
        .apply_perception_proposal_v1(&invalid_scope, &invalid)
        .is_err());

    // A valid event still commits at the untouched canonical and semantic
    // origins, proving validation did not mutate Store or hot state.
    let (valid_scope, valid_proposal) =
        stimulus_proposal(&mut runtime, &request, 0x31, 0x42, 500_000);
    let valid = runtime
        .apply_perception_proposal_v1(&valid_scope, &valid_proposal)
        .unwrap();
    assert_eq!(valid.revision, 3);
    assert_ne!(valid.receipt.state_before, valid.receipt.state_after);

    let (stale_scope, stale) = stimulus_proposal(&mut runtime, &request, 0x31, 0x43, 800_000);
    let stale_base = runtime.current_revision(&persona_scope(&request)).unwrap();
    runtime
        .apply_event(
            &persona_scope(&request),
            &delivery(&request, 0x31, 0x70, stale_base),
        )
        .unwrap();
    assert!(runtime
        .apply_perception_proposal_v1(&stale_scope, &stale)
        .is_err());
    assert_eq!(
        runtime
            .semantic_revision_v1(&persona_scope(&request))
            .unwrap(),
        1
    );

    let (fault_scope, fault_proposal) =
        stimulus_proposal(&mut runtime, &request, 0x31, 0x44, 200_000);
    Connection::open(&database)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER task7_event_lane_fault
             BEFORE INSERT ON semantic_snapshots
             BEGIN SELECT RAISE(ABORT, 'task7 event lane fault'); END;",
        )
        .unwrap();
    let before_fault = runtime.current_revision(&persona_scope(&request)).unwrap();
    assert!(runtime
        .apply_perception_proposal_v1(&fault_scope, &fault_proposal)
        .is_err());
    assert_eq!(
        runtime.current_revision(&persona_scope(&request)).unwrap(),
        before_fault
    );
    assert_eq!(
        runtime
            .semantic_revision_v1(&persona_scope(&request))
            .unwrap(),
        1
    );
    Connection::open(&database)
        .unwrap()
        .execute_batch("DROP TRIGGER task7_event_lane_fault;")
        .unwrap();
    let after_fault = runtime
        .apply_perception_proposal_v1(&fault_scope, &fault_proposal)
        .unwrap();
    assert_eq!(after_fault.receipt.state_before, valid.receipt.state_after);

    drop(runtime);
    let _ = std::fs::remove_file(&database);
}
