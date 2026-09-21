//! Current core replacements for the three retained legacy matrix runtime targets.
//! Numerical coefficients/chunking remain in ae-semantic-core/tests/matrix_time.rs.
use ae_contracts::*;
use ae_fixed::Fixed;
use ae_runtime::AstrRuntime;
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};

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
            traits: PersonalityVector {
                baseline_warmth: Fixed::from_raw(600_000),
                ..Default::default()
            },
            trait_confidence: PersonalityVector {
                baseline_warmth: Fixed::ONE,
                ..Default::default()
            },
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

fn fixture(label: &str) -> (PathBuf, AstrRuntime, PersonaScopeRef) {
    let path = std::env::temp_dir().join(format!(
        "ae-core-matrix-{label}-{}-{}.sqlite",
        std::process::id(),
        ae_store::now_ms()
    ));
    let mut runtime = AstrRuntime::open(&path).unwrap();
    let request = genesis();
    runtime.ensure_genesis(&request).unwrap();
    (path, runtime, request.source.scope)
}

fn inbound(scope: &PersonaScopeRef, tag: u8) -> CommitCoreInboundV1 {
    let p = core_persona_digest(scope);
    let turn_id = core_id(b"ae.core-inbound.turn-id.v1", &[&p, &[tag; 32]]);
    CommitCoreInboundV1 {
        schema_version: 1,
        observation: CoreInboundObservationV1 {
            schema_version: 1,
            operation_id: core_id(b"ae.core-inbound.operation-id.v1", &[&p, &turn_id]),
            scope: scope.clone(),
            turn_id,
            observed_at_utc_ms: ae_store::now_ms(),
            message_digest: [30; 32],
            astrbot_event_identity_digest: [tag; 32],
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
    }
}

fn stimulus(
    runtime: &mut AstrRuntime,
    scope: &PersonaScopeRef,
    tag: u8,
) -> (
    CommitCoreInboundV1,
    SemanticAppraisalSettleRequestV1,
    SemanticAppraisalSettleResultV1,
) {
    let request = inbound(scope, tag);
    let first = runtime.commit_core_inbound_v1(&request).unwrap();
    assert!(first.provider_authorized_now);
    let challenge = first.initial_receipt.challenge.unwrap();
    let settle = SemanticAppraisalSettleRequestV1 {
        schema_version: 1,
        scope: challenge.origin.scope,
        request_nonce_digest: challenge.request_nonce_digest,
        outcome: SemanticAppraisalOutcomeV1::Success,
        provider_usage: SemanticAppraisalProviderUsageV1 {
            known: true,
            used_tokens: Some(200),
        },
        proposal: Some(PerceptionProposalV1 {
            schema_version: 1,
            origin_digest: challenge.origin.origin_digest,
            dimensions: EvidenceVector {
                positive: Fixed::from_raw(800_000),
                affiliation: Fixed::from_raw(300_000),
                engagement: Fixed::from_raw(600_000),
                ..Default::default()
            },
            estimator_confidence: Fixed::ONE,
            protocol_version: 1,
            request_nonce_digest: challenge.request_nonce_digest,
        }),
    };
    let result = runtime.settle_semantic_appraisal_v1(&settle).unwrap();
    assert_eq!(result.status, SemanticAppraisalSettleStatusV1::Committed);
    (request, settle, result)
}

fn sidecar(path: &Path) -> (u64, Vec<u8>, Vec<u8>, Vec<u8>) {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap().query_row(
        "SELECT semantic_revision,state_digest,graph_digest,commitment_digest FROM semantic_cursor", [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    ).unwrap()
}

fn history_counts(path: &Path) -> (u64, u64, u64) {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap().query_row(
        "SELECT (SELECT COUNT(*) FROM journal),(SELECT COUNT(*) FROM semantic_commits),(SELECT COUNT(*) FROM interaction_fact)", [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).unwrap()
}

#[test]
fn nonzero_stimulus_delivery_and_reopen_preserve_matrix_lane() {
    let (path, mut runtime, scope) = fixture("event");
    let (request, _, first) = stimulus(&mut runtime, &scope, 41);
    assert_eq!(first.semantic_revision, Some(1));
    let matrix = sidecar(&path);
    let delivery = CommitCoreDeliveryOutcomeV1 {
        schema_version: 1,
        operation_id: core_id(
            b"ae.core-delivery.operation-id.v1",
            &[
                &core_persona_digest(&scope),
                &request.observation.turn_id,
                &request.observation.operation_id,
            ],
        ),
        scope: scope.clone(),
        turn_id: request.observation.turn_id,
        inbound_operation_id: request.observation.operation_id,
        delivered: true,
        observed_at_utc_ms: ae_store::now_ms(),
        visible_action_digest: [34; 32],
    };
    runtime.commit_core_delivery_outcome_v1(&delivery).unwrap();
    assert_eq!(sidecar(&path), matrix);
    assert!(
        !runtime
            .commit_core_inbound_v1(&request)
            .unwrap()
            .provider_authorized_now
    );
    let before = history_counts(&path);
    drop(runtime);
    let mut runtime = AstrRuntime::open(&path).unwrap();
    assert_eq!(sidecar(&path), matrix);
    assert_eq!(history_counts(&path), before);
    let (_, _, second) = stimulus(&mut runtime, &scope, 42);
    assert_eq!(second.semantic_revision, Some(2));
    assert_ne!(sidecar(&path).1, matrix.1);
    runtime.audit_semantic_integrity_v1().unwrap();
    drop(runtime);
    std::fs::remove_file(path).unwrap();
}

fn clock_head(runtime: &mut AstrRuntime, scope: &PersonaScopeRef) -> EmbodimentClockHeadV1 {
    match runtime.get_embodiment_persona_v1(scope).unwrap() {
        EmbodimentPersonaLookupV1::Present { clock_head, .. } => clock_head,
        _ => panic!("clock missing"),
    }
}

#[test]
fn persona_sleep_clock_projects_matrix_without_evidence_and_reopens() {
    let (path, mut runtime, scope) = fixture("time");
    stimulus(&mut runtime, &scope, 43);
    let incarnation = match runtime.get_embodiment_persona_v1(&scope).unwrap() {
        EmbodimentPersonaLookupV1::Missing { incarnation_digest } => incarnation_digest,
        _ => panic!("clock exists"),
    };
    // 2026-03-09 00:00 UTC: fixed sleep window, no mutable legacy sleep fixture.
    let now = 1773014400000;
    let frozen = freeze_embodiment_time_v1("UTC", now).unwrap();
    runtime
        .create_embodiment_persona_if_missing_v1(&CreateEmbodimentPersonaIfMissingV1 {
            schema_version: 1,
            operation_id: core_id(
                b"ae.embodiment.create.operation-id.v1",
                &[&core_persona_digest(&scope), &incarnation],
            ),
            scope: scope.clone(),
            incarnation_digest: incarnation,
            profile_template: EmbodimentTemporalProfileV1 {
                schema_version: 1,
                persona_tzid: "UTC".into(),
                tzdb_release: frozen.tzdb_release.clone(),
                tzdb_content_sha256: frozen.tzdb_content_sha256,
                circadian_period_millis: 86400000,
                homeostatic_awake_gain_per_hour: Fixed::from_raw(40000),
                homeostatic_asleep_decay_per_hour: Fixed::from_raw(100000),
                drowsy_enter_threshold: Fixed::from_raw(750000),
                drowsy_exit_threshold: Fixed::from_raw(500000),
                endogenous_phase_hysteresis: Fixed::from_raw(50000),
                maximum_analytic_horizon_ms: EMBODIMENT_MAX_HORIZON_MS,
            },
            sleep_schedule: EmbodimentSleepScheduleV1 {
                schema_version: 1,
                mode: SleepScheduleModeV1::Fixed,
                chronotype: ChronotypeV1::Intermediate,
                preferred_sleep_local_minute: 1380,
                preferred_wake_local_minute: 420,
                sleep_flex_minutes: 60,
                entrainment_rate_minutes_per_day: 15,
            },
            frozen_creation: frozen,
        })
        .unwrap();
    let before = clock_head(&mut runtime, &scope);
    assert_eq!(before.sleep_state, SleepStateV1::Asleep);
    let history = history_counts(&path);
    let semantic = sidecar(&path);
    let mut request = EmbodimentTimeAdvanceRequestV1 {
        schema_version: 1,
        operation_id: [0; 16],
        scope: scope.clone(),
        profile_revision: 1,
        schedule_revision: 1,
        frozen: freeze_embodiment_time_v1("UTC", now + 600000).unwrap(),
    };
    request.operation_id = request.expected_operation_id();
    let bytes = request.encode_wire_v1().unwrap();
    let result = runtime.advance_embodiment_time_v1(&bytes).unwrap();
    let after = clock_head(&mut runtime, &scope);
    assert!(after.matrix_epoch.asleep_ticks > before.matrix_epoch.asleep_ticks);
    assert_ne!(
        after.state.matrix_state_digest,
        before.state.matrix_state_digest
    );
    assert_eq!(
        after.matrix_anchor_graph_digest,
        before.matrix_anchor_graph_digest
    );
    assert_eq!(after.matrix_epoch.anchor_semantic_revision, 1);
    assert_eq!(sidecar(&path), semantic);
    assert_eq!(history_counts(&path), history);
    assert_eq!(
        runtime.advance_embodiment_time_v1(&bytes).unwrap().receipt,
        result.receipt
    );
    drop(runtime);
    let mut runtime = AstrRuntime::open(&path).unwrap();
    assert_eq!(clock_head(&mut runtime, &scope), after);
    assert_eq!(history_counts(&path), history);
    request.frozen = freeze_embodiment_time_v1("UTC", now - 1).unwrap();
    request.operation_id = request.expected_operation_id();
    assert!(runtime
        .advance_embodiment_time_v1(&request.encode_wire_v1().unwrap())
        .is_err());
    assert_eq!(clock_head(&mut runtime, &scope), after);
    drop(runtime);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn committed_persona_reply_affect_replays_and_corruption_is_read_only_rejected() {
    let (path, mut runtime, scope) = fixture("projection");
    let (_, settle, result) = stimulus(&mut runtime, &scope, 44);
    let affect = result
        .reply_affect
        .as_ref()
        .expect("committed persona affect");
    assert!(affect.validate_v1());
    assert_eq!(affect.semantic_revision, 1);
    assert_eq!(affect.region_mean_fxp6.len(), 9);
    assert!(affect.region_delta_fxp6.iter().any(|value| *value != 0));
    let inspection = runtime
        .inspect(&scope.bot_token, &scope.persona_token)
        .unwrap();
    let witness = sidecar(&path);
    let history = history_counts(&path);
    drop(runtime);
    let mut runtime = AstrRuntime::open(&path).unwrap();
    assert_eq!(
        runtime
            .settle_semantic_appraisal_v1(&settle)
            .unwrap()
            .reply_affect,
        result.reply_affect
    );
    assert_eq!(
        runtime
            .inspect(&scope.bot_token, &scope.persona_token)
            .unwrap()
            .revision,
        inspection.revision
    );
    assert_eq!(sidecar(&path), witness);
    assert_eq!(history_counts(&path), history);
    // Controlled fixture corruption; audit must reject without repair or history writes.
    let conn = Connection::open(&path).unwrap();
    conn.execute("UPDATE semantic_snapshots SET snapshot_bytes=x'00'", [])
        .unwrap();
    let damaged: Vec<u8> = conn
        .query_row("SELECT snapshot_bytes FROM semantic_snapshots", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(runtime.audit_semantic_integrity_v1().is_err());
    assert_eq!(
        conn.query_row("SELECT snapshot_bytes FROM semantic_snapshots", [], |r| r
            .get::<_, Vec<
            u8,
        >>(
            0
        ))
        .unwrap(),
        damaged
    );
    assert_eq!(history_counts(&path), history);
    drop(conn);
    drop(runtime);
    assert!(AstrRuntime::open(&path).is_err());
    std::fs::remove_file(path).unwrap();
}
