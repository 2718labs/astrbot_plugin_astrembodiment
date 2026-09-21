use crate::StoreError;
use ae_contracts::{
    wire::domain_hash, CapabilityGrantStateV1, ContactIntentionBasisV1, DreamResidueStateV1,
    DreamResidueV1, EcosystemCapabilityGrantV1, EcosystemProposalV1, GateDecisionV2,
    InteractionFactV1, LivedDayStateV1, ProactiveReadinessV1, RelationBudgetPolicyV1,
    RelationConsentStateV1, RelationConsentV1, RelationContactProcessV1, RelationTemporalPolicyV1,
    WorldAnchorV1, WorldLayerV1, WorldModeV1, ALPHA3_SCHEMA_VERSION,
};
use ae_fixed::Fixed;
use rusqlite::{params, Connection, Transaction};

pub(crate) const MIGRATION_DIGEST_V7: &[u8] = b"ae.autonomy.db.v7.alpha3-authority.v1";
const MAX_ALPHA3_BODY_BYTES: usize = 256 * 1024;

pub(crate) fn migrate_alpha3_v7(tx: &Transaction<'_>, from_version: u32) -> Result<(), StoreError> {
    if from_version >= 7 {
        if !table_has_column(
            tx,
            "externalization_budget_claim",
            "migrated_unknown_full_charge",
        )? {
            add_budget_authority_columns(tx)?;
            charge_unknown_legacy_reservations(tx)?;
        }
        return verify_alpha3_v7(tx);
    }

    tx.execute_batch(
        r#"
        CREATE TABLE world_anchor (
            world_anchor_id BLOB PRIMARY KEY CHECK(length(world_anchor_id)=16),
            persona_scope BLOB NOT NULL UNIQUE CHECK(length(persona_scope)=32),
            mode TEXT NOT NULL CHECK(mode='mixed_main_world'),
            revision INTEGER NOT NULL CHECK(revision=1),
            body_json TEXT NOT NULL CHECK(length(CAST(body_json AS BLOB))<=262144)
        );
        CREATE TRIGGER world_anchor_immutable_update
            BEFORE UPDATE ON world_anchor
            BEGIN SELECT RAISE(ABORT,'world_anchor is immutable'); END;
        CREATE TRIGGER world_anchor_immutable_delete
            BEFORE DELETE ON world_anchor
            BEGIN SELECT RAISE(ABORT,'world_anchor is immutable'); END;

        CREATE TABLE lived_day_state (
            persona_scope BLOB PRIMARY KEY CHECK(length(persona_scope)=32),
            world_anchor_id BLOB NOT NULL CHECK(length(world_anchor_id)=16),
            persona_day_ordinal INTEGER NOT NULL,
            revision INTEGER NOT NULL CHECK(revision>0),
            body_json TEXT NOT NULL CHECK(length(CAST(body_json AS BLOB))<=262144)
        );
        CREATE INDEX lived_day_state_anchor ON lived_day_state(world_anchor_id);

        CREATE TABLE interaction_fact (
            fact_id BLOB PRIMARY KEY CHECK(length(fact_id)=16),
            event_id BLOB NOT NULL CHECK(length(event_id)=16),
            persona_scope BLOB NOT NULL CHECK(length(persona_scope)=32),
            relation_scope BLOB NOT NULL CHECK(length(relation_scope)=32),
            observed_at_utc_ms INTEGER NOT NULL CHECK(observed_at_utc_ms>=0),
            source_digest BLOB NOT NULL CHECK(length(source_digest)=32),
            revision INTEGER NOT NULL CHECK(revision>0),
            body_json TEXT NOT NULL CHECK(length(CAST(body_json AS BLOB))<=262144),
            UNIQUE(relation_scope,event_id,fact_id)
        );
        CREATE INDEX interaction_fact_relation_time
            ON interaction_fact(relation_scope,observed_at_utc_ms,fact_id);
        CREATE INDEX interaction_fact_persona_revision
            ON interaction_fact(persona_scope,revision,fact_id);

        CREATE TABLE relation_consent (
            relation_scope BLOB NOT NULL CHECK(length(relation_scope)=32),
            consent_epoch INTEGER NOT NULL CHECK(consent_epoch>0),
            revision INTEGER NOT NULL CHECK(revision>0),
            state TEXT NOT NULL CHECK(state IN ('disabled','pending_reconfirmation','granted','paused','ended')),
            body_json TEXT NOT NULL CHECK(length(CAST(body_json AS BLOB))<=262144),
            PRIMARY KEY(relation_scope,consent_epoch,revision)
        );
        CREATE INDEX relation_consent_scope_revision
            ON relation_consent(relation_scope,revision);
        CREATE TABLE relation_consent_head (
            relation_scope BLOB PRIMARY KEY CHECK(length(relation_scope)=32),
            consent_epoch INTEGER NOT NULL CHECK(consent_epoch>0),
            revision INTEGER NOT NULL CHECK(revision>0),
            FOREIGN KEY(relation_scope,consent_epoch,revision)
                REFERENCES relation_consent(relation_scope,consent_epoch,revision)
        );

        CREATE TABLE relation_contact_process (
            relation_scope BLOB PRIMARY KEY CHECK(length(relation_scope)=32),
            revision INTEGER NOT NULL CHECK(revision>0),
            contact_due_score INTEGER NOT NULL,
            active_cause_digest BLOB CHECK(active_cause_digest IS NULL OR length(active_cause_digest)=32),
            body_json TEXT NOT NULL CHECK(length(CAST(body_json AS BLOB))<=262144)
        );

        CREATE TABLE contact_intention_basis (
            intention_id BLOB PRIMARY KEY CHECK(length(intention_id)=16),
            relation_scope BLOB NOT NULL CHECK(length(relation_scope)=32),
            cause_digest BLOB NOT NULL CHECK(length(cause_digest)=32),
            consent_epoch INTEGER NOT NULL CHECK(consent_epoch>0),
            consent_revision INTEGER NOT NULL CHECK(consent_revision>0),
            live INTEGER NOT NULL CHECK(live IN (0,1)),
            revision INTEGER NOT NULL CHECK(revision>0),
            body_json TEXT NOT NULL CHECK(length(CAST(body_json AS BLOB))<=262144)
        );
        CREATE UNIQUE INDEX contact_intention_live_cause
            ON contact_intention_basis(relation_scope,cause_digest)
            WHERE live=1;

        CREATE TABLE ecosystem_capability_grant (
            plugin_instance_digest BLOB NOT NULL CHECK(length(plugin_instance_digest)=32),
            capability_digest BLOB NOT NULL CHECK(length(capability_digest)=32),
            revision INTEGER NOT NULL CHECK(revision>0),
            state TEXT NOT NULL CHECK(state IN ('active','revoked','expired')),
            body_json TEXT NOT NULL CHECK(length(CAST(body_json AS BLOB))<=262144),
            PRIMARY KEY(plugin_instance_digest,revision)
        );
        CREATE UNIQUE INDEX ecosystem_capability_active
            ON ecosystem_capability_grant(plugin_instance_digest,capability_digest)
            WHERE state='active';

        CREATE TABLE ecosystem_proposal (
            proposal_id BLOB PRIMARY KEY CHECK(length(proposal_id)=16),
            persona_scope BLOB NOT NULL CHECK(length(persona_scope)=32),
            plugin_instance_digest BLOB NOT NULL CHECK(length(plugin_instance_digest)=32),
            semantic_digest BLOB NOT NULL UNIQUE CHECK(length(semantic_digest)=32),
            revision INTEGER NOT NULL CHECK(revision>0),
            decision TEXT NOT NULL CHECK(decision IN ('accepted','rejected','deferred')),
            body_json TEXT NOT NULL CHECK(length(CAST(body_json AS BLOB))<=262144)
        );
        CREATE INDEX ecosystem_proposal_persona_revision
            ON ecosystem_proposal(persona_scope,revision,proposal_id);

        CREATE TABLE dream_residue (
            residue_id BLOB PRIMARY KEY CHECK(length(residue_id)=16),
            persona_scope BLOB NOT NULL CHECK(length(persona_scope)=32),
            revision INTEGER NOT NULL CHECK(revision>0),
            state TEXT NOT NULL CHECK(state IN ('pending_waking_review','rejected','retained_non_fact','expired')),
            body_json TEXT NOT NULL CHECK(length(CAST(body_json AS BLOB))<=262144)
        );
        CREATE INDEX dream_residue_persona_revision
            ON dream_residue(persona_scope,revision,residue_id);

        CREATE TABLE relation_budget_policy (
            relation_scope BLOB PRIMARY KEY CHECK(length(relation_scope)=32),
            revision INTEGER NOT NULL CHECK(revision>0),
            body_json TEXT NOT NULL CHECK(length(CAST(body_json AS BLOB))<=262144)
        );
        CREATE TABLE proactive_readiness (
            relation_scope BLOB PRIMARY KEY CHECK(length(relation_scope)=32),
            revision INTEGER NOT NULL CHECK(revision>0),
            witness_digest BLOB NOT NULL CHECK(length(witness_digest)=32),
            body_json TEXT NOT NULL CHECK(length(CAST(body_json AS BLOB))<=262144)
        );
        CREATE TABLE gate_decision_latest (
            relation_scope BLOB NOT NULL CHECK(length(relation_scope)=32),
            gate_phase TEXT NOT NULL CHECK(gate_phase IN ('externalization','dispatch')),
            revision INTEGER NOT NULL CHECK(revision>0),
            body_json TEXT NOT NULL CHECK(length(CAST(body_json AS BLOB))<=262144),
            PRIMARY KEY(relation_scope,gate_phase)
        );
        "#,
    )?;

    add_budget_authority_columns(tx)?;
    backfill_world_anchors(tx)?;
    backfill_relation_authority(tx)?;
    charge_unknown_legacy_reservations(tx)?;
    verify_alpha3_v7(tx)
}

