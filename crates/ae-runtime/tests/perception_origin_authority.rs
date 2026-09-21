use ae_contracts::*;
use ae_fixed::Fixed;
use ae_runtime::{AstrRuntime, RuntimeError};
use ae_store::StoreError;
use rusqlite::Connection;
use std::sync::{Arc, Barrier};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
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
        observed_at_ms: now_ms(),
    }
}

fn relation_scope(request: &PersonaGenesisRequest) -> ScopeRef {
    ScopeRef {
        bot_token: request.source.scope.bot_token,
        persona_token: request.source.scope.persona_token,
        relation_token: Some([0x31; 16]),
        session_token: [0x32; 16],
    }
}

fn bootstrap(runtime: &mut AstrRuntime, scope: &ScopeRef) {
    let persona_scope = wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
    let relation_scope = wire::persona_scope_digest(
        &scope.bot_token,
        &scope.persona_token,
        scope.relation_token.as_ref(),
    );
    runtime
        .bootstrap_autonomy(
            scope,
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
}

fn inbound(scope: &ScopeRef, authority: InteractionSourceAuthorityV1) -> InteractionFactBatchV1 {
    InteractionFactBatchV1 {
        schema_version: ALPHA3_SCHEMA_VERSION,
        event_id: [0x41; 16],
        scope: scope.clone(),
        causal: CausalRef {
            turn_id: [0x42; 16],
            action_id: None,
            delivery_id: None,
            claim_id: None,
            base_revision: 0,
        },
        facts: vec![InteractionFactV1 {
            fact_id: [0x43; 16],
            kind: InteractionFactKindV1::InboundObserved,
            observed_at_utc_ms: now_ms(),
            source_authority: authority,
            source_digest: [0x44; 32],
            extractor_digest: [0x45; 32],
            confidence: Fixed::ONE,
            value_code: None,
            subject_public_ref: None,
            consent_terms: None,
            scheduled_at_utc_ms: None,
            expires_at_utc_ms: None,
        }],
    }
}

fn distinct_inbound(
    runtime: &mut AstrRuntime,
    scope: &ScopeRef,
    ordinal: u16,
) -> InteractionFactBatchV1 {
    assert_ne!(ordinal, 0);
    let mut batch = inbound(scope, InteractionSourceAuthorityV1::AstrbotMetadata);
    let mut identity = [0_u8; 16];
    identity[..2].copy_from_slice(&ordinal.to_le_bytes());
    batch.event_id = identity;
    batch.causal.turn_id = identity;
    batch.causal.base_revision = runtime.current_revision(scope).unwrap();
    batch.facts[0].fact_id = identity;
    batch.facts[0].source_digest[..2].copy_from_slice(&ordinal.to_le_bytes());
    batch
}

fn proposal(challenge: &PerceptionChallengeV1) -> PerceptionProposalV1 {
    PerceptionProposalV1 {
        schema_version: PerceptionProposalV1::SCHEMA_VERSION,
        origin_digest: challenge.origin.origin_digest,
        dimensions: EvidenceVector {
            positive: Fixed::from_raw(700_000),
            engagement: Fixed::from_raw(800_000),
            ..EvidenceVector::default()
        },
        estimator_confidence: Fixed::from_raw(900_000),
        protocol_version: PerceptionProposalV1::PROTOCOL_VERSION,
        request_nonce_digest: challenge.request_nonce_digest,
    }
}

fn appraisal_begin(batch: InteractionFactBatchV1) -> SemanticAppraisalBeginRequestV1 {
    SemanticAppraisalBeginRequestV1 {
        schema_version: SemanticAppraisalBeginRequestV1::SCHEMA_VERSION,
        interaction: batch,
        daily_token_limit: 16_384,
        reserved_tokens: 1_024,
        provider_digest: [0x46; 32],
    }
}

fn database(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "ae-perception-origin-{label}-{}-{:?}.db",
        std::process::id(),
        std::thread::current().id()
    ))
}

fn downgrade_appraisal_v3_to_exact_v2_with_legacy_terminal(path: &std::path::Path) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            "UPDATE semantic_appraisal_claim SET
               usage_known=NULL,usage_tokens=NULL,proposal_identity_digest=NULL,
               settlement_identity_digest=NULL,reply_affect_bytes=NULL,
               reply_affect_digest=NULL,terminal_receipt_bytes=NULL,
               terminal_receipt_digest=NULL;",
        )
        .unwrap();
    drop(connection);
    downgrade_appraisal_v3_to_exact_v2(path);
}

fn downgrade_appraisal_v3_to_exact_v2(path: &std::path::Path) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            "PRAGMA foreign_keys=OFF;
             PRAGMA legacy_alter_table=ON;
             DROP INDEX semantic_appraisal_claim_retention_v3;
             DROP INDEX semantic_appraisal_claim_budget_v3;
             DROP INDEX semantic_appraisal_budget_retention_v3;
             DROP TABLE semantic_appraisal_rollup;
             ALTER TABLE semantic_appraisal_budget
               RENAME TO __ae_semantic_appraisal_budget_v3;
             CREATE TABLE IF NOT EXISTS semantic_appraisal_budget (
               persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
               utc_day INTEGER NOT NULL CHECK(typeof(utc_day)='integer' AND utc_day>=0),
               daily_token_limit INTEGER NOT NULL CHECK(typeof(daily_token_limit)='integer' AND daily_token_limit>=0 AND daily_token_limit<=1000000),
               charged_tokens INTEGER NOT NULL CHECK(typeof(charged_tokens)='integer' AND charged_tokens>=0),
               reserved_tokens INTEGER NOT NULL CHECK(typeof(reserved_tokens)='integer' AND reserved_tokens>=0),
               blocked INTEGER NOT NULL CHECK(typeof(blocked)='integer' AND blocked IN (0,1)),
               updated_at_ms INTEGER NOT NULL CHECK(typeof(updated_at_ms)='integer' AND updated_at_ms>0),
               PRIMARY KEY(persona_scope,utc_day)
             );
             INSERT INTO semantic_appraisal_budget(
               persona_scope,utc_day,daily_token_limit,charged_tokens,
               reserved_tokens,blocked,updated_at_ms
             ) SELECT persona_scope,utc_day,daily_token_limit,charged_tokens,
                      reserved_tokens,blocked,updated_at_ms
                 FROM __ae_semantic_appraisal_budget_v3;
             DROP TABLE __ae_semantic_appraisal_budget_v3;
             CREATE INDEX IF NOT EXISTS semantic_appraisal_claim_pending_v1
               ON semantic_appraisal_claim(persona_scope,settled_at_ms);
             UPDATE meta SET value=X'02'
               WHERE key='semantic_appraisal_schema_version';",
        )
        .unwrap();
    assert!(connection
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query([])
        .unwrap()
        .next()
        .unwrap()
        .is_none());
}

fn rewrite_legacy_terminal_as_budget_exhausted(
    path: &std::path::Path,
    request_nonce_digest: Digest,
) {
    let mut connection = Connection::open(path).unwrap();
    let transaction = connection.transaction().unwrap();
    assert_eq!(
        transaction
            .execute(
                "UPDATE semantic_appraisal_claim
                    SET request_nonce_digest=?1,reserved_tokens=0,
                        settled_at_ms=created_at_ms,charged_tokens=0,
                        outcome_code='budget_exhausted',semantic_revision=NULL
                  WHERE settled_at_ms IS NOT NULL",
                [request_nonce_digest.to_vec()],
            )
            .unwrap(),
        1
    );
    assert_eq!(
        transaction
            .execute(
                "UPDATE semantic_appraisal_budget
                    SET daily_token_limit=0,charged_tokens=0,reserved_tokens=0,blocked=0,
                        updated_at_ms=(SELECT created_at_ms FROM semantic_appraisal_claim)",
                [],
            )
            .unwrap(),
        1
    );
    transaction
        .execute("DELETE FROM perception_challenges", [])
        .unwrap();
    transaction.commit().unwrap();
}

