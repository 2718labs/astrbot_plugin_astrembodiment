#![cfg(feature = "legacy-semantic-test-api")]

use ae_contracts::*;
use ae_fixed::Fixed;
use ae_runtime::{AstrRuntime, RuntimeError};
use rusqlite::{params, Connection};
use std::sync::{Arc, Barrier};

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
            traits: PersonalityVector::default(),
            trait_confidence: PersonalityVector::default(),
            expression: ExpressionPhenotype::default(),
            allostasis: AllostaticSetpoints::default(),
            epistemic: Default::default(),
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

fn scope(request: &PersonaGenesisRequest, relation: u8, session: u8) -> ScopeRef {
    ScopeRef {
        bot_token: request.source.scope.bot_token,
        persona_token: request.source.scope.persona_token,
        relation_token: Some([relation; 16]),
        session_token: [session; 16],
    }
}

fn persona_scope(request: &PersonaGenesisRequest) -> ScopeRef {
    ScopeRef {
        bot_token: request.source.scope.bot_token,
        persona_token: request.source.scope.persona_token,
        relation_token: None,
        session_token: [0; 16],
    }
}

fn interaction_id(scope: &ScopeRef, prefix: u8, relation: u8, unique: u8) -> Id128 {
    let mut id = [prefix; 16];
    id[12] = scope.bot_token[0];
    id[13] = prefix;
    id[14] = relation;
    id[15] = unique;
    id
}

fn proposal(
    runtime: &mut AstrRuntime,
    scope: &ScopeRef,
    event: u8,
    dimensions: EvidenceVector,
    confidence: Fixed,
) -> PerceptionProposalV1 {
    proposal_with_origin_identity(runtime, scope, event, event, dimensions, confidence)
}

fn proposal_with_origin_identity(
    runtime: &mut AstrRuntime,
    scope: &ScopeRef,
    semantic_event: u8,
    origin_identity: u8,
    dimensions: EvidenceVector,
    confidence: Fixed,
) -> PerceptionProposalV1 {
    let relation_identity = scope.relation_token.expect("relation scope")[0];
    let turn_id = [semantic_event; 16];
    let persona = wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
    let relation = wire::persona_scope_digest(
        &scope.bot_token,
        &scope.persona_token,
        scope.relation_token.as_ref(),
    );
    runtime
        .bootstrap_autonomy(
            scope,
            &PersonaTemporalProfileV1 {
                schema_version: 1,
                persona_scope: persona,
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
                relation_scope: relation,
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
    let base_revision = runtime.current_revision(scope).unwrap();
    let observed_at_ms = 1_700_000_000_100 + u64::from(origin_identity);
    let inbound = InteractionFactBatchV1 {
        schema_version: ALPHA3_SCHEMA_VERSION,
        event_id: interaction_id(scope, 0xA1, relation_identity, origin_identity),
        scope: scope.clone(),
        causal: CausalRef {
            turn_id,
            action_id: None,
            delivery_id: None,
            claim_id: None,
            base_revision,
        },
        facts: vec![InteractionFactV1 {
            fact_id: interaction_id(scope, 0xA2, relation_identity, origin_identity),
            kind: InteractionFactKindV1::InboundObserved,
            observed_at_utc_ms: observed_at_ms,
            source_authority: InteractionSourceAuthorityV1::AstrbotMetadata,
            source_digest: [origin_identity.wrapping_add(2); 32],
            extractor_digest: [origin_identity.wrapping_add(3); 32],
            confidence: Fixed::ONE,
            value_code: None,
            subject_public_ref: None,
            consent_terms: None,
            scheduled_at_utc_ms: None,
            expires_at_utc_ms: None,
        }],
    };
    let inbound = runtime.apply_interaction_fact_batch_v1(&inbound).unwrap();
    let challenge = runtime
        .mint_perception_challenge_from_committed_inbound_v1(inbound.receipt.event_digest)
        .unwrap();
    PerceptionProposalV1 {
        schema_version: PerceptionProposalV1::SCHEMA_VERSION,
        origin_digest: challenge.origin.origin_digest,
        dimensions,
        estimator_confidence: confidence,
        protocol_version: PerceptionProposalV1::PROTOCOL_VERSION,
        request_nonce_digest: challenge.request_nonce_digest,
    }
}

fn database(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "ae-runtime-perception-authority-{label}-{}-{:?}.db",
        std::process::id(),
        std::thread::current().id()
    ))
}