fn table_has_column(conn: &Connection, table: &str, wanted: &str) -> Result<bool, StoreError> {
    let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    for column in statement.query_map([], |row| row.get::<_, String>(1))? {
        if column? == wanted {
            return Ok(true);
        }
    }
    Ok(false)
}

fn add_budget_authority_columns(tx: &Transaction<'_>) -> Result<(), StoreError> {
    for (column, ddl) in [
        (
            "limit_tokens",
            "ALTER TABLE externalization_budget ADD COLUMN limit_tokens INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "charged_tokens",
            "ALTER TABLE externalization_budget ADD COLUMN charged_tokens INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "used_tokens",
            "ALTER TABLE externalization_budget ADD COLUMN used_tokens INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "usage_known",
            "ALTER TABLE externalization_budget ADD COLUMN usage_known INTEGER NOT NULL DEFAULT 1",
        ),
        (
            "revision",
            "ALTER TABLE externalization_budget ADD COLUMN revision INTEGER NOT NULL DEFAULT 1",
        ),
    ] {
        if !table_has_column(tx, "externalization_budget", column)? {
            tx.execute(ddl, [])?;
        }
    }
    if !table_has_column(
        tx,
        "externalization_budget_claim",
        "migrated_unknown_full_charge",
    )? {
        tx.execute(
            "ALTER TABLE externalization_budget_claim
             ADD COLUMN migrated_unknown_full_charge INTEGER NOT NULL DEFAULT 0
             CHECK(migrated_unknown_full_charge IN (0,1))",
            [],
        )?;
    }
    Ok(())
}

