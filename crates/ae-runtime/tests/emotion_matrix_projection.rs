#![cfg(feature = "legacy-semantic-test-api")]

use ae_contracts::*;
use ae_fixed::Fixed;
use ae_runtime::{frozen_time_input_digest, AstrRuntime};
use ae_semantic_core::TIME_SNAPSHOT_WIRE_LEN_V1;
use rusqlite::{params, Connection};
use serde_json::Value;
use std::path::{Path, PathBuf};

const NOW: u64 = 1_700_000_000_000;

fn db(label: &str) -> PathBuf {
    let root = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .expect("task-local CARGO_TARGET_DIR");
    let path = root.join(format!(
        "ae-runtime-affect-{label}-{}-{:?}.db",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}

fn id(tag: u8, value: u64) -> Id128 {
    let mut id = [tag; 16];
    id[8..].copy_from_slice(&value.to_le_bytes());
    id
}

fn contains_json_key(value: &Value, key: &str) -> bool {
    match value {
        Value::Object(object) => {
            object.contains_key(key) || object.values().any(|nested| contains_json_key(nested, key))
        }
        Value::Array(values) => values.iter().any(|nested| contains_json_key(nested, key)),
        _ => false,
    }
}

fn genesis() -> PersonaGenesisRequest {
    let scope = PersonaScopeRef {
        bot_token: [0x21; 16],
        persona_token: [0x22; 16],
    };
    let source = PersonaSourceRef {
        scope,
        source_digest: [0x23; 32],
        capability_digest: [0x24; 32],
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
                baseline_warmth: Fixed::from_raw(650_000),
                sensitivity: Fixed::from_raw(350_000),
                curiosity: Fixed::from_raw(550_000),
                ..PersonalityVector::default()
            },
            trait_confidence: PersonalityVector::default(),
            expression: ExpressionPhenotype::default(),
            allostasis: AllostaticSetpoints::default(),
            epistemic: EpistemicPriors::default(),
            social: SocialPriors::default(),
            compiler_protocol_digest: [0x25; 32],
            compiler_model_digest: [0x26; 32],
        },
        formula_digest: [0x27; 32],
        incarnation_nonce: [0x28; 32],
        parent_incarnation_id: None,
        observed_at_ms: NOW,
    }
}

fn scope(request: &PersonaGenesisRequest, relation: Option<u8>) -> ScopeRef {
    ScopeRef {
        bot_token: request.source.scope.bot_token,
        persona_token: request.source.scope.persona_token,
        relation_token: relation.map(|value| [value; 16]),
        session_token: [relation.unwrap_or(0x30); 16],
    }
}

fn profile(persona_scope: Digest) -> PersonaTemporalProfileV1 {
    PersonaTemporalProfileV1 {
        schema_version: AUTONOMY_SCHEMA_VERSION,
        persona_scope,
        home_timezone: "UTC".into(),
        current_timezone: "UTC".into(),
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
        schema_version: AUTONOMY_SCHEMA_VERSION,
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
    }
}

struct Fixture {
    path: PathBuf,
    runtime: AstrRuntime,
    persona: ScopeRef,
    relation_a: ScopeRef,
    persona_digest: Digest,
    relation_a_digest: Digest,
    relation_b_digest: Digest,
    state: AutonomousRuntimeStateV1,
}

fn fixture(label: &str) -> Fixture {
    let path = db(label);
    let request = genesis();
    let persona = scope(&request, None);
    let relation_a = scope(&request, Some(0x41));
    let relation_b = scope(&request, Some(0x42));
    let persona_digest =
        wire::persona_scope_digest(&persona.bot_token, &persona.persona_token, None);
    let relation_a_digest = wire::persona_scope_digest(
        &persona.bot_token,
        &persona.persona_token,
        relation_a.relation_token.as_ref(),
    );
    let relation_b_digest = wire::persona_scope_digest(
        &persona.bot_token,
        &persona.persona_token,
        relation_b.relation_token.as_ref(),
    );
    let mut runtime = AstrRuntime::open(&path).unwrap();
    runtime.ensure_genesis(&request).unwrap();
    let state = runtime
        .bootstrap_autonomy(&persona, &profile(persona_digest), None)
        .unwrap();
    runtime
        .bootstrap_autonomy(
            &relation_a,
            &profile(persona_digest),
            Some(&policy(relation_a_digest)),
        )
        .unwrap();
    runtime
        .bootstrap_autonomy(
            &relation_b,
            &profile(persona_digest),
            Some(&policy(relation_b_digest)),
        )
        .unwrap();
    let mut fixture = Fixture {
        path,
        runtime,
        persona,
        relation_a,
        persona_digest,
        relation_a_digest,
        relation_b_digest,
        state,
    };
    advance_time(&mut fixture, 0, 0);
    fixture
}

