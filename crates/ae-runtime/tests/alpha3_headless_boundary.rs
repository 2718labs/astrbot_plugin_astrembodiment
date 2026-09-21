use ae_contracts::*;
use ae_fixed::Fixed;
use ae_runtime::{frozen_time_input_digest, AstrRuntime, RuntimeError};
use rusqlite::{params, Connection};
use std::path::PathBuf;
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
            "alpha3-headless-boundary-db-{}-{nonce}",
            std::process::id()
        ));
    std::fs::create_dir_all(&root).unwrap();
    root.join("store.db")
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
        proactive_daily_max: 2,
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
    }
}

fn frozen(now_utc_ms: u64) -> FrozenTimeInputV1 {
    let day_start = now_utc_ms - now_utc_ms % 86_400_000;
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
        budget_day_start_utc_ms: day_start,
        budget_next_day_start_utc_ms: day_start + 86_400_000,
        next_timezone_transition_utc_ms: None,
        tzdb_fingerprint: [19; 32],
    }
}

fn fact(
    id: u8,
    kind: InteractionFactKindV1,
    authority: InteractionSourceAuthorityV1,
    observed_at_utc_ms: u64,
) -> InteractionFactV1 {
    InteractionFactV1 {
        fact_id: [id; 16],
        kind,
        observed_at_utc_ms,
        source_authority: authority,
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

fn seed_retired_sentinels(path: &PathBuf, persona_scope: Digest) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute(
            "INSERT INTO world_anchor(world_anchor_id,persona_scope,mode,revision,body_json)
             VALUES(?1,?2,'mixed_main_world',1,?3)",
            params![
                vec![0x91_u8; 16],
                persona_scope.to_vec(),
                r#"{ "retired": "world", "raw": [3, 2, 1] }"#
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO lived_day_state(
                 persona_scope,world_anchor_id,persona_day_ordinal,revision,body_json
             ) VALUES(?1,?2,4242,7,?3)",
            params![
                persona_scope.to_vec(),
                vec![0x91_u8; 16],
                r#"{ "retired": "lived", "raw": "  preserved  " }"#
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO dream_residue(residue_id,persona_scope,revision,state,body_json)
             VALUES(?1,?2,9,'pending_waking_review',?3)",
            params![
                vec![0x92_u8; 16],
                persona_scope.to_vec(),
                r#"{ "retired": "dream", "raw": "do not decode" }"#
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO ecosystem_capability_grant(
                 plugin_instance_digest,capability_digest,revision,state,body_json
             ) VALUES(?1,?2,3,'active',?3)",
            params![
                vec![0x93_u8; 32],
                vec![0x94_u8; 32],
                r#"{ "retired": "ecosystem-grant", "raw": 17 }"#
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO ecosystem_proposal(
                 proposal_id,persona_scope,plugin_instance_digest,semantic_digest,
                 revision,decision,body_json
             ) VALUES(?1,?2,?3,?4,4,'deferred',?5)",
            params![
                vec![0x95_u8; 16],
                persona_scope.to_vec(),
                vec![0x93_u8; 32],
                vec![0x96_u8; 32],
                r#"{ "retired": "ecosystem-proposal", "raw": false }"#
            ],
        )
        .unwrap();
}

fn retired_row_bytes(path: &PathBuf) -> Vec<(String, String)> {
    let connection = Connection::open(path).unwrap();
    let mut statement = connection
        .prepare(
            "SELECT domain,row_bytes FROM (
                 SELECT 'world' AS domain,
                        hex(world_anchor_id)||'|'||hex(persona_scope)||'|'||
                        hex(CAST(mode AS BLOB))||'|'||hex(CAST(revision AS BLOB))||'|'||
                        hex(CAST(body_json AS BLOB)) AS row_bytes
                   FROM world_anchor
                 UNION ALL
                 SELECT 'lived',
                        hex(persona_scope)||'|'||hex(world_anchor_id)||'|'||
                        hex(CAST(persona_day_ordinal AS BLOB))||'|'||
                        hex(CAST(revision AS BLOB))||'|'||hex(CAST(body_json AS BLOB))
                   FROM lived_day_state
                 UNION ALL
                 SELECT 'dream',
                        hex(residue_id)||'|'||hex(persona_scope)||'|'||
                        hex(CAST(revision AS BLOB))||'|'||hex(CAST(state AS BLOB))||'|'||
                        hex(CAST(body_json AS BLOB))
                   FROM dream_residue
                 UNION ALL
                 SELECT 'ecosystem_grant',
                        hex(plugin_instance_digest)||'|'||hex(capability_digest)||'|'||
                        hex(CAST(revision AS BLOB))||'|'||hex(CAST(state AS BLOB))||'|'||
                        hex(CAST(body_json AS BLOB))
                   FROM ecosystem_capability_grant
                 UNION ALL
                 SELECT 'ecosystem_proposal',
                        hex(proposal_id)||'|'||hex(persona_scope)||'|'||
                        hex(plugin_instance_digest)||'|'||hex(semantic_digest)||'|'||
                        hex(CAST(revision AS BLOB))||'|'||hex(CAST(decision AS BLOB))||'|'||
                        hex(CAST(body_json AS BLOB))
                   FROM ecosystem_proposal
             ) ORDER BY domain,row_bytes",
        )
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn seed_denied_gate_fixture(
    path: &PathBuf,
    persona_scope: Digest,
    relation_scope: Digest,
    consent: &RelationConsentV1,
    now_utc_ms: u64,
) -> DurableIntentionV1 {
    let intention = DurableIntentionV1 {
        schema_version: ALPHA3_SCHEMA_VERSION,
        intention_id: [0x97; 16],
        persona_scope,
        relation_scope,
        state: IntentionStateV1::Ready,
        action_class: "contact_explicit_follow_up".into(),
        salience: Fixed::ONE,
        urgency: Fixed::ONE,
        confidence: Fixed::ONE,
        created_at_utc_ms: now_utc_ms,
        not_before_utc_ms: now_utc_ms,
        expires_at_utc_ms: now_utc_ms + 86_400_000,
        externalization_attempts: 0,
        semantic_idempotency_digest: [0x98; 32],
        workspace_mapping_digest: [0x99; 32],
        workspace_residual: Fixed::ZERO,
        source_event_ids: vec![[31; 16]],
    };
    let basis = ContactIntentionBasisV1 {
        intention_id: intention.intention_id,
        relation_scope,
        purpose: ContactPurposeV1::ExplicitFollowUp,
        cause_digest: [0x9a; 32],
        cause_public_refs: vec![[0x9b; 32]],
        consent_epoch: consent.consent_epoch,
        consent_revision: consent.revision,
        created_at_utc_ms: now_utc_ms,
        expires_at_utc_ms: intention.expires_at_utc_ms,
        live: true,
    };
    let connection = Connection::open(path).unwrap();
    connection
        .execute(
            "INSERT INTO durable_intention(
                 intention_id,persona_scope,relation_scope,semantic_digest,state,revision,body_json
             ) VALUES(?1,?2,?3,?4,'ready',0,?5)",
            params![
                intention.intention_id.to_vec(),
                persona_scope.to_vec(),
                relation_scope.to_vec(),
                intention.semantic_idempotency_digest.to_vec(),
                serde_json::to_string(&intention).unwrap(),
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO contact_intention_basis(
                 intention_id,relation_scope,cause_digest,consent_epoch,
                 consent_revision,live,revision,body_json
             ) VALUES(?1,?2,?3,?4,?5,1,1,?6)",
            params![
                basis.intention_id.to_vec(),
                relation_scope.to_vec(),
                basis.cause_digest.to_vec(),
                basis.consent_epoch as i64,
                basis.consent_revision as i64,
                serde_json::to_string(&basis).unwrap(),
            ],
        )
        .unwrap();
    intention
}

fn seed_retired_wake_v2_claim(path: &PathBuf, normal: &WakeClaimV2) -> WakeClaimV2 {
    let mut retired = normal.clone();
    retired.event.event_id = [0xa0; 16];
    retired.proposal.world_anchor = Some(WorldAnchorV1 {
        schema_version: ALPHA3_SCHEMA_VERSION,
        world_anchor_id: [0xa1; 16],
        persona_scope: normal.proposal.legacy.state.persona_scope,
        mode: WorldModeV1::MixedMainWorld,
        home_context_ref: [0xa2; 32],
        lore_manifest_digest: [0xa3; 32],
        reality_policy_digest: [0xa4; 32],
        allowed_layers: vec![
            WorldLayerV1::ExternalObserved,
            WorldLayerV1::PersonaNearReal,
            WorldLayerV1::DeclaredFantasy,
        ],
        created_from_event_id: retired.event.event_id,
        revision: 1,
    });
    let caller = wire::domain_hash(
        b"ae.runtime.wake-caller.v2",
        &[&retired.event.event_id, &retired.event.scope.session_token],
    );
    retired.claim_token = wire::domain_hash(
        b"ae.autonomy.claim.v1",
        &[b"wake-v2", &retired.event.event_id, &caller],
    );
    Connection::open(path)
        .unwrap()
        .execute(
            "INSERT INTO autonomy_claim(
                 claim_token,claim_kind,record_id,caller_incarnation,
                 lease_deadline_utc_ms,body_json
             ) VALUES(?1,'wake_v2',?2,?3,?4,?5)",
            params![
                retired.claim_token.to_vec(),
                retired.event.event_id.to_vec(),
                caller.to_vec(),
                retired.lease_deadline_utc_ms as i64,
                serde_json::to_string(&retired).unwrap(),
            ],
        )
        .unwrap();
    retired
}

fn claim_count(path: &PathBuf, token: &Digest) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM autonomy_claim WHERE claim_token=?1",
            params![token.to_vec()],
            |row| row.get(0),
        )
        .unwrap()
}

fn claim_row_bytes(path: &PathBuf, token: &Digest) -> String {
    Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT hex(claim_token)||'|'||hex(CAST(claim_kind AS BLOB))||'|'||
                    hex(record_id)||'|'||hex(caller_incarnation)||'|'||
                    hex(CAST(lease_deadline_utc_ms AS BLOB))||'|'||
                    hex(CAST(body_json AS BLOB))
             FROM autonomy_claim WHERE claim_token=?1",
            params![token.to_vec()],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn retired_domains_are_inert_while_body_authority_remains() {
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

    assert_eq!(ae_store::AUTONOMY_DB_VERSION, 7);
    let version_connection = Connection::open(&path).unwrap();
    assert_eq!(
        version_connection
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get::<_, u32>(0)
            })
            .unwrap(),
        7
    );
    assert_eq!(
        version_connection
            .query_row(
                "SELECT digest FROM schema_migrations WHERE version=7",
                [],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .unwrap(),
        b"ae.autonomy.db.v7.alpha3-authority.v1"
    );
    assert_eq!(
        version_connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
            .unwrap(),
        0
    );
    drop(version_connection);
    seed_retired_sentinels(&path, persona_scope);
    let retired_before = retired_row_bytes(&path);
    assert_eq!(retired_before.len(), 5);

    let contact_before = runtime.relation_contact_v1(&relation).unwrap().revision;
    let revision_before = runtime.current_revision(&persona).unwrap();
    let now = 1_700_100_000_000;
    let mut inbound = fact(
        30,
        InteractionFactKindV1::InboundObserved,
        InteractionSourceAuthorityV1::AstrbotMetadata,
        now,
    );
    inbound.source_digest = [70; 32];
    let mut follow_up = fact(
        31,
        InteractionFactKindV1::FollowUpRequested,
        InteractionSourceAuthorityV1::ExplicitControl,
        now,
    );
    follow_up.value_code = Some(InteractionValueCodeV1::FollowUp);
    follow_up.scheduled_at_utc_ms = Some(now);
    follow_up.expires_at_utc_ms = Some(now + 86_400_000);
    let interaction = runtime
        .apply_interaction_fact_batch_v1(&InteractionFactBatchV1 {
            schema_version: ALPHA3_SCHEMA_VERSION,
            event_id: [33; 16],
            scope: relation.clone(),
            causal: CausalRef {
                turn_id: [34; 16],
                action_id: None,
                delivery_id: None,
                claim_id: None,
                base_revision: revision_before,
            },
            facts: vec![inbound, follow_up],
        })
        .unwrap();
    assert_eq!(interaction.applied_fact_count, 2);
    assert!(interaction.receipt.canonical_revision > revision_before);
    assert!(runtime.relation_contact_v1(&relation).unwrap().revision > contact_before);

    let wake_frozen = frozen(now + 2_000_000);
    let wake = TimeAdvanceV1 {
        event_id: [35; 16],
        scope: persona.clone(),
        expected_generation: initial.generation,
        frozen_input_digest: frozen_time_input_digest(&wake_frozen),
        frozen: wake_frozen.clone(),
        stimulus: AutonomousStimulusV1 {
            arousal: Fixed::ONE,
            urgency: Fixed::ONE,
            emergency_authorized: false,
            source_digest: [36; 32],
        },
    };
    let claim = runtime.claim_wake_v2(&persona, &wake).unwrap();
    assert!(claim.proposal.lived_day.is_none());
    assert!(claim.proposal.world_anchor.is_none());
    assert!(claim.proposal.dream_updates.is_empty());
    assert!(claim.proposal.legacy.intentions.is_empty());
    let settled = runtime.settle_wake_v2(&claim.claim_token).unwrap();
    assert!(settled.state_revision > initial.state_revision);
    assert!(settled.receipt.canonical_revision > interaction.receipt.canonical_revision);

    runtime
        .upsert_relation_budget_policy_v1(&RelationBudgetPolicyV1 {
            schema_version: ALPHA3_SCHEMA_VERSION,
            relation_scope,
            timezone_id: "Asia/Shanghai".into(),
            daily_token_limit: 100,
            revision: 1,
            source_digest: [37; 32],
        })
        .unwrap();
    let disabled_consent = runtime.relation_consent_v1(&relation).unwrap();
    assert_eq!(disabled_consent.state, RelationConsentStateV1::Disabled);
    let intention = seed_denied_gate_fixture(
        &path,
        persona_scope,
        relation_scope,
        &disabled_consent,
        wake_frozen.effective_now_utc_ms,
    );
    let gate = runtime
        .gate_and_claim_externalization_v2(&GateAndClaimExternalizationRequestV2 {
            scope: relation.clone(),
            intention_id: intention.intention_id,
            attempt_no: 1,
            expected_revision: 0,
            max_tokens: 10,
            caller_incarnation: [38; 32],
            readiness: ready_witness(),
            frozen: wake_frozen.clone(),
        })
        .unwrap();
    assert_eq!(gate.decision.decision, GateDecisionKindV2::Suppressed);
    assert_eq!(gate.decision.reason, GateReasonV2::ConsentRequired);
    assert!(gate.claim.is_none());
    assert!(runtime
        .relation_budget_ledger_v1(&relation_scope, wake_frozen.budget_day_start_utc_ms)
        .unwrap()
        .is_some());
    let developer = runtime
        .observe_snapshot_v2(&ObserveSnapshotRequestV2 {
            schema_version: ALPHA3_SCHEMA_VERSION,
            persona_scope,
            relation_scope: Some(relation_scope),
            committed_only: true,
            layer: ObserveLayerV2::Developer,
        })
        .unwrap();
    let ObserveProjectionV2::Developer(developer) = developer.projection else {
        panic!("expected developer projection");
    };
    assert!(developer.readiness.available().is_some());
    assert!(developer.latest_gate.available().is_some());

    for operation in [
        "conversation_context_v1",
        "review_dream_v1",
        "control_consent_v1",
        "grant_ecosystem_capability_v1",
        "observe_ecosystem_v1",
        "propose_ecosystem_v1",
    ] {
        let request = format!(r#"{{"operation":"{operation}","request":{{}}}}"#);
        let error = serde_json::from_str::<Alpha3RequestV1>(&request).unwrap_err();
        assert!(
            error.to_string().contains("unknown variant"),
            "{operation} remained a live request variant: {error}"
        );
    }

    let current_state = runtime
        .autonomy_status(Some(&persona))
        .unwrap()
        .pop()
        .unwrap()
        .1;
    let recovery_frozen = frozen(current_state.last_advanced_at_utc_ms);
    let normal_wake = TimeAdvanceV1 {
        event_id: [0xa5; 16],
        scope: persona.clone(),
        expected_generation: current_state.generation,
        frozen_input_digest: frozen_time_input_digest(&frozen(
            current_state.last_advanced_at_utc_ms + 1,
        )),
        frozen: frozen(current_state.last_advanced_at_utc_ms + 1),
        stimulus: AutonomousStimulusV1::default(),
    };
    let normal_claim = runtime.claim_wake_v2(&persona, &normal_wake).unwrap();
    assert!(normal_claim.proposal.world_anchor.is_none());
    assert!(normal_claim.proposal.lived_day.is_none());
    assert!(normal_claim.proposal.dream_updates.is_empty());
    let retired_claim = seed_retired_wake_v2_claim(&path, &normal_claim);
    assert_eq!(claim_count(&path, &normal_claim.claim_token), 1);
    assert_eq!(claim_count(&path, &retired_claim.claim_token), 1);
    let normal_claim_before = claim_row_bytes(&path, &normal_claim.claim_token);
    let retired_claim_before = claim_row_bytes(&path, &retired_claim.claim_token);

    runtime.flush_and_close().unwrap();
    let mut reopened = AstrRuntime::open(&path).unwrap();
    let reservation_connection = Connection::open(&path).unwrap();
    reservation_connection
        .execute(
            "INSERT INTO externalization_budget_claim(
                 claim_token,relation_scope,budget_day_start_utc_ms,
                 reserved_tokens,migrated_unknown_full_charge
             ) VALUES(?1,?2,0,1,0)",
            params![retired_claim.claim_token.to_vec(), relation_scope.to_vec(),],
        )
        .unwrap();
    let conflicted = reopened.recover_autonomy(&persona, &recovery_frozen);
    assert!(matches!(
        conflicted,
        Err(RuntimeError::Store(ae_store::StoreError::AutonomyConflict(ref message)))
            if message.contains("wake v2 claim cannot own budget reservation")
    ));
    assert_eq!(claim_count(&path, &normal_claim.claim_token), 1);
    assert_eq!(claim_count(&path, &retired_claim.claim_token), 1);
    assert_eq!(
        claim_row_bytes(&path, &normal_claim.claim_token),
        normal_claim_before
    );
    assert_eq!(
        claim_row_bytes(&path, &retired_claim.claim_token),
        retired_claim_before
    );
    assert_eq!(retired_row_bytes(&path), retired_before);
    reservation_connection
        .execute(
            "DELETE FROM externalization_budget_claim WHERE claim_token=?1",
            params![retired_claim.claim_token.to_vec()],
        )
        .unwrap();
    drop(reservation_connection);

    let recovered = reopened
        .recover_autonomy(&persona, &recovery_frozen)
        .unwrap();
    assert_eq!(recovered.state.generation, current_state.generation);
    assert!(!recovered.offline_gap_recorded);
    assert_eq!(claim_count(&path, &retired_claim.claim_token), 0);
    assert_eq!(claim_count(&path, &normal_claim.claim_token), 1);
    assert_eq!(
        claim_row_bytes(&path, &normal_claim.claim_token),
        normal_claim_before
    );
    assert_eq!(retired_row_bytes(&path), retired_before);
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get::<_, u32>(0)
            })
            .unwrap(),
        7
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT digest FROM schema_migrations WHERE version=7",
                [],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .unwrap(),
        b"ae.autonomy.db.v7.alpha3-authority.v1"
    );
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='table' AND name LIKE 'body_%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    drop(connection);
    assert_eq!(retired_row_bytes(&path), retired_before);
    drop(reopened);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
