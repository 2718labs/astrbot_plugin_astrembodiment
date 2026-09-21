use ae_continuum::CommitEnvelope;
use ae_contracts::*;
use ae_fixed::Fixed;
use ae_runtime::{frozen_time_input_digest, AstrRuntime};
use rusqlite::{params, Connection};
use sha2::{Digest as _, Sha256};
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
        .join(format!("alpha3-contact-db-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    root.join("store.db")
}

fn genesis_request(seed: u8) -> PersonaGenesisRequest {
    let scope = PersonaScopeRef {
        bot_token: [seed; 16],
        persona_token: [seed.wrapping_add(1); 16],
    };
    let source = PersonaSourceRef {
        scope,
        source_digest: [seed.wrapping_add(2); 32],
        capability_digest: [seed.wrapping_add(3); 32],
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
            compiler_protocol_digest: [seed.wrapping_add(4); 32],
            compiler_model_digest: [seed.wrapping_add(5); 32],
        },
        formula_digest: [seed.wrapping_add(6); 32],
        incarnation_nonce: [seed.wrapping_add(7); 32],
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

fn policy(relation_scope: Digest) -> RelationTemporalPolicyV1 {
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
        intention_ttl_ms: 10 * 86_400_000,
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

fn batch(
    seed: u8,
    scope: &ScopeRef,
    base_revision: u64,
    facts: Vec<InteractionFactV1>,
) -> InteractionFactBatchV1 {
    InteractionFactBatchV1 {
        schema_version: ALPHA3_SCHEMA_VERSION,
        event_id: [seed; 16],
        scope: scope.clone(),
        causal: CausalRef {
            turn_id: [seed.wrapping_add(1); 16],
            action_id: None,
            delivery_id: None,
            claim_id: None,
            base_revision,
        },
        facts,
    }
}

fn frozen(now: u64) -> FrozenTimeInputV1 {
    let day_start = now - now % 86_400_000;
    FrozenTimeInputV1 {
        schema_version: 1,
        observed_now_utc_ms: now,
        effective_now_utc_ms: now,
        persona_tzid: "Asia/Shanghai".into(),
        persona_utc_offset_seconds: 28_800,
        persona_local_minute: 720,
        persona_day_ordinal: 1,
        relation_tzid: "Asia/Shanghai".into(),
        relation_utc_offset_seconds: 28_800,
        relation_local_minute: 720,
        relation_day_ordinal: 1,
        budget_day_start_utc_ms: day_start,
        budget_next_day_start_utc_ms: day_start + 86_400_000,
        next_timezone_transition_utc_ms: None,
        tzdb_fingerprint: [211; 32],
    }
}

fn wake(seed: u8, scope: &ScopeRef, generation: u64, now: u64) -> TimeAdvanceV1 {
    let frozen = frozen(now);
    TimeAdvanceV1 {
        event_id: [seed; 16],
        scope: scope.clone(),
        expected_generation: generation,
        frozen_input_digest: frozen_time_input_digest(&frozen),
        frozen,
        stimulus: AutonomousStimulusV1 {
            arousal: Fixed::ONE,
            urgency: Fixed::ONE,
            emergency_authorized: false,
            source_digest: [seed.wrapping_add(2); 32],
        },
    }
}

fn activation_fact(id: u8, kind: InteractionFactKindV1, now: u64) -> InteractionFactV1 {
    let mut activation = fact(id, kind, InteractionSourceAuthorityV1::ExplicitControl, now);
    match kind {
        InteractionFactKindV1::ContactGranted => {
            activation.value_code = Some(InteractionValueCodeV1::Grant);
            activation.consent_terms = Some(ConsentTermsV1 {
                purposes: vec![ContactPurposeV1::ExplicitFollowUp],
                channels: vec![ContactChannelV1::AstrbotSession],
                valid_until_utc_ms: Some(now + 10 * 86_400_000),
                pause_until_utc_ms: None,
            });
        }
        InteractionFactKindV1::ContactResumed => {
            activation.value_code = Some(InteractionValueCodeV1::Resume);
        }
        _ => panic!("activation fixture only supports grant/resume"),
    }
    activation
}

fn assert_capability_denied(response: Alpha3ResponseV1) {
    assert!(matches!(
        response,
        Alpha3ResponseV1::ApplyInteraction(Alpha3ResultV1::Error(Alpha3ErrorV1 {
            code: Alpha3ErrorCodeV1::CapabilityDenied,
            ..
        }))
    ));
}

fn direct_store_envelope(batch: &InteractionFactBatchV1) -> CommitEnvelope {
    let canonical = CanonicalEvent::InteractionFactBatch(batch.clone());
    let persona_scope =
        wire::persona_scope_digest(&batch.scope.bot_token, &batch.scope.persona_token, None);
    CommitEnvelope {
        event_kind: wire::event_kind_name(&canonical).to_owned(),
        event_bytes: wire::encode_event(&canonical),
        receipt: TransitionReceipt {
            schema_version: 1,
            formula_digest: [0; 32],
            scope_digest: persona_scope,
            event_digest: wire::event_digest(&canonical),
            authority_digest: ae_authority::authority_projection_digest(&canonical),
            base_revision: batch.causal.base_revision,
            next_revision: batch.causal.base_revision.saturating_add(1),
            state_before: [0; 32],
            state_after: [0; 32],
            graph_after: [0; 32],
            action_contract: None,
            active_nodes: 0,
            active_edges: 0,
            residuals: InvariantResiduals::default(),
            status: CommitStatus::Committed,
        },
        chain_seed: [0; 32],
        delta_bytes: serde_json::to_vec(&AutonomyJournalDeltaV1 {
            state: None,
            inner_events: vec![ae_store::alpha3::interaction_inner_event_v1(batch)],
            intention: None,
        })
        .unwrap(),
    }
}

fn seed_historical_grant(
    path: &Path,
    scope: &ScopeRef,
    persona_scope: Digest,
    base_revision: u64,
    relation_scope: Digest,
    now: u64,
) -> RelationConsentV1 {
    let grant_fact = activation_fact(0xd1, InteractionFactKindV1::ContactGranted, now);
    let grant_batch = batch(0xd0, scope, base_revision, vec![grant_fact.clone()]);
    let canonical = CanonicalEvent::InteractionFactBatch(grant_batch.clone());
    let event_bytes = wire::encode_event(&canonical);
    let event_digest = wire::event_digest(&canonical);
    let inner_event = ae_store::alpha3::interaction_inner_event_v1(&grant_batch);
    let delta_bytes = serde_json::to_vec(&AutonomyJournalDeltaV1 {
        state: None,
        inner_events: vec![inner_event.clone()],
        intention: None,
    })
    .unwrap();
    let mut connection = Connection::open(path).unwrap();
    let transaction = connection.transaction().unwrap();
    let (prior_receipt_bytes, prior_chain): (Vec<u8>, Vec<u8>) = transaction
        .query_row(
            "SELECT receipt_bytes,chain_digest FROM journal
             WHERE scope_digest=?1 ORDER BY logical_revision DESC LIMIT 1",
            params![persona_scope.to_vec()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let prior_receipt = wire::decode_transition_receipt(&prior_receipt_bytes).unwrap();
    assert_eq!(prior_receipt.next_revision, base_revision);
    let receipt = TransitionReceipt {
        schema_version: prior_receipt.schema_version,
        formula_digest: prior_receipt.formula_digest,
        scope_digest: persona_scope,
        event_digest,
        authority_digest: ae_authority::authority_projection_digest(&canonical),
        base_revision,
        next_revision: base_revision + 1,
        state_before: prior_receipt.state_after,
        state_after: prior_receipt.state_after,
        graph_after: prior_receipt.graph_after,
        action_contract: None,
        active_nodes: prior_receipt.active_nodes,
        active_edges: prior_receipt.active_edges,
        residuals: prior_receipt.residuals,
        status: CommitStatus::Committed,
    };
    let receipt_bytes = wire::encode_transition_receipt(&receipt);
    let prior_chain: Digest = prior_chain.try_into().unwrap();
    let chain_digest = ae_continuum::chain_link_with_delta(
        &prior_chain,
        &event_bytes,
        &receipt_bytes,
        &delta_bytes,
    );
    let (epoch, revision, body): (u64, u64, String) = transaction
        .query_row(
            "SELECT h.consent_epoch,h.revision,c.body_json
             FROM relation_consent_head AS h
             JOIN relation_consent AS c
               ON c.relation_scope=h.relation_scope
              AND c.consent_epoch=h.consent_epoch AND c.revision=h.revision
             WHERE h.relation_scope=?1",
            params![relation_scope.to_vec()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    let mut consent: RelationConsentV1 = serde_json::from_str(&body).unwrap();
    assert_eq!(consent.state, RelationConsentStateV1::Disabled);
    consent.revision = revision + 1;
    consent.state = RelationConsentStateV1::Granted;
    consent.purposes = vec![ContactPurposeV1::ExplicitFollowUp];
    consent.channels = vec![ContactChannelV1::AstrbotSession];
    consent.valid_from_utc_ms = now;
    consent.valid_until_utc_ms = Some(now + 10 * 86_400_000);
    consent.pause_until_utc_ms = None;
    consent.source_event_id = grant_fact.fact_id;
    let terms = grant_fact.consent_terms.as_ref().unwrap();
    let terms_bytes = serde_json::to_vec(&Some(terms)).unwrap();
    consent.policy_digest = wire::domain_hash(
        b"ae.relation-consent.policy.v1",
        &[&relation_scope, &grant_fact.source_digest, &terms_bytes],
    );
    consent.validate().unwrap();
    transaction
        .execute(
            "INSERT INTO journal(
                 logical_revision,scope_digest,base_revision,event_kind,event_bytes,
                 event_digest,receipt_bytes,delta_bytes,chain_digest,committed_at_ms
             ) VALUES(?1,?2,?3,'interaction_fact_batch',?4,?5,?6,?7,?8,?9)",
            params![
                (base_revision + 1) as i64,
                persona_scope.to_vec(),
                base_revision as i64,
                event_bytes,
                event_digest.to_vec(),
                receipt_bytes,
                delta_bytes,
                chain_digest.to_vec(),
                now as i64,
            ],
        )
        .unwrap();
    transaction
        .execute(
            "INSERT INTO interaction_fact(
                 fact_id,event_id,persona_scope,relation_scope,observed_at_utc_ms,
                 source_digest,revision,body_json
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                grant_fact.fact_id.to_vec(),
                grant_batch.event_id.to_vec(),
                persona_scope.to_vec(),
                relation_scope.to_vec(),
                grant_fact.observed_at_utc_ms as i64,
                grant_fact.source_digest.to_vec(),
                (base_revision + 1) as i64,
                serde_json::to_string(&grant_fact).unwrap(),
            ],
        )
        .unwrap();
    transaction
        .execute(
            "INSERT INTO relation_consent(
                 relation_scope,consent_epoch,revision,state,body_json
             ) VALUES(?1,?2,?3,'granted',?4)",
            params![
                relation_scope.to_vec(),
                epoch as i64,
                consent.revision as i64,
                serde_json::to_string(&consent).unwrap(),
            ],
        )
        .unwrap();
    assert_eq!(
        transaction
            .execute(
                "UPDATE relation_consent_head
                 SET revision=?3
                 WHERE relation_scope=?1 AND consent_epoch=?2 AND revision=?4",
                params![
                    relation_scope.to_vec(),
                    epoch as i64,
                    consent.revision as i64,
                    revision as i64,
                ],
            )
            .unwrap(),
        1
    );
    let inner_body = serde_json::to_vec(&inner_event).unwrap();
    let mut manifest_hasher = Sha256::new();
    manifest_hasher.update(b"ae.autonomy.inner-event-manifest.v1");
    manifest_hasher.update(1_u64.to_le_bytes());
    manifest_hasher.update(inner_event.event_id);
    manifest_hasher.update((inner_body.len() as u64).to_le_bytes());
    manifest_hasher.update(&inner_body);
    let manifest_digest: Digest = manifest_hasher.finalize().into();
    transaction
        .execute(
            "INSERT INTO inner_event_manifest(
                 persona_scope,journal_revision,event_count,event_digest
             ) VALUES(?1,?2,1,?3)",
            params![
                persona_scope.to_vec(),
                (base_revision + 1) as i64,
                manifest_digest.to_vec(),
            ],
        )
        .unwrap();
    transaction
        .execute(
            "INSERT INTO inner_event(
                 event_id,persona_scope,journal_revision,committed_at_utc_ms,
                 kind,tombstoned,body_json
             ) VALUES(?1,?2,?3,?4,?5,0,?6)",
            params![
                inner_event.event_id.to_vec(),
                persona_scope.to_vec(),
                (base_revision + 1) as i64,
                inner_event.committed_at_utc_ms as i64,
                format!("{:?}", inner_event.kind),
                serde_json::to_string(&inner_event).unwrap(),
            ],
        )
        .unwrap();
    transaction
        .execute(
            "INSERT INTO applied_events(scope_digest,event_digest,revision)
             VALUES(?1,?2,?3)",
            params![
                persona_scope.to_vec(),
                event_digest.to_vec(),
                (base_revision + 1) as i64,
            ],
        )
        .unwrap();
    transaction.commit().unwrap();
    consent
}

fn consent_row_bytes(path: &Path, consent: &RelationConsentV1) -> Vec<u8> {
    Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT CAST(body_json AS BLOB) FROM relation_consent
             WHERE relation_scope=?1 AND consent_epoch=?2 AND revision=?3",
            params![
                consent.relation_scope.to_vec(),
                consent.consent_epoch as i64,
                consent.revision as i64,
            ],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn contact_consent_and_causes_are_relation_local_authority() {
    let path = temp_db();
    let mut runtime = AstrRuntime::open(&path).unwrap();
    let genesis = genesis_request(21);
    runtime.ensure_genesis(&genesis).unwrap();

    let persona = ScopeRef {
        bot_token: genesis.source.scope.bot_token,
        persona_token: genesis.source.scope.persona_token,
        relation_token: None,
        session_token: [30; 16],
    };
    let persona_scope =
        wire::persona_scope_digest(&persona.bot_token, &persona.persona_token, None);
    let mut relation_one = persona.clone();
    relation_one.relation_token = Some([31; 16]);
    relation_one.session_token = [32; 16];
    let mut relation_two = persona.clone();
    relation_two.relation_token = Some([33; 16]);
    relation_two.session_token = [34; 16];
    let relation_one_scope = wire::persona_scope_digest(
        &relation_one.bot_token,
        &relation_one.persona_token,
        relation_one.relation_token.as_ref(),
    );
    let relation_two_scope = wire::persona_scope_digest(
        &relation_two.bot_token,
        &relation_two.persona_token,
        relation_two.relation_token.as_ref(),
    );

    let initial = runtime
        .bootstrap_autonomy(&persona, &profile(persona_scope), None)
        .unwrap();
    runtime
        .bootstrap_autonomy(
            &relation_one,
            &profile(persona_scope),
            Some(&policy(relation_one_scope)),
        )
        .unwrap();
    runtime
        .bootstrap_autonomy(
            &relation_two,
            &profile(persona_scope),
            Some(&policy(relation_two_scope)),
        )
        .unwrap();

    let relation_one_contact_before = runtime.relation_contact_v1(&relation_one).unwrap();
    let relation_two_contact_before = runtime.relation_contact_v1(&relation_two).unwrap();
    let relation_two_consent_before = runtime.relation_consent_v1(&relation_two).unwrap();
    let now = 1_700_100_000_000;
    let inbound = fact(
        40,
        InteractionFactKindV1::InboundObserved,
        InteractionSourceAuthorityV1::AstrbotMetadata,
        now,
    );
    let inbound_result = runtime
        .apply_interaction_fact_batch_v1(&batch(41, &relation_one, 0, vec![inbound]))
        .unwrap();
    assert!(
        runtime.relation_contact_v1(&relation_one).unwrap().revision
            > relation_one_contact_before.revision
    );
    assert_eq!(
        runtime.relation_contact_v1(&relation_two).unwrap(),
        relation_two_contact_before
    );

    let disabled = runtime.relation_consent_v1(&relation_one).unwrap();
    assert_eq!(disabled.state, RelationConsentStateV1::Disabled);
    let live_revision = inbound_result.receipt.canonical_revision;
    let raw_grant = batch(
        42,
        &relation_one,
        live_revision,
        vec![activation_fact(
            43,
            InteractionFactKindV1::ContactGranted,
            now + 1,
        )],
    );
    assert_capability_denied(
        runtime
            .handle_alpha3(Alpha3RequestV1::ApplyInteraction(raw_grant.clone()))
            .unwrap(),
    );
    assert_eq!(runtime.current_revision(&persona).unwrap(), live_revision);
    assert_eq!(
        runtime.relation_consent_v1(&relation_one).unwrap(),
        disabled
    );

    let current_contact = runtime.relation_contact_v1(&relation_one).unwrap();
    let mut direct_store = ae_store::Store::open(&path).unwrap();
    for (seed, kind) in [
        (44, InteractionFactKindV1::ContactGranted),
        (46, InteractionFactKindV1::ContactResumed),
    ] {
        let raw = batch(
            seed,
            &relation_one,
            live_revision,
            vec![activation_fact(seed + 1, kind, now + u64::from(seed))],
        );
        let error = direct_store
            .apply_interaction_fact_batch_v1(&direct_store_envelope(&raw), &current_contact)
            .unwrap_err();
        assert!(
            matches!(
                error,
                ae_store::StoreError::AutonomyConflict(ref message)
                    if message.contains("ALPHA3_CAPABILITY_DENIED")
            ),
            "direct store activation was not fail-closed: {error}"
        );
        assert_eq!(
            direct_store.current_revision(&persona_scope).unwrap(),
            live_revision
        );
    }
    drop(direct_store);

    let historical_grant = seed_historical_grant(
        &path,
        &relation_one,
        persona_scope,
        live_revision,
        relation_one_scope,
        now + 100,
    );
    let historical_grant_bytes = consent_row_bytes(&path, &historical_grant);
    drop(runtime);
    let mut runtime = AstrRuntime::open(&path).unwrap();
    assert_eq!(
        runtime.relation_consent_v1(&relation_one).unwrap(),
        historical_grant
    );
    assert_eq!(
        consent_row_bytes(&path, &historical_grant),
        historical_grant_bytes
    );
    let historical_replay = runtime
        .verify_replay(&persona.bot_token, &persona.persona_token)
        .unwrap();
    assert!(historical_replay.ok, "{:?}", historical_replay.first_error);

    let cause_time = now + 1_000;
    let eligible_at = cause_time;
    let mut follow_up = fact(
        50,
        InteractionFactKindV1::FollowUpRequested,
        InteractionSourceAuthorityV1::ExplicitControl,
        cause_time,
    );
    follow_up.value_code = Some(InteractionValueCodeV1::FollowUp);
    follow_up.scheduled_at_utc_ms = Some(eligible_at);
    follow_up.expires_at_utc_ms = Some(cause_time + 10 * 86_400_000);
    let historical_revision = runtime.current_revision(&persona).unwrap();
    assert_eq!(historical_revision, live_revision + 1);
    let follow_up_result = runtime
        .apply_interaction_fact_batch_v1(&batch(
            51,
            &relation_one,
            historical_revision,
            vec![follow_up],
        ))
        .unwrap();
    let relation_one_contact = runtime.relation_contact_v1(&relation_one).unwrap();
    assert!(relation_one_contact.active_cause_digest.is_some());
    assert_eq!(
        runtime.relation_contact_v1(&relation_two).unwrap(),
        relation_two_contact_before
    );

    let contact_wake = runtime
        .claim_wake_v2(
            &persona,
            &wake(52, &persona, initial.generation, eligible_at + 3_600_000),
        )
        .unwrap();
    assert_eq!(contact_wake.proposal.legacy.intentions.len(), 1);
    assert_eq!(contact_wake.proposal.contact_bases.len(), 1);
    let intention_id = contact_wake.proposal.legacy.intentions[0].intention_id;
    let settled = runtime.settle_wake_v2(&contact_wake.claim_token).unwrap();
    assert!(settled.receipt.canonical_revision > follow_up_result.receipt.canonical_revision);

    let pause_time = eligible_at + 3_600_001;
    let mut pause = fact(
        53,
        InteractionFactKindV1::ContactPaused,
        InteractionSourceAuthorityV1::ExplicitControl,
        pause_time,
    );
    pause.value_code = Some(InteractionValueCodeV1::Pause);
    let pause_result = runtime
        .apply_interaction_fact_batch_v1(&batch(
            54,
            &relation_one,
            settled.receipt.canonical_revision,
            vec![pause],
        ))
        .unwrap();
    assert_eq!(
        runtime.relation_consent_v1(&relation_one).unwrap().state,
        RelationConsentStateV1::Paused
    );
    assert_eq!(
        runtime.intention_v1(&intention_id).unwrap().unwrap().state,
        IntentionStateV1::Suppressed
    );
    assert!(
        !runtime
            .contact_intention_basis_v1(&intention_id)
            .unwrap()
            .unwrap()
            .live
    );

    let paused = runtime.relation_consent_v1(&relation_one).unwrap();
    let raw_resume = batch(
        55,
        &relation_one,
        pause_result.receipt.canonical_revision,
        vec![activation_fact(
            56,
            InteractionFactKindV1::ContactResumed,
            pause_time + 1,
        )],
    );
    assert_capability_denied(
        runtime
            .handle_alpha3(Alpha3RequestV1::ApplyInteraction(raw_resume))
            .unwrap(),
    );
    assert_eq!(
        runtime.current_revision(&persona).unwrap(),
        pause_result.receipt.canonical_revision
    );
    assert_eq!(runtime.relation_consent_v1(&relation_one).unwrap(), paused);

    let mut end = fact(
        57,
        InteractionFactKindV1::RelationEnded,
        InteractionSourceAuthorityV1::ExplicitControl,
        pause_time + 2,
    );
    end.value_code = Some(InteractionValueCodeV1::End);
    runtime
        .apply_interaction_fact_batch_v1(&batch(
            58,
            &relation_one,
            pause_result.receipt.canonical_revision,
            vec![end],
        ))
        .unwrap();
    assert_eq!(
        runtime.relation_consent_v1(&relation_one).unwrap().state,
        RelationConsentStateV1::Ended
    );
    assert_eq!(
        runtime.relation_consent_v1(&relation_two).unwrap(),
        relation_two_consent_before
    );
    assert_eq!(
        runtime.relation_contact_v1(&relation_two).unwrap(),
        relation_two_contact_before
    );
    assert_eq!(
        consent_row_bytes(&path, &historical_grant),
        historical_grant_bytes
    );

    let replay = runtime
        .verify_replay(&persona.bot_token, &persona.persona_token)
        .unwrap();
    assert!(replay.ok, "{:?}", replay.first_error);
    runtime.flush_and_close().unwrap();

    let mut reopened = AstrRuntime::open(&path).unwrap();
    assert_eq!(
        reopened.relation_consent_v1(&relation_one).unwrap().state,
        RelationConsentStateV1::Ended
    );
    assert_eq!(
        consent_row_bytes(&path, &historical_grant),
        historical_grant_bytes
    );
    let reopened_replay = reopened
        .verify_replay(&persona.bot_token, &persona.persona_token)
        .unwrap();
    assert!(reopened_replay.ok, "{:?}", reopened_replay.first_error);
    drop(reopened);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