fn commit_perception(fixture: &mut Fixture, number: u8) {
    let base_revision = fixture
        .runtime
        .current_revision(&fixture.relation_a)
        .unwrap();
    let inbound = fixture
        .runtime
        .apply_interaction_fact_batch_v1(&InteractionFactBatchV1 {
            schema_version: ALPHA3_SCHEMA_VERSION,
            event_id: id(0x50, u64::from(number)),
            scope: fixture.relation_a.clone(),
            causal: CausalRef {
                turn_id: id(0x51, u64::from(number)),
                action_id: None,
                delivery_id: None,
                claim_id: None,
                base_revision,
            },
            facts: vec![InteractionFactV1 {
                fact_id: id(0x52, u64::from(number)),
                kind: InteractionFactKindV1::InboundObserved,
                observed_at_utc_ms: NOW + u64::from(number),
                source_authority: InteractionSourceAuthorityV1::AstrbotMetadata,
                source_digest: [number.wrapping_add(1); 32],
                extractor_digest: [number.wrapping_add(2); 32],
                confidence: Fixed::ONE,
                value_code: None,
                subject_public_ref: None,
                consent_terms: None,
                scheduled_at_utc_ms: None,
                expires_at_utc_ms: None,
            }],
        })
        .unwrap();
    let challenge = fixture
        .runtime
        .mint_perception_challenge_from_committed_inbound_v1(inbound.receipt.event_digest)
        .unwrap();
    fixture
        .runtime
        .apply_perception_proposal_v1(
            &fixture.relation_a,
            &PerceptionProposalV1 {
                schema_version: PerceptionProposalV1::SCHEMA_VERSION,
                origin_digest: challenge.origin.origin_digest,
                dimensions: EvidenceVector {
                    positive: Fixed::from_raw(700_000),
                    affiliation: Fixed::from_raw(400_000),
                    engagement: Fixed::from_raw(800_000),
                    ..EvidenceVector::default()
                },
                estimator_confidence: Fixed::from_raw(850_000),
                protocol_version: PerceptionProposalV1::PROTOCOL_VERSION,
                request_nonce_digest: challenge.request_nonce_digest,
            },
        )
        .unwrap();
}

fn observe(
    fixture: &Fixture,
    layer: ObserveLayerV2,
    relation_scope: Option<Digest>,
) -> ObserveProjectionV2 {
    fixture
        .runtime
        .observe_snapshot_v2(&ObserveSnapshotRequestV2 {
            schema_version: ALPHA3_SCHEMA_VERSION,
            persona_scope: fixture.persona_digest,
            relation_scope,
            committed_only: true,
            layer,
        })
        .unwrap()
        .projection
}

fn experience(fixture: &Fixture, relation_scope: Option<Digest>) -> ExperienceProjectionV2 {
    let ObserveProjectionV2::Experience(value) =
        observe(fixture, ObserveLayerV2::Experience, relation_scope)
    else {
        panic!("experience projection")
    };
    value
}

fn force_sleep(path: &Path, mut state: AutonomousRuntimeStateV1) {
    state.sleep_state = SleepStateV1::Asleep;
    state.last_advanced_at_utc_ms = NOW;
    state.next_wake_at_utc_ms = NOW + 600_000;
    let conn = Connection::open(path).unwrap();
    assert_eq!(
        conn.execute(
            "UPDATE autonomous_runtime_state SET body_json=?2
             WHERE persona_scope=?1 AND generation=?3 AND state_revision=?4",
            params![
                state.persona_scope.to_vec(),
                serde_json::to_string(&state).unwrap(),
                state.generation as i64,
                state.state_revision as i64,
            ],
        )
        .unwrap(),
        1
    );
}