fn fixed_id(domain: &[u8], source: &[u8]) -> [u8; 16] {
    let digest = domain_hash(domain, &[source]);
    digest[..16].try_into().expect("digest prefix")
}

fn bounded_body<T: serde::Serialize>(value: &T) -> Result<String, StoreError> {
    let body = serde_json::to_string(value)
        .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
    if body.len() > MAX_ALPHA3_BODY_BYTES {
        return Err(StoreError::AutonomyConflict(
            "alpha3 migration body exceeds 256 KiB".into(),
        ));
    }
    Ok(body)
}

fn existing_personas(tx: &Transaction<'_>) -> Result<Vec<[u8; 32]>, StoreError> {
    let mut statement = tx.prepare(
        "SELECT persona_scope FROM persona_temporal_profile
         UNION SELECT persona_scope FROM autonomous_runtime_state
         UNION SELECT persona_scope FROM autonomy_scope_binding
         UNION SELECT persona_scope FROM inner_event
         UNION SELECT persona_scope FROM durable_intention
         UNION SELECT persona_scope FROM wake_schedule
         UNION SELECT persona_scope FROM autonomy_operational_authority
         ORDER BY persona_scope",
    )?;
    let raw = statement
        .query_map([], |row| row.get::<_, Vec<u8>>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    raw.into_iter()
        .map(|value| {
            value.try_into().map_err(|_| {
                StoreError::AutonomyConflict("invalid persona scope during alpha3 migration".into())
            })
        })
        .collect()
}

fn migration_world_anchor(persona_scope: [u8; 32]) -> WorldAnchorV1 {
    WorldAnchorV1 {
        schema_version: ALPHA3_SCHEMA_VERSION,
        world_anchor_id: fixed_id(b"ae.world-anchor.id.v1", &persona_scope),
        persona_scope,
        mode: WorldModeV1::MixedMainWorld,
        home_context_ref: domain_hash(b"ae.world-anchor.home-context.v1", &[&persona_scope]),
        lore_manifest_digest: domain_hash(b"ae.world-anchor.empty-lore.v1", &[]),
        reality_policy_digest: domain_hash(
            b"ae.world-anchor.reality-policy.v1",
            &[&persona_scope, b"mixed_main_world", b"alpha3-v1"],
        ),
        allowed_layers: vec![
            WorldLayerV1::ExternalObserved,
            WorldLayerV1::PersonaNearReal,
            WorldLayerV1::DeclaredFantasy,
        ],
        created_from_event_id: fixed_id(b"ae.world-anchor.migration-event.v1", &persona_scope),
        revision: 1,
    }
}

fn backfill_world_anchors(tx: &Transaction<'_>) -> Result<(), StoreError> {
    for persona_scope in existing_personas(tx)? {
        let anchor = migration_world_anchor(persona_scope);
        anchor
            .validate()
            .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
        tx.execute(
            "INSERT INTO world_anchor(world_anchor_id,persona_scope,mode,revision,body_json)
             VALUES(?1,?2,'mixed_main_world',1,?3)",
            params![
                anchor.world_anchor_id.to_vec(),
                anchor.persona_scope.to_vec(),
                bounded_body(&anchor)?
            ],
        )?;
    }
    Ok(())
}

fn legacy_relation_policies(
    tx: &Transaction<'_>,
) -> Result<Vec<(RelationTemporalPolicyV1, Vec<u8>)>, StoreError> {
    let mut statement = tx.prepare(
        "SELECT relation_scope,revision,
                CASE WHEN length(CAST(body_json AS BLOB))<=?1 THEN CAST(body_json AS BLOB) END
         FROM relation_temporal_policy ORDER BY relation_scope",
    )?;
    let rows = statement
        .query_map(params![MAX_ALPHA3_BODY_BYTES as i64], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<Vec<u8>>>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut policies = Vec::with_capacity(rows.len());
    for (raw_scope, raw_revision, bounded) in rows {
        let relation_scope: [u8; 32] = raw_scope.try_into().map_err(|_| {
            StoreError::AutonomyConflict("invalid relation scope during alpha3 migration".into())
        })?;
        let body = bounded.ok_or_else(|| {
            StoreError::AutonomyConflict("legacy relation policy exceeds 256 KiB".into())
        })?;
        let policy: RelationTemporalPolicyV1 = serde_json::from_slice(&body).map_err(|error| {
            StoreError::AutonomyConflict(format!(
                "legacy relation policy is not a closed v1 contract: {error}"
            ))
        })?;
        let revision: u64 = raw_revision.try_into().map_err(|_| {
            StoreError::AutonomyConflict("negative legacy relation policy revision".into())
        })?;
        if policy.schema_version != 1
            || policy.relation_scope != relation_scope
            || policy.revision != revision
        {
            return Err(StoreError::AutonomyConflict(
                "legacy relation policy columns differ from its body".into(),
            ));
        }
        policies.push((policy, body));
    }
    Ok(policies)
}

fn consent_state_name(state: RelationConsentStateV1) -> &'static str {
    match state {
        RelationConsentStateV1::Disabled => "disabled",
        RelationConsentStateV1::PendingReconfirmation => "pending_reconfirmation",
        RelationConsentStateV1::Granted => "granted",
        RelationConsentStateV1::Paused => "paused",
        RelationConsentStateV1::Ended => "ended",
    }
}

fn backfill_relation_authority(tx: &Transaction<'_>) -> Result<(), StoreError> {
    for (policy, source_body) in legacy_relation_policies(tx)? {
        let relation_scope = policy.relation_scope;
        let state = if policy.proactive_enabled {
            RelationConsentStateV1::PendingReconfirmation
        } else {
            RelationConsentStateV1::Disabled
        };
        let consent = RelationConsentV1 {
            schema_version: ALPHA3_SCHEMA_VERSION,
            relation_scope,
            consent_epoch: 1,
            revision: 1,
            state,
            purposes: Vec::new(),
            channels: Vec::new(),
            valid_from_utc_ms: 0,
            valid_until_utc_ms: None,
            pause_until_utc_ms: None,
            source_event_id: fixed_id(b"ae.relation-consent.migration-event.v1", &relation_scope),
            policy_digest: domain_hash(b"ae.relation-consent.migration-policy.v1", &[&source_body]),
        };
        consent
            .validate()
            .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
        tx.execute(
            "INSERT INTO relation_consent(relation_scope,consent_epoch,revision,state,body_json)
             VALUES(?1,1,1,?2,?3)",
            params![
                relation_scope.to_vec(),
                consent_state_name(state),
                bounded_body(&consent)?
            ],
        )?;
        tx.execute(
            "INSERT INTO relation_consent_head(relation_scope,consent_epoch,revision)
             VALUES(?1,1,1)",
            params![relation_scope.to_vec()],
        )?;

        let contact = RelationContactProcessV1 {
            schema_version: ALPHA3_SCHEMA_VERSION,
            relation_scope,
            revision: 1,
            last_inbound_utc_ms: policy.last_inbound_utc_ms,
            last_outbound_submitted_utc_ms: policy.last_proactive_submitted_utc_ms,
            response_cadence_ema_ms: None,
            response_cadence_variation: Fixed::ZERO,
            contact_due_score: Fixed::ZERO,
            unfinished_follow_up_salience: Fixed::ZERO,
            repetition_penalty: Fixed::ZERO,
            consecutive_unanswered: 0,
            next_contact_eligible_utc_ms: None,
            active_cause_digest: None,
            active_source_event_ids: Vec::new(),
            formula_digest: domain_hash(b"ae.relation-contact.formula.v1", &[]),
        };
        contact
            .validate()
            .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
        tx.execute(
            "INSERT INTO relation_contact_process(
                 relation_scope,revision,contact_due_score,active_cause_digest,body_json
             ) VALUES(?1,1,0,NULL,?2)",
            params![relation_scope.to_vec(), bounded_body(&contact)?],
        )?;
    }
    Ok(())
}

