use ae_contracts::{wire, ScopeRef};
use ae_store::{Store, StoreError};
use rusqlite::{params, Connection};

fn database(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "ae-perception-schema-v3-{label}-{}-{:?}.db",
        std::process::id(),
        std::thread::current().id()
    ))
}

#[test]
fn malicious_precreated_challenge_table_fails_closed() {
    let path = database("weak-precreate");
    let _ = std::fs::remove_file(&path);
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TABLE perception_challenges(
                request_nonce_digest BLOB PRIMARY KEY
             );",
        )
        .unwrap();
    assert!(matches!(
        Store::open(&path),
        Err(StoreError::ContinuityFence("semantic_schema_version"))
    ));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn canonical_v3_clone_missing_checks_and_context_unique_fails_closed() {
    let path = database("v3-weakened-table");
    let _ = std::fs::remove_file(&path);
    Store::open(&path).unwrap().close().unwrap();

    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "PRAGMA foreign_keys=OFF;
             DROP INDEX perception_challenge_context_v1;
             DROP INDEX perception_challenge_persona_pending_v1;
             DROP INDEX perception_challenge_expiry_v1;
             DROP TABLE perception_challenges;
             CREATE TABLE perception_challenges (
                request_nonce_digest BLOB PRIMARY KEY CHECK(typeof(request_nonce_digest)='blob' AND length(request_nonce_digest)=32),
                challenge_secret BLOB NOT NULL CHECK(typeof(challenge_secret)='blob' AND length(challenge_secret)=32),
                origin_digest BLOB NOT NULL CHECK(typeof(origin_digest)='blob' AND length(origin_digest)=32),
                origin_bytes BLOB NOT NULL CHECK(typeof(origin_bytes)='blob' AND length(origin_bytes)<=4096),
                persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
                bot_token BLOB NOT NULL CHECK(typeof(bot_token)='blob' AND length(bot_token)=16),
                persona_token BLOB NOT NULL CHECK(typeof(persona_token)='blob' AND length(persona_token)=16),
                origin_event_digest BLOB NOT NULL CHECK(typeof(origin_event_digest)='blob' AND length(origin_event_digest)=32),
                origin_journal_revision INTEGER NOT NULL CHECK(typeof(origin_journal_revision)='integer' AND origin_journal_revision>0),
                base_revision INTEGER NOT NULL CHECK(typeof(base_revision)='integer' AND base_revision>=0),
                incarnation_id BLOB NOT NULL CHECK(typeof(incarnation_id)='blob' AND length(incarnation_id)=32),
                manifest_digest BLOB NOT NULL CHECK(typeof(manifest_digest)='blob' AND length(manifest_digest)=32),
                created_at_ms INTEGER NOT NULL,
                expires_at_ms INTEGER NOT NULL CHECK(typeof(expires_at_ms)='integer' AND expires_at_ms>created_at_ms),
                FOREIGN KEY(persona_scope,origin_event_digest)
                  REFERENCES applied_events(scope_digest,event_digest)
             );
             CREATE INDEX perception_challenge_context_v1
               ON perception_challenges(origin_digest,base_revision,incarnation_id);
             CREATE INDEX perception_challenge_persona_pending_v1
               ON perception_challenges(persona_scope);
             CREATE INDEX perception_challenge_expiry_v1
               ON perception_challenges(expires_at_ms);",
        )
        .unwrap();
    drop(connection);

    assert!(matches!(
        Store::open(&path),
        Err(StoreError::ContinuityFence("semantic_schema_sql_identity"))
    ));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn canonical_v3_clone_with_false_partial_nonce_predicate_fails_closed() {
    let path = database("v3-false-partial-index");
    let _ = std::fs::remove_file(&path);
    Store::open(&path).unwrap().close().unwrap();

    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "DROP INDEX semantic_evidence_perception_nonce_v1;
             CREATE UNIQUE INDEX semantic_evidence_perception_nonce_v1
               ON semantic_evidence_authority(perception_nonce_digest)
               WHERE 0;",
        )
        .unwrap();
    drop(connection);

    assert!(matches!(
        Store::open(&path),
        Err(StoreError::ContinuityFence("semantic_schema_sql_identity"))
    ));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn exact_v2_layout_migrates_transactionally_through_v3_v4_to_v5() {
    let path = database("v2-migration");
    let _ = std::fs::remove_file(&path);
    Store::open(&path).unwrap().close().unwrap();

    let mut connection = Connection::open(&path).unwrap();
    connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    let tx = connection.transaction().unwrap();
    tx.execute_batch(
        "DROP INDEX semantic_time_journal_v1;
         DROP TABLE semantic_time_authority;
         ALTER TABLE semantic_commits DROP COLUMN transition_kind;
         ALTER TABLE semantic_cursor DROP COLUMN transition_kind;
         ALTER TABLE semantic_cursor DROP COLUMN time_anchor_semantic_revision;
         ALTER TABLE semantic_cursor DROP COLUMN time_anchor_state_digest;
         ALTER TABLE semantic_cursor DROP COLUMN time_awake_ticks;
         ALTER TABLE semantic_cursor DROP COLUMN time_drowsy_ticks;
         ALTER TABLE semantic_cursor DROP COLUMN time_asleep_ticks;
         ALTER TABLE semantic_cursor DROP COLUMN time_awake_remainder_ms;
         ALTER TABLE semantic_cursor DROP COLUMN time_drowsy_remainder_ms;
         ALTER TABLE semantic_cursor DROP COLUMN time_asleep_remainder_ms;
         DROP INDEX semantic_evidence_perception_nonce_v1;
         DROP INDEX semantic_evidence_relation_revision;
         DROP INDEX perception_challenge_context_v1;
         DROP INDEX perception_challenge_persona_pending_v1;
         DROP INDEX perception_challenge_expiry_v1;
         DROP TABLE perception_challenges;
         ALTER TABLE semantic_evidence_authority RENAME TO __v3_evidence;
         CREATE TABLE semantic_evidence_authority (
            persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
            semantic_revision INTEGER NOT NULL CHECK(typeof(semantic_revision)='integer' AND semantic_revision>0),
            relation_present INTEGER NOT NULL CHECK(typeof(relation_present)='integer' AND relation_present IN (0,1)),
            relation_scope BLOB NOT NULL CHECK(typeof(relation_scope)='blob' AND length(relation_scope)=32),
            event_id BLOB NOT NULL CHECK(typeof(event_id)='blob' AND length(event_id)=16),
            incarnation_id BLOB NOT NULL CHECK(typeof(incarnation_id)='blob' AND length(incarnation_id)=32),
            manifest_digest BLOB NOT NULL CHECK(typeof(manifest_digest)='blob' AND length(manifest_digest)=32),
            route_digest BLOB NOT NULL CHECK(typeof(route_digest)='blob' AND length(route_digest)=32),
            formula_digest BLOB NOT NULL CHECK(typeof(formula_digest)='blob' AND length(formula_digest)=32),
            graph_digest BLOB NOT NULL CHECK(typeof(graph_digest)='blob' AND length(graph_digest)=32),
            state_digest BLOB NOT NULL CHECK(typeof(state_digest)='blob' AND length(state_digest)=32),
            evidence_digest BLOB NOT NULL CHECK(typeof(evidence_digest)='blob' AND length(evidence_digest)=32),
            estimator_digest BLOB NOT NULL CHECK(typeof(estimator_digest)='blob' AND length(estimator_digest)=32),
            event_digest BLOB NOT NULL CHECK(typeof(event_digest)='blob' AND length(event_digest)=32),
            commitment_digest BLOB NOT NULL CHECK(typeof(commitment_digest)='blob' AND length(commitment_digest)=32),
            evidence_bytes BLOB NOT NULL CHECK(typeof(evidence_bytes)='blob' AND length(evidence_bytes)<=262144),
            PRIMARY KEY(persona_scope,relation_present,relation_scope,event_id),
            UNIQUE(persona_scope,semantic_revision),
            CHECK(relation_present=1 OR relation_scope=persona_scope),
            FOREIGN KEY(persona_scope,semantic_revision,relation_present,relation_scope,event_id,
                        incarnation_id,manifest_digest,route_digest,formula_digest,graph_digest,
                        state_digest,evidence_digest,estimator_digest,event_digest,commitment_digest)
              REFERENCES semantic_commits(persona_scope,semantic_revision,relation_present,
                        relation_scope,event_id,incarnation_id,manifest_digest,route_digest,
                        formula_digest,graph_digest,state_digest,evidence_digest,estimator_digest,
                        event_digest,commitment_digest)
         );
         CREATE INDEX semantic_evidence_relation_revision
           ON semantic_evidence_authority(persona_scope,relation_present,relation_scope,semantic_revision);
         DROP TABLE __v3_evidence;
         CREATE TABLE perception_challenges (
            request_nonce_digest BLOB PRIMARY KEY CHECK(typeof(request_nonce_digest)='blob' AND length(request_nonce_digest)=32),
            challenge_secret BLOB NOT NULL CHECK(typeof(challenge_secret)='blob' AND length(challenge_secret)=32),
            scope_digest BLOB NOT NULL CHECK(typeof(scope_digest)='blob' AND length(scope_digest)=32),
            scope_bytes BLOB NOT NULL CHECK(typeof(scope_bytes)='blob' AND length(scope_bytes) BETWEEN 49 AND 65),
            persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
            event_id BLOB NOT NULL CHECK(typeof(event_id)='blob' AND length(event_id)=16),
            turn_id BLOB NOT NULL CHECK(typeof(turn_id)='blob' AND length(turn_id)=16),
            base_revision INTEGER NOT NULL CHECK(typeof(base_revision)='integer' AND base_revision>=0),
            incarnation_id BLOB NOT NULL CHECK(typeof(incarnation_id)='blob' AND length(incarnation_id)=32),
            status INTEGER NOT NULL CHECK(typeof(status)='integer' AND status IN (0,1)),
            consumed_event_digest BLOB,
            consumed_estimator_digest BLOB,
            consumed_semantic_revision INTEGER,
            UNIQUE(scope_digest,scope_bytes,event_id,turn_id,base_revision,incarnation_id),
            CHECK((status=0 AND consumed_event_digest IS NULL
                            AND consumed_estimator_digest IS NULL
                            AND consumed_semantic_revision IS NULL)
               OR (status=1 AND typeof(consumed_event_digest)='blob'
                            AND length(consumed_event_digest)=32
                            AND typeof(consumed_estimator_digest)='blob'
                            AND length(consumed_estimator_digest)=32
                            AND typeof(consumed_semantic_revision)='integer'
                            AND consumed_semantic_revision>0))
         );
         CREATE INDEX perception_challenge_context_v1
           ON perception_challenges(scope_digest,event_id,turn_id,base_revision,incarnation_id);",
    )
    .unwrap();
    tx.execute(
        "UPDATE meta SET value=?1 WHERE key='semantic_schema_version'",
        params![vec![2_u8]],
    )
    .unwrap();
    let scope = ScopeRef {
        bot_token: [31; 16],
        persona_token: [32; 16],
        relation_token: None,
        session_token: [33; 16],
    };
    let scope_bytes = wire::encode_scope(&scope);
    let scope_digest = wire::scope_digest(&scope);
    let persona_scope = wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
    let challenge_secret = [34; 32];
    let event_id = [35; 16];
    let turn_id = [36; 16];
    let base_revision = 0_u64;
    let incarnation_id = [37; 32];
    let base_revision_bytes = base_revision.to_le_bytes();
    let request_nonce_digest = wire::domain_hash(
        b"astr-embodiment/perception-challenge-nonce-v1",
        &[
            &challenge_secret,
            &scope_bytes,
            &scope_digest,
            &event_id,
            &turn_id,
            &base_revision_bytes,
            &incarnation_id,
        ],
    );
    tx.execute(
        "INSERT INTO perception_challenges(
             request_nonce_digest,challenge_secret,scope_digest,scope_bytes,
             persona_scope,event_id,turn_id,base_revision,incarnation_id,status,
             consumed_event_digest,consumed_estimator_digest,consumed_semantic_revision
         ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,0,NULL,NULL,NULL)",
        params![
            request_nonce_digest.to_vec(),
            challenge_secret.to_vec(),
            scope_digest.to_vec(),
            scope_bytes,
            persona_scope.to_vec(),
            event_id.to_vec(),
            turn_id.to_vec(),
            base_revision as i64,
            incarnation_id.to_vec(),
        ],
    )
    .unwrap();
    tx.commit().unwrap();
    drop(connection);

    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch("CREATE TABLE semantic_shadow_namespace(value BLOB);")
        .unwrap();
    drop(connection);
    assert!(matches!(
        Store::open(&path),
        Err(StoreError::ContinuityFence("semantic_schema_object_set"))
    ));
    let connection = Connection::open(&path).unwrap();
    let still_v2: Vec<u8> = connection
        .query_row(
            "SELECT value FROM meta WHERE key='semantic_schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let retained_challenge: i64 = connection
        .query_row("SELECT COUNT(*) FROM perception_challenges", [], |row| {
            row.get(0)
        })
        .unwrap();
    let shadow_retained: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema
             WHERE type='table' AND name='semantic_shadow_namespace')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(still_v2, vec![2]);
    assert_eq!(retained_challenge, 1);
    assert!(shadow_retained);
    connection
        .execute_batch("DROP TABLE semantic_shadow_namespace;")
        .unwrap();
    drop(connection);

    Store::open(&path).unwrap().close().unwrap();
    let connection = Connection::open(&path).unwrap();
    let version: Vec<u8> = connection
        .query_row(
            "SELECT value FROM meta WHERE key='semantic_schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let evidence_columns: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('semantic_evidence_authority')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let challenge_columns: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('perception_challenges')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let challenge_rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM perception_challenges", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(version, vec![5]);
    assert_eq!(evidence_columns, 20);
    assert_eq!(challenge_columns, 14);
    assert_eq!(challenge_rows, 0);
    drop(connection);
    let _ = std::fs::remove_file(&path);
}