fn advance_time(fixture: &mut Fixture, offset_ms: u64, event_number: u64) {
    let effective_now = NOW + offset_ms;
    let frozen = FrozenTimeInputV1 {
        schema_version: AUTONOMY_SCHEMA_VERSION,
        observed_now_utc_ms: effective_now,
        effective_now_utc_ms: effective_now,
        persona_tzid: "UTC".into(),
        persona_utc_offset_seconds: 0,
        persona_local_minute: (offset_ms / 60_000) as u16,
        persona_day_ordinal: 739_854,
        relation_tzid: "UTC".into(),
        relation_utc_offset_seconds: 0,
        relation_local_minute: (offset_ms / 60_000) as u16,
        relation_day_ordinal: 739_854,
        budget_day_start_utc_ms: NOW - NOW % 86_400_000,
        budget_next_day_start_utc_ms: NOW - NOW % 86_400_000 + 86_400_000,
        next_timezone_transition_utc_ms: None,
        tzdb_fingerprint: [0x60; 32],
    };
    let event = TimeAdvanceV1 {
        event_id: id(0x61, event_number),
        scope: fixture.persona.clone(),
        expected_generation: fixture.state.generation,
        frozen_input_digest: frozen_time_input_digest(&frozen),
        frozen,
        stimulus: AutonomousStimulusV1 {
            arousal: Fixed::ZERO,
            urgency: Fixed::ZERO,
            emergency_authorized: false,
            source_digest: [0x62u8.wrapping_add(event_number as u8); 32],
        },
    };
    let claim = fixture
        .runtime
        .claim_wake(&fixture.persona, &event)
        .unwrap();
    fixture.state = fixture.runtime.settle_wake(&claim.claim_token).unwrap();
}

