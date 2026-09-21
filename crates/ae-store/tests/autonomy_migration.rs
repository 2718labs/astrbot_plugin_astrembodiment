// Historical constructors are opt-in and absent from production/default APIs.
#![cfg(feature = "migration-test-hooks")]
use ae_continuum::CommitEnvelope;
use ae_contracts::{
    wire, AutonomyJournalDeltaV1, CanonicalEvent, CausalRef, CommitStatus, DispatchClaimV1,
    DurableIntentionV1, ExternalizationClaimV1, ExternalizationOutcomeV1, ExternalizationSettleV1,
    InnerEventKindV1, InnerEventV1, IntentionStateV1, InteractionFactBatchV1,
    InteractionFactKindV1, InteractionFactV1, InteractionSourceAuthorityV1, InvariantResiduals,
    OutboundAttemptV1, OutboundTargetEnvelopeV1, RelationTemporalPolicyV1, ScopeRef, TargetKindV1,
    TimezoneSourceV1, TransitionReceipt, ALPHA3_SCHEMA_VERSION,
};
use ae_fixed::Fixed;
use ae_store::{Store, StoreError};
use rusqlite::{params, Connection};
use std::path::PathBuf;

fn db(name: &str) -> PathBuf {
    let root = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .expect("task-local CARGO_TARGET_DIR");
    let path = root.join(format!("{name}-{}.sqlite3", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

fn legacy_v5_event_database(name: &str, insert_projection: bool) -> (PathBuf, InnerEventV1) {
    let path = db(name);
    let persona_scope = [101; 32];
    let event = InnerEventV1 {
        schema_version: 1,
        event_id: [102; 16],
        persona_scope,
        kind: InnerEventKindV1::HomeostasisChanged,
        committed_at_utc_ms: 1_700_000_000_000,
        summary_code: "migration_review".into(),
        value_before: None,
        value_after: Some(Fixed::ONE),
        source_event_ids: vec![],
        tombstoned: false,
    };
    let delta = serde_json::to_vec(&AutonomyJournalDeltaV1 {
        state: None,
        inner_events: vec![event.clone()],
        intention: None,
    })
    .unwrap();
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,digest BLOB NOT NULL,completed_at_ms INTEGER NOT NULL);
         INSERT INTO schema_migrations(version,digest,completed_at_ms) VALUES(5,X'05',0);
         CREATE TABLE journal(revision INTEGER PRIMARY KEY AUTOINCREMENT,logical_revision INTEGER NOT NULL,scope_digest BLOB NOT NULL,base_revision INTEGER NOT NULL,event_kind TEXT NOT NULL,event_bytes BLOB NOT NULL,event_digest BLOB NOT NULL,receipt_bytes BLOB NOT NULL,delta_bytes BLOB NOT NULL DEFAULT X'',chain_digest BLOB NOT NULL,committed_at_ms INTEGER NOT NULL);
         CREATE TABLE inner_event(event_id BLOB PRIMARY KEY,persona_scope BLOB NOT NULL,committed_at_utc_ms INTEGER NOT NULL,kind TEXT NOT NULL,tombstoned INTEGER NOT NULL DEFAULT 0,body_json TEXT NOT NULL);",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO journal(logical_revision,scope_digest,base_revision,event_kind,event_bytes,event_digest,receipt_bytes,delta_bytes,chain_digest,committed_at_ms) VALUES(1,?1,0,'time_advance',X'',?2,X'',?3,zeroblob(32),?4)",
        params![
            persona_scope.to_vec(),
            vec![103_u8; 32],
            delta,
            event.committed_at_utc_ms as i64,
        ],
    )
    .unwrap();
    if insert_projection {
        conn.execute(
            "INSERT INTO inner_event(event_id,persona_scope,committed_at_utc_ms,kind,tombstoned,body_json) VALUES(?1,?2,?3,'HomeostasisChanged',0,?4)",
            params![
                event.event_id.to_vec(),
                persona_scope.to_vec(),
                event.committed_at_utc_ms as i64,
                serde_json::to_string(&event).unwrap(),
            ],
        )
        .unwrap();
    }
    drop(conn);
    (path, event)
}

fn retired_rows(path: &std::path::Path) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    let conn=Connection::open(path).unwrap();
    ["durable_intention","outbound_attempt","outbound_target","autonomy_claim",
     "externalization_budget","externalization_budget_claim",
     "autonomy_operational_authority","autonomy_operational_authority_head"].iter().map(|table| {
        let mut stmt=conn.prepare(&format!("SELECT * FROM {table} ORDER BY rowid")).unwrap();
        let columns=stmt.column_count();
        stmt.query_map([],|row|(0..columns).map(|index|row.get(index)).collect::<Result<Vec<_>,_>>()).unwrap().collect::<Result<Vec<_>,_>>().unwrap()
    }).collect()
}

fn target() -> OutboundTargetEnvelopeV1 {
    OutboundTargetEnvelopeV1 {
        schema_version: 1,
        target_kind: TargetKindV1::Private,
        umo_ciphertext: vec![1, 2, 3],
        umo_nonce: vec![4; 12],
        key_id: "test".into(),
        umo_digest: [5; 32],
        platform_token: [6; 16],
        bot_token: [7; 16],
        persona_token: [8; 16],
        relation_token: [9; 16],
        session_token: [10; 16],
        bound_at_utc_ms: 1,
        binding_generation: 1,
        binding_digest: [11; 32],
    }
}