fn charge_unknown_legacy_reservations(tx: &Transaction<'_>) -> Result<(), StoreError> {
    // Every claim present when the settlement marker is first installed
    // belongs to a legacy aggregate reservation. Conservatively charge it now;
    // compatibility settlement may finish the claim, but must never refund or
    // subtract that reservation again.
    tx.execute(
        "UPDATE externalization_budget_claim
         SET migrated_unknown_full_charge=1",
        [],
    )?;
    tx.execute(
        "UPDATE externalization_budget
         SET limit_tokens=charged_tokens+reserved_tokens",
        [],
    )?;
    tx.execute(
        "UPDATE externalization_budget
         SET charged_tokens=charged_tokens+reserved_tokens,
             reserved_tokens=0,
             usage_known=CASE WHEN reserved_tokens>0 THEN 0 ELSE usage_known END,
             revision=CASE WHEN reserved_tokens>0 THEN revision+1 ELSE revision END",
        [],
    )?;
    Ok(())
}

fn required_schema_objects_exist(conn: &Connection) -> Result<bool, StoreError> {
    let required = [
        ("table", "world_anchor"),
        ("table", "lived_day_state"),
        ("table", "interaction_fact"),
        ("table", "relation_consent"),
        ("table", "relation_consent_head"),
        ("table", "relation_contact_process"),
        ("table", "contact_intention_basis"),
        ("table", "ecosystem_capability_grant"),
        ("table", "ecosystem_proposal"),
        ("table", "dream_residue"),
        ("table", "relation_budget_policy"),
        ("table", "proactive_readiness"),
        ("table", "gate_decision_latest"),
        ("trigger", "world_anchor_immutable_update"),
        ("trigger", "world_anchor_immutable_delete"),
        ("index", "interaction_fact_relation_time"),
        ("index", "contact_intention_live_cause"),
        ("index", "lived_day_state_anchor"),
        ("index", "ecosystem_capability_active"),
        ("index", "ecosystem_proposal_persona_revision"),
        ("index", "dream_residue_persona_revision"),
    ];
    for (kind, name) in required {
        let exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type=?1 AND name=?2",
            params![kind, name],
            |row| row.get(0),
        )?;
        if exists != 1 {
            return Ok(false);
        }
    }
    for column in [
        "limit_tokens",
        "charged_tokens",
        "used_tokens",
        "usage_known",
        "revision",
    ] {
        if !table_has_column(conn, "externalization_budget", column)? {
            return Ok(false);
        }
    }
    if !table_has_column(
        conn,
        "externalization_budget_claim",
        "migrated_unknown_full_charge",
    )? {
        return Ok(false);
    }
    Ok(true)
}