fn semantic_counts(path: &std::path::Path) -> (i64, i64, i64) {
    let connection = Connection::open(path).unwrap();
    connection
        .query_row(
            "SELECT
               (SELECT COUNT(*) FROM journal),
               (SELECT COUNT(*) FROM semantic_commits),
               (SELECT COUNT(*) FROM semantic_evidence_authority)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
}

fn sidecar_bytes(path: &std::path::Path) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
    let connection = Connection::open(path).unwrap();
    connection
        .query_row(
            "SELECT snapshot.snapshot_bytes, graph.graph_bytes, receipt.receipt_bytes,
                    telemetry.telemetry_bytes, cursor.commitment_digest
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
fn authenticated_proposal_commits_neutral_and_nonzero_evidence_and_reopens_exact_bytes() {
    let path = database("reopen");
    let _ = std::fs::remove_file(&path);
    let request = genesis(0x21);
    let relation = scope(&request, 0x31, 0x32);
    let mut runtime = AstrRuntime::open(&path).unwrap();
    runtime.ensure_genesis(&request).unwrap();

    let neutral = proposal(
        &mut runtime,
        &relation,
        0x41,
        EvidenceVector::default(),
        Fixed::ONE,
    );
    let neutral_decision = runtime
        .apply_perception_proposal_v1(&relation, &neutral)
        .unwrap();
    assert_eq!(neutral_decision.revision, 2);
    assert_eq!(
        runtime
            .semantic_revision_v1(&persona_scope(&request))
            .unwrap(),
        1
    );

    let nonzero = proposal(
        &mut runtime,
        &relation,
        0x42,
        EvidenceVector {
            positive: Fixed::from_raw(800_000),
            affiliation: Fixed::from_raw(600_000),
            engagement: Fixed::from_raw(700_000),
            ..EvidenceVector::default()
        },
        Fixed::from_raw(900_000),
    );
    let changed = runtime
        .apply_perception_proposal_v1(&relation, &nonzero)
        .unwrap();
    assert_eq!(changed.revision, 4);
    assert_ne!(changed.receipt.state_before, changed.receipt.state_after);
    assert_eq!(
        runtime
            .semantic_revision_v1(&persona_scope(&request))
            .unwrap(),
        2
    );
    let expected = sidecar_bytes(&path);

    drop(runtime);
    let mut reopened = AstrRuntime::open(&path).unwrap();
    assert_eq!(
        reopened
            .semantic_revision_v1(&persona_scope(&request))
            .unwrap(),
        2
    );
    assert_eq!(sidecar_bytes(&path), expected);
    reopened.audit_semantic_integrity_v1().unwrap();
    drop(reopened);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn invalid_scope_nonce_incarnation_dimensions_confidence_source_and_estimator_write_nothing() {
    let path = database("reject");
    let _ = std::fs::remove_file(&path);
    let request = genesis(0x51);
    let relation = scope(&request, 0x61, 0x62);
    let mut runtime = AstrRuntime::open(&path).unwrap();
    runtime.ensure_genesis(&request).unwrap();
    let mut bad_confidence = proposal(
        &mut runtime,
        &relation,
        0x70,
        EvidenceVector::default(),
        Fixed::ONE,
    );
    bad_confidence.estimator_confidence = Fixed::ZERO;
    assert!(runtime
        .apply_perception_proposal_v1(&relation, &bad_confidence)
        .is_err());
    assert_eq!(semantic_counts(&path), (1, 0, 0));

    let mut bad_dimension = proposal(
        &mut runtime,
        &relation,
        0x71,
        EvidenceVector::default(),
        Fixed::ONE,
    );
    bad_dimension.dimensions.harm = Fixed::from_raw(1_000_001);
    assert!(runtime
        .apply_perception_proposal_v1(&relation, &bad_dimension)
        .is_err());
    assert_eq!(semantic_counts(&path), (2, 0, 0));

    let mut bad_nonce = proposal(
        &mut runtime,
        &relation,
        0x72,
        EvidenceVector::default(),
        Fixed::ONE,
    );
    bad_nonce.request_nonce_digest[0] ^= 1;
    assert!(runtime
        .apply_perception_proposal_v1(&relation, &bad_nonce)
        .is_err());
    assert_eq!(semantic_counts(&path), (3, 0, 0));

    let wrong_scope = scope(&request, 0x63, 0x62);
    let wrong_scope_proposal = proposal(
        &mut runtime,
        &relation,
        0x73,
        EvidenceVector::default(),
        Fixed::ONE,
    );
    assert!(runtime
        .apply_perception_proposal_v1(&wrong_scope, &wrong_scope_proposal)
        .is_err());
    let mut wrong_persona = relation.clone();
    wrong_persona.persona_token[0] ^= 1;
    assert!(runtime
        .apply_perception_proposal_v1(&wrong_persona, &wrong_scope_proposal)
        .is_err());
    assert_eq!(semantic_counts(&path), (4, 0, 0));

    let incarnation = proposal(
        &mut runtime,
        &relation,
        0x74,
        EvidenceVector::default(),
        Fixed::ONE,
    );
    let connection = Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE perception_challenges SET incarnation_id=?2 WHERE request_nonce_digest=?1",
            params![incarnation.request_nonce_digest.to_vec(), vec![0xEE_u8; 32]],
        )
        .unwrap();
    drop(connection);
    assert!(runtime
        .apply_perception_proposal_v1(&relation, &incarnation)
        .is_err());
    assert_eq!(semantic_counts(&path), (5, 0, 0));

    let raw = CanonicalEvent::UserStimulus(UserStimulus {
        event_id: [0x75; 16],
        scope: relation.clone(),
        causal: CausalRef {
            turn_id: [0x76; 16],
            action_id: None,
            delivery_id: None,
            claim_id: None,
            base_revision: 0,
        },
        observed_at_ms: 1_700_000_000_200,
        evidence: SemanticEstimate {
            schema_version: 1,
            dimensions: EvidenceVector::default(),
            estimator_confidence: Fixed::ONE,
            estimator_digest: [0; 32],
        },
    });
    assert!(matches!(
        runtime.apply_event(&persona_scope(&request), &raw),
        Err(RuntimeError::UnauthenticatedUserStimulus)
    ));
    let wrong_source = CanonicalEvent::CorrectionClaim(CorrectionClaim {
        event_id: [0x77; 16],
        scope: relation.clone(),
        causal: CausalRef {
            turn_id: [0x78; 16],
            action_id: None,
            delivery_id: None,
            claim_id: None,
            base_revision: 0,
        },
        specificity: Fixed::ONE,
        supplied_evidence: Fixed::ONE,
        hostility: Fixed::ZERO,
        publicness: Fixed::ZERO,
    });
    assert!(runtime
        .apply_event(&persona_scope(&request), &wrong_source)
        .is_err());
    assert_eq!(semantic_counts(&path), (5, 0, 0));
    assert_eq!(
        runtime.current_revision(&persona_scope(&request)).unwrap(),
        5
    );
    assert_eq!(
        runtime
            .semantic_revision_v1(&persona_scope(&request))
            .unwrap(),
        0
    );
    drop(runtime);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn consumed_challenge_is_exactly_replayable_but_cannot_authorize_another_estimate() {
    let path = database("replay");
    let _ = std::fs::remove_file(&path);
    let request = genesis(0x81);
    let relation = scope(&request, 0x82, 0x83);
    let mut runtime = AstrRuntime::open(&path).unwrap();
    runtime.ensure_genesis(&request).unwrap();
    let original = proposal(
        &mut runtime,
        &relation,
        0x84,
        EvidenceVector {
            positive: Fixed::ONE,
            ..EvidenceVector::default()
        },
        Fixed::ONE,
    );
    let inserted = runtime
        .apply_perception_proposal_v1(&relation, &original)
        .unwrap();
    assert!(!inserted.deduplicated);
    let replayed = runtime
        .apply_perception_proposal_v1(&relation, &original)
        .unwrap();
    assert!(replayed.deduplicated);
    assert_eq!(replayed.receipt, inserted.receipt);
    assert_eq!(semantic_counts(&path), (2, 1, 1));

    let mut changed_estimator = original.clone();
    changed_estimator.dimensions.positive = Fixed::ZERO;
    changed_estimator.dimensions.harm = Fixed::ONE;
    assert!(runtime
        .apply_perception_proposal_v1(&relation, &changed_estimator)
        .is_err());
    assert_eq!(semantic_counts(&path), (2, 1, 1));
    drop(runtime);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn semantic_event_identity_is_relation_local_while_interaction_ids_remain_global() {
    let path = database("relation-local-event-id");
    let _ = std::fs::remove_file(&path);
    let request = genesis(0xA1);
    let relation_a = scope(&request, 0xA2, 0xA3);
    let relation_b = scope(&request, 0xA4, 0xA5);
    let mut runtime = AstrRuntime::open(&path).unwrap();
    runtime.ensure_genesis(&request).unwrap();

    let first = proposal_with_origin_identity(
        &mut runtime,
        &relation_a,
        0xB1,
        0xC1,
        EvidenceVector {
            positive: Fixed::ONE,
            ..EvidenceVector::default()
        },
        Fixed::ONE,
    );
    runtime
        .apply_perception_proposal_v1(&relation_a, &first)
        .unwrap();
    let second = proposal_with_origin_identity(
        &mut runtime,
        &relation_b,
        0xB1,
        0xC2,
        EvidenceVector {
            affiliation: Fixed::ONE,
            ..EvidenceVector::default()
        },
        Fixed::ONE,
    );
    runtime
        .apply_perception_proposal_v1(&relation_b, &second)
        .unwrap();

    let connection = Connection::open(&path).unwrap();
    let (semantic_rows, distinct_semantic_ids, interaction_rows): (i64, i64, i64) = connection
        .query_row(
            "SELECT
               (SELECT COUNT(*) FROM semantic_evidence_authority),
               (SELECT COUNT(DISTINCT hex(event_id)) FROM semantic_evidence_authority),
               (SELECT COUNT(*) FROM interaction_fact)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    let semantic_event_id: Vec<u8> = connection
        .query_row(
            "SELECT event_id FROM semantic_evidence_authority LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        (semantic_rows, distinct_semantic_ids, interaction_rows),
        (2, 1, 2)
    );
    assert_eq!(semantic_event_id, vec![0xB1; 16]);
    drop(connection);

    let conflicting = proposal_with_origin_identity(
        &mut runtime,
        &relation_a,
        0xB1,
        0xC3,
        EvidenceVector {
            harm: Fixed::ONE,
            ..EvidenceVector::default()
        },
        Fixed::ONE,
    );
    assert!(matches!(
        runtime.apply_perception_proposal_v1(&relation_a, &conflicting),
        Err(RuntimeError::InvalidPerceptionProposal)
    ));
    assert_eq!(
        runtime
            .semantic_revision_v1(&persona_scope(&request))
            .unwrap(),
        2
    );

    drop(runtime);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn sqlite_commit_fault_rolls_back_challenge_journal_semantics_and_hot_cursor() {
    let path = database("commit-fault");
    let _ = std::fs::remove_file(&path);
    let request = genesis(0x85);
    let relation = scope(&request, 0x86, 0x87);
    let persona = persona_scope(&request);
    let mut runtime = AstrRuntime::open(&path).unwrap();
    runtime.ensure_genesis(&request).unwrap();
    let proposal = proposal(
        &mut runtime,
        &relation,
        0x88,
        EvidenceVector {
            engagement: Fixed::ONE,
            ..EvidenceVector::default()
        },
        Fixed::ONE,
    );

    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER task7_semantic_commit_fault
             BEFORE INSERT ON semantic_snapshots
             BEGIN SELECT RAISE(ABORT, 'task7 injected commit fault'); END;",
        )
        .unwrap();
    drop(connection);
    assert!(runtime
        .apply_perception_proposal_v1(&relation, &proposal)
        .is_err());
    assert_eq!(semantic_counts(&path), (1, 0, 0));
    assert_eq!(runtime.current_revision(&persona).unwrap(), 1);
    assert_eq!(runtime.semantic_revision_v1(&persona).unwrap(), 0);

    Connection::open(&path)
        .unwrap()
        .execute_batch("DROP TRIGGER task7_semantic_commit_fault;")
        .unwrap();
    let committed = runtime
        .apply_perception_proposal_v1(&relation, &proposal)
        .expect("the rolled-back challenge must remain pending and retryable");
    assert!(!committed.deduplicated);
    assert_eq!(runtime.current_revision(&persona).unwrap(), 2);
    assert_eq!(runtime.semantic_revision_v1(&persona).unwrap(), 1);
    assert_eq!(semantic_counts(&path), (2, 1, 1));

    drop(runtime);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn two_runtime_instances_report_inserted_and_existing_directly_and_hydrate_latest() {
    let path = database("race");
    let _ = std::fs::remove_file(&path);
    let request = genesis(0x91);
    let relation = scope(&request, 0x92, 0x93);
    let mut setup = AstrRuntime::open(&path).unwrap();
    setup.ensure_genesis(&request).unwrap();
    let proposal = proposal(
        &mut setup,
        &relation,
        0x94,
        EvidenceVector {
            affiliation: Fixed::ONE,
            ..EvidenceVector::default()
        },
        Fixed::ONE,
    );
    drop(setup);

    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let path = path.clone();
        let relation = relation.clone();
        let proposal = proposal.clone();
        let persona = persona_scope(&request);
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            let mut runtime = AstrRuntime::open(&path).unwrap();
            barrier.wait();
            let decision = runtime
                .apply_perception_proposal_v1(&relation, &proposal)
                .unwrap();
            let canonical = runtime.current_revision(&persona).unwrap();
            let semantic = runtime.semantic_revision_v1(&persona).unwrap();
            (decision.deduplicated, canonical, semantic)
        }));
    }
    let mut outcomes = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    outcomes.sort_unstable();
    assert_eq!(outcomes, vec![(false, 2, 1), (true, 2, 1)]);
    assert_eq!(semantic_counts(&path), (2, 1, 1));
    let _ = std::fs::remove_file(&path);
}
