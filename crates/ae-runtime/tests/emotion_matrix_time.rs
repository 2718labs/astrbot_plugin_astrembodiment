#![cfg(feature = "legacy-semantic-test-api")]

use ae_contracts::*;
use ae_fixed::Fixed;
use ae_neurofield::{graph_digest, state_digest, NeuralField, SparseGraph};
use ae_runtime::{AstrRuntime, RuntimeError};
use ae_semantic_core::{
    decode_legacy_time_snapshot_v1, encode_canonical_graph_v1, MATRIX_TIME_MAX_ELAPSED_MS,
    TIME_SNAPSHOT_WIRE_LEN_V1,
};
use ae_store::Store;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

fn db(label: &str) -> PathBuf {
    let root = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .expect("task-local CARGO_TARGET_DIR");
    let path = root.join(format!(
        "ae-runtime-time-{label}-{}-{:?}.db",
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

fn scoped_id(tag: u8, seed: u8, value: u64) -> Id128 {
    id(tag, (u64::from(seed) << 56) | value)
}

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
                baseline_warmth: Fixed::from_raw(650_000),
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

fn persona_scope(request: &PersonaGenesisRequest) -> ScopeRef {
    ScopeRef {
        bot_token: request.source.scope.bot_token,
        persona_token: request.source.scope.persona_token,
        relation_token: None,
        session_token: [0x31; 16],
    }
}

fn relation_scope(request: &PersonaGenesisRequest) -> ScopeRef {
    ScopeRef {
        bot_token: request.source.scope.bot_token,
        persona_token: request.source.scope.persona_token,
        relation_token: Some([request.source.scope.bot_token[0].wrapping_add(20); 16]),
        session_token: [0x32; 16],
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

fn bootstrap(
    runtime: &mut AstrRuntime,
    request: &PersonaGenesisRequest,
) -> AutonomousRuntimeStateV1 {
    runtime.ensure_genesis(request).unwrap();
    let scope = relation_scope(request);
    let persona = wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
    let relation = wire::persona_scope_digest(
        &scope.bot_token,
        &scope.persona_token,
        scope.relation_token.as_ref(),
    );
    runtime
        .bootstrap_autonomy(&scope, &profile(persona), Some(&policy(relation)))
        .unwrap()
}

fn frozen(now: u64) -> FrozenTimeInputV1 {
    let day_start = now - now % 86_400_000;
    FrozenTimeInputV1 {
        schema_version: AUTONOMY_SCHEMA_VERSION,
        observed_now_utc_ms: now,
        effective_now_utc_ms: now,
        persona_tzid: "UTC".into(),
        persona_utc_offset_seconds: 0,
        persona_local_minute: ((now / 60_000) % 1_440) as u16,
        persona_day_ordinal: 739_854 + i32::try_from(now / 86_400_000).unwrap(),
        relation_tzid: "UTC".into(),
        relation_utc_offset_seconds: 0,
        relation_local_minute: ((now / 60_000) % 1_440) as u16,
        relation_day_ordinal: 739_854 + i32::try_from(now / 86_400_000).unwrap(),
        budget_day_start_utc_ms: day_start,
        budget_next_day_start_utc_ms: day_start + 86_400_000,
        next_timezone_transition_utc_ms: None,
        tzdb_fingerprint: [0x44; 32],
    }
}

fn wake(
    runtime: &mut AstrRuntime,
    scope: &ScopeRef,
    generation: u64,
    event_number: u64,
    now: u64,
    stimulus: AutonomousStimulusV1,
) -> Result<(Digest, AutonomousRuntimeStateV1), RuntimeError> {
    let frozen = frozen(now);
    let event = TimeAdvanceV1 {
        event_id: scoped_id(0x70, scope.bot_token[0], event_number),
        scope: scope.clone(),
        expected_generation: generation,
        frozen_input_digest: ae_runtime::frozen_time_input_digest(&frozen),
        frozen,
        stimulus,
    };
    let claim = runtime.claim_wake(scope, &event)?;
    let token = claim.claim_token;
    let state = runtime.settle_wake(&token)?;
    Ok((token, state))
}

fn perception(
    runtime: &mut AstrRuntime,
    request: &PersonaGenesisRequest,
    number: u8,
) -> TransitionReceipt {
    let scope = relation_scope(request);
    let seed = request.source.scope.bot_token[0];
    let base = runtime.current_revision(&scope).unwrap();
    let batch = InteractionFactBatchV1 {
        schema_version: ALPHA3_SCHEMA_VERSION,
        event_id: scoped_id(0x50, seed, u64::from(number)),
        scope: scope.clone(),
        causal: CausalRef {
            turn_id: scoped_id(0x51, seed, u64::from(number)),
            action_id: None,
            delivery_id: None,
            claim_id: None,
            base_revision: base,
        },
        facts: vec![InteractionFactV1 {
            fact_id: scoped_id(0x52, seed, u64::from(number)),
            kind: InteractionFactKindV1::InboundObserved,
            observed_at_utc_ms: 1_700_000_000_000 + u64::from(number),
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
    };
    let inbound = runtime.apply_interaction_fact_batch_v1(&batch).unwrap();
    let challenge = runtime
        .mint_perception_challenge_from_committed_inbound_v1(inbound.receipt.event_digest)
        .unwrap();
    runtime
        .apply_perception_proposal_v1(
            &scope,
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
        .unwrap()
        .receipt
}

fn force_sleep(path: &Path, mut state: AutonomousRuntimeStateV1, now: u64) {
    state.sleep_state = SleepStateV1::Asleep;
    state.last_advanced_at_utc_ms = now;
    state.next_wake_at_utc_ms = now + 600_000;
    let body = serde_json::to_string(&state).unwrap();
    let conn = Connection::open(path).unwrap();
    assert_eq!(
        conn.execute(
            "UPDATE autonomous_runtime_state SET body_json=?2
             WHERE persona_scope=?1 AND generation=?3 AND state_revision=?4",
            params![
                state.persona_scope.to_vec(),
                body,
                state.generation as i64,
                state.state_revision as i64,
            ],
        )
        .unwrap(),
        1
    );
}

fn cursor(path: &Path, persona: Digest) -> (u64, Digest, Digest, String, u64, u64, u64) {
    Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT semantic_revision,state_digest,graph_digest,transition_kind,
                    time_awake_ticks,time_drowsy_ticks,time_asleep_ticks
             FROM semantic_cursor WHERE persona_scope=?1",
            params![persona.to_vec()],
            |row| {
                let state: Vec<u8> = row.get(1)?;
                let graph: Vec<u8> = row.get(2)?;
                Ok((
                    row.get::<_, i64>(0)? as u64,
                    state.try_into().unwrap(),
                    graph.try_into().unwrap(),
                    row.get(3)?,
                    row.get::<_, i64>(4)? as u64,
                    row.get::<_, i64>(5)? as u64,
                    row.get::<_, i64>(6)? as u64,
                ))
            },
        )
        .unwrap()
}

fn encode_legacy_v4_time_snapshot(
    semantic_formula_digest: Digest,
    time_formula_digest: Digest,
    field: &NeuralField,
    graph: &SparseGraph,
) -> (Vec<u8>, Vec<u8>) {
    let mut field_bytes = Vec::new();
    for values in [
        &field.potential,
        &field.excitation,
        &field.inhibition,
        &field.adaptation,
        &field.precision,
        &field.prediction_error,
        &field.eligibility,
        &field.metabolic_reserve,
    ] {
        field_bytes.extend_from_slice(&(values.len() as u32).to_le_bytes());
        for value in values {
            field_bytes.extend_from_slice(&value.encode());
        }
    }
    let graph_bytes = encode_canonical_graph_v1(graph).unwrap();
    let mut snapshot = Vec::new();
    snapshot.extend_from_slice(b"AESET1\0");
    snapshot.extend_from_slice(&1_u16.to_le_bytes());
    snapshot.extend_from_slice(&semantic_formula_digest);
    snapshot.extend_from_slice(&time_formula_digest);
    snapshot.extend_from_slice(&(field_bytes.len() as u32).to_le_bytes());
    snapshot.extend_from_slice(&field_bytes);
    snapshot.extend_from_slice(&(graph_bytes.len() as u32).to_le_bytes());
    snapshot.extend_from_slice(&graph_bytes);
    snapshot.extend_from_slice(&state_digest(field, &semantic_formula_digest));
    snapshot.extend_from_slice(&graph_digest(graph));
    decode_legacy_time_snapshot_v1(&snapshot).unwrap();
    (snapshot, graph_bytes)
}

#[derive(Clone)]
struct PreparedV4TimeRow {
    persona_scope: Digest,
    semantic_revision: u64,
    old_commitment: Digest,
    new_commitment: Digest,
    old_snapshot_len: usize,
    old_budget_bytes: u64,
}

fn as_digest(value: Vec<u8>) -> Digest {
    value.try_into().unwrap()
}

fn as_id(value: Vec<u8>) -> Id128 {
    value.try_into().unwrap()
}

fn prepare_authenticated_v4_time_database(path: &Path, seed: u8) -> PreparedV4TimeRow {
    let request = genesis(seed);
    let scope = persona_scope(&request);
    let persona = wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
    let base = 1_700_200_000_000;
    let mut runtime = AstrRuntime::open(path).unwrap();
    let initial = bootstrap(&mut runtime, &request);
    force_sleep(path, initial.clone(), base);
    wake(
        &mut runtime,
        &scope,
        initial.generation,
        1,
        base + 600_000,
        AutonomousStimulusV1::default(),
    )
    .unwrap();
    runtime.audit_semantic_integrity_v1().unwrap();
    drop(runtime);

    let store = Store::open(path).unwrap();
    let hydrated = store
        .hydrated_semantic_state_v1(&request.source.scope)
        .unwrap()
        .unwrap();
    drop(store);

    struct Raw {
        semantic_revision: i64,
        journal_revision: i64,
        event_id: Vec<u8>,
        event_digest: Vec<u8>,
        incarnation_id: Vec<u8>,
        manifest_digest: Vec<u8>,
        route_digest: Vec<u8>,
        formula_digest: Vec<u8>,
        state_before: Vec<u8>,
        state_after: Vec<u8>,
        graph_digest: Vec<u8>,
        event_authority_digest: Vec<u8>,
        origin_digest: Vec<u8>,
        time_authority_digest: Vec<u8>,
        authority_bytes: Vec<u8>,
        new_commitment: Vec<u8>,
    }
    let mut conn = Connection::open(path).unwrap();
    let raw = conn
        .query_row(
            "SELECT c.semantic_revision,c.journal_revision,c.event_id,c.event_digest,
                    c.incarnation_id,c.manifest_digest,c.route_digest,c.formula_digest,
                    c.state_before,c.state_digest,c.graph_digest,c.authority_digest,
                    c.origin_digest,t.authority_digest,t.authority_bytes,c.commitment_digest
             FROM semantic_commits AS c
             JOIN semantic_time_authority AS t USING(persona_scope,semantic_revision)
             WHERE c.persona_scope=?1 AND c.transition_kind='time'
             ORDER BY c.semantic_revision DESC LIMIT 1",
            params![persona.to_vec()],
            |row| {
                Ok(Raw {
                    semantic_revision: row.get(0)?,
                    journal_revision: row.get(1)?,
                    event_id: row.get(2)?,
                    event_digest: row.get(3)?,
                    incarnation_id: row.get(4)?,
                    manifest_digest: row.get(5)?,
                    route_digest: row.get(6)?,
                    formula_digest: row.get(7)?,
                    state_before: row.get(8)?,
                    state_after: row.get(9)?,
                    graph_digest: row.get(10)?,
                    event_authority_digest: row.get(11)?,
                    origin_digest: row.get(12)?,
                    time_authority_digest: row.get(13)?,
                    authority_bytes: row.get(14)?,
                    new_commitment: row.get(15)?,
                })
            },
        )
        .unwrap();
    let semantic_revision = raw.semantic_revision as u64;
    let journal_revision = raw.journal_revision as u64;
    assert_eq!(semantic_revision, hydrated.semantic_revision);
    let formula_digest = as_digest(raw.formula_digest);
    let state_after = as_digest(raw.state_after);
    let stored_graph_digest = as_digest(raw.graph_digest);
    assert_eq!(formula_digest, hydrated.formula_digest);
    assert_eq!(state_after, hydrated.state_digest);
    assert_eq!(stored_graph_digest, hydrated.graph_digest);
    let authority: SemanticTimeAuthorityV1 = serde_json::from_slice(&raw.authority_bytes).unwrap();
    let (old_snapshot, old_graph) = encode_legacy_v4_time_snapshot(
        formula_digest,
        authority.time_formula_digest,
        &hydrated.field,
        &hydrated.graph,
    );
    let old_snapshot_digest = wire::domain_hash(
        b"astr-embodiment/semantic-snapshot-wire-v1",
        &[&old_snapshot],
    );
    let old_graph_digest =
        wire::domain_hash(b"astr-embodiment/semantic-graph-wire-v1", &[&old_graph]);
    let time_authority_digest = wire::domain_hash(
        b"astr-embodiment/semantic-time-authority-v1",
        &[&raw.authority_bytes],
    );
    assert_eq!(time_authority_digest, as_digest(raw.time_authority_digest));
    let event_id = as_id(raw.event_id);
    let event_digest = as_digest(raw.event_digest);
    let incarnation_id = as_digest(raw.incarnation_id);
    let manifest_digest = as_digest(raw.manifest_digest);
    let route_digest = as_digest(raw.route_digest);
    let state_before = as_digest(raw.state_before);
    let event_authority_digest = as_digest(raw.event_authority_digest);
    let origin_digest = as_digest(raw.origin_digest);
    let new_commitment = as_digest(raw.new_commitment);
    let old_commitment = wire::domain_hash(
        b"astr-embodiment/semantic-time-commit-v1",
        &[
            &persona,
            &semantic_revision.to_le_bytes(),
            &journal_revision.to_le_bytes(),
            &event_id,
            &event_digest,
            &incarnation_id,
            &manifest_digest,
            &route_digest,
            &formula_digest,
            &state_before,
            &state_after,
            &stored_graph_digest,
            &event_authority_digest,
            &old_snapshot_digest,
            &old_graph_digest,
            &origin_digest,
            &time_authority_digest,
        ],
    );
    assert_ne!(old_commitment, new_commitment);
    assert!(
        old_snapshot.len() <= 16 * 1024 * 1024,
        "legacy V4 snapshot fixture exceeds its schema bound: {} bytes",
        old_snapshot.len()
    );

    let tx = conn.transaction().unwrap();
    tx.execute_batch(
        "PRAGMA defer_foreign_keys=ON;
         DROP INDEX semantic_time_journal_v1;
         ALTER TABLE semantic_time_authority RENAME TO semantic_time_authority_v5_fixture;
         CREATE TABLE semantic_time_authority (
            persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
            semantic_revision INTEGER NOT NULL CHECK(typeof(semantic_revision)='integer' AND semantic_revision>0),
            journal_revision INTEGER NOT NULL UNIQUE CHECK(typeof(journal_revision)='integer' AND journal_revision>0),
            event_id BLOB NOT NULL CHECK(typeof(event_id)='blob' AND length(event_id)=16),
            event_digest BLOB NOT NULL CHECK(typeof(event_digest)='blob' AND length(event_digest)=32),
            authority_digest BLOB NOT NULL CHECK(typeof(authority_digest)='blob' AND length(authority_digest)=32),
            authority_bytes BLOB NOT NULL CHECK(typeof(authority_bytes)='blob' AND length(authority_bytes)<=16384),
            PRIMARY KEY(persona_scope,semantic_revision),
            UNIQUE(persona_scope,event_id),
            UNIQUE(persona_scope,event_digest),
            FOREIGN KEY(persona_scope,semantic_revision)
              REFERENCES semantic_commits(persona_scope,semantic_revision)
         );
         INSERT INTO semantic_time_authority
           SELECT * FROM semantic_time_authority_v5_fixture;
         DROP TABLE semantic_time_authority_v5_fixture;
         CREATE INDEX semantic_time_journal_v1
           ON semantic_time_authority(persona_scope,journal_revision);",
    )
    .unwrap();
    assert_eq!(
        tx.execute(
            "UPDATE semantic_commits
             SET commitment_digest=?3,snapshot_wire_digest=?4,graph_wire_digest=?5
             WHERE persona_scope=?1 AND semantic_revision=?2 AND commitment_digest=?6",
            params![
                persona.to_vec(),
                raw.semantic_revision,
                old_commitment.to_vec(),
                old_snapshot_digest.to_vec(),
                old_graph_digest.to_vec(),
                new_commitment.to_vec(),
            ],
        )
        .unwrap(),
        1
    );
    assert_eq!(
        tx.execute(
            "UPDATE semantic_snapshots
             SET commitment_digest=?3,snapshot_bytes=?4
             WHERE persona_scope=?1 AND semantic_revision=?2 AND commitment_digest=?5",
            params![
                persona.to_vec(),
                raw.semantic_revision,
                old_commitment.to_vec(),
                old_snapshot.clone(),
                new_commitment.to_vec(),
            ],
        )
        .unwrap(),
        1
    );
    assert_eq!(
        tx.execute(
            "INSERT INTO semantic_graphs(
               persona_scope,semantic_revision,incarnation_id,manifest_digest,route_digest,
               formula_digest,graph_digest,state_digest,evidence_digest,estimator_digest,
               event_digest,commitment_digest,graph_bytes)
             SELECT persona_scope,semantic_revision,incarnation_id,manifest_digest,route_digest,
                    formula_digest,graph_digest,state_digest,evidence_digest,estimator_digest,
                    event_digest,commitment_digest,?3
             FROM semantic_snapshots WHERE persona_scope=?1 AND semantic_revision=?2",
            params![persona.to_vec(), raw.semantic_revision, old_graph],
        )
        .unwrap(),
        1
    );
    assert_eq!(
        tx.execute(
            "UPDATE semantic_cursor SET commitment_digest=?3
             WHERE persona_scope=?1 AND semantic_revision=?2 AND commitment_digest=?4",
            params![
                persona.to_vec(),
                raw.semantic_revision,
                old_commitment.to_vec(),
                new_commitment.to_vec(),
            ],
        )
        .unwrap(),
        1
    );
    assert_eq!(
        tx.execute(
            "UPDATE semantic_budget_checkpoint SET head_commitment_digest=?3
             WHERE persona_scope=?1 AND head_revision=?2 AND head_commitment_digest=?4",
            params![
                persona.to_vec(),
                raw.semantic_revision,
                old_commitment.to_vec(),
                new_commitment.to_vec(),
            ],
        )
        .unwrap(),
        1
    );
    tx.execute_batch(
        "UPDATE semantic_budget_checkpoint SET aggregate_bytes=
           COALESCE((SELECT SUM(length(snapshot_bytes)) FROM semantic_snapshots WHERE persona_scope=semantic_budget_checkpoint.persona_scope),0)+
           COALESCE((SELECT SUM(length(graph_bytes)) FROM semantic_graphs WHERE persona_scope=semantic_budget_checkpoint.persona_scope),0)+
           COALESCE((SELECT SUM(length(receipt_bytes)) FROM semantic_receipts WHERE persona_scope=semantic_budget_checkpoint.persona_scope),0)+
           COALESCE((SELECT SUM(length(telemetry_bytes)) FROM semantic_telemetry WHERE persona_scope=semantic_budget_checkpoint.persona_scope),0)+
           COALESCE((SELECT SUM(length(evidence_bytes)) FROM semantic_evidence_authority WHERE persona_scope=semantic_budget_checkpoint.persona_scope),0)+
           COALESCE((SELECT SUM(length(authority_bytes)) FROM semantic_time_authority WHERE persona_scope=semantic_budget_checkpoint.persona_scope),0);
         UPDATE meta SET value=X'04' WHERE key='semantic_schema_version';",
    )
    .unwrap();
    let old_budget_bytes: i64 = tx
        .query_row(
            "SELECT aggregate_bytes FROM semantic_budget_checkpoint WHERE persona_scope=?1",
            params![persona.to_vec()],
            |row| row.get(0),
        )
        .unwrap();
    tx.commit().unwrap();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    PreparedV4TimeRow {
        persona_scope: persona,
        semantic_revision,
        old_commitment,
        new_commitment,
        old_snapshot_len: old_snapshot.len(),
        old_budget_bytes: old_budget_bytes as u64,
    }
}

#[test]
fn authenticated_v4_full_time_row_compacts_and_tamper_rolls_back() {
    let valid_path = db("v4-full-time-valid");
    let prepared = prepare_authenticated_v4_time_database(&valid_path, 0x19);
    assert!(prepared.old_snapshot_len > TIME_SNAPSHOT_WIRE_LEN_V1);

    let tampered_path = db("v4-full-time-tampered");
    std::fs::copy(&valid_path, &tampered_path).unwrap();
    let conn = Connection::open(&tampered_path).unwrap();
    assert_eq!(
        conn.execute(
            "UPDATE semantic_snapshots
             SET snapshot_bytes=CAST(X'00'||substr(snapshot_bytes,2) AS BLOB)
             WHERE persona_scope=?1 AND semantic_revision=?2",
            params![
                prepared.persona_scope.to_vec(),
                prepared.semantic_revision as i64
            ],
        )
        .unwrap(),
        1
    );
    let before_failed_open: (Vec<u8>, i64, i64, Vec<u8>) = conn
        .query_row(
            "SELECT c.commitment_digest,length(s.snapshot_bytes),
                    (SELECT COUNT(*) FROM semantic_graphs AS g
                     WHERE g.persona_scope=c.persona_scope
                       AND g.semantic_revision=c.semantic_revision),
                    (SELECT value FROM meta WHERE key='semantic_schema_version')
             FROM semantic_commits AS c
             JOIN semantic_snapshots AS s USING(persona_scope,semantic_revision)
             WHERE c.persona_scope=?1 AND c.semantic_revision=?2",
            params![
                prepared.persona_scope.to_vec(),
                prepared.semantic_revision as i64
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    drop(conn);
    assert!(Store::open(&tampered_path).is_err());
    let conn = Connection::open(&tampered_path).unwrap();
    let after_failed_open: (Vec<u8>, i64, i64, Vec<u8>) = conn
        .query_row(
            "SELECT c.commitment_digest,length(s.snapshot_bytes),
                    (SELECT COUNT(*) FROM semantic_graphs AS g
                     WHERE g.persona_scope=c.persona_scope
                       AND g.semantic_revision=c.semantic_revision),
                    (SELECT value FROM meta WHERE key='semantic_schema_version')
             FROM semantic_commits AS c
             JOIN semantic_snapshots AS s USING(persona_scope,semantic_revision)
             WHERE c.persona_scope=?1 AND c.semantic_revision=?2",
            params![
                prepared.persona_scope.to_vec(),
                prepared.semantic_revision as i64
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(after_failed_open, before_failed_open);
    assert_eq!(after_failed_open.0, prepared.old_commitment.to_vec());
    assert_eq!(after_failed_open.1 as usize, prepared.old_snapshot_len);
    assert_eq!(after_failed_open.2, 1);
    assert_eq!(after_failed_open.3, vec![4]);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_schema
             WHERE name='__ae_semantic_time_authority_v4'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        0
    );
    drop(conn);

    let mut migrated = Store::open(&valid_path).unwrap();
    let latest = migrated
        .latest_semantic_v1(&prepared.persona_scope)
        .unwrap()
        .unwrap();
    assert_eq!(latest.semantic_revision, prepared.semantic_revision);
    assert_eq!(latest.snapshot_bytes.len(), TIME_SNAPSHOT_WIRE_LEN_V1);
    assert_eq!(latest.commitment_digest, prepared.new_commitment);
    migrated.audit_semantic_integrity_v1().unwrap();
    drop(migrated);

    let conn = Connection::open(&valid_path).unwrap();
    let compact: (i64, i64, Vec<u8>, Vec<u8>, i64, i64) = conn
        .query_row(
            "SELECT length(s.snapshot_bytes),
                    (SELECT COUNT(*) FROM semantic_graphs AS g
                     WHERE g.persona_scope=c.persona_scope
                       AND g.semantic_revision=c.semantic_revision),
                    c.commitment_digest,cur.commitment_digest,
                    b.aggregate_bytes,
                    COALESCE((SELECT SUM(length(snapshot_bytes)) FROM semantic_snapshots WHERE persona_scope=c.persona_scope),0)+
                    COALESCE((SELECT SUM(length(graph_bytes)) FROM semantic_graphs WHERE persona_scope=c.persona_scope),0)+
                    COALESCE((SELECT SUM(length(receipt_bytes)) FROM semantic_receipts WHERE persona_scope=c.persona_scope),0)+
                    COALESCE((SELECT SUM(length(telemetry_bytes)) FROM semantic_telemetry WHERE persona_scope=c.persona_scope),0)+
                    COALESCE((SELECT SUM(length(evidence_bytes)) FROM semantic_evidence_authority WHERE persona_scope=c.persona_scope),0)+
                    COALESCE((SELECT SUM(length(authority_bytes)) FROM semantic_time_authority WHERE persona_scope=c.persona_scope),0)
             FROM semantic_commits AS c
             JOIN semantic_snapshots AS s USING(persona_scope,semantic_revision)
             JOIN semantic_cursor AS cur USING(persona_scope,semantic_revision)
             JOIN semantic_budget_checkpoint AS b
               ON b.persona_scope=c.persona_scope AND b.head_revision=c.semantic_revision
             WHERE c.persona_scope=?1 AND c.semantic_revision=?2",
            params![
                prepared.persona_scope.to_vec(),
                prepared.semantic_revision as i64
            ],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(compact.0 as usize, TIME_SNAPSHOT_WIRE_LEN_V1);
    assert_eq!(compact.1, 0);
    assert_eq!(compact.2, prepared.new_commitment.to_vec());
    assert_eq!(compact.3, prepared.new_commitment.to_vec());
    assert_eq!(compact.4, compact.5);
    assert!(compact.4 < prepared.old_budget_bytes as i64);
    drop(conn);

    let mut reopened = Store::open(&valid_path).unwrap();
    reopened.audit_semantic_integrity_v1().unwrap();
}

#[test]
fn sleep_time_advance_relaxes_without_inventing_evidence() {
    let path = db("sleep-relax");
    let request = genesis(0x21);
    let scope = persona_scope(&request);
    let persona = wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
    let base = 1_700_000_000_000;
    let mut first = AstrRuntime::open(&path).unwrap();
    let initial = bootstrap(&mut first, &request);
    let first_perception = perception(&mut first, &request, 1);
    let evidence_before: i64 = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM semantic_evidence_authority",
            [],
            |row| row.get(0),
        )
        .unwrap();

    // Bind a second Runtime to the old head, advance perception in the first,
    // then settle through the stale Runtime. Store-owned receipt projection
    // must use the fresh committed graph/revision.
    let mut stale = AstrRuntime::open(&path).unwrap();
    stale.ensure_genesis(&request).unwrap();
    let latest_perception = perception(&mut first, &request, 2);
    force_sleep(&path, initial.clone(), base);
    let emergency = AutonomousStimulusV1 {
        arousal: Fixed::ONE,
        urgency: Fixed::ONE,
        emergency_authorized: true,
        source_digest: [0xE1; 32],
    };
    let (token, settled) = wake(
        &mut stale,
        &scope,
        initial.generation,
        1,
        base + 600_000,
        emergency,
    )
    .unwrap();
    assert_eq!(stale.settle_wake(&token).unwrap(), settled);
    assert_eq!(settled.generation, initial.generation + 1);

    let conn = Connection::open(&path).unwrap();
    let (snapshot_len, time_graphs, time_receipts, time_telemetry, authority_body):
        (i64, i64, i64, i64, Vec<u8>) = conn
        .query_row(
            "SELECT length(s.snapshot_bytes),
                    (SELECT COUNT(*) FROM semantic_graphs g WHERE g.persona_scope=c.persona_scope AND g.semantic_revision=c.semantic_revision),
                    (SELECT COUNT(*) FROM semantic_receipts r WHERE r.persona_scope=c.persona_scope AND r.semantic_revision=c.semantic_revision),
                    (SELECT COUNT(*) FROM semantic_telemetry t WHERE t.persona_scope=c.persona_scope AND t.semantic_revision=c.semantic_revision),
                    a.authority_bytes
             FROM semantic_commits c
             JOIN semantic_snapshots s USING(persona_scope,semantic_revision)
             JOIN semantic_time_authority a USING(persona_scope,semantic_revision)
             WHERE c.persona_scope=?1 AND c.transition_kind='time'
             ORDER BY c.semantic_revision DESC LIMIT 1",
            params![persona.to_vec()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .unwrap();
    let authority: SemanticTimeAuthorityV1 = serde_json::from_slice(&authority_body).unwrap();
    assert_eq!(snapshot_len as usize, TIME_SNAPSHOT_WIRE_LEN_V1);
    assert_eq!((time_graphs, time_receipts, time_telemetry), (0, 0, 0));
    assert_eq!(authority.pre_sleep_phase, MatrixSleepPhaseV1::Asleep);
    assert_eq!(authority.graph_digest, latest_perception.graph_after);
    assert_ne!(authority.state_before, authority.state_after);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM semantic_evidence_authority",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        evidence_before + 1
    );
    assert_eq!(
        first_perception.formula_digest,
        latest_perception.formula_digest
    );
    drop(conn);
    drop(first);
    drop(stale);
    let mut reopened = AstrRuntime::open(&path).unwrap();
    reopened.audit_semantic_integrity_v1().unwrap();
}

#[test]
fn frozen_chunking_is_deterministic_and_backward_time_fails_closed() {
    let one_path = db("chunk-one");
    let two_path = db("chunk-two");
    let request = genesis(0x31);
    let scope = persona_scope(&request);
    let persona = wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
    let base = 1_700_000_000_000;

    let mut one = AstrRuntime::open(&one_path).unwrap();
    let one_initial = bootstrap(&mut one, &request);
    perception(&mut one, &request, 1);
    force_sleep(&one_path, one_initial.clone(), base);
    let (_, one_state) = wake(
        &mut one,
        &scope,
        one_initial.generation,
        1,
        base + 1_200_000,
        AutonomousStimulusV1::default(),
    )
    .unwrap();

    let mut two = AstrRuntime::open(&two_path).unwrap();
    let two_initial = bootstrap(&mut two, &request);
    perception(&mut two, &request, 1);
    force_sleep(&two_path, two_initial.clone(), base);
    let (_, mid) = wake(
        &mut two,
        &scope,
        two_initial.generation,
        1,
        base + 600_000,
        AutonomousStimulusV1::default(),
    )
    .unwrap();
    // Hold the pre-advance sleep phase constant so this checks semantic time
    // chunking, not the scheduler's independent sleep-state transition.
    force_sleep(&two_path, mid.clone(), base + 600_000);
    let (_, two_state) = wake(
        &mut two,
        &scope,
        mid.generation,
        2,
        base + 1_200_000,
        AutonomousStimulusV1::default(),
    )
    .unwrap();
    let one_cursor = cursor(&one_path, persona);
    let two_cursor = cursor(&two_path, persona);
    assert_eq!(one_cursor.1, two_cursor.1);
    assert_eq!(
        (one_cursor.4, one_cursor.5, one_cursor.6),
        (two_cursor.4, two_cursor.5, two_cursor.6)
    );

    let before_backward = (
        two.current_revision(&scope).unwrap(),
        two.semantic_revision_v1(&scope).unwrap(),
    );
    let backwards = TimeAdvanceV1 {
        event_id: id(0x70, 3),
        scope: scope.clone(),
        expected_generation: two_state.generation,
        frozen: frozen(base + 1_199_999),
        frozen_input_digest: ae_runtime::frozen_time_input_digest(&frozen(base + 1_199_999)),
        stimulus: AutonomousStimulusV1::default(),
    };
    let backwards_claim = two.claim_wake(&scope, &backwards).unwrap();
    assert!(two.settle_wake(&backwards_claim.claim_token).is_err());
    assert_eq!(
        before_backward,
        (
            two.current_revision(&scope).unwrap(),
            two.semantic_revision_v1(&scope).unwrap()
        )
    );

    // A fault after the journal insert must roll the whole wake back; the
    // exact same claim then succeeds once the fault seam is removed.
    let fault_now = base + 1_800_000;
    let fault_event = TimeAdvanceV1 {
        event_id: id(0x70, 4),
        scope: scope.clone(),
        expected_generation: two_state.generation,
        frozen: frozen(fault_now),
        frozen_input_digest: ae_runtime::frozen_time_input_digest(&frozen(fault_now)),
        stimulus: AutonomousStimulusV1::default(),
    };
    let fault_claim = two.claim_wake(&scope, &fault_event).unwrap();
    Connection::open(&two_path)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_time_snapshot BEFORE INSERT ON semantic_snapshots
             BEGIN SELECT RAISE(ABORT,'fault rollback'); END;",
        )
        .unwrap();
    let before_fault = (
        two.current_revision(&scope).unwrap(),
        two.semantic_revision_v1(&scope).unwrap(),
    );
    assert!(two.settle_wake(&fault_claim.claim_token).is_err());
    assert_eq!(
        before_fault,
        (
            two.current_revision(&scope).unwrap(),
            two.semantic_revision_v1(&scope).unwrap()
        )
    );
    Connection::open(&two_path)
        .unwrap()
        .execute_batch("DROP TRIGGER fail_time_snapshot;")
        .unwrap();
    let after_fault = two.settle_wake(&fault_claim.claim_token).unwrap();

    // Perception is a new authenticated anchor and resets every accumulated
    // time counter. A following time event re-enters the compact lane.
    perception(&mut two, &request, 2);
    let reset = cursor(&two_path, persona);
    assert_eq!(reset.3, "perception");
    assert_eq!((reset.4, reset.5, reset.6), (0, 0, 0));
    let (_, after_mixed) = wake(
        &mut two,
        &scope,
        after_fault.generation,
        5,
        fault_now + 600_000,
        AutonomousStimulusV1::default(),
    )
    .unwrap();
    assert_eq!(after_mixed.generation, after_fault.generation + 1);
    two.audit_semantic_integrity_v1().unwrap();

    // Equal time fails closed without an autonomy, journal, or semantic write.
    // A larger-than-seven-day gap produces one compact, explicitly capped
    // projection.
    let semantic_before_zero = two.semantic_revision_v1(&scope).unwrap();
    let canonical_before_zero = two.current_revision(&scope).unwrap();
    let zero_frozen = frozen(after_mixed.last_advanced_at_utc_ms);
    let zero_event = TimeAdvanceV1 {
        event_id: id(0x70, 6),
        scope: scope.clone(),
        expected_generation: after_mixed.generation,
        frozen_input_digest: ae_runtime::frozen_time_input_digest(&zero_frozen),
        frozen: zero_frozen,
        stimulus: AutonomousStimulusV1::default(),
    };
    let zero_claim = two.claim_wake(&scope, &zero_event).unwrap();
    assert!(two.settle_wake(&zero_claim.claim_token).is_err());
    assert_eq!(
        two.semantic_revision_v1(&scope).unwrap(),
        semantic_before_zero
    );
    assert_eq!(two.current_revision(&scope).unwrap(), canonical_before_zero);
    assert_eq!(
        Connection::open(&two_path)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM wake_time_settlement_v1 WHERE claim_token=?1",
                params![zero_claim.claim_token.to_vec()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );

    force_sleep(
        &two_path,
        after_mixed.clone(),
        after_mixed.last_advanced_at_utc_ms,
    );
    let cap_now = after_mixed.last_advanced_at_utc_ms + MATRIX_TIME_MAX_ELAPSED_MS + 600_000;
    let (_, capped_state) = wake(
        &mut two,
        &scope,
        after_mixed.generation,
        7,
        cap_now,
        AutonomousStimulusV1::default(),
    )
    .unwrap();
    let capped_authority_bytes: Vec<u8> = Connection::open(&two_path)
        .unwrap()
        .query_row(
            "SELECT authority_bytes FROM semantic_time_authority
             WHERE persona_scope=?1 ORDER BY semantic_revision DESC LIMIT 1",
            params![persona.to_vec()],
            |row| row.get(0),
        )
        .unwrap();
    let capped_authority: SemanticTimeAuthorityV1 =
        serde_json::from_slice(&capped_authority_bytes).unwrap();
    assert!(capped_authority.capped_gap);
    assert_eq!(
        capped_authority.applied_elapsed_ms,
        MATRIX_TIME_MAX_ELAPSED_MS
    );
    assert_eq!(
        capped_authority.raw_elapsed_ms,
        MATRIX_TIME_MAX_ELAPSED_MS + 600_000
    );

    // Ordinary latest, reopen, and append paths authenticate each cursor
    // epoch component, rather than deferring detection to the explicit audit.
    let mut claims = Vec::new();
    for number in 8_u64..=10 {
        let now = cap_now + (number - 7) * 600_000;
        let event = TimeAdvanceV1 {
            event_id: id(0x70, number),
            scope: scope.clone(),
            expected_generation: capped_state.generation,
            frozen: frozen(now),
            frozen_input_digest: ae_runtime::frozen_time_input_digest(&frozen(now)),
            stimulus: AutonomousStimulusV1::default(),
        };
        claims.push(two.claim_wake(&scope, &event).unwrap().claim_token);
    }
    let original_epoch: (i64, i64, Vec<u8>) = Connection::open(&two_path)
        .unwrap()
        .query_row(
            "SELECT time_asleep_ticks,time_asleep_remainder_ms,time_anchor_state_digest
             FROM semantic_cursor WHERE persona_scope=?1",
            params![persona.to_vec()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();

    Connection::open(&two_path)
        .unwrap()
        .execute(
            "UPDATE semantic_cursor SET time_asleep_ticks=time_asleep_ticks+1
             WHERE persona_scope=?1",
            params![persona.to_vec()],
        )
        .unwrap();
    assert!(two.current_revision(&scope).is_err());
    assert!(AstrRuntime::open(&two_path).is_err());
    assert!(two.settle_wake(&claims[0]).is_err());
    Connection::open(&two_path)
        .unwrap()
        .execute(
            "UPDATE semantic_cursor SET time_asleep_ticks=?2 WHERE persona_scope=?1",
            params![persona.to_vec(), original_epoch.0],
        )
        .unwrap();

    Connection::open(&two_path)
        .unwrap()
        .execute(
            "UPDATE semantic_cursor SET time_asleep_remainder_ms=time_asleep_remainder_ms+1
             WHERE persona_scope=?1",
            params![persona.to_vec()],
        )
        .unwrap();
    assert!(two.current_revision(&scope).is_err());
    assert!(AstrRuntime::open(&two_path).is_err());
    assert!(two.settle_wake(&claims[1]).is_err());
    Connection::open(&two_path)
        .unwrap()
        .execute(
            "UPDATE semantic_cursor SET time_asleep_remainder_ms=?2 WHERE persona_scope=?1",
            params![persona.to_vec(), original_epoch.1],
        )
        .unwrap();

    Connection::open(&two_path)
        .unwrap()
        .execute(
            "UPDATE semantic_cursor SET time_anchor_state_digest=zeroblob(32)
             WHERE persona_scope=?1",
            params![persona.to_vec()],
        )
        .unwrap();
    assert!(two.current_revision(&scope).is_err());
    assert!(AstrRuntime::open(&two_path).is_err());
    assert!(two.settle_wake(&claims[2]).is_err());
    Connection::open(&two_path)
        .unwrap()
        .execute(
            "UPDATE semantic_cursor SET time_anchor_state_digest=?2 WHERE persona_scope=?1",
            params![persona.to_vec(), original_epoch.2],
        )
        .unwrap();
    two.audit_semantic_integrity_v1().unwrap();
    drop(one);
    let _ = one_state;
}

#[test]
fn dream_residue_and_endogenous_intent_remain_local_and_deterministic() {
    fn run(path: &Path, seed: u8) -> (String, String) {
        let request = genesis(seed);
        let scope = persona_scope(&request);
        let base = 1_700_100_000_000;
        let mut runtime = AstrRuntime::open(path).unwrap();
        let initial = bootstrap(&mut runtime, &request);
        perception(&mut runtime, &request, 1);
        force_sleep(path, initial.clone(), base);
        wake(
            &mut runtime,
            &scope,
            initial.generation,
            1,
            base + 600_000,
            AutonomousStimulusV1::default(),
        )
        .unwrap();
        runtime.audit_semantic_integrity_v1().unwrap();
        drop(runtime);
        let conn = Connection::open(path).unwrap();
        let endogenous: String = conn
            .query_row(
                "SELECT body_json FROM endogenous_intent_state_v1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let dream: String = conn
            .query_row("SELECT body_json FROM local_dream_residue_v1", [], |row| {
                row.get(0)
            })
            .unwrap();
        let local_counts: (i64, i64, i64, i64) = conn
            .query_row(
                "SELECT
                   (SELECT COUNT(*) FROM durable_intention),
                   (SELECT COUNT(*) FROM outbound_attempt),
                   (SELECT COUNT(*) FROM outbound_target),
                   (SELECT COUNT(*) FROM journal WHERE event_kind='SelfActionCandidate')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(local_counts, (0, 0, 0, 0));
        let decoded_intent: EndogenousIntentStateV1 = serde_json::from_str(&endogenous).unwrap();
        let decoded_dream: LocalDreamResidueV1 = serde_json::from_str(&dream).unwrap();
        assert!(decoded_intent.validate_v1().is_ok());
        assert!(decoded_dream.validate_v1().is_ok());
        assert!(decoded_dream.non_fact);
        (endogenous, dream)
    }

    let first = db("local-cognition-a");
    let second = db("local-cognition-b");
    assert_eq!(run(&first, 0x41), run(&second, 0x41));

    // Exercise the explicit V7->V8 migration boundary through Runtime open.
    let conn = Connection::open(&first).unwrap();
    conn.execute_batch(
        "BEGIN IMMEDIATE;
         CREATE TABLE task8_v7_marker(value INTEGER NOT NULL);
         INSERT INTO task8_v7_marker VALUES(2718);
         DROP TABLE wake_time_settlement_v1;
         DROP INDEX local_dream_residue_persona_v1;
         DROP TABLE local_dream_residue_v1;
         DROP TABLE endogenous_intent_state_v1;
         DELETE FROM schema_migrations WHERE version=8;
         COMMIT;",
    )
    .unwrap();
    drop(conn);
    drop(AstrRuntime::open(&first).unwrap());
    let conn = Connection::open(&first).unwrap();
    assert_eq!(
        conn.query_row("SELECT MAX(version) FROM schema_migrations", [], |row| row
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        8
    );
    assert_eq!(
        conn.query_row("SELECT value FROM task8_v7_marker", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        2718
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name IN (
              'endogenous_intent_state_v1','local_dream_residue_v1','wake_time_settlement_v1')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        3
    );
}