fn table_columns_match(
    conn: &Connection,
    table: &str,
    expected: &[&str],
) -> Result<bool, StoreError> {
    let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(columns
        .iter()
        .map(String::as_str)
        .eq(expected.iter().copied()))
}

fn index_shape_matches(
    conn: &Connection,
    table: &str,
    index: &str,
    expected_columns: &[&str],
    expected_unique: bool,
    expected_partial: bool,
) -> Result<bool, StoreError> {
    let mut list = conn.prepare(&format!("PRAGMA index_list({table})"))?;
    let indexes = list
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? != 0,
                row.get::<_, i64>(4)? != 0,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let Some((_, unique, partial)) = indexes.iter().find(|(name, _, _)| name == index) else {
        return Ok(false);
    };
    if *unique != expected_unique || *partial != expected_partial {
        return Ok(false);
    }
    let mut info = conn.prepare(&format!("PRAGMA index_info({index})"))?;
    let columns = info
        .query_map([], |row| row.get::<_, String>(2))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(columns
        .iter()
        .map(String::as_str)
        .eq(expected_columns.iter().copied()))
}

fn retired_schema_shape_matches(conn: &Connection) -> Result<bool, StoreError> {
    Ok(table_columns_match(
        conn,
        "world_anchor",
        &[
            "world_anchor_id",
            "persona_scope",
            "mode",
            "revision",
            "body_json",
        ],
    )? && table_columns_match(
        conn,
        "lived_day_state",
        &[
            "persona_scope",
            "world_anchor_id",
            "persona_day_ordinal",
            "revision",
            "body_json",
        ],
    )? && table_columns_match(
        conn,
        "dream_residue",
        &[
            "residue_id",
            "persona_scope",
            "revision",
            "state",
            "body_json",
        ],
    )? && table_columns_match(
        conn,
        "ecosystem_capability_grant",
        &[
            "plugin_instance_digest",
            "capability_digest",
            "revision",
            "state",
            "body_json",
        ],
    )? && table_columns_match(
        conn,
        "ecosystem_proposal",
        &[
            "proposal_id",
            "persona_scope",
            "plugin_instance_digest",
            "semantic_digest",
            "revision",
            "decision",
            "body_json",
        ],
    )? && index_shape_matches(
        conn,
        "lived_day_state",
        "lived_day_state_anchor",
        &["world_anchor_id"],
        false,
        false,
    )? && index_shape_matches(
        conn,
        "dream_residue",
        "dream_residue_persona_revision",
        &["persona_scope", "revision", "residue_id"],
        false,
        false,
    )? && index_shape_matches(
        conn,
        "ecosystem_capability_grant",
        "ecosystem_capability_active",
        &["plugin_instance_digest", "capability_digest"],
        true,
        true,
    )? && index_shape_matches(
        conn,
        "ecosystem_proposal",
        "ecosystem_proposal_persona_revision",
        &["persona_scope", "revision", "proposal_id"],
        false,
        false,
    )?)
}