fn downgrade_appraisal_v2_to_exact_v1(path: &std::path::Path) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            "PRAGMA foreign_keys=OFF;
             PRAGMA legacy_alter_table=ON;
             DROP INDEX semantic_appraisal_claim_pending_v1;
             ALTER TABLE semantic_appraisal_claim
               RENAME TO __ae_semantic_appraisal_claim_v2;
             CREATE TABLE IF NOT EXISTS semantic_appraisal_claim (
                 request_nonce_digest BLOB PRIMARY KEY CHECK(typeof(request_nonce_digest)='blob' AND length(request_nonce_digest)=32),
                 persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
                 utc_day INTEGER NOT NULL CHECK(typeof(utc_day)='integer' AND utc_day>=0),
                 origin_event_digest BLOB NOT NULL UNIQUE CHECK(typeof(origin_event_digest)='blob' AND length(origin_event_digest)=32),
                 origin_digest BLOB NOT NULL CHECK(typeof(origin_digest)='blob' AND length(origin_digest)=32),
                 provider_digest BLOB NOT NULL CHECK(typeof(provider_digest)='blob' AND length(provider_digest)=32),
                 reserved_tokens INTEGER NOT NULL CHECK(typeof(reserved_tokens)='integer' AND reserved_tokens>=0 AND reserved_tokens<=1000000),
                 created_at_ms INTEGER NOT NULL CHECK(typeof(created_at_ms)='integer' AND created_at_ms>0),
                 settled_at_ms INTEGER,
                 charged_tokens INTEGER,
                 outcome_code TEXT,
                 canonical_revision INTEGER,
                 semantic_revision INTEGER,
                 CHECK(
                   (settled_at_ms IS NULL AND reserved_tokens>=768 AND charged_tokens IS NULL AND outcome_code IS NULL AND canonical_revision IS NULL AND semantic_revision IS NULL)
                   OR
                   (typeof(settled_at_ms)='integer' AND settled_at_ms>=created_at_ms
                    AND typeof(charged_tokens)='integer' AND charged_tokens>=0
                    AND typeof(outcome_code)='text' AND length(CAST(outcome_code AS BLOB)) BETWEEN 1 AND 32
                    AND typeof(canonical_revision)='integer' AND canonical_revision>=0
                    AND (semantic_revision IS NULL OR (typeof(semantic_revision)='integer' AND semantic_revision>0)))
                 ),
                 FOREIGN KEY(persona_scope,utc_day)
                   REFERENCES semantic_appraisal_budget(persona_scope,utc_day)
             );
             INSERT INTO semantic_appraisal_claim(
                 request_nonce_digest,persona_scope,utc_day,origin_event_digest,origin_digest,
                 provider_digest,reserved_tokens,created_at_ms,settled_at_ms,charged_tokens,
                 outcome_code,canonical_revision,semantic_revision
              ) SELECT request_nonce_digest,persona_scope,utc_day,origin_event_digest,origin_digest,
                       provider_digest,reserved_tokens,created_at_ms,settled_at_ms,charged_tokens,
                       outcome_code,canonical_revision,semantic_revision
                  FROM __ae_semantic_appraisal_claim_v2;
             DROP TABLE __ae_semantic_appraisal_claim_v2;
             CREATE INDEX IF NOT EXISTS semantic_appraisal_claim_pending_v1
               ON semantic_appraisal_claim(persona_scope,settled_at_ms);
             UPDATE meta SET value=X'01'
               WHERE key='semantic_appraisal_schema_version';",
        )
        .unwrap();
    assert!(connection
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query([])
        .unwrap()
        .next()
        .unwrap()
        .is_none());
}

