use ae_contracts::*;
use ae_fixed::Fixed;
use ae_runtime::AstrRuntime;

#[test]
#[ignore = "diagnostic reopen of an explicitly supplied Task11 test fixture"]
fn reopen_clock_fixture_from_env() {
    let path=std::env::var("AE_TASK11_REOPEN_FIXTURE").expect("test fixture path required");
    let mut runtime=AstrRuntime::open(std::path::Path::new(&path)).unwrap();
    let scope=genesis().source.scope;
    let head=runtime.get_embodiment_persona_v1(&scope).unwrap();
    match head { EmbodimentPersonaLookupV1::Present { clock_head,.. } => assert_eq!(clock_head.recent_count,64),_=>panic!("missing clock") }
}

#[test]
#[ignore = "tamper check on a copy of an explicitly supplied Task11 test fixture"]
fn missing_clock_proof_fails_reopen_from_env() {
    let source=std::env::var("AE_TASK11_REOPEN_FIXTURE").expect("test fixture path required");
    let copy=std::env::temp_dir().join(format!("ae-clock-proof-missing-{}-{}.sqlite",std::process::id(),ae_store::now_ms()));
    let conn=rusqlite::Connection::open_with_flags(source,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    conn.execute("VACUUM INTO ?1",rusqlite::params![copy.to_str().unwrap()]).unwrap();
    drop(conn);
    let conn=rusqlite::Connection::open(&copy).unwrap();
    let guard:String=conn.query_row("SELECT sql FROM sqlite_schema WHERE name='semantic_clock_proof_no_delete_v1'",[],|r|r.get(0)).unwrap();
    conn.execute_batch("DROP TRIGGER semantic_clock_proof_no_delete_v1; DELETE FROM semantic_clock_anchor_proof_v1;").unwrap();
    conn.execute_batch(&guard).unwrap();
    drop(conn);
    let error=match AstrRuntime::open(&copy) { Ok(_)=>panic!("missing proof accepted"),Err(e)=>e.to_string() };
    assert!(error.contains("SEMANTIC_CLOCK_PROOF_MISSING"),"{error}");
    let _=std::fs::remove_file(copy);
}

#[test]
#[ignore = "requires a real pre-boundary Perception1-Time2 database fixture"]
fn legacy_time_clock_first_semantic_anchor_from_env() {
    let source=std::env::var("AE_TASK11_LEGACY_FIXTURE").expect("legacy test fixture path required");
    let path=std::env::temp_dir().join(format!("ae-clock-legacy-time-{}-{}.sqlite",std::process::id(),ae_store::now_ms()));
    let conn=rusqlite::Connection::open_with_flags(source,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    conn.execute("VACUUM INTO ?1",rusqlite::params![path.to_str().unwrap()]).unwrap();drop(conn);
    let mut runtime=AstrRuntime::open(&path).unwrap();let scope=genesis().source.scope;let p=core_persona_digest(&scope);
    create_clock(&mut runtime,&scope);
    match runtime.get_embodiment_persona_v1(&scope).unwrap() { EmbodimentPersonaLookupV1::Present{first_creation_receipt,clock_head,..}=>{assert_eq!(first_creation_receipt.initial_semantic_revision,2);assert_eq!(clock_head.matrix_epoch.anchor_semantic_revision,1);assert_eq!(clock_head.matrix_epoch.asleep_ticks,1);}, _=>panic!("missing") }
    advance_clock(&mut runtime,&scope,1773003600000);
    let event=[91;32];let turn=core_id(b"ae.core-inbound.turn-id.v1",&[&p,&event]);
    let inbound=runtime.commit_core_inbound_v1(&CommitCoreInboundV1 { schema_version:1,observation:CoreInboundObservationV1 { schema_version:1,operation_id:core_id(b"ae.core-inbound.operation-id.v1",&[&p,&turn]),scope:scope.clone(),turn_id:turn,observed_at_utc_ms:ae_store::now_ms(),message_digest:[30;32],astrbot_event_identity_digest:event,astrbot_source_digest:[31;32],extractor_digest:[32;32],confidence:Fixed::ONE,relation_evidence_ref:None,session_evidence_ref:None },appraisal:Some(CoreInboundAppraisalReservationV1{daily_token_limit:5000,reserved_tokens:1000,provider_digest:[33;32]}) }).unwrap();
    let challenge=inbound.initial_receipt.challenge.unwrap();
    let request=SemanticAppraisalSettleRequestV1 { schema_version:1,scope:challenge.origin.scope,request_nonce_digest:challenge.request_nonce_digest,outcome:SemanticAppraisalOutcomeV1::Success,provider_usage:SemanticAppraisalProviderUsageV1{known:true,used_tokens:Some(200)},proposal:Some(PerceptionProposalV1{schema_version:1,origin_digest:challenge.origin.origin_digest,dimensions:EvidenceVector::default(),estimator_confidence:Fixed::ONE,protocol_version:1,request_nonce_digest:challenge.request_nonce_digest}) };
    assert_eq!(runtime.settle_semantic_appraisal_v1(&request).unwrap().status,SemanticAppraisalSettleStatusV1::Committed);
    match runtime.get_embodiment_persona_v1(&scope).unwrap() { EmbodimentPersonaLookupV1::Present{clock_head,..}=>assert_eq!(clock_head.matrix_epoch.anchor_semantic_revision,3),_=>panic!("missing") }
    drop(runtime);
    let conn=rusqlite::Connection::open(&path).unwrap();let count:u64=conn.query_row("SELECT COUNT(*) FROM semantic_clock_anchor_proof_v1",[],|r|r.get(0)).unwrap();assert_eq!(count,1);drop(conn);
    drop(AstrRuntime::open(&path).unwrap());let _=std::fs::remove_file(path);
}

fn genesis() -> PersonaGenesisRequest {
    let source = PersonaSourceRef {
        scope: PersonaScopeRef {
            bot_token: [21; 16],
            persona_token: [22; 16],
        },
        source_digest: [23; 32],
        capability_digest: [24; 32],
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
            epistemic: EpistemicPriors::default(),
            social: SocialPriors::default(),
            compiler_protocol_digest: [25; 32],
            compiler_model_digest: [26; 32],
        },
        formula_digest: [27; 32],
        incarnation_nonce: [28; 32],
        parent_incarnation_id: None,
        observed_at_ms: ae_store::now_ms(),
    }
}

fn create_clock(runtime: &mut AstrRuntime, scope: &PersonaScopeRef) {
    let incarnation = match runtime.get_embodiment_persona_v1(scope).unwrap() { EmbodimentPersonaLookupV1::Missing { incarnation_digest } => incarnation_digest, _ => panic!("already created") };
    let frozen = freeze_embodiment_time_v1("UTC",1773000000000).unwrap();
    runtime.create_embodiment_persona_if_missing_v1(&CreateEmbodimentPersonaIfMissingV1 {
        schema_version:1, operation_id:core_id(b"ae.embodiment.create.operation-id.v1", &[&core_persona_digest(scope), &incarnation]), scope:scope.clone(), incarnation_digest:incarnation,
        profile_template:EmbodimentTemporalProfileV1 { schema_version:1, persona_tzid:"UTC".into(), tzdb_release:frozen.tzdb_release.clone(), tzdb_content_sha256:frozen.tzdb_content_sha256, circadian_period_millis:86400000, homeostatic_awake_gain_per_hour:Fixed::from_raw(40000),homeostatic_asleep_decay_per_hour:Fixed::from_raw(100000),drowsy_enter_threshold:Fixed::from_raw(750000),drowsy_exit_threshold:Fixed::from_raw(500000),endogenous_phase_hysteresis:Fixed::from_raw(50000),maximum_analytic_horizon_ms:EMBODIMENT_MAX_HORIZON_MS },
        sleep_schedule:EmbodimentSleepScheduleV1 { schema_version:1,mode:SleepScheduleModeV1::Auto,chronotype:ChronotypeV1::Intermediate,preferred_sleep_local_minute:1380,preferred_wake_local_minute:420,sleep_flex_minutes:60,entrainment_rate_minutes_per_day:15 }, frozen_creation:frozen
    }).unwrap();
}

fn advance_clock(runtime: &mut AstrRuntime, scope: &PersonaScopeRef, now: u64) -> Vec<u8> {
    let mut r=EmbodimentTimeAdvanceRequestV1 { schema_version:1,operation_id:[0;16],scope:scope.clone(),profile_revision:1,schedule_revision:1,frozen:freeze_embodiment_time_v1("UTC",now).unwrap() };
    r.operation_id=r.expected_operation_id();
    let bytes=r.encode_wire_v1().unwrap();
    runtime.advance_embodiment_time_v1(&bytes).unwrap();
    bytes
}

#[test]
fn clock_status_profile_cas_and_exact_replay() {
    let path=std::env::temp_dir().join(format!("ae-clock-cas-{}-{}.sqlite",std::process::id(),ae_store::now_ms()));
    let mut runtime=AstrRuntime::open(&path).unwrap();
    let genesis=genesis();runtime.ensure_genesis(&genesis).unwrap();let scope=genesis.source.scope;
    create_clock(&mut runtime,&scope);
    let initial=runtime.get_embodiment_persona_v1(&scope).unwrap();
    let head=match &initial { EmbodimentPersonaLookupV1::Present { clock_head,.. }=>clock_head, _=>panic!("missing") };
    let mut status=EmbodimentClockStatusRequestV1 { schema_version:1,scope:scope.clone(),profile_revision:1,schedule_revision:1,frozen:freeze_embodiment_time_v1("UTC",head.last_now_utc_ms).unwrap() };
    assert!(matches!(runtime.embodiment_clock_status_v1(&status).unwrap(),EmbodimentClockStatusV1::NotDue{..}));
    assert_eq!(runtime.get_embodiment_persona_v1(&scope).unwrap(),initial);
    status.frozen=freeze_embodiment_time_v1("UTC",head.next_due_at_utc_ms).unwrap();
    let bytes=match runtime.embodiment_clock_status_v1(&status).unwrap() { EmbodimentClockStatusV1::Due { exact_advance_request_bytes,.. }=>exact_advance_request_bytes,_=>panic!("not due") };
    assert_eq!(EmbodimentTimeAdvanceRequestV1::decode_wire_v1(&bytes).unwrap().frozen,status.frozen);
    assert_eq!(runtime.get_embodiment_persona_v1(&scope).unwrap(),initial);
    let first=runtime.advance_embodiment_time_v1(&bytes).unwrap();
    assert_eq!(runtime.advance_embodiment_time_v1(&bytes).unwrap().receipt,first.receipt);
    let profile=runtime.read_embodiment_profile_v1(&scope).unwrap();
    let frozen=freeze_embodiment_time_v1("UTC+01:00",status.frozen.now_utc_ms+60000).unwrap();
    let mut replacement=profile.profile;replacement.persona_tzid=frozen.persona_tzid.clone();
    let mut schedule=profile.schedule;schedule.mode=SleepScheduleModeV1::Fixed;
    let pd=wire::domain_hash(b"ae.embodiment.profile.v1",&[&core_encode(&replacement).unwrap()]);
    let sd=wire::domain_hash(b"ae.embodiment.sleep-schedule.v1",&[&core_encode(&schedule).unwrap()]);
    let mut cas=CompareAndSwapEmbodimentProfileV1 { schema_version:1,operation_id:core_id(b"ae.embodiment.profile-cas.operation-id.v1",&[&core_persona_digest(&scope),&1u64.to_le_bytes(),&1u64.to_le_bytes(),&pd,&sd]),scope:scope.clone(),expected_profile_revision:1,expected_schedule_revision:1,replacement_profile:replacement,replacement_schedule:schedule,frozen };
    let committed=runtime.compare_and_swap_embodiment_profile_v1(&cas).unwrap();
    assert_eq!(committed.receipt.result.discrete_event_mask&16,16);
    assert_eq!(committed.receipt.result.profile_revision,2);
    let replay=runtime.compare_and_swap_embodiment_profile_v1(&cas).unwrap();
    assert_eq!(replay.commit_status,CoreCommitStatusV1::Existing);assert_eq!(replay.receipt,committed.receipt);
    cas.frozen.now_utc_ms+=1;
    assert!(runtime.compare_and_swap_embodiment_profile_v1(&cas).unwrap_err().to_string().contains("IDEMPOTENCY_CONFLICT"));
    drop(runtime);let _=std::fs::remove_file(path);
}

#[test]
fn clock_corruption_blocks_new_ingress_and_delivery_but_not_exact_replay() {
    let path=std::env::temp_dir().join(format!("ae-clock-ingress-corrupt-{}-{}.sqlite",std::process::id(),ae_store::now_ms()));
    let mut runtime=AstrRuntime::open(&path).unwrap();
    let genesis=genesis();runtime.ensure_genesis(&genesis).unwrap();let scope=genesis.source.scope;
    create_clock(&mut runtime,&scope);let p=core_persona_digest(&scope);
    let inbound=|tag:u8| { let turn=core_id(b"ae.core-inbound.turn-id.v1",&[&p,&[tag;32]]); CommitCoreInboundV1 { schema_version:1,observation:CoreInboundObservationV1 { schema_version:1,operation_id:core_id(b"ae.core-inbound.operation-id.v1",&[&p,&turn]),scope:scope.clone(),turn_id:turn,observed_at_utc_ms:1773000000000,message_digest:[30;32],astrbot_event_identity_digest:[tag;32],astrbot_source_digest:[31;32],extractor_digest:[32;32],confidence:Fixed::ONE,relation_evidence_ref:None,session_evidence_ref:None },appraisal:None } };
    let delivery=|r:&CommitCoreInboundV1| CommitCoreDeliveryOutcomeV1 { schema_version:1,operation_id:core_id(b"ae.core-delivery.operation-id.v1",&[&p,&r.observation.turn_id,&r.observation.operation_id]),scope:scope.clone(),turn_id:r.observation.turn_id,inbound_operation_id:r.observation.operation_id,delivered:true,observed_at_utc_ms:1773000000000,visible_action_digest:[34;32] };
    let first=inbound(70);let pending=inbound(71);
    runtime.commit_core_inbound_v1(&first).unwrap();runtime.commit_core_inbound_v1(&pending).unwrap();
    let sent=delivery(&first);runtime.commit_core_delivery_outcome_v1(&sent).unwrap();
    let conn=rusqlite::Connection::open(&path).unwrap();
    let guard:String=conn.query_row("SELECT sql FROM sqlite_schema WHERE name='embodiment_clock_head_update_guard_v1'",[],|r|r.get(0)).unwrap();
    conn.execute_batch("DROP TRIGGER embodiment_clock_head_update_guard_v1; UPDATE embodiment_clock_head_v1 SET head_digest=zeroblob(32);").unwrap();conn.execute_batch(&guard).unwrap();
    let count=||conn.query_row("SELECT (SELECT COUNT(*) FROM journal),(SELECT COUNT(*) FROM core_inbound_receipt_v1),(SELECT COUNT(*) FROM core_delivery_receipt_v1)",[],|r|Ok((r.get::<_,u64>(0)?,r.get::<_,u64>(1)?,r.get::<_,u64>(2)?))).unwrap();
    let before=count();
    assert!(runtime.commit_core_inbound_v1(&inbound(72)).unwrap_err().to_string().contains("CLOCK_HEAD_INVALID"));
    assert!(runtime.commit_core_delivery_outcome_v1(&delivery(&pending)).unwrap_err().to_string().contains("CLOCK_HEAD_INVALID"));
    assert_eq!(runtime.commit_core_inbound_v1(&first).unwrap().commit_status,CoreCommitStatusV1::Existing);
    assert_eq!(runtime.commit_core_delivery_outcome_v1(&sent).unwrap().commit_status,CoreCommitStatusV1::Existing);
    assert_eq!(count(),before);
    drop(conn);drop(runtime);let _=std::fs::remove_file(path);
}

#[test]
fn core_inbound_delivery_replay_never_reauthorizes_or_reuses_a_host_base() {
    let path = std::env::temp_dir().join(format!(
        "ae-core-ingress-{}-{}.sqlite",
        std::process::id(),
        ae_store::now_ms()
    ));
    let mut runtime = AstrRuntime::open(&path).unwrap();
    let genesis = genesis();
    runtime.ensure_genesis(&genesis).unwrap();
    let scope = genesis.source.scope;
    create_clock(&mut runtime, &scope);
    let advance = advance_clock(&mut runtime, &scope, 1773003600000);
    let p = core_persona_digest(&scope);
    let event_identity = [29; 32];
    let turn = core_id(b"ae.core-inbound.turn-id.v1", &[&p, &event_identity]);
    let op = core_id(b"ae.core-inbound.operation-id.v1", &[&p, &turn]);
    let request = CommitCoreInboundV1 {
        schema_version: 1,
        observation: CoreInboundObservationV1 {
            schema_version: 1,
            operation_id: op,
            scope: scope.clone(),
            turn_id: turn,
            observed_at_utc_ms: ae_store::now_ms(),
            message_digest: [30; 32],
            astrbot_event_identity_digest: event_identity,
            astrbot_source_digest: [31; 32],
            extractor_digest: [32; 32],
            confidence: Fixed::ONE,
            relation_evidence_ref: None,
            session_evidence_ref: None,
        },
        appraisal: Some(CoreInboundAppraisalReservationV1 {
            daily_token_limit: 5000,
            reserved_tokens: 1000,
            provider_digest: [33; 32],
        }),
    };
    let first = runtime.commit_core_inbound_v1(&request).unwrap();
    assert!(first.provider_authorized_now);
    assert!(first
        .initial_receipt
        .challenge
        .as_ref()
        .unwrap()
        .origin
        .validate_core_v1());
    let challenge = first.initial_receipt.challenge.as_ref().unwrap();
    let settled = runtime
        .settle_semantic_appraisal_v1(&SemanticAppraisalSettleRequestV1 {
            schema_version: 1,
            scope: challenge.origin.scope.clone(),
            request_nonce_digest: challenge.request_nonce_digest,
            outcome: SemanticAppraisalOutcomeV1::Success,
            provider_usage: SemanticAppraisalProviderUsageV1 {
                known: true,
                used_tokens: Some(200),
            },
            proposal: Some(PerceptionProposalV1 {
                schema_version: 1,
                origin_digest: challenge.origin.origin_digest,
                dimensions: EvidenceVector::default(),
                estimator_confidence: Fixed::ONE,
                protocol_version: 1,
                request_nonce_digest: challenge.request_nonce_digest,
            }),
        })
        .unwrap();
    assert_eq!(settled.status, SemanticAppraisalSettleStatusV1::Committed);
    assert_eq!(runtime.advance_embodiment_time_v1(&advance).unwrap().commit_status, CoreCommitStatusV1::Existing);
    let after_semantic=runtime.get_embodiment_persona_v1(&scope).unwrap();
    match after_semantic { EmbodimentPersonaLookupV1::Present { clock_head, .. } => { assert_eq!(clock_head.sequence,2); assert_eq!(clock_head.time_revision,2); assert_eq!(clock_head.matrix_epoch.anchor_semantic_revision,1); }, _ => panic!("missing") }
    let delivery = CommitCoreDeliveryOutcomeV1 {
        schema_version: 1,
        operation_id: core_id(b"ae.core-delivery.operation-id.v1", &[&p, &turn, &op]),
        scope: scope.clone(),
        turn_id: turn,
        inbound_operation_id: op,
        delivered: true,
        observed_at_utc_ms: ae_store::now_ms(),
        visible_action_digest: [34; 32],
    };
    let delivered = runtime.commit_core_delivery_outcome_v1(&delivery).unwrap();
    assert_eq!(
        delivered.receipt.transition.base_revision,
        settled.canonical_revision
    );
    let replay = runtime.commit_core_inbound_v1(&request).unwrap();
    assert_eq!(replay.commit_status, CoreCommitStatusV1::Existing);
    assert!(!replay.provider_authorized_now);
    assert_eq!(first.initial_receipt, replay.initial_receipt);
    let repeated = runtime.commit_core_delivery_outcome_v1(&delivery).unwrap();
    assert_eq!(repeated.commit_status, CoreCommitStatusV1::Existing);
    assert_eq!(repeated.receipt, delivered.receipt);
    let mut changed = delivery;
    changed.delivered = false;
    assert!(runtime
        .commit_core_delivery_outcome_v1(&changed)
        .unwrap_err()
        .to_string()
        .contains("IDEMPOTENCY_CONFLICT"));
    let mut exhausted = request.clone();
    exhausted.observation.astrbot_event_identity_digest = [35; 32];
    exhausted.observation.turn_id = core_id(b"ae.core-inbound.turn-id.v1", &[&p, &[35; 32]]);
    exhausted.observation.operation_id = core_id(
        b"ae.core-inbound.operation-id.v1",
        &[&p, &exhausted.observation.turn_id],
    );
    exhausted.appraisal.as_mut().unwrap().daily_token_limit = 0;
    let denied = runtime.commit_core_inbound_v1(&exhausted).unwrap();
    assert_eq!(
        denied.initial_receipt.disposition,
        CoreInitialAppraisalDispositionV1::BudgetExhausted
    );
    assert!(!denied.provider_authorized_now);
    assert_eq!(
        runtime
            .commit_core_inbound_v1(&exhausted)
            .unwrap()
            .initial_receipt,
        denied.initial_receipt
    );
    for i in 2..=66 { advance_clock(&mut runtime,&scope,1773000000000+i*3600000); }
    let conn=rusqlite::Connection::open(&path).unwrap();
    let counts:(u64,u64,u64)=conn.query_row("SELECT (SELECT COUNT(*) FROM embodiment_clock_receipt_v1),(SELECT COUNT(*) FROM semantic_clock_anchor_proof_v1),(SELECT COUNT(*) FROM semantic_commits)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(counts,(64,1,1));
    drop(conn);
    drop(runtime);
    let mut reopened=AstrRuntime::open(&path).unwrap();
    reopened.get_embodiment_persona_v1(&scope).unwrap();
    drop(reopened);
    let _ = std::fs::remove_file(path);
}

#[test]
fn persona_creation_is_genesis_anchored_and_restart_is_read_only() {
    let path=std::env::temp_dir().join(format!("ae-clock-create-{}-{}.sqlite",std::process::id(),ae_store::now_ms()));
    let mut runtime=AstrRuntime::open(&path).unwrap();
    let genesis=genesis();runtime.ensure_genesis(&genesis).unwrap();let scope=genesis.source.scope;
    let incarnation=match runtime.get_embodiment_persona_v1(&scope).unwrap(){EmbodimentPersonaLookupV1::Missing{incarnation_digest}=>incarnation_digest,_=>panic!("new persona unexpectedly initialized")};
    let frozen=freeze_embodiment_time_v1("UTC-07:00",1773000000000).unwrap();
    let mut request=CreateEmbodimentPersonaIfMissingV1{schema_version:1,operation_id:core_id(b"ae.embodiment.create.operation-id.v1",&[&core_persona_digest(&scope),&incarnation]),scope:scope.clone(),incarnation_digest:incarnation,
        profile_template:EmbodimentTemporalProfileV1{schema_version:1,persona_tzid:frozen.persona_tzid.clone(),tzdb_release:frozen.tzdb_release.clone(),tzdb_content_sha256:frozen.tzdb_content_sha256,circadian_period_millis:86400000,homeostatic_awake_gain_per_hour:Fixed::from_raw(40000),homeostatic_asleep_decay_per_hour:Fixed::from_raw(100000),drowsy_enter_threshold:Fixed::from_raw(750000),drowsy_exit_threshold:Fixed::from_raw(500000),endogenous_phase_hysteresis:Fixed::from_raw(50000),maximum_analytic_horizon_ms:EMBODIMENT_MAX_HORIZON_MS},
        sleep_schedule:EmbodimentSleepScheduleV1{schema_version:1,mode:SleepScheduleModeV1::Auto,chronotype:ChronotypeV1::Intermediate,preferred_sleep_local_minute:1380,preferred_wake_local_minute:420,sleep_flex_minutes:60,entrainment_rate_minutes_per_day:15},frozen_creation:frozen};
    let first=runtime.create_embodiment_persona_if_missing_v1(&request).unwrap();assert_eq!(first.commit_status,CoreCommitStatusV1::Committed);assert_eq!(first.first_creation_receipt.initial_semantic_revision,0);
    let before=runtime.get_embodiment_persona_v1(&scope).unwrap();drop(runtime);
    let mut runtime=AstrRuntime::open(&path).unwrap();request.frozen_creation.now_utc_ms+=86400000;request.profile_template.persona_tzid="invalid startup replacement".into();
    let repeated=runtime.create_embodiment_persona_if_missing_v1(&request).unwrap();assert_eq!(repeated.commit_status,CoreCommitStatusV1::Existing);assert_eq!(repeated.first_creation_receipt,first.first_creation_receipt);assert_eq!(runtime.get_embodiment_persona_v1(&scope).unwrap(),before);
    drop(runtime);let _=std::fs::remove_file(path);
}