#[allow(dead_code)]
fn verify_world_anchors(conn: &Connection) -> Result<(), StoreError> {
    let mut statement = conn.prepare(
        "SELECT world_anchor_id,persona_scope,mode,revision,
                CASE WHEN length(CAST(body_json AS BLOB))<=?1 THEN body_json END
         FROM world_anchor ORDER BY persona_scope",
    )?;
    let rows = statement
        .query_map(params![MAX_ALPHA3_BODY_BYTES as i64], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (raw_id, raw_scope, mode, revision, bounded) in rows {
        let anchor: WorldAnchorV1 = serde_json::from_str(&bounded.ok_or_else(|| {
            StoreError::AutonomyConflict("world anchor body exceeds 256 KiB".into())
        })?)
        .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
        anchor
            .validate()
            .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
        if raw_id != anchor.world_anchor_id
            || raw_scope != anchor.persona_scope
            || mode != "mixed_main_world"
            || revision != 1
        {
            return Err(StoreError::AutonomyConflict(
                "world anchor columns differ from its body".into(),
            ));
        }
    }
    Ok(())
}

fn verify_relation_authority(conn: &Connection) -> Result<(), StoreError> {
    let mut statement = conn.prepare(
        "SELECT relation_scope,consent_epoch,revision,state,
                CASE WHEN length(CAST(body_json AS BLOB))<=?1 THEN body_json END
         FROM relation_consent ORDER BY relation_scope,consent_epoch,revision",
    )?;
    let rows = statement
        .query_map(params![MAX_ALPHA3_BODY_BYTES as i64], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (scope, epoch, revision, state, bounded) in rows {
        let consent: RelationConsentV1 = serde_json::from_str(&bounded.ok_or_else(|| {
            StoreError::AutonomyConflict("relation consent body exceeds 256 KiB".into())
        })?)
        .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
        consent
            .validate()
            .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
        if scope != consent.relation_scope
            || u64::try_from(epoch).ok() != Some(consent.consent_epoch)
            || u64::try_from(revision).ok() != Some(consent.revision)
            || state != consent_state_name(consent.state)
        {
            return Err(StoreError::AutonomyConflict(
                "relation consent columns differ from its body".into(),
            ));
        }
    }
    drop(statement);

    let mut statement = conn.prepare(
        "SELECT relation_scope,revision,contact_due_score,active_cause_digest,
                CASE WHEN length(CAST(body_json AS BLOB))<=?1 THEN body_json END
         FROM relation_contact_process ORDER BY relation_scope",
    )?;
    let rows = statement
        .query_map(params![MAX_ALPHA3_BODY_BYTES as i64], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<Vec<u8>>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (scope, revision, due, cause, bounded) in rows {
        let contact: RelationContactProcessV1 =
            serde_json::from_str(&bounded.ok_or_else(|| {
                StoreError::AutonomyConflict("relation contact body exceeds 256 KiB".into())
            })?)
            .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
        contact
            .validate()
            .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
        if scope != contact.relation_scope
            || u64::try_from(revision).ok() != Some(contact.revision)
            || due != contact.contact_due_score.raw()
            || cause.as_deref()
                != contact
                    .active_cause_digest
                    .as_ref()
                    .map(|value| value.as_slice())
        {
            return Err(StoreError::AutonomyConflict(
                "relation contact columns differ from its body".into(),
            ));
        }
    }
    Ok(())
}

fn parse_body<T: serde::de::DeserializeOwned>(
    bounded: Option<String>,
    context: &str,
) -> Result<T, StoreError> {
    let body = bounded
        .ok_or_else(|| StoreError::AutonomyConflict(format!("{context} body exceeds 256 KiB")))?;
    serde_json::from_str(&body)
        .map_err(|error| StoreError::AutonomyConflict(format!("{context}: {error}")))
}

#[allow(dead_code)]
fn grant_state_name(state: CapabilityGrantStateV1) -> &'static str {
    match state {
        CapabilityGrantStateV1::Active => "active",
        CapabilityGrantStateV1::Revoked => "revoked",
        CapabilityGrantStateV1::Expired => "expired",
    }
}

#[allow(dead_code)]
fn dream_state_name(state: DreamResidueStateV1) -> &'static str {
    match state {
        DreamResidueStateV1::PendingWakingReview => "pending_waking_review",
        DreamResidueStateV1::Rejected => "rejected",
        DreamResidueStateV1::RetainedNonFact => "retained_non_fact",
        DreamResidueStateV1::Expired => "expired",
    }
}

#[allow(dead_code)]
fn verify_lived_day_rows(conn: &Connection) -> Result<(), StoreError> {
    let mut statement = conn.prepare(
        "SELECT persona_scope,world_anchor_id,persona_day_ordinal,revision,
                CASE WHEN length(CAST(body_json AS BLOB))<=?1 THEN body_json END
         FROM lived_day_state ORDER BY persona_scope",
    )?;
    let rows = statement
        .query_map(params![MAX_ALPHA3_BODY_BYTES as i64], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (persona_scope, anchor_id, day, revision, bounded) in rows {
        let lived: LivedDayStateV1 = parse_body(bounded, "lived day")?;
        lived
            .validate()
            .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
        if persona_scope.as_slice() != lived.persona_scope
            || anchor_id.as_slice() != lived.world_anchor_id
            || i32::try_from(day).ok() != Some(lived.persona_day_ordinal)
            || u64::try_from(revision).ok() != Some(lived.revision)
        {
            return Err(StoreError::AutonomyConflict(
                "lived day columns differ from its body".into(),
            ));
        }
    }
    Ok(())
}

fn verify_interaction_rows(conn: &Connection) -> Result<(), StoreError> {
    let mut statement = conn.prepare(
        "SELECT fact_id,observed_at_utc_ms,source_digest,
                CASE WHEN length(CAST(body_json AS BLOB))<=?1 THEN body_json END
         FROM interaction_fact ORDER BY relation_scope,revision,fact_id",
    )?;
    let rows = statement
        .query_map(params![MAX_ALPHA3_BODY_BYTES as i64], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (fact_id, observed_at, source_digest, bounded) in rows {
        let fact: InteractionFactV1 = parse_body(bounded, "interaction fact")?;
        ae_contracts::validate_interaction_fact(&fact)
            .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
        if fact_id.as_slice() != fact.fact_id
            || u64::try_from(observed_at).ok() != Some(fact.observed_at_utc_ms)
            || source_digest.as_slice() != fact.source_digest
        {
            return Err(StoreError::AutonomyConflict(
                "interaction fact columns differ from its body".into(),
            ));
        }
    }
    Ok(())
}

fn verify_contact_basis_rows(conn: &Connection) -> Result<(), StoreError> {
    let mut statement = conn.prepare(
        "SELECT intention_id,relation_scope,cause_digest,consent_epoch,consent_revision,live,
                CASE WHEN length(CAST(body_json AS BLOB))<=?1 THEN body_json END
         FROM contact_intention_basis ORDER BY relation_scope,intention_id",
    )?;
    let rows = statement
        .query_map(params![MAX_ALPHA3_BODY_BYTES as i64], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (id, scope, cause, epoch, revision, live, bounded) in rows {
        let basis: ContactIntentionBasisV1 = parse_body(bounded, "contact intention basis")?;
        basis
            .validate()
            .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
        if id.as_slice() != basis.intention_id
            || scope.as_slice() != basis.relation_scope
            || cause.as_slice() != basis.cause_digest
            || u64::try_from(epoch).ok() != Some(basis.consent_epoch)
            || u64::try_from(revision).ok() != Some(basis.consent_revision)
            || (live != 0) != basis.live
        {
            return Err(StoreError::AutonomyConflict(
                "contact intention basis columns differ from its body".into(),
            ));
        }
    }
    Ok(())
}

#[allow(dead_code)]
fn verify_ecosystem_rows(conn: &Connection) -> Result<(), StoreError> {
    let mut statement = conn.prepare(
        "SELECT plugin_instance_digest,capability_digest,revision,state,
                CASE WHEN length(CAST(body_json AS BLOB))<=?1 THEN body_json END
         FROM ecosystem_capability_grant ORDER BY plugin_instance_digest,revision",
    )?;
    let rows = statement
        .query_map(params![MAX_ALPHA3_BODY_BYTES as i64], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (plugin, capability, revision, state, bounded) in rows {
        let grant: EcosystemCapabilityGrantV1 = parse_body(bounded, "ecosystem grant")?;
        grant
            .validate()
            .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
        if plugin.as_slice() != grant.plugin_instance_digest
            || capability.as_slice() != grant.capability_digest
            || u64::try_from(revision).ok() != Some(grant.revision)
            || state != grant_state_name(grant.state)
        {
            return Err(StoreError::AutonomyConflict(
                "ecosystem grant columns differ from its body".into(),
            ));
        }
    }
    drop(statement);

    let mut statement = conn.prepare(
        "SELECT proposal_id,plugin_instance_digest,semantic_digest,
                CASE WHEN length(CAST(body_json AS BLOB))<=?1 THEN body_json END
         FROM ecosystem_proposal ORDER BY persona_scope,revision,proposal_id",
    )?;
    let rows = statement
        .query_map(params![MAX_ALPHA3_BODY_BYTES as i64], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (proposal_id, plugin, semantic, bounded) in rows {
        let proposal: EcosystemProposalV1 = parse_body(bounded, "ecosystem proposal")?;
        proposal
            .validate()
            .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
        if proposal_id.as_slice() != proposal.proposal_id
            || plugin.as_slice() != proposal.plugin_instance_digest
            || semantic.as_slice() != proposal.semantic_idempotency_digest
        {
            return Err(StoreError::AutonomyConflict(
                "ecosystem proposal columns differ from its body".into(),
            ));
        }
    }
    Ok(())
}

#[allow(dead_code)]
fn verify_dream_rows(conn: &Connection) -> Result<(), StoreError> {
    let mut statement = conn.prepare(
        "SELECT residue_id,persona_scope,state,
                CASE WHEN length(CAST(body_json AS BLOB))<=?1 THEN body_json END
         FROM dream_residue ORDER BY persona_scope,revision,residue_id",
    )?;
    let rows = statement
        .query_map(params![MAX_ALPHA3_BODY_BYTES as i64], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (id, scope, state, bounded) in rows {
        let dream: DreamResidueV1 = parse_body(bounded, "dream residue")?;
        dream
            .validate()
            .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
        if id.as_slice() != dream.residue_id
            || scope.as_slice() != dream.persona_scope
            || state != dream_state_name(dream.state)
        {
            return Err(StoreError::AutonomyConflict(
                "dream residue columns differ from its body".into(),
            ));
        }
    }
    Ok(())
}

fn verify_projection_rows(conn: &Connection) -> Result<(), StoreError> {
    let mut statement = conn.prepare(
        "SELECT relation_scope,revision,
                CASE WHEN length(CAST(body_json AS BLOB))<=?1 THEN body_json END
         FROM relation_budget_policy ORDER BY relation_scope",
    )?;
    let rows = statement
        .query_map(params![MAX_ALPHA3_BODY_BYTES as i64], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (scope, revision, bounded) in rows {
        let policy: RelationBudgetPolicyV1 = parse_body(bounded, "relation budget policy")?;
        policy
            .validate()
            .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
        if scope.as_slice() != policy.relation_scope
            || u64::try_from(revision).ok() != Some(policy.revision)
        {
            return Err(StoreError::AutonomyConflict(
                "relation budget policy columns differ from its body".into(),
            ));
        }
    }
    drop(statement);

    let mut statement = conn.prepare(
        "SELECT relation_scope,revision,witness_digest,
                CASE WHEN length(CAST(body_json AS BLOB))<=?1 THEN body_json END
         FROM proactive_readiness ORDER BY relation_scope",
    )?;
    let rows = statement
        .query_map(params![MAX_ALPHA3_BODY_BYTES as i64], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (scope, revision, witness, bounded) in rows {
        let readiness: ProactiveReadinessV1 = parse_body(bounded, "proactive readiness")?;
        readiness
            .validate()
            .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
        if scope.as_slice() != readiness.relation_scope
            || u64::try_from(revision).ok() != Some(readiness.revision)
            || witness.as_slice() != readiness.witness_digest
        {
            return Err(StoreError::AutonomyConflict(
                "proactive readiness columns differ from its body".into(),
            ));
        }
    }
    drop(statement);

    let mut statement = conn.prepare(
        "SELECT CASE WHEN length(CAST(body_json AS BLOB))<=?1 THEN body_json END
         FROM gate_decision_latest ORDER BY relation_scope,gate_phase",
    )?;
    let bodies = statement
        .query_map(params![MAX_ALPHA3_BODY_BYTES as i64], |row| {
            row.get::<_, Option<String>>(0)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for bounded in bodies {
        let gate: GateDecisionV2 = parse_body(bounded, "gate decision")?;
        gate.validate()
            .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
    }
    Ok(())
}

pub(crate) fn verify_alpha3_v7(conn: &Connection) -> Result<(), StoreError> {
    verify_alpha3_v7_impl(conn, false)
}

pub(crate) fn verify_alpha3_v7_core_boundary(conn: &Connection) -> Result<(), StoreError> {
    // Retirement records unprovable reservations as frozen/no-refund. All
    // other v7 authority checks remain identical to the legacy verifier.
    verify_alpha3_v7_impl(conn, true)
}

fn verify_alpha3_v7_impl(conn: &Connection, retirement_overlay: bool) -> Result<(), StoreError> {
    if !required_schema_objects_exist(conn)? || !retired_schema_shape_matches(conn)? {
        return Err(StoreError::AutonomyConflict(
            "autonomy schema v7 is missing required alpha3 authority objects".into(),
        ));
    }
    let malformed_rows: i64 = conn.query_row(
        "SELECT
             EXISTS(SELECT 1 FROM interaction_fact WHERE length(fact_id)!=16 OR length(event_id)!=16 OR length(persona_scope)!=32 OR length(relation_scope)!=32 OR length(source_digest)!=32 OR revision<=0 OR length(CAST(body_json AS BLOB))>?1)
          OR EXISTS(SELECT 1 FROM contact_intention_basis WHERE length(intention_id)!=16 OR length(relation_scope)!=32 OR length(cause_digest)!=32 OR revision<=0 OR length(CAST(body_json AS BLOB))>?1)
          OR EXISTS(SELECT 1 FROM relation_budget_policy WHERE length(relation_scope)!=32 OR revision<=0 OR length(CAST(body_json AS BLOB))>?1)
          OR EXISTS(SELECT 1 FROM proactive_readiness WHERE length(relation_scope)!=32 OR length(witness_digest)!=32 OR revision<=0 OR length(CAST(body_json AS BLOB))>?1)
          OR EXISTS(SELECT 1 FROM gate_decision_latest WHERE length(relation_scope)!=32 OR revision<=0 OR length(CAST(body_json AS BLOB))>?1)
          OR (?2=0 AND (EXISTS(SELECT 1 FROM externalization_budget WHERE reserved_tokens<0 OR limit_tokens<0 OR charged_tokens<0 OR used_tokens<0 OR usage_known NOT IN (0,1) OR revision<=0 OR charged_tokens+reserved_tokens>limit_tokens OR used_tokens>charged_tokens)
          OR EXISTS(
               SELECT 1 FROM externalization_budget_claim AS c
               LEFT JOIN externalization_budget AS b
                 ON b.relation_scope=c.relation_scope
                AND b.budget_day_start_utc_ms=c.budget_day_start_utc_ms
               WHERE length(c.claim_token)!=32
                  OR length(c.relation_scope)!=32
                  OR c.budget_day_start_utc_ms<0
                  OR c.reserved_tokens<0
                  OR c.migrated_unknown_full_charge NOT IN (0,1)
                  OR (c.migrated_unknown_full_charge=1 AND
                      (b.rowid IS NULL OR b.usage_known!=0 OR b.charged_tokens<c.reserved_tokens))
          )
          OR EXISTS(
               SELECT 1 FROM externalization_budget_claim AS b
               LEFT JOIN autonomy_claim AS c ON c.claim_token=b.claim_token
               WHERE c.claim_token IS NULL OR c.claim_kind!='externalization'
          )
          OR EXISTS(
               SELECT 1 FROM autonomy_claim AS c
               LEFT JOIN externalization_budget_claim AS b ON b.claim_token=c.claim_token
               WHERE c.claim_kind='externalization' AND b.claim_token IS NULL
          )))",
        params![MAX_ALPHA3_BODY_BYTES as i64, retirement_overlay],
        |row| row.get(0),
    )?;
    if malformed_rows != 0 {
        return Err(StoreError::AutonomyConflict(
            "autonomy schema v7 contains malformed alpha3 authority rows".into(),
        ));
    }
    verify_relation_authority(conn)?;
    verify_interaction_rows(conn)?;
    verify_contact_basis_rows(conn)?;
    verify_projection_rows(conn)?;

    let dangling_heads: i64 = conn.query_row(
        "SELECT COUNT(*) FROM relation_consent_head AS h
         LEFT JOIN relation_consent AS c
           ON c.relation_scope=h.relation_scope
          AND c.consent_epoch=h.consent_epoch
          AND c.revision=h.revision
         WHERE c.relation_scope IS NULL",
        [],
        |row| row.get(0),
    )?;
    if dangling_heads != 0 {
        return Err(StoreError::AutonomyConflict(
            "relation consent head is not backed by authority".into(),
        ));
    }
    Ok(())
}