fn rewrite_terminal_as_authenticated_budget_exhausted(path: &std::path::Path) {
    let mut connection = Connection::open(path).unwrap();
    let (mut receipt_bytes, usage_known, usage_tokens, proposal_identity, semantic_revision): (
        Vec<u8>,
        i64,
        Option<i64>,
        Option<Vec<u8>>,
        Option<i64>,
    ) = connection
        .query_row(
            "SELECT terminal_receipt_bytes,usage_known,usage_tokens,
                    proposal_identity_digest,semantic_revision
               FROM semantic_appraisal_claim WHERE settled_at_ms IS NOT NULL",
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
        .unwrap();
    assert_eq!(
        (
            usage_known,
            usage_tokens,
            proposal_identity,
            semantic_revision
        ),
        (0, None, None, None)
    );

    let original = b"\"outcome_code\":\"timeout\"";
    let replacement = b"\"outcome_code\":\"budget_exhausted\"";
    let offset = receipt_bytes
        .windows(original.len())
        .position(|window| window == original)
        .expect("real timeout receipt must retain its canonical outcome field");
    receipt_bytes.splice(offset..offset + original.len(), replacement.iter().copied());
    assert!(!receipt_bytes
        .windows(original.len())
        .any(|window| window == original));
    let original_charge = b"\"charged_tokens\":1024";
    let replacement_charge = b"\"charged_tokens\":0";
    let charge_offset = receipt_bytes
        .windows(original_charge.len())
        .position(|window| window == original_charge)
        .expect("real timeout receipt must commit its full reservation charge");
    receipt_bytes.splice(
        charge_offset..charge_offset + original_charge.len(),
        replacement_charge.iter().copied(),
    );

    let settlement_identity = wire::domain_hash(
        b"astr-embodiment/semantic-appraisal-settlement-v1",
        &[
            b"budget_exhausted",
            &[0],
            &[0],
            &0_u32.to_le_bytes(),
            &[0],
            &[0_u8; 32],
        ],
    );
    let receipt_digest = wire::domain_hash(
        b"astr-embodiment/semantic-appraisal-terminal-receipt-v1",
        &[receipt_bytes.as_slice()],
    );
    let transaction = connection.transaction().unwrap();
    assert_eq!(
        transaction
            .execute(
                "UPDATE semantic_appraisal_claim
                    SET reserved_tokens=0,settled_at_ms=created_at_ms,charged_tokens=0,
                        outcome_code='budget_exhausted',semantic_revision=NULL,
                        settlement_identity_digest=?1,
                        terminal_receipt_bytes=?2,
                        terminal_receipt_digest=?3
                  WHERE settled_at_ms IS NOT NULL AND outcome_code='timeout'",
                rusqlite::params![
                    settlement_identity.to_vec(),
                    receipt_bytes,
                    receipt_digest.to_vec()
                ],
            )
            .unwrap(),
        1
    );
    assert_eq!(
        transaction
            .execute(
                "UPDATE semantic_appraisal_budget
                    SET daily_token_limit=0,charged_tokens=0,reserved_tokens=0,blocked=0,
                        updated_at_ms=(SELECT created_at_ms FROM semantic_appraisal_claim)",
                [],
            )
            .unwrap(),
        1
    );
    let challenge_rows: i64 = transaction
        .query_row("SELECT COUNT(*) FROM perception_challenges", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(challenge_rows, 0);
    transaction.commit().unwrap();
}

fn rewrite_authenticated_budget_exhausted_charge(
    path: &std::path::Path,
    old_charge: u64,
    new_charge: u64,
) {
    let mut connection = Connection::open(path).unwrap();
    let mut receipt_bytes: Vec<u8> = connection
        .query_row(
            "SELECT terminal_receipt_bytes FROM semantic_appraisal_claim
             WHERE outcome_code='budget_exhausted'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let original = format!("\"charged_tokens\":{old_charge}").into_bytes();
    let replacement = format!("\"charged_tokens\":{new_charge}").into_bytes();
    let offset = receipt_bytes
        .windows(original.len())
        .position(|window| window == original)
        .expect("canonical receipt must contain the old charge");
    receipt_bytes.splice(offset..offset + original.len(), replacement.iter().copied());
    let receipt_digest = wire::domain_hash(
        b"astr-embodiment/semantic-appraisal-terminal-receipt-v1",
        &[receipt_bytes.as_slice()],
    );
    let transaction = connection.transaction().unwrap();
    assert_eq!(
        transaction
            .execute(
                "UPDATE semantic_appraisal_claim
                    SET reserved_tokens=?1,charged_tokens=?1,
                        terminal_receipt_bytes=?2,terminal_receipt_digest=?3
                  WHERE outcome_code='budget_exhausted'",
                rusqlite::params![
                    i64::try_from(new_charge).unwrap(),
                    receipt_bytes,
                    receipt_digest.to_vec()
                ],
            )
            .unwrap(),
        1
    );
    assert_eq!(
        transaction
            .execute(
                "UPDATE semantic_appraisal_budget
                    SET charged_tokens=?1,blocked=CASE WHEN ?1=0 THEN 0 ELSE 1 END",
                [i64::try_from(new_charge).unwrap()],
            )
            .unwrap(),
        1
    );
    transaction.commit().unwrap();
}

#[test]
#[cfg(feature = "legacy-semantic-test-api")]
fn committed_host_inbound_is_the_only_mint_authority_and_retry_is_exact() {
    let path = database("valid");
    let _ = std::fs::remove_file(&path);
    let request = genesis(0x21);
    let scope = relation_scope(&request);
    let mut runtime = AstrRuntime::open(&path).unwrap();
    runtime.ensure_genesis(&request).unwrap();
    bootstrap(&mut runtime, &scope);
    assert!(runtime
        .mint_perception_challenge_from_committed_inbound_v1([0x99; 32])
        .is_err());
    let batch = inbound(&scope, InteractionSourceAuthorityV1::AstrbotMetadata);
    let observed_at_ms = batch.facts[0].observed_at_utc_ms;
    let result = runtime.apply_interaction_fact_batch_v1(&batch).unwrap();
    let challenge = runtime
        .mint_perception_challenge_from_committed_inbound_v1(result.receipt.event_digest)
        .unwrap();
    assert_eq!(
        challenge.origin.source_authority,
        SourceAuthority::UserObserved
    );
    assert_eq!(challenge.origin.scope, scope);
    assert_eq!(challenge.origin.event_id, [0x42; 16]);
    assert_ne!(challenge.origin.event_id, batch.facts[0].fact_id);
    assert_eq!(challenge.origin.turn_id, [0x42; 16]);
    assert_eq!(challenge.origin.source_digest, [0x44; 32]);
    assert_eq!(challenge.origin.model_digest, [0x45; 32]);
    assert_eq!(challenge.origin.observed_at_ms, observed_at_ms);
    assert_eq!(
        challenge.origin.canonical_base_revision,
        result.receipt.canonical_revision
    );
    assert!(challenge.expires_at_ms > challenge.created_at_ms);

    let proposal = proposal(&challenge);
    let inserted = runtime
        .apply_perception_proposal_v1(&scope, &proposal)
        .unwrap();
    let existing = runtime
        .apply_perception_proposal_v1(&scope, &proposal)
        .unwrap();
    assert!(!inserted.deduplicated);
    assert!(existing.deduplicated);
    assert_eq!(inserted.receipt, existing.receipt);
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM perception_challenges", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM semantic_evidence_authority WHERE perception_nonce_digest IS NOT NULL", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
    drop(connection);
    drop(runtime);
    let _ = std::fs::remove_file(&path);
}

#[test]
#[cfg(feature = "legacy-semantic-test-api")]
fn wrong_origin_nonce_scope_and_non_host_source_write_no_semantics() {
    let path = database("wrong");
    let _ = std::fs::remove_file(&path);
    let request = genesis(0x51);
    let scope = relation_scope(&request);
    let mut runtime = AstrRuntime::open(&path).unwrap();
    runtime.ensure_genesis(&request).unwrap();
    bootstrap(&mut runtime, &scope);
    let batch = inbound(&scope, InteractionSourceAuthorityV1::AstrbotMetadata);
    let observed = batch.facts[0].observed_at_utc_ms;
    let result = runtime.apply_interaction_fact_batch_v1(&batch).unwrap();
    let challenge = runtime
        .mint_perception_challenge_from_committed_inbound_v1(result.receipt.event_digest)
        .unwrap();
    assert_eq!(challenge.origin.observed_at_ms, observed);
    let before = runtime.semantic_revision_v1(&scope).unwrap();
    let mut wrong = proposal(&challenge);
    wrong.origin_digest[0] ^= 1;
    assert!(runtime
        .apply_perception_proposal_v1(&scope, &wrong)
        .is_err());
    wrong = proposal(&challenge);
    wrong.request_nonce_digest[0] ^= 1;
    assert!(runtime
        .apply_perception_proposal_v1(&scope, &wrong)
        .is_err());
    let mut wrong_scope = scope.clone();
    wrong_scope.session_token[0] ^= 1;
    assert!(runtime
        .apply_perception_proposal_v1(&wrong_scope, &proposal(&challenge))
        .is_err());
    assert_eq!(runtime.semantic_revision_v1(&scope).unwrap(), before);
    drop(runtime);
    let _ = std::fs::remove_file(&path);

    let path = database("non-host");
    let _ = std::fs::remove_file(&path);
    let request = genesis(0x71);
    let scope = relation_scope(&request);
    let mut runtime = AstrRuntime::open(&path).unwrap();
    runtime.ensure_genesis(&request).unwrap();
    bootstrap(&mut runtime, &scope);
    let result = runtime
        .apply_interaction_fact_batch_v1(&inbound(
            &scope,
            InteractionSourceAuthorityV1::ModelCandidate,
        ))
        .unwrap();
    assert!(runtime
        .mint_perception_challenge_from_committed_inbound_v1(result.receipt.event_digest)
        .is_err());
    assert_eq!(runtime.semantic_revision_v1(&scope).unwrap(), 0);
    drop(runtime);
    let _ = std::fs::remove_file(&path);
}

#[test]
#[cfg(feature = "legacy-semantic-test-api")]
fn consumed_origin_with_missing_materialized_inbound_fails_retry_and_reopen() {
    let path = database("consumed-missing-source");
    let _ = std::fs::remove_file(&path);
    let request = genesis(0x81);
    let scope = relation_scope(&request);
    let mut runtime = AstrRuntime::open(&path).unwrap();
    runtime.ensure_genesis(&request).unwrap();
    bootstrap(&mut runtime, &scope);
    let batch = inbound(&scope, InteractionSourceAuthorityV1::AstrbotMetadata);
    let fact_id = batch.facts[0].fact_id;
    let result = runtime.apply_interaction_fact_batch_v1(&batch).unwrap();
    let challenge = runtime
        .mint_perception_challenge_from_committed_inbound_v1(result.receipt.event_digest)
        .unwrap();
    let proposal = proposal(&challenge);
    runtime
        .apply_perception_proposal_v1(&scope, &proposal)
        .unwrap();

    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch("PRAGMA foreign_keys=OFF;")
        .unwrap();
    assert_eq!(
        connection
            .execute(
                "DELETE FROM interaction_fact WHERE fact_id=?1",
                [fact_id.to_vec()],
            )
            .unwrap(),
        1
    );
    drop(connection);

    assert!(runtime.audit_semantic_integrity_v1().is_err());
    assert!(runtime
        .apply_perception_proposal_v1(&scope, &proposal)
        .is_err());
    drop(runtime);
    assert!(AstrRuntime::open(&path).is_err());
    let _ = std::fs::remove_file(&path);
}

#[test]
#[cfg(feature = "legacy-semantic-test-api")]
fn consumed_origin_with_self_consistent_but_tampered_source_fact_fails_closed() {
    let path = database("consumed-tampered-source");
    let _ = std::fs::remove_file(&path);
    let request = genesis(0x91);
    let scope = relation_scope(&request);
    let mut runtime = AstrRuntime::open(&path).unwrap();
    runtime.ensure_genesis(&request).unwrap();
    bootstrap(&mut runtime, &scope);
    let batch = inbound(&scope, InteractionSourceAuthorityV1::AstrbotMetadata);
    let fact_id = batch.facts[0].fact_id;
    let result = runtime.apply_interaction_fact_batch_v1(&batch).unwrap();
    let challenge = runtime
        .mint_perception_challenge_from_committed_inbound_v1(result.receipt.event_digest)
        .unwrap();
    let proposal = proposal(&challenge);
    runtime
        .apply_perception_proposal_v1(&scope, &proposal)
        .unwrap();

    let connection = Connection::open(&path).unwrap();
    let body_json: String = connection
        .query_row(
            "SELECT body_json FROM interaction_fact WHERE fact_id=?1",
            [fact_id.to_vec()],
            |row| row.get(0),
        )
        .unwrap();
    let mut materialized: InteractionFactV1 = serde_json::from_str(&body_json).unwrap();
    materialized.source_digest = [0xEE; 32];
    connection
        .execute(
            "UPDATE interaction_fact SET source_digest=?2,body_json=?3 WHERE fact_id=?1",
            rusqlite::params![
                fact_id.to_vec(),
                materialized.source_digest.to_vec(),
                serde_json::to_string(&materialized).unwrap(),
            ],
        )
        .unwrap();
    drop(connection);

    assert!(runtime.audit_semantic_integrity_v1().is_err());
    assert!(runtime
        .apply_perception_proposal_v1(&scope, &proposal)
        .is_err());
    drop(runtime);
    assert!(AstrRuntime::open(&path).is_err());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn appraisal_settle_rebases_after_delivery_and_exact_retry_does_not_recharge() {
    let path = database("appraisal-rebase");
    let _ = std::fs::remove_file(&path);
    let genesis_request = genesis(0xA1);
    let scope = relation_scope(&genesis_request);
    let mut runtime = AstrRuntime::open(&path).unwrap();
    runtime.ensure_genesis(&genesis_request).unwrap();
    bootstrap(&mut runtime, &scope);

    let begin = runtime
        .begin_semantic_appraisal_v1(&appraisal_begin(inbound(
            &scope,
            InteractionSourceAuthorityV1::AstrbotMetadata,
        )))
        .unwrap();
    assert_eq!(begin.status, SemanticAppraisalBeginStatusV1::Claimed);
    assert_eq!(begin.budget.as_ref().unwrap().reserved_tokens, 1_024);
    assert_eq!(
        begin.settlement_nonce_digest,
        begin
            .challenge
            .as_ref()
            .map(|challenge| challenge.request_nonce_digest)
    );
    let challenge = begin.challenge.unwrap();
    let origin_revision = challenge.origin.origin_journal_revision;

    let delivery_base = runtime.current_revision(&scope).unwrap();
    let delivery = CanonicalEvent::DeliveryOutcome(DeliveryOutcome {
        event_id: [0x51; 16],
        scope: scope.clone(),
        causal: CausalRef {
            turn_id: [0x52; 16],
            action_id: None,
            delivery_id: None,
            claim_id: None,
            base_revision: delivery_base,
        },
        delivered: true,
        visible_action_digest: [0x53; 32],
        delivered_at_ms: now_ms(),
    });
    runtime.apply_event(&scope, &delivery).unwrap();
    let after_delivery = runtime.current_revision(&scope).unwrap();
    assert!(after_delivery > origin_revision);

    let proposal = proposal(&challenge);
    let settle_request = SemanticAppraisalSettleRequestV1 {
        schema_version: SemanticAppraisalSettleRequestV1::SCHEMA_VERSION,
        scope: scope.clone(),
        request_nonce_digest: challenge.request_nonce_digest,
        outcome: SemanticAppraisalOutcomeV1::Success,
        provider_usage: SemanticAppraisalProviderUsageV1 {
            known: true,
            used_tokens: Some(500),
        },
        proposal: Some(proposal.clone()),
    };
    let settled = runtime
        .settle_semantic_appraisal_v1(&settle_request)
        .unwrap();
    assert_eq!(settled.status, SemanticAppraisalSettleStatusV1::Committed);
    assert_eq!(settled.canonical_revision, after_delivery + 1);
    assert_eq!(settled.charged_tokens, 500);
    assert!(settled.semantic_revision.is_some());
    assert!(settled.contract.is_some());
    assert!(settled.reply_affect.is_some());

    let persona = wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
    let committed_event: Vec<u8> = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT event_bytes FROM journal WHERE scope_digest=?1 AND logical_revision=?2",
            rusqlite::params![persona.to_vec(), settled.canonical_revision as i64],
            |row| row.get(0),
        )
        .unwrap();
    match wire::decode_event(&committed_event).unwrap() {
        CanonicalEvent::UserStimulus(stimulus) => {
            assert_eq!(stimulus.causal.base_revision, after_delivery);
        }
        other => panic!("unexpected semantic event: {other:?}"),
    }

    let retried = runtime
        .settle_semantic_appraisal_v1(&settle_request)
        .unwrap();
    assert_eq!(retried.canonical_revision, settled.canonical_revision);
    assert_eq!(retried.semantic_revision, settled.semantic_revision);
    assert_eq!(retried.charged_tokens, settled.charged_tokens);
    let mut drifted_usage = settle_request.clone();
    drifted_usage.provider_usage.used_tokens = Some(501);
    assert!(runtime
        .settle_semantic_appraisal_v1(&drifted_usage)
        .is_err());
    let connection = Connection::open(&path).unwrap();
    let accounting: (i64, i64) = connection
        .query_row(
            "SELECT charged_tokens,reserved_tokens FROM semantic_appraisal_budget",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(accounting, (500, 0));
    drop(connection);
    drop(runtime);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn reopening_a_second_runtime_preserves_a_fresh_appraisal_claim() {
    let path = database("appraisal-reopen-race");
    let _ = std::fs::remove_file(&path);
    let genesis_request = genesis(0xA7);
    let scope = relation_scope(&genesis_request);
    let mut first = AstrRuntime::open(&path).unwrap();
    first.ensure_genesis(&genesis_request).unwrap();
    bootstrap(&mut first, &scope);

    let challenge = first
        .begin_semantic_appraisal_v1(&appraisal_begin(inbound(
            &scope,
            InteractionSourceAuthorityV1::AstrbotMetadata,
        )))
        .unwrap()
        .challenge
        .expect("fresh appraisal must be claimed");

    let second = AstrRuntime::open(&path).expect("a concurrent reopen is read-safe");
    drop(second);

    let settled = first
        .settle_semantic_appraisal_v1(&SemanticAppraisalSettleRequestV1 {
            schema_version: SemanticAppraisalSettleRequestV1::SCHEMA_VERSION,
            scope: scope.clone(),
            request_nonce_digest: challenge.request_nonce_digest,
            outcome: SemanticAppraisalOutcomeV1::Success,
            provider_usage: SemanticAppraisalProviderUsageV1 {
                known: true,
                used_tokens: Some(500),
            },
            proposal: Some(proposal(&challenge)),
        })
        .expect("the original owner must still settle its nonce");
    assert_eq!(settled.status, SemanticAppraisalSettleStatusV1::Committed);
    assert_eq!(settled.charged_tokens, 500);

    drop(first);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn authenticated_legacy_v2_terminal_migrates_then_retries_as_unknown() {
    let path = database("appraisal-v2-terminal-migration");
    let _ = std::fs::remove_file(&path);
    let genesis_request = genesis(0xAA);
    let scope = relation_scope(&genesis_request);
    let mut runtime = AstrRuntime::open(&path).unwrap();
    runtime.ensure_genesis(&genesis_request).unwrap();
    bootstrap(&mut runtime, &scope);

    let challenge = runtime
        .begin_semantic_appraisal_v1(&appraisal_begin(inbound(
            &scope,
            InteractionSourceAuthorityV1::AstrbotMetadata,
        )))
        .unwrap()
        .challenge
        .unwrap();
    let settle_request = SemanticAppraisalSettleRequestV1 {
        schema_version: SemanticAppraisalSettleRequestV1::SCHEMA_VERSION,
        scope: scope.clone(),
        request_nonce_digest: challenge.request_nonce_digest,
        outcome: SemanticAppraisalOutcomeV1::Success,
        provider_usage: SemanticAppraisalProviderUsageV1 {
            known: true,
            used_tokens: Some(377),
        },
        proposal: Some(proposal(&challenge)),
    };
    runtime
        .settle_semantic_appraisal_v1(&settle_request)
        .unwrap();
    drop(runtime);

    downgrade_appraisal_v3_to_exact_v2_with_legacy_terminal(&path);
    let mut migrated = AstrRuntime::open(&path).expect("authenticated V2 terminal must migrate");
    let inspect = Connection::open(&path).unwrap();
    let migrated_shape: (Vec<u8>, i64, i64) = inspect
        .query_row(
            "SELECT (SELECT value FROM meta
                       WHERE key='semantic_appraisal_schema_version'),
                    COUNT(*),
                    COUNT(*) FILTER (
                      WHERE usage_known IS NULL AND usage_tokens IS NULL
                        AND proposal_identity_digest IS NULL
                        AND settlement_identity_digest IS NULL
                        AND reply_affect_bytes IS NULL
                        AND reply_affect_digest IS NULL
                        AND terminal_receipt_bytes IS NULL
                        AND terminal_receipt_digest IS NULL
                    )
               FROM semantic_appraisal_claim",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(migrated_shape, (vec![3], 1, 1));
    drop(inspect);

    assert!(matches!(
        migrated.settle_semantic_appraisal_v1(&settle_request),
        Err(RuntimeError::SemanticAppraisalRetryExpiredOrUnknown)
    ));
    assert_eq!(
        Connection::open(&path)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM semantic_appraisal_claim", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        1
    );
    drop(migrated);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn authenticated_v2_budget_exhausted_terminal_migrates_retries_and_folds() {
    let path = database("appraisal-v2-budget-exhausted-migration");
    let _ = std::fs::remove_file(&path);
    let genesis_request = genesis(0xAB);
    let scope = relation_scope(&genesis_request);
    let mut runtime = AstrRuntime::open(&path).unwrap();
    runtime.ensure_genesis(&genesis_request).unwrap();
    bootstrap(&mut runtime, &scope);

    let begin_request = appraisal_begin(inbound(
        &scope,
        InteractionSourceAuthorityV1::AstrbotMetadata,
    ));
    let challenge = runtime
        .begin_semantic_appraisal_v1(&begin_request)
        .unwrap()
        .challenge
        .unwrap();
    let settle_request = SemanticAppraisalSettleRequestV1 {
        schema_version: SemanticAppraisalSettleRequestV1::SCHEMA_VERSION,
        scope: scope.clone(),
        request_nonce_digest: challenge.request_nonce_digest,
        outcome: SemanticAppraisalOutcomeV1::Timeout,
        provider_usage: SemanticAppraisalProviderUsageV1 {
            known: false,
            used_tokens: None,
        },
        proposal: None,
    };
    runtime
        .settle_semantic_appraisal_v1(&settle_request)
        .unwrap();
    drop(runtime);

    // Reconstruct the modern state emitted by the immediate V2 predecessor:
    // the outcome, settlement identity, canonical receipt bytes, and receipt
    // digest all change together before the exact V2 catalog is restored.
    rewrite_terminal_as_authenticated_budget_exhausted(&path);
    downgrade_appraisal_v3_to_exact_v2(&path);

    let mut migrated =
        AstrRuntime::open(&path).expect("authenticated modern V2 terminal must migrate");
    migrated
        .audit_semantic_integrity_v1()
        .expect("migrated budget-exhausted receipt must remain auditable");
    rewrite_authenticated_budget_exhausted_charge(&path, 0, 1);
    let tampered = migrated.audit_semantic_integrity_v1();
    assert!(
        matches!(
            &tampered,
            Err(RuntimeError::Store(StoreError::ContinuityFence(
                "semantic_appraisal_budget_exhausted_shape"
            )))
        ),
        "unexpected tamper result: {tampered:?}"
    );
    rewrite_authenticated_budget_exhausted_charge(&path, 1, 0);
    migrated
        .audit_semantic_integrity_v1()
        .expect("restored zero-charge budget-exhausted receipt must audit");
    let inspect = Connection::open(&path).unwrap();
    let migrated_shape: (Vec<u8>, String, i64, i64, i64, i64, i64) = inspect
        .query_row(
            "SELECT (SELECT value FROM meta
                       WHERE key='semantic_appraisal_schema_version'),
                    outcome_code,usage_known,length(settlement_identity_digest),
                    length(reply_affect_digest),length(terminal_receipt_bytes),
                    length(terminal_receipt_digest)
               FROM semantic_appraisal_claim",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(migrated_shape.0, vec![3]);
    assert_eq!(migrated_shape.1, "budget_exhausted");
    assert_eq!(migrated_shape.2, 0);
    assert_eq!(
        (migrated_shape.3, migrated_shape.4, migrated_shape.6),
        (32, 32, 32)
    );
    assert!(migrated_shape.5 > 0);
    let historical_shape: (i64, i64, i64, i64, i64) = inspect
        .query_row(
            "SELECT claim.reserved_tokens,claim.charged_tokens,
                    claim.settled_at_ms=claim.created_at_ms,
                    budget.charged_tokens,
                    (SELECT COUNT(*) FROM perception_challenges)
               FROM semantic_appraisal_claim AS claim
               JOIN semantic_appraisal_budget AS budget
                 ON budget.persona_scope=claim.persona_scope AND budget.utc_day=claim.utc_day",
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
        .unwrap();
    assert_eq!(historical_shape, (0, 0, 1, 0, 0));
    drop(inspect);

    assert!(matches!(
        migrated.begin_semantic_appraisal_v1(&begin_request),
        Err(RuntimeError::SemanticAppraisalRetryExpiredOrUnknown)
    ));
    assert!(matches!(
        migrated.settle_semantic_appraisal_v1(&settle_request),
        Err(RuntimeError::SemanticAppraisalRetryExpiredOrUnknown)
    ));
    let connection = Connection::open(&path).unwrap();
    let settled_at_ms: i64 = connection
        .query_row(
            "SELECT settled_at_ms FROM semantic_appraisal_claim",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE semantic_appraisal_rollup
                    SET last_authoritative_now_ms=?1 WHERE singleton=1",
                [settled_at_ms + 300_000],
            )
            .unwrap(),
        1
    );
    drop(connection);

    assert!(matches!(
        migrated.settle_semantic_appraisal_v1(&settle_request),
        Err(RuntimeError::SemanticAppraisalRetryExpiredOrUnknown)
    ));
    migrated
        .audit_semantic_integrity_v1()
        .expect("folded budget-exhausted receipt must preserve closure");
    let connection = Connection::open(&path).unwrap();
    let folded: (i64, i64, i64, Vec<u8>) = connection
        .query_row(
            "SELECT (SELECT COUNT(*) FROM semantic_appraisal_claim),
                    compacted_claim_rows,compacted_charged_tokens,
                    compacted_chain_digest
               FROM semantic_appraisal_budget",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!((folded.0, folded.1, folded.2), (0, 1, 0));
    assert_ne!(folded.3, vec![0; 32]);
    drop(connection);
    drop(migrated);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn authenticated_v1_and_v2_legacy_null_budget_exhausted_migrate_retry_and_fold() {
    for (label, legacy_version) in [
        ("appraisal-v1-null-budget-exhausted", 1_u8),
        ("appraisal-v2-null-budget-exhausted", 2_u8),
    ] {
        let path = database(label);
        let _ = std::fs::remove_file(&path);
        let genesis_request = genesis(0xAC_u8.wrapping_add(legacy_version));
        let scope = relation_scope(&genesis_request);
        let mut runtime = AstrRuntime::open(&path).unwrap();
        runtime.ensure_genesis(&genesis_request).unwrap();
        bootstrap(&mut runtime, &scope);

        let begin_request = appraisal_begin(inbound(
            &scope,
            InteractionSourceAuthorityV1::AstrbotMetadata,
        ));
        let challenge = runtime
            .begin_semantic_appraisal_v1(&begin_request)
            .unwrap()
            .challenge
            .unwrap();
        let mut settle_request = SemanticAppraisalSettleRequestV1 {
            schema_version: SemanticAppraisalSettleRequestV1::SCHEMA_VERSION,
            scope: scope.clone(),
            request_nonce_digest: challenge.request_nonce_digest,
            outcome: SemanticAppraisalOutcomeV1::Timeout,
            provider_usage: SemanticAppraisalProviderUsageV1 {
                known: false,
                used_tokens: None,
            },
            proposal: None,
        };
        runtime
            .settle_semantic_appraisal_v1(&settle_request)
            .unwrap();
        drop(runtime);

        downgrade_appraisal_v3_to_exact_v2_with_legacy_terminal(&path);
        let historical_nonce = [0xD0_u8.wrapping_add(legacy_version); 32];
        rewrite_legacy_terminal_as_budget_exhausted(&path, historical_nonce);
        settle_request.request_nonce_digest = historical_nonce;
        if legacy_version == 1 {
            downgrade_appraisal_v2_to_exact_v1(&path);
        }

        let mut migrated = AstrRuntime::open(&path).unwrap_or_else(|error| {
            panic!("exact V{legacy_version} history must migrate: {error}")
        });
        migrated
            .audit_semantic_integrity_v1()
            .unwrap_or_else(|error| {
                panic!("migrated V{legacy_version} history must audit: {error}")
            });
        let connection = Connection::open(&path).unwrap();
        let migrated_shape: (Vec<u8>, String, i64, i64, i64, i64, i64, i64) = connection
            .query_row(
                "SELECT (SELECT value FROM meta
                           WHERE key='semantic_appraisal_schema_version'),
                        outcome_code,reserved_tokens,charged_tokens,
                        COUNT(*) FILTER (
                          WHERE usage_known IS NULL AND usage_tokens IS NULL
                            AND proposal_identity_digest IS NULL
                            AND settlement_identity_digest IS NULL
                            AND reply_affect_bytes IS NULL
                            AND reply_affect_digest IS NULL
                            AND terminal_receipt_bytes IS NULL
                            AND terminal_receipt_digest IS NULL
                        ),
                        (SELECT daily_token_limit FROM semantic_appraisal_budget),
                        (SELECT charged_tokens FROM semantic_appraisal_budget),
                        (SELECT reserved_tokens FROM semantic_appraisal_budget)
                   FROM semantic_appraisal_claim",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            migrated_shape,
            (vec![3], "budget_exhausted".into(), 0, 0, 1, 0, 0, 0)
        );
        let settled_at_ms: i64 = connection
            .query_row(
                "SELECT settled_at_ms FROM semantic_appraisal_claim",
                [],
                |row| row.get(0),
            )
            .unwrap();
        drop(connection);

        assert!(matches!(
            migrated.begin_semantic_appraisal_v1(&begin_request),
            Err(RuntimeError::SemanticAppraisalRetryExpiredOrUnknown)
        ));
        assert!(matches!(
            migrated.settle_semantic_appraisal_v1(&settle_request),
            Err(RuntimeError::SemanticAppraisalRetryExpiredOrUnknown)
        ));

        let connection = Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .execute(
                    "UPDATE semantic_appraisal_rollup
                        SET last_authoritative_now_ms=?1 WHERE singleton=1",
                    [settled_at_ms + 300_000],
                )
                .unwrap(),
            1
        );
        drop(connection);
        assert!(matches!(
            migrated.settle_semantic_appraisal_v1(&settle_request),
            Err(RuntimeError::SemanticAppraisalRetryExpiredOrUnknown)
        ));
        migrated
            .audit_semantic_integrity_v1()
            .unwrap_or_else(|error| panic!("folded V{legacy_version} history must audit: {error}"));
        let connection = Connection::open(&path).unwrap();
        let folded: (i64, i64, i64, Vec<u8>) = connection
            .query_row(
                "SELECT (SELECT COUNT(*) FROM semantic_appraisal_claim),
                        compacted_claim_rows,compacted_charged_tokens,
                        compacted_chain_digest
                   FROM semantic_appraisal_budget",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!((folded.0, folded.1, folded.2), (0, 1, 0));
        assert_ne!(folded.3, vec![0; 32]);
        drop(connection);
        drop(migrated);
        let _ = std::fs::remove_file(&path);
    }
}

#[test]
fn target_terminal_folds_before_more_than_sixty_four_generic_terminals() {
    let path = database("appraisal-target-first-seed");
    let settle_path = database("appraisal-target-first-settle");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&settle_path);
    let genesis_request = genesis(0xC0);
    let scope = relation_scope(&genesis_request);
    let mut secondary_genesis = genesis(0xD0);
    secondary_genesis.source.scope.bot_token = genesis_request.source.scope.bot_token;
    secondary_genesis.proposal.source = secondary_genesis.source.clone();
    secondary_genesis.proposal.traits.baseline_warmth = Fixed::from_raw(1);
    secondary_genesis.proposal.trait_confidence.baseline_warmth = Fixed::ONE;
    let secondary_scope = relation_scope(&secondary_genesis);
    let mut runtime = AstrRuntime::open(&path).unwrap();
    runtime.ensure_genesis(&genesis_request).unwrap();
    bootstrap(&mut runtime, &scope);

    let mut generic_origins = Vec::new();
    for ordinal in 1_u16..=62 {
        let batch = distinct_inbound(&mut runtime, &scope, ordinal);
        let result = runtime.apply_interaction_fact_batch_v1(&batch).unwrap();
        generic_origins.push((batch, result.receipt));
    }
    let target_begin = appraisal_begin(distinct_inbound(&mut runtime, &scope, 63));
    let target_challenge = runtime
        .begin_semantic_appraisal_v1(&target_begin)
        .unwrap()
        .challenge
        .expect("target terminal seed must claim");
    let target_settle = SemanticAppraisalSettleRequestV1 {
        schema_version: SemanticAppraisalSettleRequestV1::SCHEMA_VERSION,
        scope: scope.clone(),
        request_nonce_digest: target_challenge.request_nonce_digest,
        outcome: SemanticAppraisalOutcomeV1::Timeout,
        provider_usage: SemanticAppraisalProviderUsageV1 {
            known: true,
            used_tokens: Some(0),
        },
        proposal: None,
    };
    runtime
        .settle_semantic_appraisal_v1(&target_settle)
        .unwrap();

    let secondary_identity = runtime.ensure_genesis(&secondary_genesis).unwrap();
    bootstrap(&mut runtime, &secondary_scope);
    let mut secondary_origins = Vec::new();
    for ordinal in 1_001_u16..=1_003 {
        let batch = distinct_inbound(&mut runtime, &secondary_scope, ordinal);
        let result = runtime.apply_interaction_fact_batch_v1(&batch).unwrap();
        secondary_origins.push((batch, result.receipt));
    }
    runtime.ensure_genesis(&genesis_request).unwrap();

    let mut connection = Connection::open(&path).unwrap();
    let transaction = connection.transaction().unwrap();
    let utc_day: i64 = transaction
        .query_row(
            "SELECT utc_day FROM semantic_appraisal_claim
             WHERE request_nonce_digest=?1",
            [target_settle.request_nonce_digest.to_vec()],
            |row| row.get(0),
        )
        .unwrap();
    let base_time = utc_day.checked_mul(86_400_000).unwrap();
    for (index, (batch, receipt)) in generic_origins.iter().enumerate() {
        let fact = &batch.facts[0];
        let mut origin = target_challenge.origin.clone();
        origin.source_digest = fact.source_digest;
        origin.model_digest = fact.extractor_digest;
        origin.provider_digest = wire::domain_hash(
            b"astr-embodiment/perception-provider-v1",
            &[
                &[wire::interaction_source_authority_code(
                    fact.source_authority,
                )],
                &fact.extractor_digest,
            ],
        );
        origin.scope = batch.scope.clone();
        origin.scope_digest = wire::scope_digest(&batch.scope);
        origin.persona_scope =
            wire::persona_scope_digest(&batch.scope.bot_token, &batch.scope.persona_token, None);
        origin.relation_present = true;
        origin.relation_scope = wire::persona_scope_digest(
            &batch.scope.bot_token,
            &batch.scope.persona_token,
            batch.scope.relation_token.as_ref(),
        );
        origin.event_id = batch.causal.turn_id;
        origin.turn_id = batch.causal.turn_id;
        origin.observed_at_ms = fact.observed_at_utc_ms;
        origin.canonical_base_revision = receipt.canonical_revision;
        origin.origin_event_digest = receipt.event_digest;
        origin.origin_journal_revision = receipt.canonical_revision;
        origin.origin_digest = [0; 32];
        origin.origin_digest = origin.digest_v1();
        assert!(origin.validate_v1());

        let ordinal = u64::try_from(index + 1).unwrap();
        let nonce = wire::domain_hash(
            b"astr-embodiment/test/semantic-appraisal-backlog-v1",
            &[&ordinal.to_le_bytes(), &receipt.event_digest],
        );
        let settled_at_ms = base_time
            .checked_add(i64::try_from(ordinal).unwrap())
            .unwrap();
        assert_eq!(
            transaction
                .execute(
                    "INSERT INTO semantic_appraisal_claim(
                       request_nonce_digest,persona_scope,utc_day,origin_event_digest,
                       origin_digest,provider_digest,reserved_tokens,created_at_ms,
                       settled_at_ms,charged_tokens,outcome_code,canonical_revision,
                       semantic_revision,usage_known,usage_tokens,proposal_identity_digest,
                       settlement_identity_digest,reply_affect_bytes,reply_affect_digest,
                       terminal_receipt_bytes,terminal_receipt_digest
                     ) SELECT ?1,persona_scope,utc_day,?2,?3,provider_digest,reserved_tokens,
                              ?4,?4,charged_tokens,outcome_code,canonical_revision,
                              semantic_revision,usage_known,usage_tokens,proposal_identity_digest,
                              settlement_identity_digest,reply_affect_bytes,reply_affect_digest,
                              terminal_receipt_bytes,terminal_receipt_digest
                         FROM semantic_appraisal_claim WHERE request_nonce_digest=?5",
                    rusqlite::params![
                        nonce.to_vec(),
                        receipt.event_digest.to_vec(),
                        origin.origin_digest.to_vec(),
                        settled_at_ms,
                        target_settle.request_nonce_digest.to_vec(),
                    ],
                )
                .unwrap(),
            1
        );
    }
    let secondary_persona = wire::persona_scope_digest(
        &secondary_scope.bot_token,
        &secondary_scope.persona_token,
        None,
    );
    assert_eq!(
        transaction
            .execute(
                "INSERT INTO semantic_appraisal_budget(
                   persona_scope,utc_day,daily_token_limit,charged_tokens,reserved_tokens,
                   blocked,updated_at_ms,compacted_claim_rows,compacted_charged_tokens,
                   compacted_chain_digest
                 ) VALUES(?1,?2,16384,0,0,0,?3,0,0,zeroblob(32))",
                rusqlite::params![secondary_persona.to_vec(), utc_day, base_time + 65],
            )
            .unwrap(),
        1
    );
    for (index, (batch, receipt)) in secondary_origins.iter().enumerate() {
        let fact = &batch.facts[0];
        let provider_code = [wire::interaction_source_authority_code(
            fact.source_authority,
        )];
        let mut origin = PerceptionOriginCommitmentV1 {
            schema_version: PerceptionOriginCommitmentV1::SCHEMA_VERSION,
            source_authority: SourceAuthority::UserObserved,
            source_digest: fact.source_digest,
            model_digest: fact.extractor_digest,
            provider_digest: wire::domain_hash(
                b"astr-embodiment/perception-provider-v1",
                &[&provider_code, &fact.extractor_digest],
            ),
            scope: batch.scope.clone(),
            scope_digest: wire::scope_digest(&batch.scope),
            persona_scope: secondary_persona,
            relation_present: true,
            relation_scope: wire::persona_scope_digest(
                &batch.scope.bot_token,
                &batch.scope.persona_token,
                batch.scope.relation_token.as_ref(),
            ),
            event_id: batch.causal.turn_id,
            turn_id: batch.causal.turn_id,
            observed_at_ms: fact.observed_at_utc_ms,
            canonical_base_revision: receipt.canonical_revision,
            incarnation_id: secondary_identity.incarnation_id,
            manifest_digest: secondary_identity.manifest_digest,
            origin_event_digest: receipt.event_digest,
            origin_journal_revision: receipt.canonical_revision,
            origin_digest: [0; 32],
        };
        origin.origin_digest = origin.digest_v1();
        assert!(origin.validate_v1());
        let ordinal = u64::try_from(index + 63).unwrap();
        let nonce = wire::domain_hash(
            b"astr-embodiment/test/semantic-appraisal-backlog-v1",
            &[&ordinal.to_le_bytes(), &receipt.event_digest],
        );
        let settled_at_ms = base_time + i64::try_from(ordinal).unwrap();
        assert_eq!(
            transaction
                .execute(
                    "INSERT INTO semantic_appraisal_claim(
                       request_nonce_digest,persona_scope,utc_day,origin_event_digest,
                       origin_digest,provider_digest,reserved_tokens,created_at_ms,
                       settled_at_ms,charged_tokens,outcome_code,canonical_revision,
                       semantic_revision,usage_known,usage_tokens,proposal_identity_digest,
                       settlement_identity_digest,reply_affect_bytes,reply_affect_digest,
                       terminal_receipt_bytes,terminal_receipt_digest
                     ) VALUES(?1,?2,?3,?4,?5,?6,1024,?7,?7,0,'timeout',?8,
                              NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL)",
                    rusqlite::params![
                        nonce.to_vec(),
                        secondary_persona.to_vec(),
                        utc_day,
                        receipt.event_digest.to_vec(),
                        origin.origin_digest.to_vec(),
                        vec![0x46_u8; 32],
                        settled_at_ms,
                        i64::try_from(receipt.canonical_revision).unwrap(),
                    ],
                )
                .unwrap(),
            1
        );
    }
    let target_time = base_time + 66;
    assert_eq!(
        transaction
            .execute(
                "UPDATE semantic_appraisal_claim
                    SET created_at_ms=?1,settled_at_ms=?1
                  WHERE request_nonce_digest=?2",
                rusqlite::params![target_time, target_settle.request_nonce_digest.to_vec()],
            )
            .unwrap(),
        1
    );
    assert_eq!(
        transaction
            .execute(
                "UPDATE semantic_appraisal_budget
                    SET updated_at_ms=(
                      SELECT MAX(claim.settled_at_ms)
                        FROM semantic_appraisal_claim AS claim
                       WHERE claim.persona_scope=semantic_appraisal_budget.persona_scope
                         AND claim.utc_day=semantic_appraisal_budget.utc_day
                    )",
                [],
            )
            .unwrap(),
        2
    );
    assert_eq!(
        transaction
            .execute(
                "UPDATE semantic_appraisal_rollup
                    SET last_authoritative_now_ms=?1 WHERE singleton=1",
                [target_time + 300_000],
            )
            .unwrap(),
        1
    );
    let admitted: (i64, i64) = transaction
        .query_row(
            "SELECT SUM(claim_rows),MAX(claim_rows)
               FROM (
                 SELECT COUNT(*) AS claim_rows
                   FROM semantic_appraisal_claim GROUP BY persona_scope
               )",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(admitted, (66, 63));
    transaction.commit().unwrap();
    let settle_path_sql = settle_path.to_string_lossy().into_owned();
    connection
        .execute("VACUUM INTO ?1", [settle_path_sql])
        .unwrap();
    drop(connection);

    let assert_target_won = |database_path: &std::path::Path| {
        let connection = Connection::open(database_path).unwrap();
        let retained: (i64, i64) = connection
            .query_row(
                "SELECT COUNT(*),COUNT(*) FILTER (WHERE request_nonce_digest=?1)
                   FROM semantic_appraisal_claim",
                [target_settle.request_nonce_digest.to_vec()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(retained, (2, 0), "target must consume one of 64 mutations");
    };

    let begin_result = runtime.begin_semantic_appraisal_v1(&target_begin);
    assert!(
        matches!(
            &begin_result,
            Err(RuntimeError::SemanticAppraisalRetryExpiredOrUnknown)
        ),
        "unexpected begin retry result: {begin_result:?}"
    );
    assert_target_won(&path);
    assert!(matches!(
        runtime.begin_semantic_appraisal_v1(&target_begin),
        Err(RuntimeError::SemanticAppraisalRetryExpiredOrUnknown)
    ));

    let mut settle_runtime = AstrRuntime::open(&settle_path).unwrap();
    assert!(matches!(
        settle_runtime.settle_semantic_appraisal_v1(&target_settle),
        Err(RuntimeError::SemanticAppraisalRetryExpiredOrUnknown)
    ));
    assert_target_won(&settle_path);
    assert!(matches!(
        settle_runtime.settle_semantic_appraisal_v1(&target_settle),
        Err(RuntimeError::SemanticAppraisalRetryExpiredOrUnknown)
    ));

    drop(settle_runtime);
    drop(runtime);
    let _ = std::fs::remove_file(&settle_path);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn appraisal_unknown_overage_and_restart_are_charged_without_semantic_mutation() {
    let unknown_path = database("appraisal-unknown");
    let _ = std::fs::remove_file(&unknown_path);
    let unknown_genesis = genesis(0xB1);
    let unknown_scope = relation_scope(&unknown_genesis);
    let mut unknown_runtime = AstrRuntime::open(&unknown_path).unwrap();
    unknown_runtime.ensure_genesis(&unknown_genesis).unwrap();
    bootstrap(&mut unknown_runtime, &unknown_scope);
    let unknown_begin = unknown_runtime
        .begin_semantic_appraisal_v1(&appraisal_begin(inbound(
            &unknown_scope,
            InteractionSourceAuthorityV1::AstrbotMetadata,
        )))
        .unwrap();
    let unknown_nonce = unknown_begin.challenge.unwrap().request_nonce_digest;
    let unknown = unknown_runtime
        .settle_semantic_appraisal_v1(&SemanticAppraisalSettleRequestV1 {
            schema_version: SemanticAppraisalSettleRequestV1::SCHEMA_VERSION,
            scope: unknown_scope.clone(),
            request_nonce_digest: unknown_nonce,
            outcome: SemanticAppraisalOutcomeV1::Timeout,
            provider_usage: SemanticAppraisalProviderUsageV1 {
                known: false,
                used_tokens: None,
            },
            proposal: None,
        })
        .unwrap();
    assert_eq!(
        unknown.status,
        SemanticAppraisalSettleStatusV1::ZeroMutation
    );
    assert_eq!(unknown.charged_tokens, 1_024);
    assert_eq!(unknown.semantic_revision, None);
    assert_eq!(
        unknown_runtime
            .semantic_revision_v1(&unknown_scope)
            .unwrap(),
        0
    );
    let drifted_unknown = SemanticAppraisalSettleRequestV1 {
        schema_version: SemanticAppraisalSettleRequestV1::SCHEMA_VERSION,
        scope: unknown_scope.clone(),
        request_nonce_digest: unknown_nonce,
        outcome: SemanticAppraisalOutcomeV1::Timeout,
        provider_usage: SemanticAppraisalProviderUsageV1 {
            known: true,
            used_tokens: Some(0),
        },
        proposal: None,
    };
    assert!(unknown_runtime
        .settle_semantic_appraisal_v1(&drifted_unknown)
        .is_err());
    drop(unknown_runtime);
    let _ = std::fs::remove_file(&unknown_path);

    let overage_path = database("appraisal-overage");
    let _ = std::fs::remove_file(&overage_path);
    let overage_genesis = genesis(0xC1);
    let overage_scope = relation_scope(&overage_genesis);
    let mut overage_runtime = AstrRuntime::open(&overage_path).unwrap();
    overage_runtime.ensure_genesis(&overage_genesis).unwrap();
    bootstrap(&mut overage_runtime, &overage_scope);
    let overage_begin = overage_runtime
        .begin_semantic_appraisal_v1(&appraisal_begin(inbound(
            &overage_scope,
            InteractionSourceAuthorityV1::AstrbotMetadata,
        )))
        .unwrap();
    let overage_challenge = overage_begin.challenge.unwrap();
    let overage = overage_runtime
        .settle_semantic_appraisal_v1(&SemanticAppraisalSettleRequestV1 {
            schema_version: SemanticAppraisalSettleRequestV1::SCHEMA_VERSION,
            scope: overage_scope.clone(),
            request_nonce_digest: overage_challenge.request_nonce_digest,
            outcome: SemanticAppraisalOutcomeV1::Success,
            provider_usage: SemanticAppraisalProviderUsageV1 {
                known: true,
                used_tokens: Some(1_025),
            },
            proposal: Some(proposal(&overage_challenge)),
        })
        .unwrap();
    assert_eq!(
        overage.status,
        SemanticAppraisalSettleStatusV1::ZeroMutation
    );
    assert_eq!(overage.charged_tokens, 1_025);
    assert_eq!(
        overage_runtime
            .semantic_revision_v1(&overage_scope)
            .unwrap(),
        0
    );
    let mut blocked_batch = inbound(
        &overage_scope,
        InteractionSourceAuthorityV1::AstrbotMetadata,
    );
    blocked_batch.event_id[0] ^= 1;
    blocked_batch.causal.turn_id[0] ^= 1;
    blocked_batch.causal.base_revision = overage_runtime.current_revision(&overage_scope).unwrap();
    blocked_batch.facts[0].fact_id[0] ^= 1;
    blocked_batch.facts[0].source_digest[0] ^= 1;
    let blocked = overage_runtime
        .begin_semantic_appraisal_v1(&appraisal_begin(blocked_batch))
        .unwrap();
    assert_eq!(
        blocked.status,
        SemanticAppraisalBeginStatusV1::BudgetExhausted
    );
    assert!(blocked.challenge.is_none());
    assert!(blocked.budget.as_ref().unwrap().blocked);
    drop(overage_runtime);
    let _ = std::fs::remove_file(&overage_path);

    let restart_path = database("appraisal-restart");
    let _ = std::fs::remove_file(&restart_path);
    let restart_genesis = genesis(0xD1);
    let restart_scope = relation_scope(&restart_genesis);
    let restart_request = appraisal_begin(inbound(
        &restart_scope,
        InteractionSourceAuthorityV1::AstrbotMetadata,
    ));
    let mut restart_runtime = AstrRuntime::open(&restart_path).unwrap();
    restart_runtime.ensure_genesis(&restart_genesis).unwrap();
    bootstrap(&mut restart_runtime, &restart_scope);
    let pending = restart_runtime
        .begin_semantic_appraisal_v1(&restart_request)
        .unwrap();
    assert_eq!(pending.status, SemanticAppraisalBeginStatusV1::Claimed);
    let pending_nonce = pending.challenge.unwrap().request_nonce_digest;
    drop(restart_runtime);

    let mut restarted = AstrRuntime::open(&restart_path).unwrap();
    assert!(matches!(
        restarted.begin_semantic_appraisal_v1(&restart_request),
        Err(RuntimeError::SemanticAppraisalRetryExpiredOrUnknown)
    ));
    assert_eq!(restarted.current_revision(&restart_scope).unwrap(), 1);
    let mut disabled_request =
        appraisal_begin(distinct_inbound(&mut restarted, &restart_scope, 0x71));
    disabled_request.daily_token_limit = 0;
    let disabled = restarted
        .begin_semantic_appraisal_v1(&disabled_request)
        .unwrap();
    assert_eq!(
        disabled.status,
        SemanticAppraisalBeginStatusV1::BudgetExhausted
    );
    assert!(disabled.challenge.is_none());
    assert!(disabled.settlement_nonce_digest.is_none());
    assert!(disabled.capacity_reason.is_none());
    assert_eq!(disabled.budget.as_ref().unwrap().daily_token_limit, 16_384);
    assert_eq!(restarted.semantic_revision_v1(&restart_scope).unwrap(), 0);
    let pending_shape: (i64, i64, i64) = Connection::open(&restart_path)
        .unwrap()
        .query_row(
            "SELECT (SELECT COUNT(*) FROM semantic_appraisal_claim),
                    (SELECT COUNT(*) FROM perception_challenges),
                    (SELECT reserved_tokens FROM semantic_appraisal_budget)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(pending_shape, (1, 1, 1_024));
    let settled = restarted
        .settle_semantic_appraisal_v1(&SemanticAppraisalSettleRequestV1 {
            schema_version: SemanticAppraisalSettleRequestV1::SCHEMA_VERSION,
            scope: restart_scope.clone(),
            request_nonce_digest: pending_nonce,
            outcome: SemanticAppraisalOutcomeV1::Timeout,
            provider_usage: SemanticAppraisalProviderUsageV1 {
                known: false,
                used_tokens: None,
            },
            proposal: None,
        })
        .unwrap();
    assert_eq!(
        settled.status,
        SemanticAppraisalSettleStatusV1::ZeroMutation
    );
    assert_eq!(settled.charged_tokens, 1_024);
    let restart_accounting: (i64, i64, String) = Connection::open(&restart_path)
        .unwrap()
        .query_row(
            "SELECT budget.charged_tokens,budget.reserved_tokens,claim.outcome_code
             FROM semantic_appraisal_budget AS budget
             JOIN semantic_appraisal_claim AS claim
               ON claim.persona_scope=budget.persona_scope AND claim.utc_day=budget.utc_day",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(restart_accounting, (1_024, 0, "timeout".into()));
    drop(restarted);
    let _ = std::fs::remove_file(&restart_path);
}

#[test]
fn concurrent_appraisal_success_returns_one_durable_receipt_and_one_mutation() {
    let path = database("appraisal-concurrent-success");
    let _ = std::fs::remove_file(&path);
    let genesis_request = genesis(0xD1);
    let scope = relation_scope(&genesis_request);
    let mut first = AstrRuntime::open(&path).unwrap();
    let second = AstrRuntime::open(&path).unwrap();
    first.ensure_genesis(&genesis_request).unwrap();
    bootstrap(&mut first, &scope);
    let challenge = first
        .begin_semantic_appraisal_v1(&appraisal_begin(inbound(
            &scope,
            InteractionSourceAuthorityV1::AstrbotMetadata,
        )))
        .unwrap()
        .challenge
        .unwrap();
    let request = SemanticAppraisalSettleRequestV1 {
        schema_version: SemanticAppraisalSettleRequestV1::SCHEMA_VERSION,
        scope: scope.clone(),
        request_nonce_digest: challenge.request_nonce_digest,
        outcome: SemanticAppraisalOutcomeV1::Success,
        provider_usage: SemanticAppraisalProviderUsageV1 {
            known: true,
            used_tokens: Some(377),
        },
        proposal: Some(proposal(&challenge)),
    };
    let barrier = Arc::new(Barrier::new(2));
    let barrier_first = Arc::clone(&barrier);
    let request_first = request.clone();
    let first_thread = std::thread::spawn(move || {
        let mut runtime = first;
        barrier_first.wait();
        runtime
            .settle_semantic_appraisal_v1(&request_first)
            .unwrap()
    });
    let barrier_second = Arc::clone(&barrier);
    let request_second = request.clone();
    let second_thread = std::thread::spawn(move || {
        let mut runtime = second;
        barrier_second.wait();
        runtime
            .settle_semantic_appraisal_v1(&request_second)
            .unwrap()
    });
    let left = first_thread.join().unwrap();
    let right = second_thread.join().unwrap();
    assert_eq!(left, right);
    assert_eq!(left.status, SemanticAppraisalSettleStatusV1::Committed);
    assert_eq!(left.charged_tokens, 377);
    let connection = Connection::open(&path).unwrap();
    let accounting: (i64, i64) = connection
        .query_row(
            "SELECT charged_tokens,reserved_tokens FROM semantic_appraisal_budget",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(accounting, (377, 0));
    let semantic_rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM semantic_commits", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(semantic_rows, 1);
    let durable_fields: (i64, i64, i64, i64) = connection
        .query_row(
            "SELECT length(settlement_identity_digest),length(reply_affect_bytes),
                    length(reply_affect_digest),length(terminal_receipt_digest)
             FROM semantic_appraisal_claim WHERE request_nonce_digest=?1",
            rusqlite::params![challenge.request_nonce_digest.to_vec()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(durable_fields.0, 32);
    assert!(durable_fields.1 > 0);
    assert_eq!(durable_fields.2, 32);
    assert_eq!(durable_fields.3, 32);
    drop(connection);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn concurrent_appraisal_zero_mutation_returns_one_durable_receipt() {
    let path = database("appraisal-concurrent-zero");
    let _ = std::fs::remove_file(&path);
    let genesis_request = genesis(0xE1);
    let scope = relation_scope(&genesis_request);
    let mut first = AstrRuntime::open(&path).unwrap();
    let second = AstrRuntime::open(&path).unwrap();
    first.ensure_genesis(&genesis_request).unwrap();
    bootstrap(&mut first, &scope);
    let challenge = first
        .begin_semantic_appraisal_v1(&appraisal_begin(inbound(
            &scope,
            InteractionSourceAuthorityV1::AstrbotMetadata,
        )))
        .unwrap()
        .challenge
        .unwrap();
    let request = SemanticAppraisalSettleRequestV1 {
        schema_version: SemanticAppraisalSettleRequestV1::SCHEMA_VERSION,
        scope: scope.clone(),
        request_nonce_digest: challenge.request_nonce_digest,
        outcome: SemanticAppraisalOutcomeV1::Timeout,
        provider_usage: SemanticAppraisalProviderUsageV1 {
            known: false,
            used_tokens: None,
        },
        proposal: None,
    };
    let barrier = Arc::new(Barrier::new(2));
    let left_barrier = Arc::clone(&barrier);
    let left_request = request.clone();
    let left = std::thread::spawn(move || {
        let mut runtime = first;
        left_barrier.wait();
        runtime.settle_semantic_appraisal_v1(&left_request).unwrap()
    });
    let right_barrier = Arc::clone(&barrier);
    let right_request = request.clone();
    let right = std::thread::spawn(move || {
        let mut runtime = second;
        right_barrier.wait();
        runtime
            .settle_semantic_appraisal_v1(&right_request)
            .unwrap()
    });
    let left = left.join().unwrap();
    let right = right.join().unwrap();
    assert_eq!(left, right);
    assert_eq!(left.status, SemanticAppraisalSettleStatusV1::ZeroMutation);
    assert_eq!(left.charged_tokens, 1_024);
    assert_eq!(left.semantic_revision, None);
    let connection = Connection::open(&path).unwrap();
    let accounting: (i64, i64) = connection
        .query_row(
            "SELECT charged_tokens,reserved_tokens FROM semantic_appraisal_budget",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(accounting, (1_024, 0));
    let semantic_rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM semantic_commits", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(semantic_rows, 0);
    drop(connection);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn expired_with_unauthenticated_binding_loss_is_corruption_without_mutation() {
    let path = database("appraisal-binding-lost");
    let _ = std::fs::remove_file(&path);
    let genesis_request = genesis(0xF1);
    let scope = relation_scope(&genesis_request);
    let mut runtime = AstrRuntime::open(&path).unwrap();
    runtime.ensure_genesis(&genesis_request).unwrap();
    bootstrap(&mut runtime, &scope);
    let challenge = runtime
        .begin_semantic_appraisal_v1(&appraisal_begin(inbound(
            &scope,
            InteractionSourceAuthorityV1::AstrbotMetadata,
        )))
        .unwrap()
        .challenge
        .unwrap();
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE semantic_appraisal_rollup SET last_authoritative_now_ms=?1
                 WHERE singleton=1",
                [i64::try_from(challenge.expires_at_ms).unwrap()],
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .execute("DELETE FROM active_bindings", [])
            .unwrap(),
        1
    );
    drop(connection);

    let result = runtime.settle_semantic_appraisal_v1(&SemanticAppraisalSettleRequestV1 {
        schema_version: SemanticAppraisalSettleRequestV1::SCHEMA_VERSION,
        scope: scope.clone(),
        request_nonce_digest: challenge.request_nonce_digest,
        outcome: SemanticAppraisalOutcomeV1::Timeout,
        provider_usage: SemanticAppraisalProviderUsageV1 {
            known: false,
            used_tokens: None,
        },
        proposal: None,
    });
    let settlement_error = result.expect_err("missing binding has no authenticated transition");
    assert!(
        matches!(
            &settlement_error,
            RuntimeError::Store(StoreError::ContinuityFence("perception_challenge_binding"))
        ),
        "{settlement_error:?}"
    );
    let connection = Connection::open(&path).unwrap();
    let terminal: (i64, i64, Option<String>, Option<i64>, i64) = connection
        .query_row(
            "SELECT budget.charged_tokens,budget.reserved_tokens,claim.outcome_code,
                     length(claim.terminal_receipt_digest),
                    (SELECT COUNT(*) FROM perception_challenges)
             FROM semantic_appraisal_budget AS budget
             JOIN semantic_appraisal_claim AS claim
               ON claim.persona_scope=budget.persona_scope AND claim.utc_day=budget.utc_day",
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
        .unwrap();
    assert_eq!(terminal, (0, 1_024, None, None, 1));
    drop(connection);
    drop(runtime);
    assert!(matches!(
        AstrRuntime::open(&path),
        Err(RuntimeError::Store(StoreError::ContinuityFence(
            "perception_challenge_binding"
        )))
    ));
    let _ = std::fs::remove_file(&path);
}