fn legacy_alpha3_policy(relation_scope: [u8; 32], proactive_enabled: bool) -> String {
    serde_json::to_string(&RelationTemporalPolicyV1 {
        schema_version: 1,
        relation_scope,
        user_timezone: "Asia/Shanghai".into(),
        timezone_source: TimezoneSourceV1::Explicit,
        quiet_hours_start_minute: 1_320,
        quiet_hours_end_minute: 420,
        quiet_hours_emergency_bypass: false,
        proactive_enabled,
        proactive_daily_max: 2,
        min_proactive_cooldown_ms: 3_600_000,
        intention_ttl_ms: 86_400_000,
        unanswered_backoff_base_ms: 3_600_000,
        unanswered_hard_stop: 3,
        emergency_threshold: Fixed::ONE,
        daily_submitted: 0,
        consecutive_unanswered: 0,
        last_inbound_utc_ms: Some(1_700_000_000_000),
        last_proactive_submitted_utc_ms: None,
        revision: 4,
        auto_policy_version: 0,
        next_claim_reservation_tokens: 0,
    })
    .unwrap()
}

fn legacy_alpha3_externalization_claim() -> ExternalizationClaimV1 {
    let mut claim = ExternalizationClaimV1 {
        claim_token: [0; 32],
        intention_id: [86; 16],
        attempt_no: 1,
        max_tokens: 512,
        prompt_contract: "migration_claim".into(),
        prompt_contract_digest: [90; 32],
        relation_policy_revision: 4,
        target_binding_digest: [91; 32],
        capability_snapshot_digest: [92; 32],
        frozen_input_digest: [93; 32],
        caller_incarnation: [94; 32],
        lease_deadline_utc_ms: 4_102_444_800_000,
    };
    claim.claim_token = wire::domain_hash(
        b"ae.externalization-claim.v2",
        &[
            &claim.intention_id,
            &[claim.attempt_no],
            &claim.max_tokens.to_le_bytes(),
            claim.prompt_contract.as_bytes(),
            &claim.prompt_contract_digest,
            &claim.relation_policy_revision.to_le_bytes(),
            &claim.target_binding_digest,
            &claim.capability_snapshot_digest,
            &claim.frozen_input_digest,
            &claim.caller_incarnation,
            &claim.lease_deadline_utc_ms.to_le_bytes(),
        ],
    );
    claim
}

fn legacy_alpha3_settle_request() -> ExternalizationSettleV1 {
    ExternalizationSettleV1 {
        claim_token: legacy_alpha3_externalization_claim().claim_token,
        outcome: ExternalizationOutcomeV1::RejectedTerminal,
        used_tokens: Some(17),
        candidate_digest: None,
        candidate_ciphertext: None,
        caller_incarnation: [94; 32],
    }
}