fn database_witness(path: &Path, persona: Digest) -> (u64, u64, Vec<u8>, Vec<u8>, Vec<i64>) {
    let conn = Connection::open(path).unwrap();
    let canonical = conn
        .query_row(
            "SELECT COALESCE(MAX(logical_revision),0) FROM journal WHERE scope_digest=?1",
            params![persona.to_vec()],
            |row| row.get::<_, i64>(0),
        )
        .unwrap() as u64;
    let (semantic, state, graph) = conn
        .query_row(
            "SELECT semantic_revision,state_digest,graph_digest FROM semantic_cursor WHERE persona_scope=?1",
            params![persona.to_vec()],
            |row| Ok((row.get::<_, i64>(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    let counts = [
        "semantic_commits",
        "semantic_snapshots",
        "semantic_graphs",
        "semantic_receipts",
        "semantic_telemetry",
        "semantic_evidence_authority",
        "semantic_time_authority",
    ]
    .into_iter()
    .map(|table| {
        conn.query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE persona_scope=?1"),
            params![persona.to_vec()],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
    })
    .collect();
    (semantic as u64, canonical, state, graph, counts)
}

#[test]
fn committed_affect_projection_is_persona_global_and_non_identifying() {
    let mut fixture = fixture("global");
    commit_perception(&mut fixture, 1);
    let before_fingerprint = fixture
        .runtime
        .alpha3_authoritative_fingerprint_v1(&fixture.persona_digest)
        .unwrap();
    let before_rows = database_witness(&fixture.path, fixture.persona_digest);

    let global = experience(&fixture, None).affect.value().clone();
    let relation_a = experience(&fixture, Some(fixture.relation_a_digest))
        .affect
        .value()
        .clone();
    let relation_b = experience(&fixture, Some(fixture.relation_b_digest))
        .affect
        .value()
        .clone();
    assert_eq!(global, relation_a);
    assert_eq!(global, relation_b);
    assert_eq!(global.semantic_revision, 2);
    assert_eq!(global.confidence_fxp6, 850_000);
    assert_eq!(global.region_mean_fxp6.len(), 9);
    assert_eq!(global.region_delta_fxp6.len(), 9);

    let value = serde_json::to_value(&global).unwrap();
    let bytes = serde_json::to_vec(&value).unwrap();
    let json = String::from_utf8(bytes).unwrap();
    for forbidden in [
        hex::encode32(&fixture.relation_a_digest),
        hex::encode32(&fixture.relation_b_digest),
        hex::encode16(&fixture.relation_a.session_token),
        hex::encode16(&id(0x50, 1)),
    ] {
        assert!(
            !json.contains(&forbidden),
            "affect leaked identity {forbidden}"
        );
    }
    for forbidden_key in [
        "relation_scope",
        "session_token",
        "event_id",
        "evidence",
        "source_text",
        "dimensions",
        "node_vector",
    ] {
        assert!(
            !contains_json_key(&value, forbidden_key),
            "affect leaked key {forbidden_key}"
        );
    }

    assert_eq!(
        fixture
            .runtime
            .alpha3_authoritative_fingerprint_v1(&fixture.persona_digest)
            .unwrap(),
        before_fingerprint
    );
    assert_eq!(
        database_witness(&fixture.path, fixture.persona_digest),
        before_rows
    );
}

#[test]
fn compact_time_head_projects_attested_previous_to_current_delta() {
    let mut fixture = fixture("time");
    commit_perception(&mut fixture, 1);
    let before = experience(&fixture, None).affect.value().clone();
    force_sleep(&fixture.path, fixture.state.clone());
    advance_time(&mut fixture, 600_000, 1);
    let snapshot_len: i64 = Connection::open(&fixture.path)
        .unwrap()
        .query_row(
            "SELECT length(snapshot_bytes) FROM semantic_snapshots
             WHERE persona_scope=?1 ORDER BY semantic_revision DESC LIMIT 1",
            params![fixture.persona_digest.to_vec()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(snapshot_len as usize, TIME_SNAPSHOT_WIRE_LEN_V1);

    let before_fingerprint = fixture
        .runtime
        .alpha3_authoritative_fingerprint_v1(&fixture.persona_digest)
        .unwrap();
    let before_rows = database_witness(&fixture.path, fixture.persona_digest);
    let after = experience(&fixture, None).affect.value().clone();
    assert_eq!(after.semantic_revision, before.semantic_revision + 1);
    assert_eq!(after.confidence_fxp6, before.confidence_fxp6);
    assert_eq!(after.personality_revision, before.personality_revision);
    for region in 0..9 {
        assert!(
            after.region_delta_fxp6[region]
                .abs_diff(after.region_mean_fxp6[region] - before.region_mean_fxp6[region])
                <= 1
        );
    }
    assert!(after.region_delta_fxp6.iter().any(|delta| *delta != 0));
    assert_ne!(after.state_digest, before.state_digest);
    assert_eq!(
        fixture
            .runtime
            .alpha3_authoritative_fingerprint_v1(&fixture.persona_digest)
            .unwrap(),
        before_fingerprint
    );
    assert_eq!(
        database_witness(&fixture.path, fixture.persona_digest),
        before_rows
    );

    let conn = Connection::open(&fixture.path).unwrap();
    let (time_revision, saved_authority_digest): (i64, Vec<u8>) = conn
        .query_row(
            "SELECT a.semantic_revision,a.authority_digest
             FROM semantic_time_authority AS a
             JOIN semantic_cursor AS c
               ON c.persona_scope=a.persona_scope
              AND c.semantic_revision=a.semantic_revision
             WHERE a.persona_scope=?1",
            params![fixture.persona_digest.to_vec()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    conn.execute_batch("PRAGMA ignore_check_constraints=ON;")
        .unwrap();
    assert_eq!(
        conn.execute(
            "UPDATE semantic_time_authority
             SET authority_digest=zeroblob(1048577)
             WHERE persona_scope=?1 AND semantic_revision=(
                 SELECT semantic_revision FROM semantic_cursor WHERE persona_scope=?1
             )",
            params![fixture.persona_digest.to_vec()],
        )
        .unwrap(),
        1
    );
    conn.execute_batch("PRAGMA ignore_check_constraints=OFF;")
        .unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT length(authority_digest) FROM semantic_time_authority
             WHERE persona_scope=?1 AND semantic_revision=(
                 SELECT semantic_revision FROM semantic_cursor WHERE persona_scope=?1
             )",
            params![fixture.persona_digest.to_vec()],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        1_048_577
    );
    drop(conn);
    let corrupt_fingerprint = fixture
        .runtime
        .alpha3_authoritative_fingerprint_v1(&fixture.persona_digest)
        .unwrap();
    let corrupt_rows = database_witness(&fixture.path, fixture.persona_digest);
    assert_eq!(
        experience(&fixture, None).affect,
        ProjectionFieldV1::Inconsistent
    );
    assert_eq!(
        fixture
            .runtime
            .alpha3_authoritative_fingerprint_v1(&fixture.persona_digest)
            .unwrap(),
        corrupt_fingerprint
    );
    assert_eq!(
        database_witness(&fixture.path, fixture.persona_digest),
        corrupt_rows
    );

    let conn = Connection::open(&fixture.path).unwrap();
    assert_eq!(
        conn.execute(
            "UPDATE semantic_time_authority SET authority_digest=?3
             WHERE persona_scope=?1 AND semantic_revision=?2",
            params![
                fixture.persona_digest.to_vec(),
                time_revision,
                saved_authority_digest,
            ],
        )
        .unwrap(),
        1
    );
    drop(conn);
    commit_perception(&mut fixture, 2);
    assert!(experience(&fixture, None).affect.available().is_some());
}

#[test]
fn uncommitted_corrupt_or_renorm_failed_projection_is_unavailable_and_read_only() {
    let mut fixture = fixture("corrupt");
    commit_perception(&mut fixture, 1);
    let conn = Connection::open(&fixture.path).unwrap();
    let (revision, mut original): (i64, Vec<u8>) = conn
        .query_row(
            "SELECT c.semantic_revision,s.snapshot_bytes
             FROM semantic_cursor c JOIN semantic_snapshots s
               ON s.persona_scope=c.persona_scope AND s.semantic_revision=c.semantic_revision
             WHERE c.persona_scope=?1",
            params![fixture.persona_digest.to_vec()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let saved = original.clone();
    original[0] ^= 0xff;
    assert_eq!(
        conn.execute(
            "UPDATE semantic_snapshots SET snapshot_bytes=?3
             WHERE persona_scope=?1 AND semantic_revision=?2",
            params![fixture.persona_digest.to_vec(), revision, original],
        )
        .unwrap(),
        1
    );
    drop(conn);
    let before_fingerprint = fixture
        .runtime
        .alpha3_authoritative_fingerprint_v1(&fixture.persona_digest)
        .unwrap();
    let before_rows = database_witness(&fixture.path, fixture.persona_digest);
    let projected = experience(&fixture, None);
    assert_eq!(projected.affect, ProjectionFieldV1::Inconsistent);
    assert_eq!(
        fixture
            .runtime
            .alpha3_authoritative_fingerprint_v1(&fixture.persona_digest)
            .unwrap(),
        before_fingerprint
    );
    assert_eq!(
        database_witness(&fixture.path, fixture.persona_digest),
        before_rows
    );

    let conn = Connection::open(&fixture.path).unwrap();
    assert_eq!(
        conn.execute(
            "UPDATE semantic_snapshots SET snapshot_bytes=?3
             WHERE persona_scope=?1 AND semantic_revision=?2",
            params![fixture.persona_digest.to_vec(), revision, saved],
        )
        .unwrap(),
        1
    );
    assert_eq!(
        conn.execute(
            "UPDATE semantic_evidence_authority SET
               perception_nonce_digest=NULL,perception_proposal_digest=NULL,
               perception_origin_digest=NULL,perception_origin_bytes=NULL
             WHERE persona_scope=?1 AND semantic_revision=?2",
            params![fixture.persona_digest.to_vec(), revision],
        )
        .unwrap(),
        1
    );
    drop(conn);
    assert_eq!(experience(&fixture, None).affect.value().confidence_fxp6, 0);
    commit_perception(&mut fixture, 2);
    assert_eq!(
        experience(&fixture, None).affect.value().confidence_fxp6,
        850_000
    );

    let conn = Connection::open(&fixture.path).unwrap();
    let saved_source_digest: Vec<u8> = conn
        .query_row(
            "SELECT source_digest FROM interaction_fact WHERE fact_id=?1",
            params![id(0x52, 2).to_vec()],
            |row| row.get(0),
        )
        .unwrap();
    conn.execute_batch("PRAGMA ignore_check_constraints=ON;")
        .unwrap();
    assert_eq!(
        conn.execute(
            "UPDATE interaction_fact SET source_digest=zeroblob(1048577) WHERE fact_id=?1",
            params![id(0x52, 2).to_vec()],
        )
        .unwrap(),
        1
    );
    conn.execute_batch("PRAGMA ignore_check_constraints=OFF;")
        .unwrap();
    drop(conn);
    let fixed_blob_fingerprint = fixture
        .runtime
        .alpha3_authoritative_fingerprint_v1(&fixture.persona_digest)
        .unwrap();
    let fixed_blob_rows = database_witness(&fixture.path, fixture.persona_digest);
    assert_eq!(
        experience(&fixture, None).affect,
        ProjectionFieldV1::Inconsistent
    );
    assert_eq!(
        fixture
            .runtime
            .alpha3_authoritative_fingerprint_v1(&fixture.persona_digest)
            .unwrap(),
        fixed_blob_fingerprint
    );
    assert_eq!(
        database_witness(&fixture.path, fixture.persona_digest),
        fixed_blob_rows
    );

    let conn = Connection::open(&fixture.path).unwrap();
    assert_eq!(
        conn.execute(
            "UPDATE interaction_fact SET source_digest=?2 WHERE fact_id=?1",
            params![id(0x52, 2).to_vec(), saved_source_digest],
        )
        .unwrap(),
        1
    );
    drop(conn);
    commit_perception(&mut fixture, 3);
    assert_eq!(
        experience(&fixture, None).affect.value().confidence_fxp6,
        850_000
    );
}

#[test]
fn projection_layers_remain_relation_private() {
    let mut fixture = fixture("layers");
    commit_perception(&mut fixture, 1);
    let experience = experience(&fixture, Some(fixture.relation_a_digest));
    let ObserveProjectionV2::Private(private) = observe(
        &fixture,
        ObserveLayerV2::Private,
        Some(fixture.relation_a_digest),
    ) else {
        panic!("private projection")
    };
    let ObserveProjectionV2::Developer(developer) = observe(
        &fixture,
        ObserveLayerV2::Developer,
        Some(fixture.relation_a_digest),
    ) else {
        panic!("developer projection")
    };
    assert!(experience.affect.available().is_some());
    let private_value = serde_json::to_value(&private).unwrap();
    let mut private_top_level = private_value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    private_top_level.sort_unstable();
    assert_eq!(
        private_top_level,
        [
            "budget",
            "consent",
            "contact",
            "dreams",
            "privacy_controls",
            "why_contacted",
        ]
    );
    for forbidden in [
        "affect",
        "semantic_revision",
        "state_digest",
        "renorm_mapping_digest",
        "graph_digest",
        "evidence",
    ] {
        assert!(
            !contains_json_key(&private_value, forbidden),
            "private leaked key {forbidden}"
        );
    }

    let health = developer.semantic_health.value();
    assert_eq!(health.semantic_revision, 2);
    assert!(health.active_node_count <= 16_384);
    assert!(health.active_edge_count <= 524_288);
    assert_ne!(health.graph_digest, [0; 32]);
    assert_ne!(health.renorm_mapping_digest, [0; 32]);
    let developer_value = serde_json::to_value(&developer).unwrap();
    for forbidden in [
        "region_mean_fxp6",
        "region_delta_fxp6",
        "confidence_fxp6",
        "event_id",
        "evidence",
        "dimensions",
    ] {
        assert!(
            !contains_json_key(&developer_value, forbidden),
            "developer leaked key {forbidden}"
        );
    }
}