fn legacy_v6_alpha3_database(name: &str, malformed_policy: bool) -> PathBuf {
    let path = db(name);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,digest BLOB NOT NULL,completed_at_ms INTEGER NOT NULL);
         INSERT INTO schema_migrations(version,digest,completed_at_ms) VALUES(6,X'61652E6175746F6E6F6D792E64622E76362E696E6E65722D6576656E742D6D616E69666573742E7631',0);
         CREATE TABLE relation_temporal_policy(relation_scope BLOB PRIMARY KEY,revision INTEGER NOT NULL,body_json TEXT NOT NULL);
         CREATE TABLE autonomy_scope_binding(work_scope BLOB PRIMARY KEY,persona_scope BLOB NOT NULL,relation_scope BLOB,scope_json TEXT NOT NULL);
         CREATE TABLE autonomous_runtime_state(persona_scope BLOB PRIMARY KEY,generation INTEGER NOT NULL,state_revision INTEGER NOT NULL,body_json TEXT NOT NULL);
         CREATE TABLE durable_intention(intention_id BLOB PRIMARY KEY,persona_scope BLOB NOT NULL,relation_scope BLOB NOT NULL,semantic_digest BLOB NOT NULL UNIQUE,state TEXT NOT NULL,revision INTEGER NOT NULL,body_json TEXT NOT NULL);
         CREATE TABLE autonomy_claim(claim_token BLOB PRIMARY KEY,claim_kind TEXT NOT NULL,record_id BLOB NOT NULL,caller_incarnation BLOB NOT NULL,lease_deadline_utc_ms INTEGER NOT NULL,body_json TEXT NOT NULL,UNIQUE(claim_kind,record_id));
         CREATE TABLE externalization_budget(relation_scope BLOB NOT NULL,budget_day_start_utc_ms INTEGER NOT NULL,reserved_tokens INTEGER NOT NULL,PRIMARY KEY(relation_scope,budget_day_start_utc_ms));
         CREATE TABLE externalization_budget_claim(claim_token BLOB PRIMARY KEY,relation_scope BLOB NOT NULL,budget_day_start_utc_ms INTEGER NOT NULL,reserved_tokens INTEGER NOT NULL);",
    )
    .unwrap();

    let personas = [[31_u8; 32], [32_u8; 32]];
    let relations = [[41_u8; 32], [42_u8; 32]];
    for index in 0..2 {
        conn.execute(
            "INSERT INTO autonomy_scope_binding(work_scope,persona_scope,relation_scope,scope_json) VALUES(?1,?2,?3,'{}')",
            params![vec![51_u8 + index as u8; 32], personas[index].to_vec(), relations[index].to_vec()],
        )
        .unwrap();
        let body = if malformed_policy && index == 0 {
            "{".to_owned()
        } else {
            legacy_alpha3_policy(relations[index], index == 0)
        };
        conn.execute(
            "INSERT INTO relation_temporal_policy(relation_scope,revision,body_json) VALUES(?1,4,?2)",
            params![relations[index].to_vec(), body],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO autonomous_runtime_state(persona_scope,generation,state_revision,body_json) VALUES(?1,1,1,?2)",
            params![
                personas[index].to_vec(),
                format!(
                    "{{\"schema_version\":1,\"persona_scope\":\"{}\",\"relation_scope\":null,\"generation\":1,\"state_revision\":1,\"last_advanced_at_utc_ms\":0,\"next_wake_at_utc_ms\":1,\"wake_intensity\":\"maintenance\",\"sleep_state\":\"awake\",\"process_s\":0,\"process_c\":0,\"arousal\":0,\"sleep_threshold_held_ms\":0,\"circadian_phase_minutes\":0,\"affiliation_need\":900000,\"unfinished_topic_salience\":0,\"social_energy\":0,\"formula_digest\":\"{}\",\"mapping_digest\":\"{}\",\"workspace_residual\":0}}",
                    ae_contracts::hex::encode32(&personas[index]),
                    ae_contracts::hex::encode32(&[61; 32]),
                    ae_contracts::hex::encode32(&[62; 32]),
                ),
            ],
        )
        .unwrap();
    }
    let intention = DurableIntentionV1 {
        schema_version: 1,
        intention_id: [86; 16],
        persona_scope: personas[0],
        relation_scope: relations[0],
        state: IntentionStateV1::Externalizing,
        action_class: "relationship_connection".into(),
        salience: Fixed::ONE,
        urgency: Fixed::ONE,
        confidence: Fixed::ONE,
        created_at_utc_ms: 1,
        not_before_utc_ms: 1,
        expires_at_utc_ms: 4_102_444_800_000,
        externalization_attempts: 1,
        semantic_idempotency_digest: [87; 32],
        workspace_mapping_digest: [88; 32],
        workspace_residual: Fixed::ZERO,
        source_event_ids: vec![[89; 16]],
    };
    conn.execute(
        "INSERT INTO durable_intention(intention_id,persona_scope,relation_scope,semantic_digest,state,revision,body_json) VALUES(?1,?2,?3,?4,'externalizing',4,?5)",
        params![
            intention.intention_id.to_vec(),
            intention.persona_scope.to_vec(),
            intention.relation_scope.to_vec(),
            intention.semantic_idempotency_digest.to_vec(),
            serde_json::to_string(&intention).unwrap()
        ],
    )
    .unwrap();
    let claim = legacy_alpha3_externalization_claim();
    conn.execute(
        "INSERT INTO autonomy_claim(claim_token,claim_kind,record_id,caller_incarnation,lease_deadline_utc_ms,body_json) VALUES(?1,'externalization',?2,?3,?4,?5)",
        params![
            claim.claim_token.to_vec(),
            intention.intention_id.to_vec(),
            claim.caller_incarnation.to_vec(),
            claim.lease_deadline_utc_ms as i64,
            serde_json::to_string(&claim).unwrap()
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO externalization_budget(relation_scope,budget_day_start_utc_ms,reserved_tokens) VALUES(?1,1000,549)",
        params![relations[0].to_vec()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO externalization_budget_claim(claim_token,relation_scope,budget_day_start_utc_ms,reserved_tokens) VALUES(?1,?2,1000,512)",
        params![claim.claim_token.to_vec(), relations[0].to_vec()],
    )
    .unwrap();
    drop(conn);
    path
}

fn legacy_v7_without_settlement_marker(name: &str) -> PathBuf {
    let path = legacy_v6_alpha3_database(name, false);
    drop(Store::open_legacy_v8_fixture(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "BEGIN IMMEDIATE;
         ALTER TABLE externalization_budget_claim
             RENAME TO externalization_budget_claim_with_marker;
         CREATE TABLE externalization_budget_claim(
             claim_token BLOB PRIMARY KEY,
             relation_scope BLOB NOT NULL,
             budget_day_start_utc_ms INTEGER NOT NULL,
             reserved_tokens INTEGER NOT NULL
         );
         INSERT INTO externalization_budget_claim(
             claim_token,relation_scope,budget_day_start_utc_ms,reserved_tokens
         )
         SELECT claim_token,relation_scope,budget_day_start_utc_ms,reserved_tokens
         FROM externalization_budget_claim_with_marker;
         DROP TABLE externalization_budget_claim_with_marker;
         COMMIT;",
    )
    .unwrap();
    drop(conn);
    path
}

fn has_settlement_marker(conn: &Connection) -> bool {
    let mut statement = conn
        .prepare("PRAGMA table_info(externalization_budget_claim)")
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .iter()
        .any(|column| column == "migrated_unknown_full_charge")
}

fn alpha3_v7_fingerprint(
    path: &PathBuf,
) -> (
    Vec<u8>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    (i64, i64, i64, i64, i64),
) {
    let conn = Connection::open(path).unwrap();
    let digest = conn
        .query_row(
            "SELECT digest FROM schema_migrations WHERE version=7",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let bodies = |table: &str, order: &str| {
        let mut statement = conn
            .prepare(&format!("SELECT body_json FROM {table} ORDER BY {order}"))
            .unwrap();
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let budget = conn
        .query_row(
            "SELECT reserved_tokens,charged_tokens,used_tokens,usage_known,limit_tokens
             FROM externalization_budget
             WHERE relation_scope=?1 AND budget_day_start_utc_ms=1000",
            params![vec![41_u8; 32]],
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
        .unwrap();
    (
        digest,
        bodies("world_anchor", "persona_scope"),
        bodies("relation_consent", "relation_scope,consent_epoch,revision"),
        bodies("relation_contact_process", "relation_scope"),
        budget,
    )
}

fn assert_kind10_rejected_by_legacy_commit_lane() {
    let mut store = Store::open_in_memory().unwrap();
    let scope = ScopeRef {
        bot_token: [71; 16],
        persona_token: [72; 16],
        relation_token: Some([73; 16]),
        session_token: [74; 16],
    };
    let event = CanonicalEvent::InteractionFactBatch(InteractionFactBatchV1 {
        schema_version: ALPHA3_SCHEMA_VERSION,
        event_id: [75; 16],
        scope: scope.clone(),
        causal: CausalRef {
            turn_id: [76; 16],
            action_id: None,
            delivery_id: None,
            claim_id: None,
            base_revision: 0,
        },
        facts: vec![InteractionFactV1 {
            fact_id: [77; 16],
            kind: InteractionFactKindV1::InboundObserved,
            observed_at_utc_ms: 1,
            source_authority: InteractionSourceAuthorityV1::AstrbotMetadata,
            source_digest: [78; 32],
            extractor_digest: [79; 32],
            confidence: Fixed::ONE,
            value_code: None,
            subject_public_ref: None,
            consent_terms: None,
            scheduled_at_utc_ms: None,
            expires_at_utc_ms: None,
        }],
    });
    let event_bytes = wire::encode_event_checked(&event).unwrap();
    let event_digest = wire::event_digest(&event);
    let event_scope = wire::persona_scope_digest(
        &scope.bot_token,
        &scope.persona_token,
        scope.relation_token.as_ref(),
    );
    let receipt_scope = [80; 32];
    let envelope = CommitEnvelope {
        event_kind: "interaction_fact_batch".into(),
        event_bytes,
        receipt: TransitionReceipt {
            schema_version: 1,
            formula_digest: [81; 32],
            scope_digest: receipt_scope,
            event_digest,
            authority_digest: [82; 32],
            base_revision: 0,
            next_revision: 1,
            state_before: [83; 32],
            state_after: [83; 32],
            graph_after: [84; 32],
            action_contract: None,
            active_nodes: 0,
            active_edges: 0,
            residuals: InvariantResiduals::default(),
            status: CommitStatus::Committed,
        },
        chain_seed: [85; 32],
        delta_bytes: Vec::new(),
    };

    let error = store.commit_journal(&envelope).unwrap_err();
    assert!(matches!(
        error,
        StoreError::ContinuityFence("UNSUPPORTED_CORE_BOUNDARY")
    ));
    assert!(store.read_journal(&receipt_scope).unwrap().is_empty());
    assert!(store.read_journal(&event_scope).unwrap().is_empty());
    assert!(store
        .lookup_event(&receipt_scope, &event_digest)
        .unwrap()
        .is_none());
    assert!(store
        .lookup_event(&event_scope, &event_digest)
        .unwrap()
        .is_none());
}

#[test]
fn alpha3_v6_to_v7() {
    assert_kind10_rejected_by_legacy_commit_lane();
    let fresh = db("alpha3-v9-fresh");
    drop(Store::open(&fresh).unwrap());
    let conn = Connection::open(&fresh).unwrap();
    assert_eq!(conn.query_row("SELECT MAX(version) FROM schema_migrations", [], |r|r.get::<_,i64>(0)).unwrap(),9);
    assert!(has_settlement_marker(&conn));
    drop(conn);
    let path = legacy_v6_alpha3_database("alpha3-v6-valid", false);
    drop(Store::open(&path).unwrap());
    let before=alpha3_v7_fingerprint(&path);
    assert_eq!(before.1.len(),2);
    assert_eq!(before.4,(0,549,0,0,549));
    let mut store=Store::open(&path).unwrap();
    assert!(matches!(store.settle_externalization(&legacy_alpha3_settle_request()),Err(StoreError::ContinuityFence("UNSUPPORTED_CORE_BOUNDARY"))));
    drop(store);
    let conn=Connection::open(&path).unwrap();
    for sql in ["UPDATE externalization_budget SET charged_tokens=-1","DELETE FROM autonomy_claim"] {
        assert!(conn.execute(sql,[]).unwrap_err().to_string().contains("CORE_BOUNDARY_LEGACY_WRITE_DENIED"));
    }
    drop(conn);
    drop(Store::open(&path).unwrap());
    assert_eq!(alpha3_v7_fingerprint(&path),before);

    // An unauthenticated partial historical schema is rejected, never repaired
    // underneath a v9 fence. The source bytes and absent marker survive.
    let incomplete=legacy_v7_without_settlement_marker("alpha3-v8-missing-marker");
    let before=alpha3_v7_fingerprint(&incomplete);
    assert!(Store::open(&incomplete).is_err());
    assert_eq!(alpha3_v7_fingerprint(&incomplete),before);
    assert!(!has_settlement_marker(&Connection::open(&incomplete).unwrap()));

    let malformed=legacy_v6_alpha3_database("alpha3-v6-malformed",true);
    assert!(Store::open(&malformed).is_err());
    let conn=Connection::open(&malformed).unwrap();
    assert_eq!(conn.query_row("SELECT MAX(version) FROM schema_migrations",[],|r|r.get::<_,i64>(0)).unwrap(),6);
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE name='world_anchor'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
}

#[test]
fn autonomy_v7_to_v8_is_transactional_and_preserves_legacy_data() {
    let path = db("autonomy-v7-to-v8");
    drop(Store::open_legacy_v8_fixture(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "BEGIN IMMEDIATE;
         CREATE TABLE v7_preserved_marker(value TEXT NOT NULL);
         INSERT INTO v7_preserved_marker VALUES('preserved');
         DROP TABLE wake_time_settlement_v1;
         DROP INDEX local_dream_residue_persona_v1;
         DROP TABLE local_dream_residue_v1;
         DROP TABLE endogenous_intent_state_v1;
         DELETE FROM schema_migrations WHERE version=8;
         COMMIT;",
    )
    .unwrap();
    assert_eq!(
        conn.query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        7
    );
    drop(conn);

    drop(Store::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        9
    );
    assert_eq!(
        conn.query_row("SELECT value FROM v7_preserved_marker", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap(),
        "preserved"
    );
    for table in [
        "endogenous_intent_state_v1",
        "local_dream_residue_v1",
        "wake_time_settlement_v1",
    ] {
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name=?1",
                params![table],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            1
        );
    }
}

#[test]
fn autonomy_v8_rejects_weakened_or_unreceipted_same_name_objects() {
    let weakened = db("autonomy-v8-weakened-index");
    drop(Store::open_legacy_v8_fixture(&weakened).unwrap());
    let conn = Connection::open(&weakened).unwrap();
    conn.execute_batch(
        "DROP INDEX local_dream_residue_persona_v1;
         CREATE INDEX local_dream_residue_persona_v1
           ON local_dream_residue_v1(persona_scope);",
    )
    .unwrap();
    drop(conn);
    assert!(matches!(
        Store::open(&weakened),
        Err(StoreError::AutonomyConflict(message)) if message.contains("SQL identity")
    ));

    let collision = db("autonomy-v8-unreceipted-collision");
    drop(Store::open_legacy_v8_fixture(&collision).unwrap());
    let conn = Connection::open(&collision).unwrap();
    conn.execute_batch(
        "BEGIN IMMEDIATE;
         DROP TABLE wake_time_settlement_v1;
         DROP INDEX local_dream_residue_persona_v1;
         DROP TABLE local_dream_residue_v1;
         DROP TABLE endogenous_intent_state_v1;
         CREATE TABLE endogenous_intent_state_v1(persona_scope BLOB PRIMARY KEY);
         DELETE FROM schema_migrations WHERE version=8;
         COMMIT;",
    )
    .unwrap();
    drop(conn);
    assert!(matches!(
        Store::open(&collision),
        Err(StoreError::AutonomyConflict(message)) if message.contains("before their migration receipt")
    ));
}

#[test]
fn autonomy_claim_body_is_bounded_in_schema_and_before_legacy_migration_reads() {
    let path = db("autonomy-claim-body-bound");
    drop(Store::open_legacy_v8_fixture(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    let claim_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name='autonomy_claim'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let canonical = claim_sql
        .split_whitespace()
        .collect::<String>()
        .to_ascii_lowercase();
    assert!(canonical.contains("typeof(body_json)='text'"));
    assert!(canonical.contains("length(cast(body_jsonasblob))<=1048576"));

    // Model an already-installed V7 database: its legacy table may not carry
    // the new CHECK, so open must reject the row from SQL metadata before any
    // migration code can materialize body_json into a Rust String.
    conn.execute_batch(
        "BEGIN IMMEDIATE;
         PRAGMA ignore_check_constraints=ON;
         INSERT INTO autonomy_claim(
           claim_token,claim_kind,record_id,caller_incarnation,
           lease_deadline_utc_ms,body_json
         ) VALUES(zeroblob(32),'orphan_fixture',zeroblob(16),zeroblob(32),1,
                  zeroblob(1048577));
         DROP TABLE wake_time_settlement_v1;
         DROP INDEX local_dream_residue_persona_v1;
         DROP TABLE local_dream_residue_v1;
         DROP TABLE endogenous_intent_state_v1;
         DELETE FROM schema_migrations WHERE version=8;
         COMMIT;",
    )
    .unwrap();
    drop(conn);

    assert!(matches!(
        Store::open(&path),
        Err(StoreError::AutonomyConflict(message))
            if message.contains("materialization verification bound")
    ));
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        7
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name='wake_time_settlement_v1'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        0
    );
}

#[test]
fn legacy_database_migration_preserves_journal_bytes() {
    let path = db("autonomy-legacy");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE legacy_journal(bytes BLOB NOT NULL); INSERT INTO legacy_journal VALUES(X'010203');").unwrap();
    drop(conn);
    drop(Store::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    let bytes: Vec<u8> = conn
        .query_row("SELECT bytes FROM legacy_journal", [], |r| r.get(0))
        .unwrap();
    assert_eq!(bytes, vec![1, 2, 3]);
}

#[test]
fn failed_migration_rolls_back_and_reopens() {
    let path = db("autonomy-newer");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,digest BLOB NOT NULL,completed_at_ms INTEGER NOT NULL); INSERT INTO schema_migrations VALUES(7,X'07',0); CREATE TABLE marker(value INTEGER); INSERT INTO marker VALUES(7);").unwrap();
    drop(conn);
    assert!(Store::open(&path).is_err());
    let conn = Connection::open(&path).unwrap();
    let marker: i64 = conn
        .query_row("SELECT value FROM marker", [], |r| r.get(0))
        .unwrap();
    assert_eq!(marker, 7);
}

#[test]
fn migration_review_rejects_wrong_current_v6_digest() {
    let path = db("autonomy-v6-wrong-digest");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,digest BLOB NOT NULL,completed_at_ms INTEGER NOT NULL);
         INSERT INTO schema_migrations VALUES(6,X'06',0);
         CREATE TABLE marker(value INTEGER);
         INSERT INTO marker VALUES(11);",
    )
    .unwrap();
    drop(conn);

    assert!(Store::open(&path).is_err());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT value FROM marker", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        11
    );
}

#[test]
fn current_v6_open_rejects_missing_inner_event_projection_row() {
    let (path, event) = legacy_v5_event_database("autonomy-v6-missing-projection", true);
    drop(Store::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "DELETE FROM inner_event WHERE event_id=?1",
        params![event.event_id.to_vec()],
    )
    .unwrap();
    drop(conn);

    assert!(Store::open(&path).is_err());
}

#[test]
fn current_v6_open_rejects_manifest_and_materialization_bound_tamper() {
    for (name, sql) in [
        (
            "autonomy-v6-missing-manifest",
            "DELETE FROM inner_event_manifest",
        ),
        (
            "autonomy-v6-zero-manifest",
            "UPDATE inner_event_manifest SET event_count=0",
        ),
        (
            "autonomy-v6-negative-manifest",
            "UPDATE inner_event_manifest SET event_count=-1",
        ),
        (
            "autonomy-v6-oversized-delta",
            "UPDATE journal SET delta_bytes=zeroblob(1048577)",
        ),
        (
            "autonomy-v6-oversized-runtime",
            "INSERT INTO autonomous_runtime_state(persona_scope,generation,state_revision,body_json) VALUES(zeroblob(32),0,0,printf('%.*c',262145,'x'))",
        ),
        (
            "autonomy-v6-oversized-runtime-utf8",
            "INSERT INTO autonomous_runtime_state(persona_scope,generation,state_revision,body_json) VALUES(zeroblob(32),0,0,replace(hex(zeroblob(90000)),'00','界'))",
        ),
        (
            "autonomy-v6-oversized-snapshot",
            "INSERT INTO autonomy_snapshot(persona_scope,journal_revision,state_digest,state_bytes) VALUES(zeroblob(32),1,zeroblob(32),zeroblob(262145))",
        ),
    ] {
        let (path, _) = legacy_v5_event_database(name, true);
        drop(Store::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        if sql.starts_with("INSERT INTO autonomous_runtime_state")
            || sql.starts_with("INSERT INTO autonomy_snapshot")
        {
            // V9 prevents writes to these historical tables at the SQL boundary.
            let error = conn.execute_batch(sql).unwrap_err();
            assert!(error.to_string().contains("CORE_BOUNDARY_LEGACY_WRITE_DENIED"));
            drop(conn);
            assert!(Store::open(&path).is_ok(), "denied mutation changed {name}");
            continue;
        }
        conn.execute_batch(sql).unwrap();
        drop(conn);

        assert!(Store::open(&path).is_err(), "{name}");
    }
}

#[test]
fn migration_review_v5_backfill_is_bidirectionally_complete_and_repeatable() {
    let (valid_path, event) = legacy_v5_event_database("autonomy-v5-valid", true);
    drop(Store::open(&valid_path).unwrap());
    drop(Store::open(&valid_path).unwrap());
    let conn = Connection::open(&valid_path).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT journal_revision FROM inner_event WHERE event_id=?1",
            params![event.event_id.to_vec()],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT event_count,length(event_digest) FROM inner_event_manifest WHERE persona_scope=?1 AND journal_revision=1",
            params![event.persona_scope.to_vec()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .unwrap(),
        (1, 32)
    );
    drop(conn);

    let (missing_path, _) = legacy_v5_event_database("autonomy-v5-missing", false);
    assert!(Store::open(&missing_path).is_err());
    let conn = Connection::open(&missing_path).unwrap();
    let columns = conn
        .prepare("PRAGMA table_info(inner_event)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(!columns.iter().any(|column| column == "journal_revision"));
    assert_eq!(
        conn.query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        5
    );
}

#[test]
fn migration_review_v5_backfill_rejects_tampered_projection_columns_and_rolls_back() {
    for (name, sql) in [
        (
            "autonomy-v5-bad-time",
            "UPDATE inner_event SET committed_at_utc_ms=committed_at_utc_ms+1",
        ),
        (
            "autonomy-v5-bad-kind",
            "UPDATE inner_event SET kind='tampered'",
        ),
        (
            "autonomy-v5-bad-tombstone",
            "UPDATE inner_event SET tombstoned=1",
        ),
        (
            "autonomy-v5-bad-body",
            "UPDATE inner_event SET body_json='{}'",
        ),
    ] {
        let (path, _) = legacy_v5_event_database(name, true);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(sql).unwrap();
        drop(conn);
        assert!(Store::open(&path).is_err(), "{name}");
        let conn = Connection::open(&path).unwrap();
        let columns = conn
            .prepare("PRAGMA table_info(inner_event)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            !columns.iter().any(|column| column == "journal_revision"),
            "{name}"
        );
    }
}

#[test]
fn orphaned_adapter_call_recovers_unknown_without_second_outbound() {
    let path = db("autonomy-orphan");
    drop(Store::open_legacy_v8_fixture(&path).unwrap());
    let target = target();
    let attempt = OutboundAttemptV1 {
        outbound_id: [12; 16],
        intention_id: [13; 16],
        state: IntentionStateV1::AdapterCallStarted,
        target: target.clone(),
        candidate_ciphertext: vec![14],
        candidate_digest: [15; 32],
        created_at_utc_ms: 1,
        settled_at_utc_ms: None,
        counted_budget_day_start_utc_ms: 0,
    };
    let intention = DurableIntentionV1 {
        schema_version: 1,
        intention_id: attempt.intention_id,
        persona_scope: wire::persona_scope_digest(&target.bot_token,&target.persona_token,None),
        relation_scope: wire::persona_scope_digest(&target.bot_token,&target.persona_token,Some(&target.relation_token)),
        state: IntentionStateV1::DispatchPending,
        action_class: "relationship_connection".into(),
        salience: Fixed::ONE,
        urgency: Fixed::ONE,
        confidence: Fixed::ONE,
        created_at_utc_ms: 1,
        not_before_utc_ms: 1,
        expires_at_utc_ms: 2,
        externalization_attempts: 1,
        semantic_idempotency_digest: [18; 32],
        workspace_mapping_digest: [19; 32],
        workspace_residual: Fixed::ZERO,
        source_event_ids: vec![[20; 16]],
    };
    let conn = Connection::open(&path).unwrap();
    conn.execute("INSERT INTO durable_intention(intention_id,persona_scope,relation_scope,semantic_digest,state,revision,body_json) VALUES(?1,?2,?3,?4,'dispatch_pending',1,?5)", params![intention.intention_id.to_vec(),intention.persona_scope.to_vec(),intention.relation_scope.to_vec(),intention.semantic_idempotency_digest.to_vec(),serde_json::to_string(&intention).unwrap()]).unwrap();
    conn.execute("INSERT INTO outbound_target(binding_digest,relation_scope,generation,body_json) VALUES(?1,?2,1,?3)", params![target.binding_digest.to_vec(), intention.relation_scope.to_vec(), serde_json::to_string(&target).unwrap()]).unwrap();
    conn.execute("INSERT INTO outbound_attempt(outbound_id,intention_id,state,target_digest,body_json) VALUES(?1,?2,'adapter_call_started',?3,?4)", params![attempt.outbound_id.to_vec(),attempt.intention_id.to_vec(),target.binding_digest.to_vec(),serde_json::to_string(&attempt).unwrap()]).unwrap();
    let mut dispatch_claim = DispatchClaimV1 {
        claim_token: [21; 32],
        outbound_id: attempt.outbound_id,
        target,
        candidate_ciphertext: attempt.candidate_ciphertext.clone(),
        preflight_digest: [22; 32],
        relation_policy_revision: 1,
        capability_snapshot_digest: [23; 32],
        frozen_input_digest: [24; 32],
        caller_incarnation: [25; 32],
        lease_deadline_utc_ms: 1,
    };
    dispatch_claim.claim_token=wire::domain_hash(b"ae.dispatch-claim.v2",&[
        &dispatch_claim.preflight_digest,&dispatch_claim.outbound_id,&serde_json::to_vec(&dispatch_claim.target).unwrap(),
        &dispatch_claim.candidate_ciphertext,&dispatch_claim.relation_policy_revision.to_le_bytes(),
        &dispatch_claim.capability_snapshot_digest,&dispatch_claim.frozen_input_digest,&dispatch_claim.caller_incarnation,
        &dispatch_claim.lease_deadline_utc_ms.to_le_bytes()]);
    conn.execute(
        "INSERT INTO autonomy_claim(
             claim_token,claim_kind,record_id,caller_incarnation,lease_deadline_utc_ms,body_json
         ) VALUES(?1,'dispatch',?2,?3,1,?4)",
        params![
            dispatch_claim.claim_token.to_vec(),
            attempt.outbound_id.to_vec(),
            dispatch_claim.caller_incarnation.to_vec(),
            serde_json::to_string(&dispatch_claim).unwrap()
        ],
    )
    .unwrap();
    drop(conn);
    let before=retired_rows(&path);
    let mut store=Store::open(&path).unwrap();
    assert!(matches!(store.recover_orphaned_dispatches(&intention.persona_scope,1),Err(StoreError::ContinuityFence("UNSUPPORTED_CORE_BOUNDARY"))));
    drop(store);
    assert_eq!(retired_rows(&path),before);
    let conn=Connection::open(&path).unwrap();
    let disposition:String=conn.query_row("SELECT effective_disposition FROM core_boundary_disposition_v1 WHERE record_kind=6",[],|r|r.get(0)).unwrap();
    assert_eq!(disposition,"dispatch_unknown_no_retry");
    assert_eq!(conn.query_row("SELECT state FROM core_boundary_control_v1",[],|r|r.get::<_,String>(0)).unwrap(),"applied");
    drop(conn);
    drop(Store::open(&path).unwrap());
    assert_eq!(retired_rows(&path),before);
}

#[test]
fn legacy_v2_uncertain_outbound_bootstraps_anchored_unknown_snapshot() {
    let path = db("autonomy-v2-operational-genesis");
    drop(Store::open_legacy_v8_fixture(&path).unwrap());
    let target = target();
    let attempt = OutboundAttemptV1 {
        outbound_id: [32; 16],
        intention_id: [33; 16],
        state: IntentionStateV1::AdapterCallStarted,
        target: target.clone(),
        candidate_ciphertext: vec![34],
        candidate_digest: [35; 32],
        created_at_utc_ms: 1,
        settled_at_utc_ms: None,
        counted_budget_day_start_utc_ms: 0,
    };
    let intention = DurableIntentionV1 {
        schema_version: 1,
        intention_id: attempt.intention_id,
        persona_scope: [36; 32],
        relation_scope: [37; 32],
        state: IntentionStateV1::DispatchPending,
        action_class: "relationship_connection".into(),
        salience: Fixed::ONE,
        urgency: Fixed::ONE,
        confidence: Fixed::ONE,
        created_at_utc_ms: 1,
        not_before_utc_ms: 1,
        expires_at_utc_ms: 2,
        externalization_attempts: 1,
        semantic_idempotency_digest: [38; 32],
        workspace_mapping_digest: [39; 32],
        workspace_residual: Fixed::ZERO,
        source_event_ids: vec![[40; 16]],
    };
    let conn = Connection::open(&path).unwrap();
    conn.execute("INSERT INTO durable_intention(intention_id,persona_scope,relation_scope,semantic_digest,state,revision,body_json) VALUES(?1,?2,?3,?4,'dispatch_pending',1,?5)", params![intention.intention_id.to_vec(),intention.persona_scope.to_vec(),intention.relation_scope.to_vec(),intention.semantic_idempotency_digest.to_vec(),serde_json::to_string(&intention).unwrap()]).unwrap();
    conn.execute("INSERT INTO outbound_attempt(outbound_id,intention_id,state,target_digest,body_json) VALUES(?1,?2,'adapter_call_started',?3,?4)", params![attempt.outbound_id.to_vec(),attempt.intention_id.to_vec(),target.binding_digest.to_vec(),serde_json::to_string(&attempt).unwrap()]).unwrap();
    drop(conn);
    let before=retired_rows(&path);
    let store=Store::open(&path).unwrap();
    assert!(matches!(store.list_pending_autonomy_work(&intention.persona_scope),Err(StoreError::ContinuityFence("UNSUPPORTED_CORE_BOUNDARY"))));
    drop(store);
    assert_eq!(retired_rows(&path),before);
    drop(Store::open(&path).unwrap());
    assert_eq!(retired_rows(&path),before);
}

#[test]
fn legacy_ready_and_deferred_keep_claimable_revisions_in_pending_work() {
    let path = db("autonomy-v2-pending-revisions");
    drop(Store::open_legacy_v8_fixture(&path).unwrap());
    let persona_scope = [61; 32];
    let relation_scope = [62; 32];
    let make_intention = |id: u8, state, attempts| DurableIntentionV1 {
        schema_version: 1,
        intention_id: [id; 16],
        persona_scope,
        relation_scope,
        state,
        action_class: "relationship_connection".into(),
        salience: Fixed::ONE,
        urgency: Fixed::ONE,
        confidence: Fixed::ONE,
        created_at_utc_ms: 1,
        not_before_utc_ms: 1,
        expires_at_utc_ms: 2,
        externalization_attempts: attempts,
        semantic_idempotency_digest: [id.wrapping_add(20); 32],
        workspace_mapping_digest: [63; 32],
        workspace_residual: Fixed::ZERO,
        source_event_ids: vec![[64; 16]],
    };
    let ready = make_intention(65, IntentionStateV1::Ready, 0);
    let deferred = make_intention(66, IntentionStateV1::Deferred, 1);
    let conn = Connection::open(&path).unwrap();
    for (value, revision, state) in [(&ready, 4_i64, "ready"), (&deferred, 5_i64, "deferred")] {
        conn.execute(
            "INSERT INTO durable_intention(intention_id,persona_scope,relation_scope,semantic_digest,state,revision,body_json) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![value.intention_id.to_vec(), persona_scope.to_vec(), relation_scope.to_vec(), value.semantic_idempotency_digest.to_vec(), state, revision, serde_json::to_string(value).unwrap()],
        )
        .unwrap();
    }
    drop(conn);
    let before=retired_rows(&path);
    let store=Store::open(&path).unwrap();
    assert!(matches!(store.list_pending_autonomy_work(&persona_scope),Err(StoreError::ContinuityFence("UNSUPPORTED_CORE_BOUNDARY"))));
    drop(store);
    assert_eq!(retired_rows(&path),before);
    drop(Store::open(&path).unwrap());
    assert_eq!(retired_rows(&path),before);
}
