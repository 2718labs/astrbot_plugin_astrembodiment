use super::{blob, bounded_typed_value, now_ms, Store, StoreError};
use crate::alpha3::schema::{self, MIGRATION_DIGEST_V7};
use ae_contracts::*;
use ae_fixed::Fixed;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use sha2::{Digest as ShaDigest, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;

pub const AUTONOMY_DB_VERSION: u32 = crate::core_boundary_v9::VERSION;
pub(crate) const AUTONOMY_DB_VERSION_V8: u32 = 8;
const AUTONOMY_DB_VERSION_V6: u32 = 6;
const AUTONOMY_DB_VERSION_V7: u32 = 7;
const MIGRATION_DIGEST_V6: &[u8] = b"ae.autonomy.db.v6.inner-event-manifest.v1";
const MIGRATION_DIGEST_V8: &[u8] = b"ae.autonomy.db.v8.local-time-cognition.v1";
const OPERATIONAL_CHECKPOINT_KIND: &str = "operational_checkpoint_v1";
const MAX_AUTONOMY_DELTA_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_AUTONOMY_CLAIM_BODY_BYTES: usize = MAX_AUTONOMY_DELTA_BYTES;
const MAX_CANONICAL_EVENT_BYTES: usize = 256 * 1024;
const MAX_RUNTIME_STATE_BYTES: usize = 256 * 1024;
const MAX_SNAPSHOT_STATE_BYTES: usize = 256 * 1024;
const MAX_INNER_EVENTS_PER_DELTA: usize = 1024;
// Retained historical verification data; no active executor is restored.
#[allow(dead_code)]
const MAX_INTENTIONS_PER_WAKE: usize = 1024;
const MAX_EVENT_SOURCE_IDS: usize = 64;
const MAX_INNER_EVENT_SUMMARY_BYTES: usize = 256;
const MAX_INTENTION_ACTION_CLASS_BYTES: usize = 128;
const MAX_JOURNAL_EVENT_KIND_BYTES: usize = 64;
const MAX_OBSERVE_MANIFESTS_PER_PAGE: usize = 65;
const MAX_OBSERVE_WITNESS_CACHE_REVISIONS: usize = 8;
const MAX_V8_LOCAL_BODY_BYTES: u64 = 65_536;
const MAX_V8_SCHEMA_OBJECTS: u64 = 16;
const MAX_V8_SCHEMA_NAME_BYTES: u64 = 96;
const MAX_V8_SCHEMA_SQL_BYTES: u64 = 32 * 1024;
const MAX_V8_ROWS: u64 = 65_536;
const MAX_V8_AGGREGATE_BODY_BYTES: u64 = 64 * 1024 * 1024;

const AUTONOMY_SCHEMA_V8_SQL: &str = r#"
CREATE TABLE endogenous_intent_state_v1 (
    persona_scope BLOB PRIMARY KEY CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
    revision INTEGER NOT NULL CHECK(typeof(revision)='integer' AND revision>0),
    semantic_revision INTEGER NOT NULL CHECK(typeof(semantic_revision)='integer' AND semantic_revision>0),
    state_digest BLOB NOT NULL CHECK(typeof(state_digest)='blob' AND length(state_digest)=32),
    body_json TEXT NOT NULL CHECK(typeof(body_json)='text' AND length(CAST(body_json AS BLOB))<=65536)
);
CREATE TABLE local_dream_residue_v1 (
    residue_id BLOB PRIMARY KEY CHECK(typeof(residue_id)='blob' AND length(residue_id)=16),
    persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
    revision INTEGER NOT NULL CHECK(typeof(revision)='integer' AND revision>0),
    semantic_revision INTEGER NOT NULL CHECK(typeof(semantic_revision)='integer' AND semantic_revision>0),
    state_digest BLOB NOT NULL CHECK(typeof(state_digest)='blob' AND length(state_digest)=32),
    body_json TEXT NOT NULL CHECK(typeof(body_json)='text' AND length(CAST(body_json AS BLOB))<=65536),
    UNIQUE(persona_scope,residue_id)
);
CREATE INDEX local_dream_residue_persona_v1
  ON local_dream_residue_v1(persona_scope,semantic_revision);
CREATE TABLE wake_time_settlement_v1 (
    claim_token BLOB PRIMARY KEY CHECK(typeof(claim_token)='blob' AND length(claim_token)=32),
    persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
    event_id BLOB NOT NULL CHECK(typeof(event_id)='blob' AND length(event_id)=16),
    event_digest BLOB NOT NULL CHECK(typeof(event_digest)='blob' AND length(event_digest)=32),
    canonical_revision INTEGER NOT NULL CHECK(typeof(canonical_revision)='integer' AND canonical_revision>0),
    semantic_revision INTEGER CHECK(semantic_revision IS NULL OR (typeof(semantic_revision)='integer' AND semantic_revision>0)),
    state_generation INTEGER NOT NULL CHECK(typeof(state_generation)='integer' AND state_generation>0),
    state_revision INTEGER NOT NULL CHECK(typeof(state_revision)='integer' AND state_revision>0),
    state_digest BLOB NOT NULL CHECK(typeof(state_digest)='blob' AND length(state_digest)=32),
    result_bytes BLOB NOT NULL CHECK(typeof(result_bytes)='blob' AND length(result_bytes)<=65536),
    result_digest BLOB NOT NULL CHECK(typeof(result_digest)='blob' AND length(result_digest)=32),
    settled_at_ms INTEGER NOT NULL CHECK(typeof(settled_at_ms)='integer' AND settled_at_ms>0),
    UNIQUE(persona_scope,event_id),
    UNIQUE(persona_scope,event_digest)
);
"#;

fn autonomy_sql_i64(value: u64, field: &str) -> Result<i64, StoreError> {
    value
        .try_into()
        .map_err(|_| StoreError::AutonomyConflict(format!("{field} is out of SQLite range")))
}

fn autonomy_u64(value: i64, field: &str) -> Result<u64, StoreError> {
    value
        .try_into()
        .map_err(|_| StoreError::AutonomyConflict(format!("negative {field}")))
}

pub(crate) fn bounded_autonomy_claim_body(body: Option<String>) -> Result<String, StoreError> {
    body.ok_or_else(|| {
        StoreError::AutonomyConflict(
            "autonomy claim body exceeds its materialization verification bound".into(),
        )
    })
}

fn autonomy_delta_bound_violation(delta: &AutonomyJournalDeltaV1) -> Option<&'static str> {
    if delta.state.as_ref().is_some_and(|state| {
        serde_json::to_vec(state).map_or(true, |bytes| {
            bytes.len() > MAX_RUNTIME_STATE_BYTES || bytes.len() > MAX_SNAPSHOT_STATE_BYTES
        })
    }) {
        return Some("runtime state exceeds its 256 KiB verification bound");
    }
    if delta.inner_events.len() > MAX_INNER_EVENTS_PER_DELTA {
        return Some("inner event count exceeds 1024; future batch support is required");
    }
    for event in &delta.inner_events {
        if event.source_event_ids.len() > MAX_EVENT_SOURCE_IDS {
            return Some(
                "inner event source id count exceeds 64; future batch support is required",
            );
        }
        if event.summary_code.len() > MAX_INNER_EVENT_SUMMARY_BYTES {
            return Some("inner event summary exceeds 256 bytes");
        }
    }
    if let Some(intention) = &delta.intention {
        if intention.source_event_ids.len() > MAX_EVENT_SOURCE_IDS {
            return Some("intention source id count exceeds 64; future batch support is required");
        }
        if intention.action_class.len() > MAX_INTENTION_ACTION_CLASS_BYTES {
            return Some("intention action class exceeds 128 bytes");
        }
    }
    None
}

fn parse_bounded_autonomy_delta(
    bytes: &[u8],
    context: &str,
) -> Result<AutonomyJournalDeltaV1, StoreError> {
    if bytes.len() > MAX_AUTONOMY_DELTA_BYTES {
        return Err(StoreError::AutonomyConflict(format!(
            "{context} exceeds 1 MiB; future batch support is required"
        )));
    }
    let delta: AutonomyJournalDeltaV1 = serde_json::from_slice(bytes).map_err(|error| {
        StoreError::AutonomyConflict(format!("{context} decode failed: {error}"))
    })?;
    if let Some(reason) = autonomy_delta_bound_violation(&delta) {
        return Err(StoreError::AutonomyConflict(format!("{context}: {reason}")));
    }
    Ok(delta)
}

pub(crate) fn inner_event_manifest_digest(events: &[InnerEventV1]) -> Result<Digest, StoreError> {
    let mut ordered = events
        .iter()
        .map(|event| {
            serde_json::to_vec(event)
                .map(|body| (event.event_id, body))
                .map_err(|error| StoreError::AutonomyConflict(error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    ordered.sort_by_key(|(event_id, _)| *event_id);
    if ordered.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(StoreError::AutonomyConflict(
            "duplicate inner event id in canonical delta".into(),
        ));
    }
    let mut hasher = Sha256::new();
    hasher.update(b"ae.autonomy.inner-event-manifest.v1");
    hasher.update((ordered.len() as u64).to_le_bytes());
    for (event_id, body) in ordered {
        hasher.update(event_id);
        hasher.update((body.len() as u64).to_le_bytes());
        hasher.update(body);
    }
    Ok(hasher.finalize().into())
}

#[derive(Debug, PartialEq, Eq)]
struct AutonomyV8SchemaObject {
    kind: String,
    name: String,
    table: String,
    canonical_sql: Option<String>,
}

fn canonical_autonomy_schema_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn autonomy_v8_schema_object_count(conn: &Connection) -> Result<u64, StoreError> {
    let raw: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_schema
         WHERE type IN ('table','index','trigger') AND tbl_name IN (
           'endogenous_intent_state_v1','local_dream_residue_v1','wake_time_settlement_v1')",
        [],
        |row| row.get(0),
    )?;
    let count = autonomy_u64(raw, "autonomy v8 schema object count")?;
    if count > MAX_V8_SCHEMA_OBJECTS {
        return Err(StoreError::AutonomyConflict(
            "autonomy schema v8 has an unexpected object set".into(),
        ));
    }
    Ok(count)
}

fn autonomy_v8_schema_objects(
    conn: &Connection,
) -> Result<Vec<AutonomyV8SchemaObject>, StoreError> {
    let count = autonomy_v8_schema_object_count(conn)?;
    let mut statement = conn.prepare(
        "SELECT
           CASE WHEN typeof(type)='text' AND length(CAST(type AS BLOB))<=?1 THEN type END,
           CASE WHEN typeof(type)='text' THEN length(CAST(type AS BLOB)) ELSE -1 END,
           CASE WHEN typeof(name)='text' AND length(CAST(name AS BLOB))<=?1 THEN name END,
           CASE WHEN typeof(name)='text' THEN length(CAST(name AS BLOB)) ELSE -1 END,
           CASE WHEN typeof(tbl_name)='text' AND length(CAST(tbl_name AS BLOB))<=?1 THEN tbl_name END,
           CASE WHEN typeof(tbl_name)='text' THEN length(CAST(tbl_name AS BLOB)) ELSE -1 END,
           CASE WHEN sql IS NULL THEN NULL
                WHEN typeof(sql)='text' AND length(CAST(sql AS BLOB))<=?2 THEN sql END,
           CASE WHEN sql IS NULL THEN -2
                WHEN typeof(sql)='text' THEN length(CAST(sql AS BLOB)) ELSE -1 END
         FROM sqlite_schema
         WHERE type IN ('table','index','trigger') AND tbl_name IN (
           'endogenous_intent_state_v1','local_dream_residue_v1','wake_time_settlement_v1')
         ORDER BY type,name",
    )?;
    let mut rows = statement.query(params![
        MAX_V8_SCHEMA_NAME_BYTES as i64,
        MAX_V8_SCHEMA_SQL_BYTES as i64,
    ])?;
    let mut objects = Vec::with_capacity(usize::try_from(count).unwrap_or(0));
    while let Some(row) = rows.next()? {
        let kind = bounded_typed_value(
            row.get::<_, Option<String>>(0)?,
            row.get(1)?,
            MAX_V8_SCHEMA_NAME_BYTES,
            "autonomy_v8.schema_kind",
            "autonomy_v8_schema_type",
        )?;
        let name = bounded_typed_value(
            row.get::<_, Option<String>>(2)?,
            row.get(3)?,
            MAX_V8_SCHEMA_NAME_BYTES,
            "autonomy_v8.schema_name",
            "autonomy_v8_schema_type",
        )?;
        let table = bounded_typed_value(
            row.get::<_, Option<String>>(4)?,
            row.get(5)?,
            MAX_V8_SCHEMA_NAME_BYTES,
            "autonomy_v8.schema_table",
            "autonomy_v8_schema_type",
        )?;
        let raw_sql: Option<String> = row.get(6)?;
        let raw_sql_len: i64 = row.get(7)?;
        let canonical_sql = if raw_sql_len == -2 {
            if raw_sql.is_some() {
                return Err(StoreError::AutonomyConflict(
                    "autonomy schema v8 SQL nullability mismatch".into(),
                ));
            }
            None
        } else {
            Some(canonical_autonomy_schema_sql(&bounded_typed_value(
                raw_sql,
                raw_sql_len,
                MAX_V8_SCHEMA_SQL_BYTES,
                "autonomy_v8.schema_sql",
                "autonomy_v8_schema_type",
            )?))
        };
        objects.push(AutonomyV8SchemaObject {
            kind,
            name,
            table,
            canonical_sql,
        });
    }
    if u64::try_from(objects.len()).unwrap_or(u64::MAX) != count {
        return Err(StoreError::AutonomyConflict(
            "autonomy schema v8 object enumeration changed".into(),
        ));
    }
    Ok(objects)
}

fn require_exact_autonomy_schema_v8(conn: &Connection) -> Result<(), StoreError> {
    let reference = Connection::open_in_memory()?;
    reference.execute_batch(AUTONOMY_SCHEMA_V8_SQL)?;
    if autonomy_v8_schema_objects(conn)? != autonomy_v8_schema_objects(&reference)? {
        return Err(StoreError::AutonomyConflict(
            "autonomy schema v8 SQL identity mismatch".into(),
        ));
    }
    Ok(())
}

pub(crate) fn verify_autonomy_v8_schema_identity(conn: &Connection) -> Result<(), StoreError> {
    require_exact_autonomy_schema_v8(conn)?;
    let marker: bool=conn.query_row("SELECT COUNT(*)=1 FROM pragma_table_info('externalization_budget_claim') WHERE name='migrated_unknown_full_charge' AND type='INTEGER' AND \"notnull\"=1",[],|r|r.get(0))?;
    if !marker {
        return Err(StoreError::ContinuityFence("V9_LEGACY_SCHEMA_IDENTITY"));
    }
    Ok(())
}

fn verify_autonomy_v8_row_budget(conn: &Connection) -> Result<(), StoreError> {
    let (raw_rows, raw_bytes, malformed): (i64, i64, i64) = conn.query_row(
        "SELECT
           (SELECT COUNT(*) FROM endogenous_intent_state_v1)+
           (SELECT COUNT(*) FROM local_dream_residue_v1)+
           (SELECT COUNT(*) FROM wake_time_settlement_v1),
           COALESCE((SELECT SUM(length(CAST(body_json AS BLOB))) FROM endogenous_intent_state_v1),0)+
           COALESCE((SELECT SUM(length(CAST(body_json AS BLOB))) FROM local_dream_residue_v1),0)+
           COALESCE((SELECT SUM(length(result_bytes)) FROM wake_time_settlement_v1),0),
           EXISTS(SELECT 1 FROM endogenous_intent_state_v1 WHERE
             typeof(persona_scope)!='blob' OR length(persona_scope)!=32 OR
             typeof(revision)!='integer' OR revision<=0 OR
             typeof(semantic_revision)!='integer' OR semantic_revision<=0 OR
             typeof(state_digest)!='blob' OR length(state_digest)!=32 OR
             typeof(body_json)!='text' OR length(CAST(body_json AS BLOB))>65536)
           OR EXISTS(SELECT 1 FROM local_dream_residue_v1 WHERE
             typeof(residue_id)!='blob' OR length(residue_id)!=16 OR
             typeof(persona_scope)!='blob' OR length(persona_scope)!=32 OR
             typeof(revision)!='integer' OR revision<=0 OR
             typeof(semantic_revision)!='integer' OR semantic_revision<=0 OR
             typeof(state_digest)!='blob' OR length(state_digest)!=32 OR
             typeof(body_json)!='text' OR length(CAST(body_json AS BLOB))>65536)
           OR EXISTS(SELECT 1 FROM wake_time_settlement_v1 WHERE
             typeof(claim_token)!='blob' OR length(claim_token)!=32 OR
             typeof(persona_scope)!='blob' OR length(persona_scope)!=32 OR
             typeof(event_id)!='blob' OR length(event_id)!=16 OR
             typeof(event_digest)!='blob' OR length(event_digest)!=32 OR
             typeof(canonical_revision)!='integer' OR canonical_revision<=0 OR
             (semantic_revision IS NOT NULL AND
                (typeof(semantic_revision)!='integer' OR semantic_revision<=0)) OR
             typeof(state_generation)!='integer' OR state_generation<=0 OR
             typeof(state_revision)!='integer' OR state_revision<=0 OR
             typeof(state_digest)!='blob' OR length(state_digest)!=32 OR
             typeof(result_bytes)!='blob' OR length(result_bytes)>65536 OR
             typeof(result_digest)!='blob' OR length(result_digest)!=32 OR
             typeof(settled_at_ms)!='integer' OR settled_at_ms<=0)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let rows = autonomy_u64(raw_rows, "autonomy v8 row count")?;
    let bytes = autonomy_u64(raw_bytes, "autonomy v8 aggregate body bytes")?;
    if malformed != 0 || rows > MAX_V8_ROWS || bytes > MAX_V8_AGGREGATE_BODY_BYTES {
        return Err(StoreError::AutonomyConflict(
            "autonomy schema v8 rows violate their verification bound".into(),
        ));
    }
    Ok(())
}

fn verify_autonomy_v8_rows(conn: &Connection) -> Result<(), StoreError> {
    {
        type Row = (Vec<u8>, i64, i64, Vec<u8>, Option<String>, i64);
        let mut statement = conn.prepare(
            "SELECT persona_scope,revision,semantic_revision,state_digest,
                    CASE WHEN typeof(body_json)='text' AND length(CAST(body_json AS BLOB))<=?1
                         THEN body_json END,
                    CASE WHEN typeof(body_json)='text' THEN length(CAST(body_json AS BLOB)) ELSE -1 END
             FROM endogenous_intent_state_v1 ORDER BY persona_scope",
        )?;
        let mut rows = statement.query(params![MAX_V8_LOCAL_BODY_BYTES as i64])?;
        while let Some(row) = rows.next()? {
            let raw: Row = (
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            );
            let persona_scope: Digest = raw.0.try_into().map_err(|_| {
                StoreError::AutonomyConflict("endogenous persona digest is malformed".into())
            })?;
            let revision = autonomy_u64(raw.1, "endogenous intent revision")?;
            let semantic_revision = autonomy_u64(raw.2, "endogenous semantic revision")?;
            let state_digest: Digest = raw.3.try_into().map_err(|_| {
                StoreError::AutonomyConflict("endogenous state digest is malformed".into())
            })?;
            let body = bounded_typed_value(
                raw.4,
                raw.5,
                MAX_V8_LOCAL_BODY_BYTES,
                "endogenous_intent.body_json",
                "endogenous_intent_body_type",
            )?;
            let decoded: EndogenousIntentStateV1 = parse(body.clone())?;
            decoded
                .validate_v1()
                .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
            if serde_json::to_string(&decoded)
                .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?
                != body
                || decoded.persona_scope != persona_scope
                || decoded.revision != revision
                || decoded.source_semantic_revision != semantic_revision
                || decoded.commitment_digest != state_digest
            {
                return Err(StoreError::AutonomyConflict(
                    "endogenous intent row is not canonical".into(),
                ));
            }
        }
    }
    {
        type Row = (Vec<u8>, Vec<u8>, i64, i64, Vec<u8>, Option<String>, i64);
        let mut statement = conn.prepare(
            "SELECT residue_id,persona_scope,revision,semantic_revision,state_digest,
                    CASE WHEN typeof(body_json)='text' AND length(CAST(body_json AS BLOB))<=?1
                         THEN body_json END,
                    CASE WHEN typeof(body_json)='text' THEN length(CAST(body_json AS BLOB)) ELSE -1 END
             FROM local_dream_residue_v1 ORDER BY persona_scope,semantic_revision,residue_id",
        )?;
        let mut rows = statement.query(params![MAX_V8_LOCAL_BODY_BYTES as i64])?;
        while let Some(row) = rows.next()? {
            let raw: Row = (
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            );
            let residue_id: Id128 = raw.0.try_into().map_err(|_| {
                StoreError::AutonomyConflict("dream residue id is malformed".into())
            })?;
            let persona_scope: Digest = raw.1.try_into().map_err(|_| {
                StoreError::AutonomyConflict("dream persona digest is malformed".into())
            })?;
            let revision = autonomy_u64(raw.2, "dream residue revision")?;
            let semantic_revision = autonomy_u64(raw.3, "dream semantic revision")?;
            let state_digest: Digest = raw.4.try_into().map_err(|_| {
                StoreError::AutonomyConflict("dream state digest is malformed".into())
            })?;
            let body = bounded_typed_value(
                raw.5,
                raw.6,
                MAX_V8_LOCAL_BODY_BYTES,
                "dream_residue.body_json",
                "dream_residue_body_type",
            )?;
            let decoded: LocalDreamResidueV1 = parse(body.clone())?;
            decoded
                .validate_v1()
                .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
            let source_event_id = decoded.source_event_ids.first().copied();
            let semantic_closes = if let Some(source_event_id) = source_event_id {
                conn.query_row(
                    "SELECT COUNT(*) FROM semantic_time_authority
                     WHERE persona_scope=?1 AND semantic_revision=?2 AND event_id=?3",
                    params![
                        blob(persona_scope),
                        autonomy_sql_i64(semantic_revision, "dream semantic revision")?,
                        source_event_id.to_vec(),
                    ],
                    |row| row.get::<_, i64>(0),
                )? == 1
            } else {
                false
            };
            if serde_json::to_string(&decoded)
                .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?
                != body
                || decoded.residue_id != residue_id
                || decoded.persona_scope != persona_scope
                || revision != 1
                || decoded.commitment_digest != state_digest
                || !semantic_closes
            {
                return Err(StoreError::AutonomyConflict(
                    "local dream residue row is not canonical".into(),
                ));
            }
        }
    }
    let claim_tokens = {
        let mut statement = conn.prepare(
            "SELECT CASE WHEN typeof(claim_token)='blob' AND length(claim_token)=32
                         THEN claim_token END
             FROM wake_time_settlement_v1 ORDER BY claim_token",
        )?;
        let mut rows = statement.query([])?;
        let mut tokens = Vec::new();
        while let Some(row) = rows.next()? {
            let token: Option<Vec<u8>> = row.get(0)?;
            let token: Digest = token
                .ok_or_else(|| {
                    StoreError::AutonomyConflict("wake settlement claim token is malformed".into())
                })?
                .try_into()
                .map_err(|_| {
                    StoreError::AutonomyConflict("wake settlement claim token is malformed".into())
                })?;
            tokens.push(token);
        }
        tokens
    };
    for token in claim_tokens {
        if read_wake_time_settlement_v1(conn, &token)?.is_none() {
            return Err(StoreError::AutonomyConflict(
                "wake settlement row disappeared during validation".into(),
            ));
        }
    }
    Ok(())
}

/// V9 admission must never use the legacy verifier that rebuilds authority
/// from checkpoints. This reader rejects disagreements without repairing them.
pub(crate) fn verify_autonomy_v8_read_only(conn: &Connection) -> Result<(), StoreError> {
    for (version, digest) in [
        (AUTONOMY_DB_VERSION_V6, MIGRATION_DIGEST_V6),
        (AUTONOMY_DB_VERSION_V7, MIGRATION_DIGEST_V7),
        (AUTONOMY_DB_VERSION_V8, MIGRATION_DIGEST_V8),
    ] {
        let actual: Option<Vec<u8>> = conn
            .query_row(
                "SELECT digest FROM schema_migrations WHERE version=?1",
                params![version],
                |row| row.get(0),
            )
            .optional()?;
        if actual.as_deref() != Some(digest) {
            return Err(StoreError::ContinuityFence("V9_LEGACY_MIGRATION_IDENTITY"));
        }
    }
    // Authenticate the new deny triggers separately, then compare only the
    // unchanged legacy objects. The ordinary v8 verifier remains strict.
    let reference = Connection::open_in_memory()?;
    reference.execute_batch(AUTONOMY_SCHEMA_V8_SQL)?;
    let expected = autonomy_v8_schema_objects(&reference)?;
    let catalog = crate::core_boundary_v9::bounded_catalog(conn)?;
    let v9 = crate::core_boundary_v9::render_schema()?;
    let has_v9 = catalog
        .iter()
        .any(|object| object.name == "core_boundary_control_v1");
    if has_v9 {
        crate::core_boundary_v9::verify_schema_catalog(&catalog, &v9)?;
    }
    let actual: Vec<_> = catalog
        .iter()
        .filter(|object| {
            matches!(
                object.table.as_str(),
                "endogenous_intent_state_v1" | "local_dream_residue_v1" | "wake_time_settlement_v1"
            ) && !(has_v9 && v9.objects.contains(object))
        })
        .map(|object| AutonomyV8SchemaObject {
            kind: object.kind.clone(),
            name: object.name.clone(),
            table: object.table.clone(),
            canonical_sql: if object.sql.is_empty() {
                None
            } else {
                Some(canonical_autonomy_schema_sql(&object.sql))
            },
        })
        .collect();
    let mut actual = actual;
    actual.sort_by(|a, b| (&a.kind, &a.name).cmp(&(&b.kind, &b.name)));
    if actual != expected {
        return Err(StoreError::ContinuityFence("V9_LEGACY_SCHEMA_IDENTITY"));
    }
    verify_autonomy_storage_bounds_v6(conn)?;
    verify_autonomy_v8_row_budget(conn)?;
    verify_autonomy_v8_rows(conn)?;
    schema::verify_alpha3_v7_core_boundary(conn)?;
    verify_all_inner_event_manifests_v6(conn)?;
    verify_relation_consent_authority_v7(conn)?;
    let mut statement = conn.prepare(
        "SELECT persona_scope FROM autonomy_operational_authority
         UNION SELECT persona_scope FROM autonomy_operational_authority_head
         UNION SELECT scope_digest FROM journal WHERE event_kind='operational_checkpoint_v1'
         ORDER BY persona_scope LIMIT 65537",
    )?;
    let mut rows = statement.query([])?;
    let mut count = 0usize;
    while let Some(row) = rows.next()? {
        count += 1;
        if count > 65_536 {
            return Err(StoreError::ContinuityFence("V9_LEGACY_OWNER_LIMIT"));
        }
        let scope: Digest = row
            .get::<_, Vec<u8>>(0)?
            .try_into()
            .map_err(|_| StoreError::ContinuityFence("V9_LEGACY_OWNER_IDENTITY"))?;
        let checkpoints = read_operational_checkpoints(conn, &scope)?;
        let expected: Vec<_> = checkpoints
            .into_iter()
            .map(|(ordinal, checkpoint)| (ordinal, checkpoint.delta))
            .collect();
        if read_operational_authority(conn, &scope)? != expected {
            return Err(StoreError::ContinuityFence(
                "V9_LEGACY_OPERATIONAL_AUTHORITY",
            ));
        }
    }
    Ok(())
}

pub(crate) fn migrate_autonomy(tx: &Transaction<'_>, from_version: u32) -> Result<u32, StoreError> {
    if from_version > AUTONOMY_DB_VERSION_V8 {
        return Err(StoreError::AutonomyConflict(
            "database is newer than binary".into(),
        ));
    }
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY, digest BLOB NOT NULL, completed_at_ms INTEGER NOT NULL
        );",
    )?;
    let v7_digest: Option<Vec<u8>> = tx
        .query_row(
            "SELECT digest FROM schema_migrations WHERE version=?1",
            params![i64::from(AUTONOMY_DB_VERSION_V7)],
            |row| row.get(0),
        )
        .optional()?;
    if v7_digest
        .as_deref()
        .is_some_and(|digest| digest != MIGRATION_DIGEST_V7)
    {
        return Err(StoreError::AutonomyConflict(
            "autonomy schema v7 migration digest mismatch".into(),
        ));
    }
    if from_version >= AUTONOMY_DB_VERSION_V6 {
        let v6_digest: Option<Vec<u8>> = tx
            .query_row(
                "SELECT digest FROM schema_migrations WHERE version=?1",
                params![i64::from(AUTONOMY_DB_VERSION_V6)],
                |row| row.get(0),
            )
            .optional()?;
        if v6_digest.as_deref() != Some(MIGRATION_DIGEST_V6) {
            return Err(StoreError::AutonomyConflict(
                "autonomy schema v6 migration digest mismatch".into(),
            ));
        }
    }
    tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS persona_temporal_profile (
            persona_scope BLOB PRIMARY KEY, revision INTEGER NOT NULL, body_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS relation_temporal_policy (
            relation_scope BLOB PRIMARY KEY, revision INTEGER NOT NULL, body_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS autonomous_runtime_state (
            persona_scope BLOB PRIMARY KEY, generation INTEGER NOT NULL,
            state_revision INTEGER NOT NULL, body_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS autonomy_snapshot (
            persona_scope BLOB NOT NULL, journal_revision INTEGER NOT NULL,
            state_digest BLOB NOT NULL, state_bytes BLOB NOT NULL,
            PRIMARY KEY(persona_scope, journal_revision)
        );
        CREATE TABLE IF NOT EXISTS autonomy_scope_binding (
            work_scope BLOB PRIMARY KEY, persona_scope BLOB NOT NULL,
            relation_scope BLOB, scope_json TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS autonomy_scope_binding_persona
            ON autonomy_scope_binding(persona_scope);
        CREATE TABLE IF NOT EXISTS inner_event (
            event_id BLOB PRIMARY KEY, persona_scope BLOB NOT NULL,
            journal_revision INTEGER NOT NULL, committed_at_utc_ms INTEGER NOT NULL, kind TEXT NOT NULL,
            tombstoned INTEGER NOT NULL DEFAULT 0, body_json TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS inner_event_scope_order
            ON inner_event(persona_scope, committed_at_utc_ms, event_id);
        CREATE TABLE IF NOT EXISTS inner_event_manifest (
            persona_scope BLOB NOT NULL, journal_revision INTEGER NOT NULL,
            event_count INTEGER NOT NULL, event_digest BLOB NOT NULL,
            PRIMARY KEY(persona_scope, journal_revision)
        );
        CREATE TABLE IF NOT EXISTS durable_intention (
            intention_id BLOB PRIMARY KEY, persona_scope BLOB NOT NULL,
            relation_scope BLOB NOT NULL, semantic_digest BLOB NOT NULL UNIQUE,
            state TEXT NOT NULL, revision INTEGER NOT NULL, body_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS wake_schedule (
            persona_scope BLOB PRIMARY KEY, generation INTEGER NOT NULL UNIQUE,
            next_wake_at_utc_ms INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS outbound_target (
            binding_digest BLOB PRIMARY KEY, relation_scope BLOB NOT NULL,
            generation INTEGER NOT NULL, revoked INTEGER NOT NULL DEFAULT 0,
            body_json TEXT NOT NULL,
            UNIQUE(relation_scope, generation)
        );
        CREATE TABLE IF NOT EXISTS outbound_attempt (
            outbound_id BLOB PRIMARY KEY, intention_id BLOB NOT NULL UNIQUE,
            state TEXT NOT NULL, target_digest BLOB NOT NULL,
            body_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS autonomy_claim (
            claim_token BLOB PRIMARY KEY, claim_kind TEXT NOT NULL,
            record_id BLOB NOT NULL, caller_incarnation BLOB NOT NULL,
            lease_deadline_utc_ms INTEGER NOT NULL,
            body_json TEXT NOT NULL CHECK(typeof(body_json)='text' AND length(CAST(body_json AS BLOB))<=1048576),
            UNIQUE(claim_kind, record_id)
        );
        CREATE TABLE IF NOT EXISTS externalization_budget (
            relation_scope BLOB NOT NULL, budget_day_start_utc_ms INTEGER NOT NULL,
            reserved_tokens INTEGER NOT NULL,
            PRIMARY KEY(relation_scope, budget_day_start_utc_ms)
        );
        CREATE TABLE IF NOT EXISTS externalization_budget_claim (
            claim_token BLOB PRIMARY KEY, relation_scope BLOB NOT NULL,
            budget_day_start_utc_ms INTEGER NOT NULL, reserved_tokens INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS autonomy_operational_authority (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            persona_scope BLOB NOT NULL,
            persona_ordinal INTEGER NOT NULL DEFAULT 0,
            relation_scope BLOB NOT NULL,
            intention_id BLOB NOT NULL,
            event_kind TEXT NOT NULL,
            delta_json TEXT NOT NULL,
            previous_digest BLOB NOT NULL,
            chain_digest BLOB NOT NULL,
            committed_at_utc_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS autonomy_operational_authority_scope_order
            ON autonomy_operational_authority(persona_scope, sequence);
        CREATE TABLE IF NOT EXISTS autonomy_operational_authority_head (
            persona_scope BLOB PRIMARY KEY,
            entry_count INTEGER NOT NULL,
            head_digest BLOB NOT NULL,
            ever_seen INTEGER NOT NULL
        );
        "#,
    )?;
    if !table_has_column(tx, "autonomy_operational_authority", "persona_ordinal")? {
        tx.execute(
            "ALTER TABLE autonomy_operational_authority ADD COLUMN persona_ordinal INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    if !table_has_column(tx, "autonomy_scope_binding", "work_scope")? {
        migrate_scope_bindings_v4(tx)?;
    }
    tx.execute_batch(
        "CREATE INDEX IF NOT EXISTS autonomy_scope_binding_persona_relation ON autonomy_scope_binding(persona_scope,relation_scope);",
    )?;
    // Reject oversized or malformed stored values before any migration path
    // asks SQLite to materialize their contents into Rust.
    verify_autonomy_storage_bounds_v6(tx)?;
    if !table_has_column(tx, "inner_event", "journal_revision")? {
        tx.execute(
            "ALTER TABLE inner_event ADD COLUMN journal_revision INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
        backfill_inner_event_journal_revisions_v6(tx)?;
    }
    if from_version < AUTONOMY_DB_VERSION_V6 {
        rebuild_inner_event_manifests_v6(tx)?;
    }
    let unbound_inner_events: i64 = tx.query_row(
        "SELECT COUNT(*) FROM inner_event WHERE journal_revision<=0",
        [],
        |row| row.get(0),
    )?;
    if unbound_inner_events != 0 {
        return Err(StoreError::AutonomyConflict(
            "inner event lacks reconstructible canonical journal revision".into(),
        ));
    }
    tx.execute_batch(
        "CREATE INDEX IF NOT EXISTS inner_event_scope_revision_order ON inner_event(persona_scope,journal_revision,event_id);",
    )?;
    verify_all_inner_event_manifests_v6(tx)?;
    if from_version < 4 {
        backfill_operational_authority_anchors(tx)?;
        bootstrap_legacy_operational_authority(tx)?;
    }
    tx.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS autonomy_operational_authority_scope_ordinal ON autonomy_operational_authority(persona_scope, persona_ordinal);",
    )?;
    if from_version < 5 {
        backfill_operational_checkpoints(tx)?;
    }
    verify_all_operational_anchors(tx)?;
    let completed_at_ms: i64 = now_ms().try_into().map_err(|_| {
        StoreError::AutonomyConflict("migration completion time is out of range".into())
    })?;
    tx.execute(
        "INSERT OR IGNORE INTO schema_migrations(version,digest,completed_at_ms) VALUES (?1,?2,?3)",
        params![
            i64::from(AUTONOMY_DB_VERSION_V6),
            MIGRATION_DIGEST_V6,
            completed_at_ms
        ],
    )?;
    let installed_v6_digest: Vec<u8> = tx.query_row(
        "SELECT digest FROM schema_migrations WHERE version=?1",
        params![i64::from(AUTONOMY_DB_VERSION_V6)],
        |row| row.get(0),
    )?;
    if installed_v6_digest != MIGRATION_DIGEST_V6 {
        return Err(StoreError::AutonomyConflict(
            "autonomy schema v6 migration digest mismatch".into(),
        ));
    }

    // The installed v6 receipt is verified before schema v7 can execute any
    // DDL or conservative backfill.
    schema::migrate_alpha3_v7(tx, from_version)?;
    verify_relation_consent_authority_v7(tx)?;
    tx.execute(
        "INSERT OR IGNORE INTO schema_migrations(version,digest,completed_at_ms) VALUES (?1,?2,?3)",
        params![
            i64::from(AUTONOMY_DB_VERSION_V7),
            MIGRATION_DIGEST_V7,
            completed_at_ms
        ],
    )?;
    let installed_v7_digest: Vec<u8> = tx.query_row(
        "SELECT digest FROM schema_migrations WHERE version=?1",
        params![i64::from(AUTONOMY_DB_VERSION_V7)],
        |row| row.get(0),
    )?;
    if installed_v7_digest != MIGRATION_DIGEST_V7 {
        return Err(StoreError::AutonomyConflict(
            "autonomy schema v7 migration digest mismatch".into(),
        ));
    }
    if from_version < AUTONOMY_DB_VERSION_V8 {
        if autonomy_v8_schema_object_count(tx)? != 0 {
            return Err(StoreError::AutonomyConflict(
                "autonomy schema v8 objects exist before their migration receipt".into(),
            ));
        }
        tx.execute_batch(AUTONOMY_SCHEMA_V8_SQL)?;
        tx.execute(
            "INSERT INTO schema_migrations(version,digest,completed_at_ms) VALUES (?1,?2,?3)",
            params![
                i64::from(AUTONOMY_DB_VERSION_V8),
                MIGRATION_DIGEST_V8,
                completed_at_ms
            ],
        )?;
    }
    require_exact_autonomy_schema_v8(tx)?;
    verify_autonomy_v8_row_budget(tx)?;
    verify_autonomy_v8_rows(tx)?;
    let installed_v8_digest: Vec<u8> = tx.query_row(
        "SELECT digest FROM schema_migrations WHERE version=?1",
        params![i64::from(AUTONOMY_DB_VERSION_V8)],
        |row| row.get(0),
    )?;
    if installed_v8_digest != MIGRATION_DIGEST_V8 {
        return Err(StoreError::AutonomyConflict(
            "autonomy schema v8 migration digest mismatch".into(),
        ));
    }
    let newest: u32 = tx.query_row(
        "SELECT COALESCE(MAX(version),0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?;
    if newest > AUTONOMY_DB_VERSION_V8 {
        return Err(StoreError::AutonomyConflict(
            "database is newer than binary".into(),
        ));
    }
    Ok(AUTONOMY_DB_VERSION_V8)
}

fn table_has_column(tx: &Transaction<'_>, table: &str, wanted: &str) -> Result<bool, StoreError> {
    let mut statement = tx.prepare(&format!("PRAGMA table_info({table})"))?;
    for column in statement.query_map([], |row| row.get::<_, String>(1))? {
        if column? == wanted {
            return Ok(true);
        }
    }
    Ok(false)
}

fn verify_autonomy_storage_bounds_v6(conn: &Connection) -> Result<(), StoreError> {
    let malformed_identity: i64 = conn.query_row(
        "SELECT
            EXISTS(SELECT 1 FROM journal WHERE length(scope_digest)!=32 OR length(event_digest)!=32 OR length(chain_digest)!=32)
            OR EXISTS(SELECT 1 FROM autonomous_runtime_state WHERE length(persona_scope)!=32)
            OR EXISTS(SELECT 1 FROM autonomy_snapshot WHERE length(persona_scope)!=32 OR length(state_digest)!=32)
            OR EXISTS(SELECT 1 FROM inner_event WHERE length(event_id)!=16 OR length(persona_scope)!=32)
            OR EXISTS(SELECT 1 FROM inner_event_manifest WHERE length(persona_scope)!=32 OR length(event_digest)!=32)",
        [],
        |row| row.get(0),
    )?;
    if malformed_identity != 0 {
        return Err(StoreError::AutonomyConflict(
            "autonomy storage contains a malformed fixed-size identity".into(),
        ));
    }
    let oversized: i64 = conn.query_row(
        "SELECT
            EXISTS(SELECT 1 FROM journal WHERE length(CAST(event_kind AS BLOB))>?1 OR length(event_bytes)>?2 OR length(delta_bytes)>?3)
            OR EXISTS(SELECT 1 FROM autonomous_runtime_state WHERE length(CAST(body_json AS BLOB))>?4)
            OR EXISTS(SELECT 1 FROM autonomy_snapshot WHERE length(state_bytes)>?5)
            OR EXISTS(SELECT 1 FROM inner_event WHERE length(CAST(kind AS BLOB))>?1 OR length(CAST(body_json AS BLOB))>?3)
            OR EXISTS(SELECT 1 FROM autonomy_claim WHERE
                typeof(body_json)!='text' OR length(CAST(body_json AS BLOB))>?6)",
        params![
            MAX_JOURNAL_EVENT_KIND_BYTES as i64,
            MAX_CANONICAL_EVENT_BYTES as i64,
            MAX_AUTONOMY_DELTA_BYTES as i64,
            MAX_RUNTIME_STATE_BYTES as i64,
            MAX_SNAPSHOT_STATE_BYTES as i64,
            MAX_AUTONOMY_CLAIM_BODY_BYTES as i64,
        ],
        |row| row.get(0),
    )?;
    if oversized != 0 {
        return Err(StoreError::AutonomyConflict(
            "autonomy storage exceeds a materialization verification bound".into(),
        ));
    }
    let negative_generation: i64 = conn.query_row(
        "SELECT
            EXISTS(SELECT 1 FROM autonomous_runtime_state WHERE generation<0 OR state_revision<0)
            OR EXISTS(SELECT 1 FROM wake_schedule WHERE generation<0)
            OR EXISTS(SELECT 1 FROM outbound_target WHERE generation<0)",
        [],
        |row| row.get(0),
    )?;
    if negative_generation != 0 {
        return Err(StoreError::AutonomyConflict(
            "autonomy generation is outside the unsigned contract range".into(),
        ));
    }
    Ok(())
}

fn backfill_inner_event_journal_revisions_v6(tx: &Transaction<'_>) -> Result<(), StoreError> {
    let mut statement = tx.prepare(
        "SELECT scope_digest,logical_revision,CASE WHEN length(delta_bytes)<=?1 THEN delta_bytes END FROM journal WHERE length(delta_bytes)>0 ORDER BY scope_digest,logical_revision",
    )?;
    let mut rows = statement.query(params![MAX_AUTONOMY_DELTA_BYTES as i64])?;
    while let Some(row) = rows.next()? {
        let raw_scope: Vec<u8> = row.get(0)?;
        let raw_revision: i64 = row.get(1)?;
        let bounded_delta: Option<Vec<u8>> = row.get(2)?;
        let delta_bytes = bounded_delta.ok_or_else(|| {
            StoreError::AutonomyConflict(
                "migration autonomy delta exceeds its verification bound".into(),
            )
        })?;
        let delta = parse_bounded_autonomy_delta(&delta_bytes, "migration autonomy delta")?;
        let scope: Digest = raw_scope.try_into().map_err(|_| {
            StoreError::AutonomyConflict("invalid canonical inner event scope".into())
        })?;
        let revision: u64 = raw_revision.try_into().map_err(|_| {
            StoreError::AutonomyConflict("invalid canonical inner event revision".into())
        })?;
        for event in delta.inner_events {
            if event.persona_scope != scope {
                return Err(StoreError::AutonomyConflict(
                    "cross-scope inner event in canonical journal".into(),
                ));
            }
            let stored_revision = validate_projected_inner_event_v6(tx, &event)?;
            if stored_revision != 0 {
                return Err(StoreError::AutonomyConflict(
                    "legacy inner event already has an unexpected journal revision".into(),
                ));
            }
            let revision_sql = autonomy_sql_i64(revision, "canonical inner event revision")?;
            let changed = tx.execute(
                "UPDATE inner_event SET journal_revision=?2 WHERE event_id=?1 AND journal_revision=0",
                params![event.event_id.to_vec(), revision_sql],
            )?;
            if changed != 1 {
                return Err(StoreError::AutonomyConflict(
                    "duplicate canonical event or inner event revision backfill failure".into(),
                ));
            }
        }
    }
    drop(rows);
    drop(statement);
    let unbound: i64 = tx.query_row(
        "SELECT COUNT(*) FROM inner_event WHERE journal_revision=0",
        [],
        |row| row.get(0),
    )?;
    if unbound != 0 {
        return Err(StoreError::AutonomyConflict(
            "live inner event cannot be reconstructed from canonical journal".into(),
        ));
    }
    Ok(())
}

fn validate_projected_inner_event_v6(
    conn: &Connection,
    event: &InnerEventV1,
) -> Result<i64, StoreError> {
    let row: (Vec<u8>, i64, i64, String, i64, Option<String>) = conn
        .query_row(
            "SELECT persona_scope,journal_revision,committed_at_utc_ms,kind,tombstoned,CASE WHEN length(CAST(body_json AS BLOB))<=?2 THEN body_json END FROM inner_event WHERE event_id=?1",
            params![event.event_id.to_vec(), MAX_AUTONOMY_DELTA_BYTES as i64],
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
        .optional()?
        .ok_or_else(|| {
            StoreError::AutonomyConflict(
                "canonical inner event lacks a projection row".into(),
            )
        })?;
    let scope: Digest = row
        .0
        .try_into()
        .map_err(|_| StoreError::AutonomyConflict("invalid projected inner event scope".into()))?;
    let committed_at = autonomy_u64(row.2, "projected inner event time")?;
    let tombstoned = match row.4 {
        0 => false,
        1 => true,
        _ => {
            return Err(StoreError::AutonomyConflict(
                "invalid projected inner event tombstone".into(),
            ))
        }
    };
    let body = row.5.ok_or_else(|| {
        StoreError::AutonomyConflict(
            "projected inner event body exceeds its verification bound".into(),
        )
    })?;
    let stored: InnerEventV1 = parse(body)?;
    if scope != event.persona_scope
        || stored != *event
        || committed_at != event.committed_at_utc_ms
        || row.3 != format!("{:?}", event.kind)
        || row.3.len() > MAX_JOURNAL_EVENT_KIND_BYTES
        || tombstoned != event.tombstoned
    {
        return Err(StoreError::AutonomyConflict(
            "inner event projection differs from canonical event".into(),
        ));
    }
    Ok(row.1)
}

fn rebuild_inner_event_manifests_v6(tx: &Transaction<'_>) -> Result<(), StoreError> {
    tx.execute("DELETE FROM inner_event_manifest", [])?;
    let mut statement = tx.prepare(
        "SELECT scope_digest,logical_revision,CASE WHEN length(delta_bytes)<=?1 THEN delta_bytes END FROM journal WHERE event_kind='time_advance' ORDER BY scope_digest,logical_revision",
    )?;
    let mut rows = statement.query(params![MAX_AUTONOMY_DELTA_BYTES as i64])?;
    while let Some(row) = rows.next()? {
        let scope: Digest = row
            .get::<_, Vec<u8>>(0)?
            .try_into()
            .map_err(|_| StoreError::AutonomyConflict("invalid manifest scope".into()))?;
        let revision = autonomy_u64(row.get(1)?, "manifest revision")?;
        let bounded_delta: Option<Vec<u8>> = row.get(2)?;
        let delta_bytes = bounded_delta.ok_or_else(|| {
            StoreError::AutonomyConflict(
                "manifest canonical delta exceeds its verification bound".into(),
            )
        })?;
        let delta = parse_bounded_autonomy_delta(&delta_bytes, "manifest canonical delta")?;
        if delta
            .inner_events
            .iter()
            .any(|event| event.persona_scope != scope)
        {
            return Err(StoreError::AutonomyConflict(
                "cross-scope inner event in canonical TimeAdvance".into(),
            ));
        }
        let event_count = u64::try_from(delta.inner_events.len()).map_err(|_| {
            StoreError::AutonomyConflict("manifest event count is out of range".into())
        })?;
        let event_digest = inner_event_manifest_digest(&delta.inner_events)?;
        let revision_sql = autonomy_sql_i64(revision, "manifest revision")?;
        let event_count_sql = autonomy_sql_i64(event_count, "manifest event count")?;
        let changed = tx.execute(
            "INSERT INTO inner_event_manifest(persona_scope,journal_revision,event_count,event_digest) VALUES(?1,?2,?3,?4)",
            params![blob(scope), revision_sql, event_count_sql, blob(event_digest)],
        )?;
        if changed != 1 {
            return Err(StoreError::AutonomyConflict(
                "inner event manifest rebuild insert failed".into(),
            ));
        }
    }
    Ok(())
}

// Preserve the established storage/API shape in this compatibility boundary.
#[allow(clippy::too_many_arguments)]
fn verify_interaction_fact_batch_manifest_v7(
    conn: &Connection,
    scope: &Digest,
    revision: u64,
    base_revision: u64,
    event_bytes: &[u8],
    stored_event_digest: &[u8],
    receipt_bytes: &[u8],
    delta: &AutonomyJournalDeltaV1,
) -> Result<(), StoreError> {
    let decoded = wire::decode_event(event_bytes).map_err(|error| {
        StoreError::AutonomyConflict(format!(
            "interaction fact batch canonical event is invalid: {error}"
        ))
    })?;
    let CanonicalEvent::InteractionFactBatch(batch) = decoded else {
        return Err(StoreError::AutonomyConflict(
            "interaction fact batch kind differs from its canonical event".into(),
        ));
    };
    ae_contracts::validate_interaction_fact_batch(&batch)
        .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
    let relation_token = batch.scope.relation_token.ok_or_else(|| {
        StoreError::AutonomyConflict("interaction fact batch relation scope is absent".into())
    })?;
    let expected_scope =
        wire::persona_scope_digest(&batch.scope.bot_token, &batch.scope.persona_token, None);
    let relation_scope = wire::persona_scope_digest(
        &batch.scope.bot_token,
        &batch.scope.persona_token,
        Some(&relation_token),
    );
    let expected_event_digest =
        wire::event_digest(&CanonicalEvent::InteractionFactBatch(batch.clone()));
    let expected_inner = crate::alpha3::interaction_inner_event_v1(&batch);
    let receipt = wire::decode_transition_receipt(receipt_bytes).map_err(|error| {
        StoreError::AutonomyConflict(format!(
            "interaction fact batch receipt is invalid: {error}"
        ))
    })?;
    if expected_scope != *scope
        || stored_event_digest != expected_event_digest
        || batch.causal.base_revision != base_revision
        || base_revision.checked_add(1) != Some(revision)
        || receipt.scope_digest != *scope
        || receipt.event_digest != expected_event_digest
        || receipt.base_revision != base_revision
        || receipt.next_revision != revision
        || delta.state.is_some()
        || delta.intention.is_some()
        || delta.inner_events != [expected_inner.clone()]
        || expected_inner.persona_scope != *scope
        || expected_inner
            .source_event_ids
            .iter()
            .any(|source| !batch.facts.iter().any(|fact| fact.fact_id == *source))
    {
        return Err(StoreError::AutonomyConflict(
            "interaction fact batch delta is inconsistent".into(),
        ));
    }

    let fact_rows = {
        let mut statement = conn.prepare(
            "SELECT fact_id,event_id,persona_scope,relation_scope,observed_at_utc_ms,
                    source_digest,revision,
                    CASE WHEN length(CAST(body_json AS BLOB))<=?2 THEN body_json END
             FROM interaction_fact WHERE event_id=?1 ORDER BY fact_id",
        )?;
        let rows = statement
            .query_map(
                params![batch.event_id.to_vec(), MAX_CANONICAL_EVENT_BYTES as i64],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, Vec<u8>>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, Option<String>>(7)?,
                    ))
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let mut expected_facts = batch.facts.clone();
    expected_facts.sort_by_key(|fact| fact.fact_id);
    if fact_rows.len() != expected_facts.len() {
        return Err(StoreError::AutonomyConflict(
            "interaction fact batch materialized fact count is inconsistent".into(),
        ));
    }
    let materialized_revision = fact_rows
        .first()
        .map(|row| autonomy_u64(row.6, "interaction fact revision"))
        .transpose()?
        .ok_or_else(|| {
            StoreError::AutonomyConflict(
                "interaction fact batch has no materialized fact authority".into(),
            )
        })?;
    for (row, expected) in fact_rows.into_iter().zip(expected_facts) {
        let (
            fact_id,
            event_id,
            fact_persona,
            fact_relation,
            observed_at,
            source_digest,
            fact_revision,
            body,
        ) = row;
        let materialized: InteractionFactV1 = parse(body.ok_or_else(|| {
            StoreError::AutonomyConflict(
                "interaction fact batch materialized fact exceeds its bound".into(),
            )
        })?)?;
        if fact_id != expected.fact_id
            || event_id != batch.event_id
            || fact_persona != *scope
            || fact_relation != relation_scope
            || autonomy_u64(observed_at, "interaction fact observed time")?
                != expected.observed_at_utc_ms
            || source_digest != expected.source_digest
            || autonomy_u64(fact_revision, "interaction fact revision")? != materialized_revision
            || materialized != expected
        {
            return Err(StoreError::AutonomyConflict(
                "interaction fact batch materialized fact is inconsistent".into(),
            ));
        }
    }
    let (revision_span_count, invalid_span_count): (i64, i64) = conn.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(CASE
                    WHEN logical_revision=?2 AND event_kind='interaction_fact_batch' THEN 0
                    WHEN logical_revision>?2 AND event_kind=?4 THEN 0
                    ELSE 1
                END),0)
         FROM journal
         WHERE scope_digest=?1 AND logical_revision BETWEEN ?2 AND ?3",
        params![
            blob(*scope),
            autonomy_sql_i64(revision, "interaction journal revision")?,
            autonomy_sql_i64(materialized_revision, "interaction materialized revision")?,
            OPERATIONAL_CHECKPOINT_KIND,
        ],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let expected_span_count = materialized_revision
        .checked_sub(revision)
        .and_then(|span| span.checked_add(1))
        .ok_or_else(|| {
            StoreError::AutonomyConflict(
                "interaction fact batch materialized revision regressed".into(),
            )
        })?;
    if autonomy_u64(revision_span_count, "interaction revision span count")? != expected_span_count
        || invalid_span_count != 0
    {
        return Err(StoreError::AutonomyConflict(
            "interaction fact batch materialized revision span is inconsistent".into(),
        ));
    }

    let projected_events = {
        let mut statement = conn.prepare(
            "SELECT event_id,persona_scope,journal_revision,committed_at_utc_ms,kind,
                    tombstoned,
                    CASE WHEN length(CAST(body_json AS BLOB))<=?3 THEN body_json END
             FROM inner_event WHERE persona_scope=?1 AND journal_revision=?2
             ORDER BY event_id",
        )?;
        let rows = statement
            .query_map(
                params![
                    blob(*scope),
                    autonomy_sql_i64(revision, "interaction event revision")?,
                    MAX_CANONICAL_EVENT_BYTES as i64,
                ],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, Option<String>>(6)?,
                    ))
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let [projected] = projected_events.as_slice() else {
        return Err(StoreError::AutonomyConflict(
            "interaction fact batch inner event projection is inconsistent".into(),
        ));
    };
    let projected_body: InnerEventV1 = parse(projected.6.clone().ok_or_else(|| {
        StoreError::AutonomyConflict("interaction fact batch inner event exceeds its bound".into())
    })?)?;
    if projected.0 != expected_inner.event_id
        || projected.1 != *scope
        || autonomy_u64(projected.2, "interaction inner event revision")? != revision
        || autonomy_u64(projected.3, "interaction inner event time")?
            != expected_inner.committed_at_utc_ms
        || projected.4 != format!("{:?}", expected_inner.kind)
        || projected.5 != 0
        || projected_body != expected_inner
    {
        return Err(StoreError::AutonomyConflict(
            "interaction fact batch inner event projection is inconsistent".into(),
        ));
    }

    let manifest: Option<(i64, Vec<u8>)> = conn
        .query_row(
            "SELECT event_count,event_digest FROM inner_event_manifest
             WHERE persona_scope=?1 AND journal_revision=?2",
            params![
                blob(*scope),
                autonomy_sql_i64(revision, "interaction manifest revision")?
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let expected_manifest = inner_event_manifest_digest(&[expected_inner])?;
    if manifest != Some((1, expected_manifest.to_vec())) {
        return Err(StoreError::AutonomyConflict(
            "interaction fact batch manifest is inconsistent".into(),
        ));
    }
    let applied_revision: Option<i64> = conn
        .query_row(
            "SELECT revision FROM applied_events WHERE scope_digest=?1 AND event_digest=?2",
            params![blob(*scope), blob(expected_event_digest)],
            |row| row.get(0),
        )
        .optional()?;
    if applied_revision
        .map(|value| autonomy_u64(value, "interaction applied revision"))
        .transpose()?
        != Some(revision)
    {
        return Err(StoreError::AutonomyConflict(
            "interaction fact batch digest projection is inconsistent".into(),
        ));
    }
    Ok(())
}

fn verify_relation_consent_authority_v7(conn: &Connection) -> Result<(), StoreError> {
    let mut histories = BTreeMap::<Digest, Vec<RelationConsentV1>>::new();
    {
        let mut statement = conn.prepare(
            "SELECT relation_scope,consent_epoch,revision,
                    CASE WHEN length(CAST(body_json AS BLOB))<=?1 THEN body_json END
             FROM relation_consent
             ORDER BY relation_scope,revision,consent_epoch",
        )?;
        let rows = statement
            .query_map(params![MAX_CANONICAL_EVENT_BYTES as i64], |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (raw_scope, raw_epoch, raw_revision, bounded_body) in rows {
            let relation_scope: Digest = raw_scope.try_into().map_err(|_| {
                StoreError::AutonomyConflict("invalid relation consent scope".into())
            })?;
            let consent: RelationConsentV1 = parse(bounded_body.ok_or_else(|| {
                StoreError::AutonomyConflict(
                    "relation consent replay body exceeds its verification bound".into(),
                )
            })?)?;
            consent
                .validate()
                .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
            if consent.relation_scope != relation_scope
                || consent.consent_epoch
                    != autonomy_u64(raw_epoch, "relation consent replay epoch")?
                || consent.revision
                    != autonomy_u64(raw_revision, "relation consent replay revision")?
            {
                return Err(StoreError::AutonomyConflict(
                    "relation consent replay columns differ from its body".into(),
                ));
            }
            histories.entry(relation_scope).or_default().push(consent);
        }
    }

    let mut facts_by_relation = BTreeMap::<Digest, Vec<InteractionFactV1>>::new();
    {
        let mut statement = conn.prepare(
            "SELECT CASE WHEN length(event_bytes)<=?1 THEN event_bytes END
             FROM journal WHERE event_kind='interaction_fact_batch'
             ORDER BY scope_digest,logical_revision",
        )?;
        let rows = statement
            .query_map(params![MAX_CANONICAL_EVENT_BYTES as i64], |row| {
                row.get::<_, Option<Vec<u8>>>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for bounded_event in rows {
            let event_bytes = bounded_event.ok_or_else(|| {
                StoreError::AutonomyConflict(
                    "relation consent replay event exceeds its verification bound".into(),
                )
            })?;
            let event = wire::decode_event(&event_bytes).map_err(|error| {
                StoreError::AutonomyConflict(format!(
                    "relation consent replay event is invalid: {error}"
                ))
            })?;
            let CanonicalEvent::InteractionFactBatch(batch) = event else {
                return Err(StoreError::AutonomyConflict(
                    "relation consent replay kind differs from its canonical event".into(),
                ));
            };
            if batch.scope.relation_token.is_none()
                && crate::core_ingress::verify_core_inbound_journal(conn, &event_bytes)?
            {
                continue;
            }
            ae_contracts::validate_interaction_fact_batch(&batch)
                .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
            let relation_token = batch.scope.relation_token.ok_or_else(|| {
                StoreError::AutonomyConflict(
                    "relation consent replay batch lacks relation scope".into(),
                )
            })?;
            let relation_scope = wire::persona_scope_digest(
                &batch.scope.bot_token,
                &batch.scope.persona_token,
                Some(&relation_token),
            );
            let mut ordered = batch.facts;
            ordered.sort_by_key(|fact| (fact.observed_at_utc_ms, fact.fact_id));
            facts_by_relation
                .entry(relation_scope)
                .or_default()
                .extend(ordered);
        }
    }

    for (relation_scope, history) in histories {
        let initial = history.first().ok_or_else(|| {
            StoreError::AutonomyConflict("relation consent history is empty".into())
        })?;
        if initial.consent_epoch != 1
            || initial.revision != 1
            || !matches!(
                initial.state,
                RelationConsentStateV1::Disabled | RelationConsentStateV1::PendingReconfirmation
            )
        {
            return Err(StoreError::AutonomyConflict(
                "relation consent history lacks a non-authorizing initial state".into(),
            ));
        }
        let mut replayed = initial.clone();
        let mut history_index = 1usize;
        for fact in facts_by_relation
            .remove(&relation_scope)
            .unwrap_or_default()
        {
            let Some(next) =
                crate::alpha3::interaction::reduce_consent_transition_v1(&replayed, &fact)?
            else {
                continue;
            };
            if history.get(history_index) != Some(&next) {
                return Err(StoreError::AutonomyConflict(
                    "relation consent history differs from canonical interaction replay".into(),
                ));
            }
            replayed = next;
            history_index += 1;
        }
        if history_index != history.len() {
            return Err(StoreError::AutonomyConflict(
                "relation consent history contains non-canonical transitions".into(),
            ));
        }

        let head: Option<(i64, i64, Option<String>)> = conn
            .query_row(
                "SELECT h.consent_epoch,h.revision,
                        CASE WHEN length(CAST(c.body_json AS BLOB))<=?2 THEN c.body_json END
                 FROM relation_consent_head AS h
                 JOIN relation_consent AS c
                   ON c.relation_scope=h.relation_scope
                  AND c.consent_epoch=h.consent_epoch
                  AND c.revision=h.revision
                 WHERE h.relation_scope=?1",
                params![blob(relation_scope), MAX_CANONICAL_EVENT_BYTES as i64],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let (raw_epoch, raw_revision, bounded_head) = head.ok_or_else(|| {
            StoreError::AutonomyConflict("relation consent head is missing".into())
        })?;
        let head_body: RelationConsentV1 = parse(bounded_head.ok_or_else(|| {
            StoreError::AutonomyConflict(
                "relation consent head exceeds its verification bound".into(),
            )
        })?)?;
        if autonomy_u64(raw_epoch, "relation consent head epoch")? != replayed.consent_epoch
            || autonomy_u64(raw_revision, "relation consent head revision")? != replayed.revision
            || head_body != replayed
        {
            return Err(StoreError::AutonomyConflict(
                "relation consent head differs from canonical interaction replay".into(),
            ));
        }
    }
    if !facts_by_relation.is_empty() {
        return Err(StoreError::AutonomyConflict(
            "canonical interaction relation lacks consent authority".into(),
        ));
    }
    Ok(())
}

fn verify_all_inner_event_manifests_v6(conn: &Connection) -> Result<(), StoreError> {
    let mut statement = conn.prepare(
        "SELECT scope_digest,logical_revision,base_revision,
                CASE WHEN length(CAST(event_kind AS BLOB))<=?1 THEN event_kind END,
                CASE WHEN length(event_bytes)<=?2 THEN event_bytes END,event_digest,
                CASE WHEN length(receipt_bytes)<=?3 THEN receipt_bytes END,
                CASE WHEN length(delta_bytes)<=?4 THEN delta_bytes END
         FROM journal AS journal_row
         WHERE (event_kind IN ('time_advance','interaction_fact_batch')
            OR length(delta_bytes)>0)
           AND NOT EXISTS (
                SELECT 1 FROM legacy_semantic_formula_upgrades AS semantic_upgrade
                WHERE semantic_upgrade.event_digest=journal_row.event_digest
                  AND semantic_upgrade.scope_digest=journal_row.scope_digest
                  AND semantic_upgrade.next_revision=journal_row.logical_revision
                  AND semantic_upgrade.base_revision=journal_row.base_revision
                  AND semantic_upgrade.upgrade_bytes=journal_row.delta_bytes
            )
         ORDER BY scope_digest,logical_revision",
    )?;
    let mut rows = statement.query(params![
        MAX_JOURNAL_EVENT_KIND_BYTES as i64,
        MAX_CANONICAL_EVENT_BYTES as i64,
        MAX_CANONICAL_EVENT_BYTES as i64,
        MAX_AUTONOMY_DELTA_BYTES as i64
    ])?;
    while let Some(row) = rows.next()? {
        let scope: Digest = row
            .get::<_, Vec<u8>>(0)?
            .try_into()
            .map_err(|_| StoreError::AutonomyConflict("invalid canonical scope".into()))?;
        let revision = autonomy_u64(row.get(1)?, "canonical inner event revision")?;
        let base_revision = autonomy_u64(row.get(2)?, "canonical inner event base revision")?;
        let event_kind: Option<String> = row.get(3)?;
        let event_kind = event_kind.ok_or_else(|| {
            StoreError::AutonomyConflict("journal event kind exceeds its verification bound".into())
        })?;
        let event_bytes: Option<Vec<u8>> = row.get(4)?;
        let stored_event_digest: Vec<u8> = row.get(5)?;
        let receipt_bytes: Option<Vec<u8>> = row.get(6)?;
        let bounded_delta: Option<Vec<u8>> = row.get(7)?;
        let delta_bytes = bounded_delta.ok_or_else(|| {
            StoreError::AutonomyConflict("canonical delta exceeds its verification bound".into())
        })?;
        if event_kind == "interaction_fact_batch" && delta_bytes.is_empty() {
            let bytes = event_bytes.as_ref().ok_or_else(|| {
                StoreError::AutonomyConflict(
                    "canonical event exceeds its verification bound".into(),
                )
            })?;
            if crate::core_ingress::verify_core_inbound_journal(conn, bytes)? {
                continue;
            }
        }
        let delta = parse_bounded_autonomy_delta(&delta_bytes, "canonical autonomy delta")?;
        if event_kind != "time_advance" {
            if event_kind == "interaction_fact_batch" {
                verify_interaction_fact_batch_manifest_v7(
                    conn,
                    &scope,
                    revision,
                    base_revision,
                    &event_bytes.ok_or_else(|| {
                        StoreError::AutonomyConflict(
                            "interaction fact batch event exceeds its verification bound".into(),
                        )
                    })?,
                    &stored_event_digest,
                    &receipt_bytes.ok_or_else(|| {
                        StoreError::AutonomyConflict(
                            "interaction fact batch receipt exceeds its verification bound".into(),
                        )
                    })?,
                    &delta,
                )?;
                continue;
            }
            if !delta.inner_events.is_empty() {
                return Err(StoreError::AutonomyConflict(
                    "non-TimeAdvance canonical delta contains inner events".into(),
                ));
            }
            continue;
        }
        if delta
            .inner_events
            .iter()
            .any(|event| event.persona_scope != scope)
        {
            return Err(StoreError::AutonomyConflict(
                "cross-scope inner event in canonical TimeAdvance".into(),
            ));
        }
        let event_count = u64::try_from(delta.inner_events.len()).map_err(|_| {
            StoreError::AutonomyConflict("manifest event count is out of range".into())
        })?;
        let event_digest = inner_event_manifest_digest(&delta.inner_events)?;
        let manifest: Option<(i64, Vec<u8>)> = conn
            .query_row(
                "SELECT event_count,event_digest FROM inner_event_manifest WHERE persona_scope=?1 AND journal_revision=?2",
                params![blob(scope), autonomy_sql_i64(revision, "manifest revision")?],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let (raw_count, raw_digest) = manifest.ok_or_else(|| {
            StoreError::AutonomyConflict(
                "canonical TimeAdvance revision lacks an inner event manifest".into(),
            )
        })?;
        let stored_count = autonomy_u64(raw_count, "inner event manifest count")?;
        let stored_digest: Digest = raw_digest.try_into().map_err(|_| {
            StoreError::AutonomyConflict("invalid inner event manifest digest".into())
        })?;
        if (stored_count, stored_digest) != (event_count, event_digest) {
            return Err(StoreError::AutonomyConflict(
                "inner event manifest differs from canonical TimeAdvance delta".into(),
            ));
        }
        let projected_count = autonomy_u64(
            conn.query_row(
                "SELECT COUNT(*) FROM inner_event WHERE persona_scope=?1 AND journal_revision=?2",
                params![
                    blob(scope),
                    autonomy_sql_i64(revision, "projection revision")?
                ],
                |row| row.get(0),
            )?,
            "inner event projection count",
        )?;
        if projected_count != event_count {
            return Err(StoreError::AutonomyConflict(
                "inner event projection count differs from canonical TimeAdvance".into(),
            ));
        }
        for event in delta.inner_events {
            let stored_revision = validate_projected_inner_event_v6(conn, &event)?;
            if autonomy_u64(stored_revision, "projected inner event revision")? != revision {
                return Err(StoreError::AutonomyConflict(
                    "inner event projection revision differs from canonical event".into(),
                ));
            }
        }
    }
    let extra_manifest: i64 = conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM inner_event_manifest AS m
             LEFT JOIN journal AS j
               ON j.scope_digest=m.persona_scope
              AND j.logical_revision=m.journal_revision
              AND j.event_kind IN ('time_advance','interaction_fact_batch')
             WHERE j.logical_revision IS NULL
         )",
        [],
        |row| row.get(0),
    )?;
    let extra_projection: i64 = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM inner_event AS e LEFT JOIN inner_event_manifest AS m ON m.persona_scope=e.persona_scope AND m.journal_revision=e.journal_revision WHERE m.journal_revision IS NULL)",
        [],
        |row| row.get(0),
    )?;
    if extra_manifest != 0 || extra_projection != 0 {
        return Err(StoreError::AutonomyConflict(
            "inner event manifest or projection lacks canonical authority".into(),
        ));
    }
    Ok(())
}

fn migrate_scope_bindings_v4(tx: &Transaction<'_>) -> Result<(), StoreError> {
    let legacy = {
        let mut statement = tx.prepare(
            "SELECT persona_scope,scope_json FROM autonomy_scope_binding ORDER BY persona_scope",
        )?;
        let values = statement
            .query_map([], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        values
    };
    tx.execute_batch(
        "DROP INDEX IF EXISTS autonomy_scope_binding_persona;
         ALTER TABLE autonomy_scope_binding RENAME TO autonomy_scope_binding_v3;
         CREATE TABLE autonomy_scope_binding(
             work_scope BLOB PRIMARY KEY,
             persona_scope BLOB NOT NULL,
             relation_scope BLOB,
             scope_json TEXT NOT NULL
         );
         CREATE INDEX autonomy_scope_binding_persona ON autonomy_scope_binding(persona_scope);",
    )?;
    for (stored_persona, raw) in legacy {
        let scope: ScopeRef = parse(raw)?;
        let persona_scope =
            wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
        if stored_persona != persona_scope.to_vec() {
            return Err(StoreError::AutonomyConflict(
                "legacy autonomy scope binding digest mismatch".into(),
            ));
        }
        upsert_scope_binding_tx(tx, &scope)?;
    }
    tx.execute("DROP TABLE autonomy_scope_binding_v3", [])?;
    Ok(())
}

fn upsert_scope_binding_tx(tx: &Transaction<'_>, scope: &ScopeRef) -> Result<(), StoreError> {
    let persona_scope = wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
    let mut persona = scope.clone();
    persona.relation_token = None;
    tx.execute(
        "INSERT INTO autonomy_scope_binding(work_scope,persona_scope,relation_scope,scope_json) VALUES(?1,?1,NULL,?2) ON CONFLICT(work_scope) DO UPDATE SET persona_scope=excluded.persona_scope,relation_scope=NULL,scope_json=excluded.scope_json",
        params![blob(persona_scope), json(&persona)?],
    )?;
    if let Some(relation_token) = scope.relation_token {
        let relation_scope = wire::persona_scope_digest(
            &scope.bot_token,
            &scope.persona_token,
            Some(&relation_token),
        );
        tx.execute(
            "INSERT INTO autonomy_scope_binding(work_scope,persona_scope,relation_scope,scope_json) VALUES(?1,?2,?1,?3) ON CONFLICT(work_scope) DO UPDATE SET persona_scope=excluded.persona_scope,relation_scope=excluded.relation_scope,scope_json=excluded.scope_json",
            params![blob(relation_scope), blob(persona_scope), json(scope)?],
        )?;
    }
    Ok(())
}

fn backfill_operational_authority_anchors(tx: &Transaction<'_>) -> Result<(), StoreError> {
    let rows = {
        let mut statement = tx.prepare(
            "SELECT sequence,persona_scope,delta_json,previous_digest,chain_digest FROM autonomy_operational_authority ORDER BY persona_scope,sequence",
        )?;
        let values = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        values
    };
    let mut current_scope = Vec::new();
    let mut ordinal = 0_i64;
    let mut previous = [0_u8; 32];
    for (sequence, persona_scope, body, stored_previous, stored_chain) in rows {
        if persona_scope != current_scope {
            if !current_scope.is_empty() {
                tx.execute(
                    "INSERT OR REPLACE INTO autonomy_operational_authority_head(persona_scope,entry_count,head_digest,ever_seen) VALUES(?1,?2,?3,1)",
                    params![current_scope, ordinal, blob(previous)],
                )?;
            }
            current_scope = persona_scope.clone();
            ordinal = 0;
            previous = [0; 32];
        }
        ordinal += 1;
        let expected = operational_chain_digest(&previous, body.as_bytes());
        if stored_previous != previous.to_vec() || stored_chain != expected.to_vec() {
            return Err(StoreError::AutonomyConflict(
                "legacy operational authority chain mismatch".into(),
            ));
        }
        tx.execute(
            "UPDATE autonomy_operational_authority SET persona_ordinal=?2 WHERE sequence=?1",
            params![sequence, ordinal],
        )?;
        previous = expected;
    }
    if !current_scope.is_empty() {
        tx.execute(
            "INSERT OR REPLACE INTO autonomy_operational_authority_head(persona_scope,entry_count,head_digest,ever_seen) VALUES(?1,?2,?3,1)",
            params![current_scope, ordinal, blob(previous)],
        )?;
    }
    Ok(())
}

fn bootstrap_legacy_operational_authority(tx: &Transaction<'_>) -> Result<(), StoreError> {
    let intention_ids = {
        let mut statement = tx.prepare(
            "SELECT intention_id FROM durable_intention ORDER BY persona_scope,intention_id",
        )?;
        let values = statement
            .query_map([], |row| row.get::<_, Vec<u8>>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        values
    };
    for raw_id in intention_ids {
        let intention_id: Id128 = raw_id
            .as_slice()
            .try_into()
            .map_err(|_| StoreError::AutonomyConflict("invalid legacy intention id".into()))?;
        let represented: i64 = tx.query_row(
            "SELECT COUNT(*) FROM autonomy_operational_authority WHERE intention_id=?1",
            params![raw_id],
            |row| row.get(0),
        )?;
        if represented != 0 {
            continue;
        }
        let (body, stored_revision): (String, i64) = tx.query_row(
            "SELECT body_json,revision FROM durable_intention WHERE intention_id=?1",
            params![intention_id.to_vec()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let mut intention: DurableIntentionV1 = parse(body)?;
        let original_intention = intention.clone();
        let outbound_body: Option<String> = tx
            .query_row(
                "SELECT body_json FROM outbound_attempt WHERE intention_id=?1",
                params![intention_id.to_vec()],
                |row| row.get(0),
            )
            .optional()?;
        let mut outbound = outbound_body.map(parse::<OutboundAttemptV1>).transpose()?;
        let externalization_claim: Option<(Vec<u8>, Option<String>, Option<i64>)> = tx
            .query_row(
                "SELECT c.claim_token,
                        CASE WHEN typeof(c.body_json)='text'
                                   AND length(CAST(c.body_json AS BLOB))<=?2
                             THEN c.body_json END,
                        b.budget_day_start_utc_ms
                 FROM autonomy_claim AS c
                 LEFT JOIN externalization_budget_claim AS b ON b.claim_token=c.claim_token
                 WHERE c.claim_kind='externalization' AND c.record_id=?1",
                params![intention_id.to_vec(), MAX_AUTONOMY_CLAIM_BODY_BYTES as i64],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let dispatch_claim: Option<Vec<u8>> = if let Some(value) = &outbound {
            tx.query_row(
                "SELECT claim_token FROM autonomy_claim WHERE claim_kind='dispatch' AND record_id=?1",
                params![value.outbound_id.to_vec()],
                |row| row.get(0),
            )
            .optional()?
        } else {
            None
        };
        let uncertain_externalization =
            externalization_claim.is_some() || intention.state == IntentionStateV1::Externalizing;
        if uncertain_externalization {
            intention.state = if intention.externalization_attempts < 2 {
                IntentionStateV1::Deferred
            } else {
                IntentionStateV1::Terminal
            };
            intention.externalization_attempts = intention.externalization_attempts.max(1);
        }
        if let Some(value) = outbound.as_mut() {
            if value.state == IntentionStateV1::AdapterCallStarted || dispatch_claim.is_some() {
                value.state = IntentionStateV1::DispatchUnknown;
                value.settled_at_utc_ms = Some(now_ms());
            }
            intention.state = if value.state == IntentionStateV1::Terminal {
                IntentionStateV1::Terminal
            } else {
                IntentionStateV1::DispatchPending
            };
            intention.externalization_attempts = intention.externalization_attempts.max(1);
            tx.execute(
                "UPDATE outbound_attempt SET state=?2,body_json=?3 WHERE outbound_id=?1",
                params![
                    value.outbound_id.to_vec(),
                    state_name(value.state)?,
                    json(value)?
                ],
            )?;
        } else if matches!(
            intention.state,
            IntentionStateV1::DispatchPending
                | IntentionStateV1::AdapterCallStarted
                | IntentionStateV1::AdapterSubmitted
                | IntentionStateV1::PlatformAccepted
                | IntentionStateV1::DeliveryConfirmed
                | IntentionStateV1::DispatchUnknown
        ) {
            intention.state = IntentionStateV1::Terminal;
            intention.externalization_attempts = intention.externalization_attempts.max(1);
        }
        if let Some((claim_token, claim_body, budget_day)) = externalization_claim {
            let claim_body = bounded_autonomy_claim_body(claim_body)?;
            let _: ExternalizationClaimV1 = parse(claim_body)?;
            let budget_day = budget_day.ok_or_else(|| {
                StoreError::AutonomyConflict(
                    "legacy externalization claim omitted budget reservation".into(),
                )
            })?;
            tx.execute(
                "INSERT INTO externalization_budget(relation_scope,budget_day_start_utc_ms,reserved_tokens) VALUES(?1,?2,512) ON CONFLICT(relation_scope,budget_day_start_utc_ms) DO UPDATE SET reserved_tokens=MAX(reserved_tokens,512)",
                params![blob(intention.relation_scope), budget_day],
            )?;
            tx.execute(
                "DELETE FROM externalization_budget_claim WHERE claim_token=?1",
                params![claim_token],
            )?;
        }
        tx.execute(
            "DELETE FROM autonomy_claim WHERE record_id=?1 OR record_id IN (SELECT outbound_id FROM outbound_attempt WHERE intention_id=?1)",
            params![intention_id.to_vec()],
        )?;
        let normalized_revision = if intention == original_intention {
            stored_revision
        } else {
            stored_revision.checked_add(1).ok_or_else(|| {
                StoreError::AutonomyConflict("legacy intention revision overflow".into())
            })?
        };
        tx.execute(
            "UPDATE durable_intention SET state=?2,revision=?3,body_json=?4 WHERE intention_id=?1",
            params![
                intention_id.to_vec(),
                state_name(intention.state)?,
                normalized_revision,
                json(&intention)?
            ],
        )?;
        append_operational_authority(tx, "legacy_genesis", &intention_id)?;
    }
    Ok(())
}

fn backfill_operational_checkpoints(tx: &Transaction<'_>) -> Result<(), StoreError> {
    let scopes = {
        let mut statement = tx.prepare(
            "SELECT DISTINCT persona_scope FROM autonomy_operational_authority ORDER BY persona_scope",
        )?;
        let values = statement
            .query_map([], |row| row.get::<_, Vec<u8>>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        values
    };
    for raw in scopes {
        let persona_scope: Digest = raw.try_into().map_err(|_| {
            StoreError::AutonomyConflict("invalid operational checkpoint scope".into())
        })?;
        let history = read_operational_authority(tx, &persona_scope)?;
        let existing = read_operational_checkpoints(tx, &persona_scope)?;
        if existing.len() > history.len()
            || existing
                .iter()
                .zip(history.iter())
                .any(|((_, checkpoint), (_, delta))| checkpoint.delta != *delta)
        {
            return Err(StoreError::AutonomyConflict(
                "operational checkpoint migration mismatch".into(),
            ));
        }
        let mut previous = existing
            .last()
            .map(|(_, value)| value.operational_head_digest)
            .unwrap_or([0; 32]);
        for (ordinal, delta) in history.into_iter().skip(existing.len()) {
            let body = json(&delta)?;
            previous = operational_chain_digest(&previous, body.as_bytes());
            append_operational_checkpoint(tx, ordinal, previous, &delta)?;
        }
    }
    Ok(())
}

fn verify_all_operational_anchors(tx: &Transaction<'_>) -> Result<(), StoreError> {
    let scopes = {
        let mut statement = tx.prepare(
            "SELECT persona_scope FROM autonomy_operational_authority UNION SELECT persona_scope FROM autonomy_operational_authority_head UNION SELECT scope_digest FROM journal WHERE event_kind='operational_checkpoint_v1' ORDER BY persona_scope",
        )?;
        let values = statement
            .query_map([], |row| row.get::<_, Vec<u8>>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        values
    };
    for raw in scopes {
        let scope: Digest = raw
            .try_into()
            .map_err(|_| StoreError::AutonomyConflict("invalid operational anchor scope".into()))?;
        let checkpoints = read_operational_checkpoints(tx, &scope)?;
        if checkpoints.is_empty() {
            if !read_operational_authority(tx, &scope)?.is_empty() {
                return Err(StoreError::AutonomyConflict(
                    "operational authority lacks canonical checkpoint".into(),
                ));
            }
            continue;
        }
        let expected = checkpoints
            .iter()
            .map(|(ordinal, checkpoint)| (*ordinal, checkpoint.delta.clone()))
            .collect::<Vec<_>>();
        let actual = read_operational_authority(tx, &scope);
        if actual
            .as_ref()
            .is_ok_and(|history| history.len() > expected.len())
        {
            return Err(StoreError::AutonomyConflict(
                "canonical operational checkpoint suffix is missing".into(),
            ));
        }
        if actual.as_ref().ok() != Some(&expected) {
            restore_operational_authority_from_checkpoints(tx, &scope, &checkpoints)?;
        }
        if read_operational_authority(tx, &scope)? != expected {
            return Err(StoreError::AutonomyConflict(
                "operational authority differs from canonical checkpoint".into(),
            ));
        }
        reconcile_relation_policies_from_authority(tx, &expected)?;
    }
    Ok(())
}

/// Hash-chained, append-only authority evidence for the operational lifecycle.
/// These snapshots are written in the same IMMEDIATE transaction as each
/// externalization/outbox/dispatch transition. Mutable tables are projections.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "claim_kind", content = "claim", rename_all = "snake_case")]
enum OperationalClaimV1 {
    Externalization(ExternalizationClaimV1),
    Dispatch(DispatchClaimV1),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationalBudgetV1 {
    budget_day_start_utc_ms: u64,
    #[serde(default)]
    limit_tokens: u64,
    reserved_tokens: u64,
    #[serde(default)]
    charged_tokens: u64,
    #[serde(default)]
    used_tokens: u64,
    #[serde(default = "operational_budget_usage_known_default")]
    usage_known: bool,
    #[serde(default = "operational_budget_revision_default")]
    revision: u64,
}

fn operational_budget_usage_known_default() -> bool {
    true
}

fn operational_budget_revision_default() -> u64 {
    1
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationalAuthorityDeltaV1 {
    schema_version: u16,
    event_kind: String,
    #[serde(with = "hex::d32")]
    persona_scope: Digest,
    #[serde(with = "hex::d32")]
    relation_scope: Digest,
    #[serde(with = "hex::d16")]
    intention_id: Id128,
    intention: DurableIntentionV1,
    intention_revision: u64,
    outbound: Option<OutboundAttemptV1>,
    active_claim: Option<OperationalClaimV1>,
    #[serde(default)]
    active_claim_budget_day_start_utc_ms: Option<u64>,
    #[serde(default)]
    active_claim_unknown_full_charged: bool,
    budgets: Vec<OperationalBudgetV1>,
    policy: Option<RelationTemporalPolicyV1>,
    recorded_at_utc_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationalCheckpointV1 {
    schema_version: u16,
    #[serde(with = "hex::d32")]
    persona_scope: Digest,
    operational_ordinal: u64,
    operational_count: u64,
    #[serde(with = "hex::d32")]
    operational_head_digest: Digest,
    transition_kind: String,
    #[serde(with = "hex::d16")]
    intention_id: Id128,
    #[serde(with = "hex::d16_opt")]
    outbound_id: Option<Id128>,
    #[serde(with = "hex::d32")]
    frozen_delta_digest: Digest,
    delta: OperationalAuthorityDeltaV1,
}

fn operational_chain_digest(previous: &Digest, delta_json: &[u8]) -> Digest {
    wire::domain_hash(b"ae.operational-authority.v1", &[previous, delta_json])
}

fn operational_delta_digest(delta: &OperationalAuthorityDeltaV1) -> Result<Digest, StoreError> {
    Ok(wire::domain_hash(
        b"ae.operational-delta.v1",
        &[json(delta)?.as_bytes()],
    ))
}

fn append_operational_checkpoint(
    tx: &Transaction<'_>,
    ordinal: u64,
    head: Digest,
    delta: &OperationalAuthorityDeltaV1,
) -> Result<(), StoreError> {
    let checkpoint = OperationalCheckpointV1 {
        schema_version: 1,
        persona_scope: delta.persona_scope,
        operational_ordinal: ordinal,
        operational_count: ordinal,
        operational_head_digest: head,
        transition_kind: delta.event_kind.clone(),
        intention_id: delta.intention_id,
        outbound_id: delta.outbound.as_ref().map(|value| value.outbound_id),
        frozen_delta_digest: operational_delta_digest(delta)?,
        delta: delta.clone(),
    };
    let event_bytes = serde_json::to_vec(&checkpoint)
        .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
    let event_digest = wire::domain_hash(b"ae.operational-checkpoint-event.v1", &[&event_bytes]);
    let receipt_bytes = wire::domain_hash(
        b"ae.operational-checkpoint-receipt.v1",
        &[&event_digest, &head, &ordinal.to_le_bytes()],
    )
    .to_vec();
    let raw_current = tx.query_row(
        "SELECT COALESCE(MAX(logical_revision),0) FROM journal WHERE scope_digest=?1",
        params![blob(delta.persona_scope)],
        |row| row.get::<_, i64>(0),
    )?;
    let current: u64 = raw_current
        .try_into()
        .map_err(|_| StoreError::AutonomyConflict("negative canonical journal revision".into()))?;
    let previous: Option<Vec<u8>> = tx
        .query_row(
            "SELECT chain_digest FROM journal WHERE scope_digest=?1 ORDER BY logical_revision DESC LIMIT 1",
            params![blob(delta.persona_scope)],
            |row| row.get(0),
        )
        .optional()?;
    let chain_seed: Digest = previous
        .map(|value| {
            value.try_into().map_err(|_| {
                StoreError::AutonomyConflict("invalid canonical journal chain digest".into())
            })
        })
        .transpose()?
        .unwrap_or([0; 32]);
    let chain = ae_continuum::chain_link(&chain_seed, &event_bytes, &receipt_bytes);
    tx.execute(
        "INSERT INTO journal(logical_revision,scope_digest,base_revision,event_kind,event_bytes,event_digest,receipt_bytes,delta_bytes,chain_digest,committed_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,X'',?8,?9)",
        params![current as i64 + 1, blob(delta.persona_scope), current as i64, OPERATIONAL_CHECKPOINT_KIND, event_bytes, blob(event_digest), receipt_bytes, blob(chain), delta.recorded_at_utc_ms as i64],
    )?;
    tx.execute(
        "INSERT INTO applied_events(scope_digest,event_digest,revision) VALUES(?1,?2,?3)",
        params![
            blob(delta.persona_scope),
            blob(event_digest),
            current as i64 + 1
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod autonomy_projection_tests {
    use super::*;
    use ae_continuum::CommitEnvelope;
    use ae_fixed::Fixed;

    fn intention(id: u8) -> DurableIntentionV1 {
        let persona_scope = wire::persona_scope_digest(&[5; 16], &[6; 16], None);
        let relation_scope = wire::persona_scope_digest(&[5; 16], &[6; 16], Some(&[7; 16]));
        DurableIntentionV1 {
            schema_version: 1,
            intention_id: [id; 16],
            persona_scope,
            relation_scope,
            state: IntentionStateV1::Ready,
            action_class: "relationship_connection".into(),
            salience: Fixed::ONE,
            urgency: Fixed::ONE,
            confidence: Fixed::ONE,
            created_at_utc_ms: 1,
            not_before_utc_ms: 1,
            expires_at_utc_ms: 2,
            externalization_attempts: 0,
            semantic_idempotency_digest: [id.wrapping_add(1); 32],
            workspace_mapping_digest: [3; 32],
            workspace_residual: Fixed::ZERO,
            source_event_ids: vec![[4; 16]],
        }
    }

    fn action(value: &DurableIntentionV1) -> CanonicalEvent {
        CanonicalEvent::SelfActionCandidate(SelfActionCandidate {
            event_id: value.intention_id,
            scope: ScopeRef {
                bot_token: [5; 16],
                persona_token: [6; 16],
                relation_token: Some([7; 16]),
                session_token: [8; 16],
            },
            causal: CausalRef {
                turn_id: [9; 16],
                action_id: Some(value.intention_id),
                delivery_id: None,
                claim_id: None,
                base_revision: 1,
            },
            visible_action_digest: value.semantic_idempotency_digest,
            claims: vec![],
        })
    }

    fn policy(value: &DurableIntentionV1) -> RelationTemporalPolicyV1 {
        RelationTemporalPolicyV1 {
            schema_version: 1,
            relation_scope: value.relation_scope,
            user_timezone: "Asia/Shanghai".into(),
            timezone_source: TimezoneSourceV1::Explicit,
            quiet_hours_start_minute: 0,
            quiet_hours_end_minute: 0,
            quiet_hours_emergency_bypass: true,
            proactive_enabled: true,
            proactive_daily_max: 2,
            min_proactive_cooldown_ms: 100,
            intention_ttl_ms: 1_000,
            unanswered_backoff_base_ms: 100,
            unanswered_hard_stop: 3,
            emergency_threshold: Fixed::ONE,
            daily_submitted: 1,
            consecutive_unanswered: 2,
            last_inbound_utc_ms: Some(3),
            last_proactive_submitted_utc_ms: Some(4),
            revision: 7,
            auto_policy_version: 0,
            next_claim_reservation_tokens: 256,
        }
    }

    #[test]
    fn policy_config_persists_auto_mode_and_reservation_changes() {
        let value = intention(211);
        let mut store = Store::open_in_memory().unwrap();
        let mut legacy = policy(&value);
        legacy.proactive_daily_max = 2;
        legacy.min_proactive_cooldown_ms = 6 * 60 * 60 * 1_000;
        legacy.auto_policy_version = 0;
        legacy.next_claim_reservation_tokens = 0;
        let first = store.upsert_relation_policy(&legacy).unwrap();
        assert_eq!(first.revision, 1);

        let mut auto = legacy.clone();
        auto.auto_policy_version = 1;
        auto.next_claim_reservation_tokens = 256;
        let second = store.upsert_relation_policy(&auto).unwrap();
        assert_eq!(second.revision, 2);
        assert_eq!(
            (
                second.auto_policy_version,
                second.next_claim_reservation_tokens
            ),
            (1, 256)
        );

        let mut restrained = auto.clone();
        restrained.auto_policy_version = 0;
        let third = store.upsert_relation_policy(&restrained).unwrap();
        assert_eq!(third.revision, 3);
        assert_eq!(third.auto_policy_version, 0);

        let mut new_reservation = restrained;
        new_reservation.next_claim_reservation_tokens = 512;
        let fourth = store.upsert_relation_policy(&new_reservation).unwrap();
        assert_eq!(fourth.revision, 4);
        assert_eq!(fourth.next_claim_reservation_tokens, 512);
    }

    fn pending_outbound(
        intention: &DurableIntentionV1,
        relation_token: Id128,
        outbound_id: Id128,
    ) -> OutboundAttemptV1 {
        OutboundAttemptV1 {
            outbound_id,
            intention_id: intention.intention_id,
            state: IntentionStateV1::DispatchPending,
            target: OutboundTargetEnvelopeV1 {
                schema_version: 1,
                target_kind: TargetKindV1::Private,
                umo_ciphertext: vec![1],
                umo_nonce: vec![2],
                key_id: "pending-test".into(),
                umo_digest: [3; 32],
                platform_token: [4; 16],
                bot_token: [5; 16],
                persona_token: [6; 16],
                relation_token,
                session_token: [8; 16],
                bound_at_utc_ms: 1,
                binding_generation: 1,
                binding_digest: [9; 32],
            },
            candidate_ciphertext: vec![10],
            candidate_digest: [11; 32],
            created_at_utc_ms: 1,
            settled_at_utc_ms: None,
            counted_budget_day_start_utc_ms: 0,
        }
    }

    #[test]
    fn self_action_set_rejects_missing_duplicate_and_mismatched_candidates() {
        let expected = intention(10);
        let wake_event_id = [9; 16];
        let wake_scope = match action(&expected) {
            CanonicalEvent::SelfActionCandidate(value) => value.scope,
            _ => unreachable!(),
        };
        assert!(validate_self_action_set(
            &[expected.clone()],
            &[],
            &[wake_scope.clone()],
            &wake_event_id,
            0,
        )
        .is_err());
        let valid = (action(&expected), expected.clone());
        assert!(validate_self_action_set(
            &[expected.clone()],
            &[valid.clone()],
            &[wake_scope.clone()],
            &wake_event_id,
            0
        )
        .is_ok());
        assert!(validate_self_action_set(
            &[expected.clone()],
            &[valid.clone(), valid],
            &[wake_scope.clone()],
            &wake_event_id,
            0
        )
        .is_err());
        let mut wrong = expected.clone();
        wrong.semantic_idempotency_digest = [99; 32];
        assert!(validate_self_action_set(
            &[expected.clone()],
            &[(action(&wrong), wrong)],
            &[wake_scope.clone()],
            &wake_event_id,
            0
        )
        .is_err());
        for forged in [
            {
                let mut value = action(&expected);
                let CanonicalEvent::SelfActionCandidate(inner) = &mut value else {
                    unreachable!()
                };
                inner.causal.turn_id = [44; 16];
                value
            },
            {
                let mut value = action(&expected);
                let CanonicalEvent::SelfActionCandidate(inner) = &mut value else {
                    unreachable!()
                };
                inner.causal.delivery_id = Some([45; 16]);
                value
            },
            {
                let mut value = action(&expected);
                let CanonicalEvent::SelfActionCandidate(inner) = &mut value else {
                    unreachable!()
                };
                inner.causal.claim_id = Some([46; 16]);
                value
            },
            {
                let mut value = action(&expected);
                let CanonicalEvent::SelfActionCandidate(inner) = &mut value else {
                    unreachable!()
                };
                inner.causal.base_revision = 99;
                value
            },
            {
                let mut value = action(&expected);
                let CanonicalEvent::SelfActionCandidate(inner) = &mut value else {
                    unreachable!()
                };
                inner.claims.push(ClaimCommitment {
                    claim_id: [47; 16],
                    confidence: Fixed::ONE,
                    assertiveness: Fixed::ONE,
                    stakes: Fixed::ONE,
                    audience_publicness: Fixed::ONE,
                    expires_at_ms: 2,
                });
                value
            },
        ] {
            assert!(validate_self_action_set(
                &[expected.clone()],
                &[(forged, expected.clone())],
                &[wake_scope.clone()],
                &wake_event_id,
                0
            )
            .is_err());
        }
    }

    fn anchored_store() -> (Store, Digest) {
        let mut store = Store::open_in_memory().unwrap();
        let value = intention(30);
        store
            .conn
            .as_mut()
            .unwrap()
            .execute(
                "INSERT INTO durable_intention(intention_id,persona_scope,relation_scope,semantic_digest,state,revision,body_json) VALUES(?1,?2,?3,?4,'ready',0,?5)",
                params![
                    value.intention_id.to_vec(),
                    blob(value.persona_scope),
                    blob(value.relation_scope),
                    blob(value.semantic_idempotency_digest),
                    json(&value).unwrap()
                ],
            )
            .unwrap();
        let relation_policy = policy(&value);
        store
            .conn
            .as_mut()
            .unwrap()
            .execute(
                "INSERT INTO relation_temporal_policy(relation_scope,revision,body_json) VALUES(?1,?2,?3)",
                params![blob(value.relation_scope), relation_policy.revision as i64, json(&relation_policy).unwrap()],
            )
            .unwrap();
        let tx = store
            .conn
            .as_mut()
            .unwrap()
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        append_operational_authority(&tx, "test_genesis", &value.intention_id).unwrap();
        append_operational_authority(&tx, "test_second", &value.intention_id).unwrap();
        tx.commit().unwrap();
        (store, value.persona_scope)
    }

    #[test]
    fn operational_anchor_detects_suffix_empty_and_head_tampering() {
        let (mut suffix, persona_scope) = anchored_store();
        suffix
            .conn
            .as_mut()
            .unwrap()
            .execute(
                "DELETE FROM autonomy_operational_authority WHERE persona_scope=?1 AND persona_ordinal=2",
                params![blob(persona_scope)],
            )
            .unwrap();
        assert!(read_operational_authority(suffix.connection().unwrap(), &persona_scope).is_err());

        let (mut empty, persona_scope) = anchored_store();
        empty
            .conn
            .as_mut()
            .unwrap()
            .execute(
                "DELETE FROM autonomy_operational_authority WHERE persona_scope=?1",
                params![blob(persona_scope)],
            )
            .unwrap();
        assert!(read_operational_authority(empty.connection().unwrap(), &persona_scope).is_err());

        let (mut head, persona_scope) = anchored_store();
        head.conn
            .as_mut()
            .unwrap()
            .execute(
                "UPDATE autonomy_operational_authority_head SET head_digest=?2 WHERE persona_scope=?1",
                params![blob(persona_scope), blob([99; 32])],
            )
            .unwrap();
        assert!(read_operational_authority(head.connection().unwrap(), &persona_scope).is_err());

        let (mut missing, persona_scope) = anchored_store();
        missing
            .conn
            .as_mut()
            .unwrap()
            .execute(
                "DELETE FROM autonomy_operational_authority_head WHERE persona_scope=?1",
                params![blob(persona_scope)],
            )
            .unwrap();
        assert!(read_operational_authority(missing.connection().unwrap(), &persona_scope).is_err());
    }

    #[test]
    fn operational_transition_is_checkpointed_in_canonical_journal() {
        let (store, persona_scope) = anchored_store();
        let checkpoint_count: i64 = store
            .connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM journal WHERE scope_digest=?1 AND event_kind='operational_checkpoint_v1'",
                params![blob(persona_scope)],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(checkpoint_count, 2);
    }

    #[test]
    fn canonical_checkpoint_repairs_coordinated_operational_prefix_rollback() {
        let (mut store, persona_scope) = anchored_store();
        let first_head: Vec<u8> = store
            .connection()
            .unwrap()
            .query_row(
                "SELECT chain_digest FROM autonomy_operational_authority WHERE persona_scope=?1 AND persona_ordinal=1",
                params![blob(persona_scope)],
                |row| row.get(0),
            )
            .unwrap();
        let tx = store
            .conn
            .as_mut()
            .unwrap()
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        tx.execute(
            "DELETE FROM autonomy_operational_authority WHERE persona_scope=?1 AND persona_ordinal>1",
            params![blob(persona_scope)],
        )
        .unwrap();
        tx.execute(
            "UPDATE autonomy_operational_authority_head SET entry_count=1,head_digest=?2 WHERE persona_scope=?1",
            params![blob(persona_scope), first_head],
        )
        .unwrap();
        tx.commit().unwrap();

        let tx = store
            .conn
            .as_mut()
            .unwrap()
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        verify_all_operational_anchors(&tx).unwrap();
        tx.commit().unwrap();
        assert_eq!(
            read_operational_authority(store.connection().unwrap(), &persona_scope)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn canonical_checkpoint_suffix_deletion_fails_closed() {
        let (mut store, persona_scope) = anchored_store();
        store
            .conn
            .as_mut()
            .unwrap()
            .execute(
                "DELETE FROM journal WHERE scope_digest=?1 AND event_kind='operational_checkpoint_v1' AND logical_revision=(SELECT MAX(logical_revision) FROM journal WHERE scope_digest=?1)",
                params![blob(persona_scope)],
            )
            .unwrap();
        let tx = store
            .conn
            .as_mut()
            .unwrap()
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert!(verify_all_operational_anchors(&tx).is_err());
    }

    #[test]
    fn operational_authority_repairs_policy_counters_without_reverting_new_config() {
        let (mut store, persona_scope) = anchored_store();
        let relation_scope = intention(30).relation_scope;
        let mut tampered: RelationTemporalPolicyV1 = parse(
            store
                .connection()
                .unwrap()
                .query_row(
                    "SELECT body_json FROM relation_temporal_policy WHERE relation_scope=?1",
                    params![blob(relation_scope)],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
        )
        .unwrap();
        tampered.proactive_daily_max = 9;
        tampered.revision = 8;
        tampered.daily_submitted = 99;
        tampered.consecutive_unanswered = 99;
        store
            .conn
            .as_mut()
            .unwrap()
            .execute(
                "UPDATE relation_temporal_policy SET revision=?2,body_json=?3 WHERE relation_scope=?1",
                params![blob(relation_scope), tampered.revision as i64, json(&tampered).unwrap()],
            )
            .unwrap();
        let tx = store
            .conn
            .as_mut()
            .unwrap()
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        verify_all_operational_anchors(&tx).unwrap();
        tx.commit().unwrap();
        let repaired: RelationTemporalPolicyV1 = parse(
            store
                .connection()
                .unwrap()
                .query_row(
                    "SELECT body_json FROM relation_temporal_policy WHERE relation_scope=?1",
                    params![blob(relation_scope)],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(repaired.proactive_daily_max, 9);
        assert_eq!(repaired.revision, 8);
        assert_eq!(repaired.daily_submitted, 1);
        assert_eq!(repaired.consecutive_unanswered, 2);
        assert_eq!(repaired.last_proactive_submitted_utc_ms, Some(4));
        assert_eq!(repaired.last_inbound_utc_ms, Some(3));
        assert_eq!(
            read_operational_authority(store.connection().unwrap(), &persona_scope)
                .unwrap()
                .len(),
            2
        );

        store
            .conn
            .as_mut()
            .unwrap()
            .execute(
                "DELETE FROM relation_temporal_policy WHERE relation_scope=?1",
                params![blob(relation_scope)],
            )
            .unwrap();
        let tx = store
            .conn
            .as_mut()
            .unwrap()
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        verify_all_operational_anchors(&tx).unwrap();
        tx.commit().unwrap();
        let restored: RelationTemporalPolicyV1 = parse(
            store
                .connection()
                .unwrap()
                .query_row(
                    "SELECT body_json FROM relation_temporal_policy WHERE relation_scope=?1",
                    params![blob(relation_scope)],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(restored, policy(&intention(30)));
    }

    #[test]
    fn generic_journal_rejects_autonomy_event_and_delta() {
        let mut store = Store::open_in_memory().unwrap();
        let value = intention(31);
        let event = action(&value);
        let event_bytes = wire::encode_event(&event);
        let event_digest = wire::event_digest(&event);
        let envelope = CommitEnvelope {
            event_kind: wire::event_kind_name(&event).into(),
            event_bytes,
            receipt: TransitionReceipt {
                schema_version: 1,
                formula_digest: [1; 32],
                scope_digest: value.persona_scope,
                event_digest,
                authority_digest: ae_authority::authority_projection_digest(&event),
                base_revision: 0,
                next_revision: 1,
                state_before: [2; 32],
                state_after: [2; 32],
                graph_after: [3; 32],
                action_contract: None,
                active_nodes: 16_384,
                active_edges: 0,
                residuals: Default::default(),
                status: CommitStatus::Committed,
            },
            chain_seed: [4; 32],
            delta_bytes: serde_json::to_vec(&AutonomyJournalDeltaV1 {
                state: None,
                inner_events: vec![],
                intention: Some(value),
            })
            .unwrap(),
        };
        assert!(matches!(
            store.commit_journal(&envelope),
            Err(StoreError::AutonomyConflict(_))
        ));
    }

    #[test]
    fn scope_binding_preserves_two_relations_for_one_persona() {
        let mut store = Store::open_in_memory().unwrap();
        let first = ScopeRef {
            bot_token: [5; 16],
            persona_token: [6; 16],
            relation_token: Some([7; 16]),
            session_token: [8; 16],
        };
        let mut second = first.clone();
        second.relation_token = Some([9; 16]);
        second.session_token = [10; 16];
        let persona_scope = wire::persona_scope_digest(&[5; 16], &[6; 16], None);
        store.upsert_autonomy_scope(&persona_scope, &first).unwrap();
        store
            .upsert_autonomy_scope(&persona_scope, &second)
            .unwrap();
        let scopes = store.list_autonomy_scopes().unwrap();
        assert!(scopes.contains(&first));
        assert!(scopes.contains(&second));
        let first_relation = wire::persona_scope_digest(&[5; 16], &[6; 16], Some(&[7; 16]));
        let second_relation = wire::persona_scope_digest(&[5; 16], &[6; 16], Some(&[9; 16]));
        for (id, relation_scope) in [(41, first_relation), (42, second_relation)] {
            let mut value = intention(id);
            value.relation_scope = relation_scope;
            store
                .conn
                .as_mut()
                .unwrap()
                .execute(
                    "INSERT INTO durable_intention(intention_id,persona_scope,relation_scope,semantic_digest,state,revision,body_json) VALUES(?1,?2,?3,?4,'ready',0,?5)",
                    params![
                        value.intention_id.to_vec(),
                        blob(value.persona_scope),
                        blob(value.relation_scope),
                        blob(value.semantic_idempotency_digest),
                        json(&value).unwrap()
                    ],
                )
                .unwrap();
        }
        let first_pending = store
            .list_pending_autonomy_work_for_relation(&persona_scope, Some(&first_relation))
            .unwrap();
        let second_pending = store
            .list_pending_autonomy_work_for_relation(&persona_scope, Some(&second_relation))
            .unwrap();
        assert_eq!(first_pending.intentions.len(), 1);
        assert_eq!(second_pending.intentions.len(), 1);
        assert_ne!(
            first_pending.intentions[0].intention.intention_id,
            second_pending.intentions[0].intention.intention_id
        );
    }

    #[test]
    fn relation_pending_query_does_not_decode_unrelated_malformed_outbound() {
        let mut store = Store::open_in_memory().unwrap();
        let persona_scope = wire::persona_scope_digest(&[5; 16], &[6; 16], None);
        let first_relation = wire::persona_scope_digest(&[5; 16], &[6; 16], Some(&[7; 16]));
        let second_relation = wire::persona_scope_digest(&[5; 16], &[6; 16], Some(&[9; 16]));
        let mut first = intention(51);
        first.state = IntentionStateV1::DispatchPending;
        first.relation_scope = first_relation;
        let mut second = intention(52);
        second.state = IntentionStateV1::DispatchPending;
        second.relation_scope = second_relation;
        let first_outbound = pending_outbound(&first, [7; 16], [61; 16]);
        let conn = store.conn.as_mut().unwrap();
        for value in [&first, &second] {
            conn.execute(
                "INSERT INTO durable_intention(
                     intention_id,persona_scope,relation_scope,semantic_digest,state,revision,body_json
                 ) VALUES(?1,?2,?3,?4,'dispatch_pending',0,?5)",
                params![
                    value.intention_id.to_vec(),
                    blob(value.persona_scope),
                    blob(value.relation_scope),
                    blob(value.semantic_idempotency_digest),
                    json(value).unwrap(),
                ],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO outbound_attempt(outbound_id,intention_id,state,target_digest,body_json)
             VALUES(?1,?2,'dispatch_pending',?3,?4)",
            params![
                first_outbound.outbound_id.to_vec(),
                first_outbound.intention_id.to_vec(),
                blob(first_outbound.target.binding_digest),
                json(&first_outbound).unwrap(),
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO outbound_attempt(outbound_id,intention_id,state,target_digest,body_json)
             VALUES(?1,?2,'dispatch_pending',?3,'{malformed')",
            params![
                [62_u8; 16].to_vec(),
                second.intention_id.to_vec(),
                vec![12_u8; 32]
            ],
        )
        .unwrap();

        let pending = store
            .list_pending_autonomy_work_for_relation(&persona_scope, Some(&first_relation))
            .unwrap();
        assert_eq!(pending.outbounds, vec![first_outbound]);
    }

    #[test]
    fn relation_pending_query_rejects_selected_outbound_column_body_mismatch() {
        let mut store = Store::open_in_memory().unwrap();
        let persona_scope = wire::persona_scope_digest(&[5; 16], &[6; 16], None);
        let relation_scope = wire::persona_scope_digest(&[5; 16], &[6; 16], Some(&[7; 16]));
        let mut value = intention(53);
        value.state = IntentionStateV1::DispatchPending;
        let mut outbound = pending_outbound(&value, [7; 16], [63; 16]);
        let stored_outbound_id = outbound.outbound_id;
        outbound.outbound_id = [64; 16];
        let conn = store.conn.as_mut().unwrap();
        conn.execute(
            "INSERT INTO durable_intention(
                 intention_id,persona_scope,relation_scope,semantic_digest,state,revision,body_json
             ) VALUES(?1,?2,?3,?4,'dispatch_pending',0,?5)",
            params![
                value.intention_id.to_vec(),
                blob(value.persona_scope),
                blob(value.relation_scope),
                blob(value.semantic_idempotency_digest),
                json(&value).unwrap(),
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO outbound_attempt(outbound_id,intention_id,state,target_digest,body_json)
             VALUES(?1,?2,'dispatch_pending',?3,?4)",
            params![
                stored_outbound_id.to_vec(),
                value.intention_id.to_vec(),
                blob(outbound.target.binding_digest),
                json(&outbound).unwrap(),
            ],
        )
        .unwrap();

        assert!(matches!(
            store.list_pending_autonomy_work_for_relation(&persona_scope, Some(&relation_scope)),
            Err(StoreError::AutonomyConflict(_))
        ));
    }

    #[test]
    fn historical_alpha3_claim_fixture_is_accepted_only_by_internal_recovery_adapter() {
        let mut store = Store::open_in_memory().unwrap();
        let mut value = intention(43);
        value.relation_scope = [41; 32];
        value.state = IntentionStateV1::Externalizing;
        let mut outbound = pending_outbound(&value, [7; 16], [43; 16]);
        outbound.target.binding_digest = [55; 32];
        let conn = store.conn.as_mut().unwrap();
        conn.execute(
            "INSERT INTO durable_intention(
                 intention_id,persona_scope,relation_scope,semantic_digest,state,revision,body_json
             ) VALUES(?1,?2,?3,?4,'externalizing',1,?5)",
            params![
                value.intention_id.to_vec(),
                blob(value.persona_scope),
                blob(value.relation_scope),
                blob(value.semantic_idempotency_digest),
                json(&value).unwrap(),
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO outbound_target(binding_digest,relation_scope,generation,revoked,body_json)
             VALUES(?1,?2,1,0,?3)",
            params![
                blob(outbound.target.binding_digest),
                blob(value.relation_scope),
                json(&outbound.target).unwrap(),
            ],
        )
        .unwrap();

        let legacy_intention_ref = [
            152, 97, 42, 25, 17, 252, 129, 233, 33, 78, 141, 148, 39, 183, 238, 22, 39, 74, 199,
            114, 68, 145, 71, 196, 31, 111, 177, 187, 88, 87, 176, 204,
        ];
        let claim = ExternalizationClaimV2 {
            claim_token: [56; 32],
            intention_public_ref: legacy_intention_ref,
            attempt_no: 1,
            reserved_tokens: 40,
            budget_day_start_utc_ms: 1,
            consent_epoch: 1,
            consent_revision: 1,
            policy_revision: 1,
            capability_snapshot_digest: [57; 32],
            lease_deadline_utc_ms: 100,
            gate_decision_snapshot: None,
        };
        let mut historical_claim = serde_json::to_value(&claim).unwrap();
        historical_claim
            .as_object_mut()
            .unwrap()
            .remove("gate_decision_snapshot");
        let recovered = operational_externalization_claim(
            conn,
            &value.intention_id,
            [58; 32].to_vec(),
            serde_json::to_string(&historical_claim).unwrap(),
        )
        .unwrap();
        assert_eq!(recovered.intention_id, value.intention_id);
        assert_eq!(recovered.claim_token, claim.claim_token);

        let mut forged = claim;
        forged.intention_public_ref[31] ^= 1;
        assert!(operational_externalization_claim(
            conn,
            &value.intention_id,
            [58; 32].to_vec(),
            json(&forged).unwrap(),
        )
        .is_err());
    }

    fn observe_state(persona_scope: Digest) -> AutonomousRuntimeStateV1 {
        AutonomousRuntimeStateV1 {
            schema_version: 1,
            persona_scope,
            relation_scope: None,
            generation: 7,
            state_revision: 11,
            last_advanced_at_utc_ms: 1_700_000_000_000,
            next_wake_at_utc_ms: 1_700_000_060_000,
            wake_intensity: WakeIntensityV1::Micro,
            sleep_state: SleepStateV1::Awake,
            process_s: Fixed::ZERO,
            process_c: Fixed::ZERO,
            arousal: Fixed::ZERO,
            sleep_threshold_held_ms: 0,
            circadian_phase_minutes: Fixed::ZERO,
            affiliation_need: Fixed::ZERO,
            unfinished_topic_salience: Fixed::ZERO,
            social_energy: Fixed::ZERO,
            formula_digest: [12; 32],
            mapping_digest: [13; 32],
            workspace_residual: Fixed::ZERO,
        }
    }

    fn observe_event(persona_scope: Digest, id: u8, committed_at_utc_ms: u64) -> InnerEventV1 {
        InnerEventV1 {
            schema_version: 1,
            event_id: [id; 16],
            persona_scope,
            kind: InnerEventKindV1::HomeostasisChanged,
            committed_at_utc_ms,
            summary_code: format!("observe_{id}"),
            value_before: None,
            value_after: Some(Fixed::from_raw(i64::from(id))),
            source_event_ids: vec![],
            tombstoned: false,
        }
    }

    fn observe_scope(with_relation: bool) -> ScopeRef {
        ScopeRef {
            bot_token: [61; 16],
            persona_token: [62; 16],
            relation_token: with_relation.then_some([63; 16]),
            session_token: [64; 16],
        }
    }

    fn try_commit_observe_wake(
        store: &mut Store,
        scope: &ScopeRef,
        state: &AutonomousRuntimeStateV1,
        inner_events: Vec<InnerEventV1>,
        intentions: Vec<DurableIntentionV1>,
        wake_id: u8,
    ) -> Result<u64, StoreError> {
        let persona_scope =
            wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
        let current_state = store
            .load_autonomous_state(&persona_scope)
            .unwrap()
            .unwrap();
        let current_revision = store.current_revision(&persona_scope).unwrap();
        let frozen = FrozenTimeInputV1 {
            schema_version: 1,
            observed_now_utc_ms: state.last_advanced_at_utc_ms,
            effective_now_utc_ms: state.last_advanced_at_utc_ms,
            persona_tzid: "UTC".into(),
            persona_utc_offset_seconds: 0,
            persona_local_minute: 0,
            persona_day_ordinal: 1,
            relation_tzid: "UTC".into(),
            relation_utc_offset_seconds: 0,
            relation_local_minute: 0,
            relation_day_ordinal: 1,
            budget_day_start_utc_ms: 1_699_920_000_000,
            budget_next_day_start_utc_ms: 1_700_006_400_000,
            next_timezone_transition_utc_ms: None,
            tzdb_fingerprint: [wake_id; 32],
        };
        let event = TimeAdvanceV1 {
            event_id: [wake_id; 16],
            scope: scope.clone(),
            expected_generation: current_state.generation,
            frozen_input_digest: [wake_id.wrapping_add(1); 32],
            frozen,
            stimulus: AutonomousStimulusV1::default(),
        };
        let request = WakeClaimRequestV1 {
            event: event.clone(),
            caller_incarnation: [wake_id.wrapping_add(2); 32],
        };
        let proposal = WakeProposalV1 {
            state: state.clone(),
            inner_events,
            intentions,
        };
        let claim = store.claim_wake_proposal(&request, &proposal).unwrap();
        let event = CanonicalEvent::TimeAdvance(event);
        let event_bytes = wire::encode_event(&event);
        let event_digest = wire::event_digest(&event);
        let before_bytes = serde_json::to_vec(&current_state).unwrap();
        let before_digest = wire::domain_hash(b"ae.autonomy.snapshot.v1", &[&before_bytes]);
        let state_bytes = serde_json::to_vec(state).unwrap();
        let state_digest = wire::domain_hash(b"ae.autonomy.snapshot.v1", &[&state_bytes]);
        let delta_bytes = serde_json::to_vec(&AutonomyJournalDeltaV1 {
            state: Some(state.clone()),
            inner_events: proposal.inner_events.clone(),
            intention: None,
        })
        .unwrap();
        let chain_seed = store
            .last_chain_digest(&persona_scope)
            .unwrap()
            .unwrap_or([0; 32]);
        let envelope = CommitEnvelope {
            event_kind: wire::event_kind_name(&event).into(),
            event_bytes,
            receipt: TransitionReceipt {
                schema_version: 1,
                formula_digest: state.formula_digest,
                scope_digest: persona_scope,
                event_digest,
                authority_digest: ae_authority::authority_projection_digest(&event),
                base_revision: current_revision,
                next_revision: current_revision + 1,
                state_before: before_digest,
                state_after: state_digest,
                graph_after: [wake_id.wrapping_add(3); 32],
                action_contract: None,
                active_nodes: 0,
                active_edges: 0,
                residuals: Default::default(),
                status: CommitStatus::Committed,
            },
            chain_seed,
            delta_bytes,
        };
        let mut next_seed = ae_continuum::chain_link_with_delta(
            &envelope.chain_seed,
            &envelope.event_bytes,
            &wire::encode_transition_receipt(&envelope.receipt),
            &envelope.delta_bytes,
        );
        let mut action_envelopes = Vec::with_capacity(proposal.intentions.len());
        for (index, intention) in proposal.intentions.iter().enumerate() {
            let base_revision = current_revision + 1 + index as u64;
            let action = CanonicalEvent::SelfActionCandidate(SelfActionCandidate {
                event_id: intention.intention_id,
                scope: scope.clone(),
                causal: CausalRef {
                    turn_id: [wake_id; 16],
                    action_id: Some(intention.intention_id),
                    delivery_id: None,
                    claim_id: None,
                    base_revision,
                },
                visible_action_digest: intention.semantic_idempotency_digest,
                claims: vec![],
            });
            let action_bytes = wire::encode_event(&action);
            let action_digest = wire::event_digest(&action);
            let action_delta = serde_json::to_vec(&AutonomyJournalDeltaV1 {
                state: None,
                inner_events: vec![],
                intention: Some(intention.clone()),
            })
            .unwrap();
            let action_envelope = CommitEnvelope {
                event_kind: wire::event_kind_name(&action).into(),
                event_bytes: action_bytes,
                receipt: TransitionReceipt {
                    schema_version: 1,
                    formula_digest: state.formula_digest,
                    scope_digest: persona_scope,
                    event_digest: action_digest,
                    authority_digest: ae_authority::authority_projection_digest(&action),
                    base_revision,
                    next_revision: base_revision + 1,
                    state_before: state_digest,
                    state_after: state_digest,
                    graph_after: [wake_id.wrapping_add(3); 32],
                    action_contract: None,
                    active_nodes: 0,
                    active_edges: 0,
                    residuals: Default::default(),
                    status: CommitStatus::Committed,
                },
                chain_seed: next_seed,
                delta_bytes: action_delta,
            };
            next_seed = ae_continuum::chain_link_with_delta(
                &action_envelope.chain_seed,
                &action_envelope.event_bytes,
                &wire::encode_transition_receipt(&action_envelope.receipt),
                &action_envelope.delta_bytes,
            );
            action_envelopes.push(action_envelope);
        }
        store
            .commit_autonomous_wake(
                &claim.claim_token,
                &envelope,
                &state_digest,
                &state_bytes,
                &action_envelopes,
            )
            .map(|(_, revision)| revision)
    }

    fn commit_observe_wake(
        store: &mut Store,
        scope: &ScopeRef,
        state: &AutonomousRuntimeStateV1,
        inner_events: Vec<InnerEventV1>,
        intentions: Vec<DurableIntentionV1>,
        wake_id: u8,
    ) -> u64 {
        try_commit_observe_wake(store, scope, state, inner_events, intentions, wake_id).unwrap()
    }

    fn committed_observe_store_with_intentions(
        ids: &[u8],
    ) -> (Store, ScopeRef, Digest, Digest, Vec<Id128>) {
        let scope = observe_scope(true);
        let persona_scope =
            wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
        let relation_scope = wire::persona_scope_digest(
            &scope.bot_token,
            &scope.persona_token,
            scope.relation_token.as_ref(),
        );
        let mut store = Store::open_in_memory().unwrap();
        let mut initial = observe_state(persona_scope);
        initial.relation_scope = Some(relation_scope);
        initial.generation = 0;
        initial.state_revision = 0;
        store.initialize_autonomous_state(&initial).unwrap();
        store.upsert_autonomy_scope(&persona_scope, &scope).unwrap();
        let mut committed = initial.clone();
        committed.generation = 1;
        committed.state_revision = 1;
        let intention_ids = ids.iter().map(|id| [*id; 16]).collect::<Vec<_>>();
        let pending = ids
            .iter()
            .map(|id| DurableIntentionV1 {
                schema_version: 1,
                intention_id: [*id; 16],
                persona_scope,
                relation_scope,
                state: IntentionStateV1::Ready,
                action_class: "relationship_connection".into(),
                salience: Fixed::ONE,
                urgency: Fixed::ONE,
                confidence: Fixed::ONE,
                created_at_utc_ms: 1_700_000_000_000,
                not_before_utc_ms: 1_700_000_000_000,
                expires_at_utc_ms: 1_700_086_400_000,
                externalization_attempts: 0,
                semantic_idempotency_digest: [id.wrapping_add(1); 32],
                workspace_mapping_digest: committed.mapping_digest,
                workspace_residual: Fixed::ZERO,
                source_event_ids: vec![[id.wrapping_add(2); 16]],
            })
            .collect();
        commit_observe_wake(
            &mut store,
            &scope,
            &committed,
            vec![observe_event(persona_scope, 68, 1_700_000_000_000)],
            pending,
            69,
        );
        assert!(store.verify_autonomy_projection(&persona_scope).unwrap().ok);
        (store, scope, persona_scope, relation_scope, intention_ids)
    }

    fn committed_observe_store_with_intention() -> (Store, ScopeRef, Digest, Digest, Id128) {
        let (store, scope, persona_scope, relation_scope, intention_ids) =
            committed_observe_store_with_intentions(&[65]);
        (
            store,
            scope,
            persona_scope,
            relation_scope,
            intention_ids[0],
        )
    }

    #[test]
    fn observe_v5_to_v6_migration_backfills_inner_event_journal_revision() {
        let persona_scope = [21; 32];
        let event = observe_event(persona_scope, 22, 1_700_000_000_000);
        let delta = serde_json::to_vec(&AutonomyJournalDeltaV1 {
            state: None,
            inner_events: vec![event.clone()],
            intention: None,
        })
        .unwrap();
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,digest BLOB NOT NULL,completed_at_ms INTEGER NOT NULL);
             INSERT INTO schema_migrations(version,digest,completed_at_ms) VALUES(5,X'05',0);
             CREATE TABLE journal(revision INTEGER PRIMARY KEY AUTOINCREMENT,logical_revision INTEGER NOT NULL,scope_digest BLOB NOT NULL,base_revision INTEGER NOT NULL,event_kind TEXT NOT NULL,event_bytes BLOB NOT NULL,event_digest BLOB NOT NULL,receipt_bytes BLOB NOT NULL,delta_bytes BLOB NOT NULL DEFAULT X'',chain_digest BLOB NOT NULL,committed_at_ms INTEGER NOT NULL);
             CREATE TABLE inner_event(event_id BLOB PRIMARY KEY,persona_scope BLOB NOT NULL,committed_at_utc_ms INTEGER NOT NULL,kind TEXT NOT NULL,tombstoned INTEGER NOT NULL DEFAULT 0,body_json TEXT NOT NULL);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO journal(logical_revision,scope_digest,base_revision,event_kind,event_bytes,event_digest,receipt_bytes,delta_bytes,chain_digest,committed_at_ms) VALUES(8,?1,7,'time_advance',X'',?2,X'',?3,zeroblob(32),?4)",
            params![
                blob(persona_scope),
                vec![23_u8; 32],
                delta,
                event.committed_at_utc_ms as i64,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO inner_event(event_id,persona_scope,committed_at_utc_ms,kind,tombstoned,body_json) VALUES(?1,?2,?3,'HomeostasisChanged',0,?4)",
            params![
                event.event_id.to_vec(),
                blob(persona_scope),
                event.committed_at_utc_ms as i64,
                json(&event).unwrap(),
            ],
        )
        .unwrap();

        let tx = conn.transaction().unwrap();
        assert_eq!(migrate_autonomy(&tx, 5).unwrap(), 7);
        tx.commit().unwrap();

        let journal_revision: i64 = conn
            .query_row(
                "SELECT journal_revision FROM inner_event WHERE event_id=?1",
                params![event.event_id.to_vec()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(journal_revision, 8);
        assert_eq!(
            conn.query_row(
                "SELECT event_count,length(event_digest) FROM inner_event_manifest WHERE persona_scope=?1 AND journal_revision=8",
                params![blob(persona_scope)],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap(),
            (1, 32)
        );
        let columns = conn
            .prepare("PRAGMA index_info(inner_event_scope_revision_order)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(2))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(columns, ["persona_scope", "journal_revision", "event_id"]);
    }

    #[test]
    fn observe_events_pins_high_water_across_normal_wake_commits() {
        let scope = observe_scope(false);
        let persona_scope =
            wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
        let mut store = Store::open_in_memory().unwrap();
        let mut initial = observe_state(persona_scope);
        initial.generation = 0;
        initial.state_revision = 0;
        store.initialize_autonomous_state(&initial).unwrap();
        store.upsert_autonomy_scope(&persona_scope, &scope).unwrap();
        let mut first_state = initial.clone();
        first_state.generation = 1;
        first_state.state_revision = 1;
        commit_observe_wake(
            &mut store,
            &scope,
            &first_state,
            vec![
                observe_event(persona_scope, 33, 1_700_000_000_000),
                observe_event(persona_scope, 32, 1_700_000_000_000),
            ],
            vec![],
            34,
        );
        let first = store
            .observe_events_v1(&ObserveEventsRequestV1 {
                schema_version: 1,
                persona_scope,
                relation_scope: None,
                through_revision: None,
                after: None,
                limit: 1,
                mode: ObserveModeV1::CommittedOnly,
            })
            .unwrap();
        assert_eq!(first.items.len(), 1);
        assert_eq!(first.items[0].summary_code, "observe_32");
        assert_eq!(first.high_water.through_revision, 1);
        assert_eq!(
            store.observe_witness_cache.borrow().canonical_parse_count,
            1
        );
        let cursor = first.next.clone().expect("a second pinned item exists");
        assert_eq!(cursor.persona_scope, persona_scope);

        let mut second_state = first_state;
        second_state.generation = 2;
        second_state.state_revision = 2;
        second_state.last_advanced_at_utc_ms += 60_000;
        second_state.next_wake_at_utc_ms += 60_000;
        commit_observe_wake(
            &mut store,
            &scope,
            &second_state,
            vec![observe_event(persona_scope, 35, 1_700_000_000_000)],
            vec![],
            36,
        );
        let current = store
            .observe_events_v1(&ObserveEventsRequestV1 {
                schema_version: 1,
                persona_scope,
                relation_scope: None,
                through_revision: None,
                after: None,
                limit: 64,
                mode: ObserveModeV1::CommittedOnly,
            })
            .unwrap();
        assert_eq!(current.items.len(), 3);
        assert_eq!(current.high_water.through_revision, 2);
        assert_eq!(
            store.observe_witness_cache.borrow().canonical_parse_count,
            3,
            "same-connection commit must invalidate the old witness epoch"
        );
        let second = store
            .observe_events_v1(&ObserveEventsRequestV1 {
                schema_version: 1,
                persona_scope,
                relation_scope: None,
                through_revision: Some(first.high_water.through_revision),
                after: Some(cursor),
                limit: 64,
                mode: ObserveModeV1::CommittedOnly,
            })
            .unwrap();
        assert_eq!(second.high_water.through_revision, 1);
        assert_eq!(second.items.len(), 1);
        assert_eq!(second.items[0].committed_at_utc_ms, 1_700_000_000_000);
        assert_eq!(second.items[0].summary_code, "observe_33");
        assert!(second.next.is_some());
        assert_eq!(second.high_water.operational_ordinal, None);
        assert!(!second
            .items
            .iter()
            .any(|event| event.summary_code == "observe_35"));

        let tail = store
            .observe_events_v1(&ObserveEventsRequestV1 {
                schema_version: 1,
                persona_scope,
                relation_scope: None,
                through_revision: None,
                after: second.next,
                limit: 64,
                mode: ObserveModeV1::CommittedOnly,
            })
            .unwrap();
        assert_eq!(tail.high_water.through_revision, 2);
        assert_eq!(tail.high_water.operational_ordinal, None);
        assert_eq!(tail.items.len(), 1);
        assert_eq!(tail.items[0].summary_code, "observe_35");
        assert_eq!(tail.next.as_ref().unwrap().journal_revision, 2);

        let mut rollback_state = second_state.clone();
        rollback_state.generation = 3;
        rollback_state.state_revision = 3;
        rollback_state.last_advanced_at_utc_ms += 60_000;
        rollback_state.next_wake_at_utc_ms += 60_000;
        let mut rollback_event = observe_event(persona_scope, 31, 1_699_999_940_000);
        rollback_event.kind = InnerEventKindV1::ClockRollback;
        commit_observe_wake(
            &mut store,
            &scope,
            &rollback_state,
            vec![rollback_event],
            vec![],
            37,
        );
        let mut smaller_id_state = rollback_state;
        smaller_id_state.generation = 4;
        smaller_id_state.state_revision = 4;
        smaller_id_state.last_advanced_at_utc_ms += 60_000;
        smaller_id_state.next_wake_at_utc_ms += 60_000;
        commit_observe_wake(
            &mut store,
            &scope,
            &smaller_id_state,
            vec![observe_event(persona_scope, 30, 1_700_000_000_000)],
            vec![],
            38,
        );

        let resumed = store
            .observe_events_v1(&ObserveEventsRequestV1 {
                schema_version: 1,
                persona_scope,
                relation_scope: None,
                through_revision: None,
                after: tail.next,
                limit: 64,
                mode: ObserveModeV1::CommittedOnly,
            })
            .unwrap();
        assert_eq!(resumed.high_water.through_revision, 4);
        assert_eq!(resumed.items.len(), 2);
        assert_eq!(resumed.items[0].kind, InnerEventKindV1::ClockRollback);
        assert_eq!(resumed.items[0].summary_code, "observe_31");
        assert_eq!(resumed.items[0].committed_at_utc_ms, 1_699_999_940_000);
        assert_eq!(resumed.items[1].summary_code, "observe_30");
        assert_eq!(resumed.items[1].committed_at_utc_ms, 1_700_000_000_000);

        let mut empty_state = smaller_id_state;
        empty_state.generation = 5;
        empty_state.state_revision = 5;
        empty_state.last_advanced_at_utc_ms += 60_000;
        empty_state.next_wake_at_utc_ms += 60_000;
        commit_observe_wake(&mut store, &scope, &empty_state, vec![], vec![], 39);
        assert_eq!(
            store
                .connection()
                .unwrap()
                .query_row(
                    "SELECT event_count FROM inner_event_manifest WHERE persona_scope=?1 AND journal_revision=5",
                    params![blob(persona_scope)],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
        let empty_tail = store
            .observe_events_v1(&ObserveEventsRequestV1 {
                schema_version: 1,
                persona_scope,
                relation_scope: None,
                through_revision: None,
                after: resumed.next.clone(),
                limit: 64,
                mode: ObserveModeV1::CommittedOnly,
            })
            .unwrap();
        assert_eq!(empty_tail.high_water.through_revision, 5);
        assert!(empty_tail.items.is_empty());
        assert_eq!(empty_tail.next.as_ref().unwrap().journal_revision, 5);
        assert_eq!(empty_tail.next.as_ref().unwrap().event_id, None);
    }

    #[test]
    fn observe_review_commit_rejects_inner_event_batches_requiring_future_chunking() {
        let scope = observe_scope(false);
        let persona_scope =
            wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
        let mut store = Store::open_in_memory().unwrap();
        let mut initial = observe_state(persona_scope);
        initial.generation = 0;
        initial.state_revision = 0;
        store.initialize_autonomous_state(&initial).unwrap();
        store.upsert_autonomy_scope(&persona_scope, &scope).unwrap();
        let mut committed = initial;
        committed.generation = 1;
        committed.state_revision = 1;
        let events = (0_u64..=1024)
            .map(|index| {
                let mut event = observe_event(persona_scope, 41, 1_700_000_000_000);
                event.event_id[..8].copy_from_slice(&index.to_be_bytes());
                event.summary_code = format!("bounded_{index}");
                event
            })
            .collect();

        let error = try_commit_observe_wake(&mut store, &scope, &committed, events, vec![], 42)
            .unwrap_err()
            .to_string();
        assert!(error.contains("future batch"), "{error}");
    }

    #[test]
    fn observe_review_snapshot_leaves_unverified_operational_fields_unavailable() {
        let (mut store, _scope, persona_scope, relation_scope, _intention_id) =
            committed_observe_store_with_intention();
        store
            .conn
            .as_mut()
            .unwrap()
            .execute(
                "INSERT INTO externalization_budget(relation_scope,budget_day_start_utc_ms,reserved_tokens) VALUES(?1,?2,512)",
                params![blob(relation_scope), 1_699_920_000_000_i64],
            )
            .unwrap();

        let snapshot = store
            .observe_snapshot_v1(&ObserveSnapshotRequestV1 {
                schema_version: 1,
                persona_scope,
                relation_scope: Some(relation_scope),
                mode: ObserveModeV1::CommittedOnly,
            })
            .unwrap();
        assert_eq!(snapshot.operational_ordinal, None);
        assert_eq!(snapshot.budget.day_start_utc_ms, None);
        assert_eq!(snapshot.budget.reserved_tokens, None);
        assert_eq!(snapshot.budget.used_tokens, None);
    }

    #[test]
    fn observe_review_snapshot_validates_only_the_latest_state_head() {
        let scope = observe_scope(false);
        let persona_scope =
            wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
        let mut store = Store::open_in_memory().unwrap();
        let mut initial = observe_state(persona_scope);
        initial.generation = 0;
        initial.state_revision = 0;
        store.initialize_autonomous_state(&initial).unwrap();
        store.upsert_autonomy_scope(&persona_scope, &scope).unwrap();

        let mut first = initial.clone();
        first.generation = 1;
        first.state_revision = 1;
        commit_observe_wake(
            &mut store,
            &scope,
            &first,
            vec![observe_event(persona_scope, 81, 1_700_000_000_000)],
            vec![],
            82,
        );
        let mut latest = first;
        latest.generation = 2;
        latest.state_revision = 2;
        latest.last_advanced_at_utc_ms += 60_000;
        latest.next_wake_at_utc_ms += 60_000;
        commit_observe_wake(
            &mut store,
            &scope,
            &latest,
            vec![observe_event(persona_scope, 83, 1_700_000_060_000)],
            vec![],
            84,
        );
        store
            .conn
            .as_mut()
            .unwrap()
            .execute(
                "UPDATE journal SET delta_bytes=X'FF' WHERE scope_digest=?1 AND logical_revision=1",
                params![blob(persona_scope)],
            )
            .unwrap();

        let snapshot = store
            .observe_snapshot_v1(&ObserveSnapshotRequestV1 {
                schema_version: 1,
                persona_scope,
                relation_scope: None,
                mode: ObserveModeV1::CommittedOnly,
            })
            .unwrap();
        assert_eq!(snapshot.runtime, latest);
        assert!(snapshot.projection.head_consistent);
    }

    #[test]
    fn observe_review_events_fail_closed_on_page_tamper_or_missing_canonical_delta() {
        for sql in [
            "DELETE FROM inner_event",
            "UPDATE inner_event SET kind='tampered'",
            "UPDATE inner_event SET tombstoned=1",
            "UPDATE inner_event SET committed_at_utc_ms=committed_at_utc_ms+1",
            "UPDATE inner_event SET body_json='{}'",
            "UPDATE inner_event SET journal_revision=-1",
            "DELETE FROM inner_event_manifest",
            "UPDATE inner_event_manifest SET event_count=0",
            "UPDATE inner_event_manifest SET event_count=-1",
            "DELETE FROM journal WHERE logical_revision=1",
        ] {
            let (mut store, _scope, persona_scope, _relation_scope, _intention_id) =
                committed_observe_store_with_intention();
            store.conn.as_mut().unwrap().execute_batch(sql).unwrap();
            let error = store
                .observe_events_v1(&ObserveEventsRequestV1 {
                    schema_version: 1,
                    persona_scope,
                    relation_scope: None,
                    through_revision: None,
                    after: None,
                    limit: 64,
                    mode: ObserveModeV1::CommittedOnly,
                })
                .unwrap_err()
                .to_string();
            assert!(error.contains(OBSERVE_PROJECTION_UNAVAILABLE), "{error}");
        }
    }

    #[test]
    fn observe_review_rejects_oversized_state_before_materializing_it() {
        for sql in [
            &format!(
                "UPDATE autonomous_runtime_state SET body_json=printf('%.*c',{},'x')",
                256 * 1024 + 1
            ),
            &format!(
                "UPDATE autonomy_snapshot SET state_bytes=zeroblob({})",
                256 * 1024 + 1
            ),
        ] {
            let (mut store, _scope, persona_scope, relation_scope, _intention_id) =
                committed_observe_store_with_intention();
            store.conn.as_mut().unwrap().execute_batch(sql).unwrap();
            let error = store
                .observe_snapshot_v1(&ObserveSnapshotRequestV1 {
                    schema_version: 1,
                    persona_scope,
                    relation_scope: Some(relation_scope),
                    mode: ObserveModeV1::CommittedOnly,
                })
                .unwrap_err()
                .to_string();
            assert!(error.contains(OBSERVE_PROJECTION_UNAVAILABLE), "{error}");
            assert!(error.contains("verification bound"), "{error}");
        }
    }

    #[test]
    fn autonomy_rebuild_rejects_oversized_delta_before_materializing_it() {
        let (mut store, _scope, persona_scope, _relation_scope, _intention_id) =
            committed_observe_store_with_intention();
        store
            .conn
            .as_mut()
            .unwrap()
            .execute(
                "UPDATE journal SET delta_bytes=zeroblob(?2) WHERE scope_digest=?1 AND event_kind='time_advance'",
                params![blob(persona_scope), MAX_AUTONOMY_DELTA_BYTES as i64 + 1],
            )
            .unwrap();

        let error = store
            .rebuild_autonomy_projection(&persona_scope)
            .unwrap_err()
            .to_string();
        assert!(error.contains("verification bound"), "{error}");
    }

    #[test]
    fn autonomy_generation_overflow_fails_closed_before_sqlite_conversion() {
        let mut store = Store::open_in_memory().unwrap();
        let mut state = observe_state([97; 32]);
        state.generation = u64::MAX;
        let error = store.initialize_autonomous_state(&state).unwrap_err();
        assert!(matches!(error, StoreError::AutonomyConflict(_)), "{error}");
        assert!(store
            .load_autonomous_state(&state.persona_scope)
            .unwrap()
            .is_none());
    }

    #[test]
    fn autonomy_negative_generation_fails_closed_on_read() {
        let mut store = Store::open_in_memory().unwrap();
        let mut state = observe_state([98; 32]);
        state.generation = 0;
        state.state_revision = 0;
        store.initialize_autonomous_state(&state).unwrap();
        store
            .conn
            .as_mut()
            .unwrap()
            .execute(
                "UPDATE autonomous_runtime_state SET generation=-1 WHERE persona_scope=?1",
                params![blob(state.persona_scope)],
            )
            .unwrap();

        let error = store
            .load_autonomous_state(&state.persona_scope)
            .unwrap_err();
        assert!(matches!(error, StoreError::AutonomyConflict(_)), "{error}");
    }

    #[test]
    fn observe_pagination_reuses_a_bounded_canonical_leaf_witness() {
        let scope = observe_scope(false);
        let persona_scope =
            wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
        let mut store = Store::open_in_memory().unwrap();
        let mut initial = observe_state(persona_scope);
        initial.generation = 0;
        initial.state_revision = 0;
        store.initialize_autonomous_state(&initial).unwrap();
        store.upsert_autonomy_scope(&persona_scope, &scope).unwrap();
        let mut committed = initial;
        committed.generation = 1;
        committed.state_revision = 1;
        let events = (0_u64..1024)
            .map(|ordinal| {
                let mut event = observe_event(persona_scope, 99, 1_700_000_000_000);
                event.event_id[..8].copy_from_slice(&ordinal.to_be_bytes());
                event.summary_code = format!("witness_{ordinal}");
                event
            })
            .collect();
        commit_observe_wake(&mut store, &scope, &committed, events, vec![], 100);

        let mut page = store
            .observe_events_v1(&ObserveEventsRequestV1 {
                schema_version: 1,
                persona_scope,
                relation_scope: None,
                through_revision: None,
                after: None,
                limit: 1,
                mode: ObserveModeV1::CommittedOnly,
            })
            .unwrap();
        assert_eq!(
            store.observe_witness_cache.borrow().canonical_parse_count,
            1
        );
        for _ in 0..8 {
            let lookups_before = store.observe_witness_cache.borrow().witness_lookup_count;
            page = store
                .observe_events_v1(&ObserveEventsRequestV1 {
                    schema_version: 1,
                    persona_scope,
                    relation_scope: None,
                    through_revision: Some(page.high_water.through_revision),
                    after: page.next,
                    limit: 1,
                    mode: ObserveModeV1::CommittedOnly,
                })
                .unwrap();
            assert_eq!(page.items.len(), 1);
            assert_eq!(
                store.observe_witness_cache.borrow().witness_lookup_count - lookups_before,
                1,
                "cursor validation and page slicing must share one revision witness lookup"
            );
        }
        assert_eq!(
            store.observe_witness_cache.borrow().canonical_parse_count,
            1,
            "one 1024-event revision must not be reparsed on every limit=1 page"
        );
    }

    #[test]
    fn observe_witness_is_invalidated_by_external_projection_or_canonical_writes() {
        for (index, sql) in [
            "DELETE FROM inner_event_manifest",
            "UPDATE inner_event_manifest SET event_count=0",
            "DELETE FROM inner_event WHERE event_id=(SELECT event_id FROM inner_event ORDER BY event_id DESC LIMIT 1)",
            "UPDATE inner_event SET kind='tampered' WHERE event_id=(SELECT event_id FROM inner_event ORDER BY event_id DESC LIMIT 1)",
            "UPDATE journal SET delta_bytes=zeroblob(length(delta_bytes)) WHERE event_kind='time_advance'",
        ]
        .into_iter()
        .enumerate()
        {
            let path = std::env::temp_dir().join(format!(
                "ae-observe-witness-{}-{index}.db",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&path);
            let mut store = Store::open(&path).unwrap();
            let scope = observe_scope(false);
            let persona_scope =
                wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
            let mut initial = observe_state(persona_scope);
            initial.generation = 0;
            initial.state_revision = 0;
            store.initialize_autonomous_state(&initial).unwrap();
            store.upsert_autonomy_scope(&persona_scope, &scope).unwrap();
            let mut committed = initial;
            committed.generation = 1;
            committed.state_revision = 1;
            commit_observe_wake(
                &mut store,
                &scope,
                &committed,
                vec![
                    observe_event(persona_scope, 101, 1_700_000_000_000),
                    observe_event(persona_scope, 102, 1_700_000_000_001),
                ],
                vec![],
                103,
            );
            let first = store
                .observe_events_v1(&ObserveEventsRequestV1 {
                    schema_version: 1,
                    persona_scope,
                    relation_scope: None,
                    through_revision: None,
                    after: None,
                    limit: 1,
                    mode: ObserveModeV1::CommittedOnly,
                })
                .unwrap();
            assert_eq!(
                store.observe_witness_cache.borrow().canonical_parse_count,
                1
            );
            let external = Connection::open(&path).unwrap();
            external.execute_batch(sql).unwrap();
            drop(external);

            let error = store
                .observe_events_v1(&ObserveEventsRequestV1 {
                    schema_version: 1,
                    persona_scope,
                    relation_scope: None,
                    through_revision: Some(first.high_water.through_revision),
                    after: first.next,
                    limit: 1,
                    mode: ObserveModeV1::CommittedOnly,
                })
                .unwrap_err()
                .to_string();
            assert!(error.contains(OBSERVE_PROJECTION_UNAVAILABLE), "{sql}: {error}");
            drop(store);
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(path.with_extension("db-wal"));
            let _ = std::fs::remove_file(path.with_extension("db-shm"));
        }
    }

    #[test]
    fn observe_review_snapshot_fails_closed_on_state_head_or_digest_tamper() {
        for sql in [
            "UPDATE autonomous_runtime_state SET generation=-1",
            "UPDATE autonomous_runtime_state SET body_json='{}'",
            "UPDATE autonomy_snapshot SET state_digest=zeroblob(32)",
            "UPDATE autonomy_snapshot SET state_bytes=X'7B7D'",
        ] {
            let (mut store, _scope, persona_scope, relation_scope, _intention_id) =
                committed_observe_store_with_intention();
            store.conn.as_mut().unwrap().execute_batch(sql).unwrap();
            let error = store
                .observe_snapshot_v1(&ObserveSnapshotRequestV1 {
                    schema_version: 1,
                    persona_scope,
                    relation_scope: Some(relation_scope),
                    mode: ObserveModeV1::CommittedOnly,
                })
                .unwrap_err()
                .to_string();
            assert!(error.contains(OBSERVE_PROJECTION_UNAVAILABLE), "{error}");
        }
    }

    #[test]
    fn observe_review_snapshot_rejects_coordinated_state_bytes_and_digest_rewrite() {
        let (mut store, _scope, persona_scope, relation_scope, _intention_id) =
            committed_observe_store_with_intention();
        let mut rewritten: Vec<u8> = store
            .connection()
            .unwrap()
            .query_row(
                "SELECT state_bytes FROM autonomy_snapshot WHERE persona_scope=?1 ORDER BY journal_revision DESC LIMIT 1",
                params![blob(persona_scope)],
                |row| row.get(0),
            )
            .unwrap();
        rewritten.push(b' ');
        let rewritten_digest = wire::domain_hash(b"ae.autonomy.snapshot.v1", &[&rewritten]);
        store
            .conn
            .as_mut()
            .unwrap()
            .execute(
                "UPDATE autonomy_snapshot SET state_bytes=?2,state_digest=?3 WHERE persona_scope=?1",
                params![blob(persona_scope), rewritten, blob(rewritten_digest)],
            )
            .unwrap();

        let error = store
            .observe_snapshot_v1(&ObserveSnapshotRequestV1 {
                schema_version: 1,
                persona_scope,
                relation_scope: Some(relation_scope),
                mode: ObserveModeV1::CommittedOnly,
            })
            .unwrap_err()
            .to_string();
        assert!(error.contains(OBSERVE_PROJECTION_UNAVAILABLE), "{error}");
    }

    #[test]
    fn observe_review_cursor_remains_persona_bound() {
        let (store, _scope, persona_scope, _relation_scope, _intention_id) =
            committed_observe_store_with_intention();
        let first = store
            .observe_events_v1(&ObserveEventsRequestV1 {
                schema_version: 1,
                persona_scope,
                relation_scope: None,
                through_revision: None,
                after: None,
                limit: 64,
                mode: ObserveModeV1::CommittedOnly,
            })
            .unwrap();
        let mut cursor = first.next.unwrap();
        cursor.persona_scope = [94; 32];
        let error = store
            .observe_events_v1(&ObserveEventsRequestV1 {
                schema_version: 1,
                persona_scope,
                relation_scope: None,
                through_revision: Some(first.high_water.through_revision),
                after: Some(cursor),
                limit: 64,
                mode: ObserveModeV1::CommittedOnly,
            })
            .unwrap_err()
            .to_string();
        assert!(error.contains(OBSERVE_INVALID_CURSOR), "{error}");
    }

    #[test]
    fn observe_review_request_separates_through_revision_from_resume_cursor() {
        let request = serde_json::json!({
            "schema_version": 1,
            "persona_scope": ae_contracts::hex::encode32(&[91; 32]),
            "relation_scope": null,
            "through_revision": 7,
            "after": {
                "schema_version": 1,
                "persona_scope": ae_contracts::hex::encode32(&[91; 32]),
                "journal_revision": 6,
                "event_id": ae_contracts::hex::encode16(&[92; 16])
            },
            "limit": 64,
            "mode": "committed_only"
        });
        let request = serde_json::from_value::<ObserveEventsRequestV1>(request).unwrap();
        assert_eq!(request.after.unwrap().event_id, Some([92; 16]));
        let completed = serde_json::json!({
            "schema_version": 1,
            "persona_scope": ae_contracts::hex::encode32(&[91; 32]),
            "relation_scope": null,
            "through_revision": 7,
            "after": {
                "schema_version": 1,
                "persona_scope": ae_contracts::hex::encode32(&[91; 32]),
                "journal_revision": 6,
                "event_id": null
            },
            "limit": 64,
            "mode": "committed_only"
        });
        assert_eq!(
            serde_json::from_value::<ObserveEventsRequestV1>(completed)
                .unwrap()
                .after
                .unwrap()
                .event_id,
            None
        );
    }

    #[test]
    fn observe_review_invalid_requests_have_a_stable_code() {
        let error = Store::open_in_memory()
            .unwrap()
            .observe_events_v1(&ObserveEventsRequestV1 {
                schema_version: 1,
                persona_scope: [93; 32],
                relation_scope: None,
                through_revision: None,
                after: None,
                limit: 0,
                mode: ObserveModeV1::CommittedOnly,
            })
            .unwrap_err()
            .to_string();
        assert!(error.contains("OBSERVE_INVALID_REQUEST"), "{error}");
    }

    #[test]
    fn observe_budget_does_not_report_ambiguous_charged_tokens_as_reserved() {
        let (mut store, _scope, persona_scope, relation_scope, intention_id) =
            committed_observe_store_with_intention();
        let tx = store
            .conn
            .as_mut()
            .unwrap()
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        tx.execute(
            "INSERT INTO externalization_budget(relation_scope,budget_day_start_utc_ms,reserved_tokens) VALUES(?1,?2,123)",
            params![blob(relation_scope), 1_699_920_000_000_i64],
        )
        .unwrap();
        append_operational_authority(&tx, "externalization_settled", &intention_id).unwrap();
        tx.commit().unwrap();

        let snapshot = store
            .observe_snapshot_v1(&ObserveSnapshotRequestV1 {
                schema_version: 1,
                persona_scope,
                relation_scope: Some(relation_scope),
                mode: ObserveModeV1::CommittedOnly,
            })
            .unwrap();
        assert_eq!(snapshot.budget.day_start_utc_ms, None);
        assert_eq!(snapshot.budget.reserved_tokens, None);
        assert_eq!(snapshot.budget.used_tokens, None);
    }

    #[test]
    fn observe_snapshot_keeps_operational_fields_unavailable_after_real_budget_transition() {
        let (mut store, scope, persona_scope, relation_scope, intention_ids) =
            committed_observe_store_with_intentions(&[65, 70]);
        let policy = RelationTemporalPolicyV1 {
            schema_version: 1,
            relation_scope,
            user_timezone: "UTC".into(),
            timezone_source: TimezoneSourceV1::Explicit,
            quiet_hours_start_minute: 0,
            quiet_hours_end_minute: 0,
            quiet_hours_emergency_bypass: false,
            proactive_enabled: true,
            proactive_daily_max: 2,
            min_proactive_cooldown_ms: 0,
            intention_ttl_ms: 86_400_000,
            unanswered_backoff_base_ms: 1_000,
            unanswered_hard_stop: 3,
            emergency_threshold: Fixed::ONE,
            daily_submitted: 0,
            consecutive_unanswered: 0,
            last_inbound_utc_ms: None,
            last_proactive_submitted_utc_ms: None,
            revision: 1,
            auto_policy_version: 0,
            next_claim_reservation_tokens: 256,
        };
        store.upsert_relation_policy(&policy).unwrap();
        let target = OutboundTargetEnvelopeV1 {
            schema_version: 1,
            target_kind: TargetKindV1::Private,
            umo_ciphertext: vec![71; 32],
            umo_nonce: vec![72; 12],
            key_id: "test-key".into(),
            umo_digest: [73; 32],
            platform_token: [74; 16],
            bot_token: scope.bot_token,
            persona_token: scope.persona_token,
            relation_token: scope.relation_token.unwrap(),
            session_token: scope.session_token,
            bound_at_utc_ms: 1_700_000_000_000,
            binding_generation: 1,
            binding_digest: [75; 32],
        };
        store
            .store_outbound_target(&relation_scope, &target)
            .unwrap();
        let mut capability = HostCapabilitySnapshotV1 {
            schema_version: 1,
            astrbot_send_available: true,
            credential_store_available: true,
            platform_idempotent: false,
            provider_identifier: "test-provider".into(),
            config_source_digest: [76; 32],
            config_revision: 1,
            content_boundary_version: 1,
            policy_version: 1,
            snapshot_digest: [0; 32],
        };
        capability.snapshot_digest = capability_snapshot_digest(&capability).unwrap();
        let frozen = FrozenTimeInputV1 {
            schema_version: 1,
            observed_now_utc_ms: 1_700_000_000_000,
            effective_now_utc_ms: 1_700_000_000_000,
            persona_tzid: "UTC".into(),
            persona_utc_offset_seconds: 0,
            persona_local_minute: 0,
            persona_day_ordinal: 1,
            relation_tzid: "UTC".into(),
            relation_utc_offset_seconds: 0,
            relation_local_minute: 0,
            relation_day_ordinal: 1,
            budget_day_start_utc_ms: 1_699_920_000_000,
            budget_next_day_start_utc_ms: 1_700_006_400_000,
            next_timezone_transition_utc_ms: None,
            tzdb_fingerprint: [77; 32],
        };
        let caller = [78; 32];
        let claim = store
            .gate_and_claim_externalization(&GateAndClaimExternalizationRequestV1 {
                scope,
                intention_id: intention_ids[0],
                attempt_no: 1,
                expected_revision: 0,
                caller_incarnation: caller,
                capability,
                frozen,
            })
            .unwrap()
            .claim
            .unwrap();
        store
            .settle_externalization(&ExternalizationSettleV1 {
                claim_token: claim.claim_token,
                outcome: ExternalizationOutcomeV1::RejectedTerminal,
                used_tokens: Some(123),
                candidate_digest: None,
                candidate_ciphertext: None,
                caller_incarnation: caller,
            })
            .unwrap();

        let snapshot = store
            .observe_snapshot_v1(&ObserveSnapshotRequestV1 {
                schema_version: 1,
                persona_scope,
                relation_scope: Some(relation_scope),
                mode: ObserveModeV1::CommittedOnly,
            })
            .unwrap();
        assert_eq!(snapshot.pending.intentions, None);
        assert_eq!(snapshot.pending.outbounds, None);
        assert_eq!(snapshot.pending.active_claims, None);
        assert_eq!(snapshot.operational_ordinal, None);
        assert_eq!(snapshot.budget.day_start_utc_ms, None);
        assert_eq!(snapshot.budget.reserved_tokens, None);
        assert_eq!(snapshot.budget.used_tokens, None);
    }

    #[test]
    fn observe_reads_leave_authoritative_and_operational_state_unchanged() {
        let scope = observe_scope(false);
        let persona_scope =
            wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
        let mut store = Store::open_in_memory().unwrap();
        let mut initial = observe_state(persona_scope);
        initial.generation = 0;
        initial.state_revision = 0;
        store.initialize_autonomous_state(&initial).unwrap();
        store.upsert_autonomy_scope(&persona_scope, &scope).unwrap();
        let mut state = initial;
        state.generation = 1;
        state.state_revision = 1;
        commit_observe_wake(
            &mut store,
            &scope,
            &state,
            vec![observe_event(persona_scope, 42, 1_700_000_000_000)],
            vec![],
            43,
        );
        store
            .conn
            .as_mut()
            .unwrap()
            .execute(
                "INSERT INTO autonomy_claim(claim_token,claim_kind,record_id,caller_incarnation,lease_deadline_utc_ms,body_json) VALUES(?1,'observe_fixture',?2,?3,?4,'{}')",
                params![blob([43_u8; 32]), [44_u8; 16].to_vec(), blob([45_u8; 32]), 1_700_000_060_000_i64],
            )
            .unwrap();
        let before: (i64, i64, i64, i64, i64, i64) = store
            .connection()
            .unwrap()
            .query_row(
                "SELECT
                    (SELECT COALESCE(MAX(logical_revision),0) FROM journal WHERE scope_digest=?1),
                    (SELECT generation FROM autonomous_runtime_state WHERE persona_scope=?1),
                    (SELECT next_wake_at_utc_ms FROM wake_schedule WHERE persona_scope=?1),
                    (SELECT COUNT(*) FROM autonomy_claim),
                    (SELECT COUNT(*) FROM outbound_attempt),
                    (SELECT COALESCE(SUM(reserved_tokens),0) FROM externalization_budget)",
                params![blob(persona_scope)],
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

        let snapshot = store
            .observe_snapshot_v1(&ObserveSnapshotRequestV1 {
                schema_version: 1,
                persona_scope,
                relation_scope: None,
                mode: ObserveModeV1::CommittedOnly,
            })
            .unwrap();
        let events = store
            .observe_events_v1(&ObserveEventsRequestV1 {
                schema_version: 1,
                persona_scope,
                relation_scope: None,
                through_revision: None,
                after: None,
                limit: 64,
                mode: ObserveModeV1::CommittedOnly,
            })
            .unwrap();
        assert_eq!(snapshot.runtime, state);
        assert_eq!(snapshot.actor_epoch, None);
        assert_eq!(snapshot.actor_sequence, None);
        assert!(snapshot.projection.head_consistent);
        assert_eq!(snapshot.operational_ordinal, None);
        assert_eq!(snapshot.pending.intentions, None);
        assert_eq!(snapshot.pending.outbounds, None);
        assert_eq!(snapshot.pending.active_claims, None);
        assert_eq!(snapshot.budget.day_start_utc_ms, None);
        assert_eq!(snapshot.budget.reserved_tokens, None);
        assert_eq!(snapshot.budget.used_tokens, None);
        assert_eq!(events.items.len(), 1);
        let response_json = format!(
            "{}{}",
            serde_json::to_string(&snapshot).unwrap(),
            serde_json::to_string(&events).unwrap()
        );
        for forbidden_key in [
            "seed_code",
            "message_body",
            "prompt_contract",
            "candidate_ciphertext",
            "target",
            "caller_incarnation",
            "umo_ciphertext",
        ] {
            assert!(!response_json.contains(&format!("\"{forbidden_key}\":")));
        }

        let after: (i64, i64, i64, i64, i64, i64) = store
            .connection()
            .unwrap()
            .query_row(
                "SELECT
                    (SELECT COALESCE(MAX(logical_revision),0) FROM journal WHERE scope_digest=?1),
                    (SELECT generation FROM autonomous_runtime_state WHERE persona_scope=?1),
                    (SELECT next_wake_at_utc_ms FROM wake_schedule WHERE persona_scope=?1),
                    (SELECT COUNT(*) FROM autonomy_claim),
                    (SELECT COUNT(*) FROM outbound_attempt),
                    (SELECT COALESCE(SUM(reserved_tokens),0) FROM externalization_budget)",
                params![blob(persona_scope)],
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
        assert_eq!(after, before);
    }
}

fn json<T: serde::Serialize>(value: &T) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(|e| StoreError::AutonomyConflict(e.to_string()))
}

fn parse<T: serde::de::DeserializeOwned>(value: String) -> Result<T, StoreError> {
    serde_json::from_str(&value).map_err(|e| StoreError::AutonomyConflict(e.to_string()))
}

fn operational_externalization_claim(
    conn: &Connection,
    intention_id: &Id128,
    raw_caller: Vec<u8>,
    body: String,
) -> Result<ExternalizationClaimV1, StoreError> {
    if let Ok(claim) = serde_json::from_str::<ExternalizationClaimV1>(&body) {
        return Ok(claim);
    }
    let claim: ExternalizationClaimV2 = parse(body)?;
    let caller_incarnation: Digest = raw_caller.try_into().map_err(|_| {
        StoreError::AutonomyConflict("invalid alpha3 externalization caller".into())
    })?;
    let intention_body: String = conn.query_row(
        "SELECT body_json FROM durable_intention WHERE intention_id=?1",
        params![intention_id.to_vec()],
        |row| row.get(0),
    )?;
    let intention: DurableIntentionV1 = parse(intention_body)?;
    let current_locator =
        encode_intention_scoped_locator_v1(&intention.relation_scope, intention_id);
    if claim.intention_public_ref != current_locator
        && !legacy_v2_intention_public_ref_matches(
            &intention.relation_scope,
            intention_id,
            &claim.intention_public_ref,
        )
    {
        return Err(StoreError::AutonomyConflict(
            "alpha3 externalization claim public reference mismatch".into(),
        ));
    }
    let target_body: String = conn.query_row(
        "SELECT body_json FROM outbound_target
         WHERE relation_scope=?1 AND revoked=0 ORDER BY generation DESC LIMIT 1",
        params![blob(intention.relation_scope)],
        |row| row.get(0),
    )?;
    let target: OutboundTargetEnvelopeV1 = parse(target_body)?;
    let max_tokens = claim.reserved_tokens.try_into().map_err(|_| {
        StoreError::AutonomyConflict(
            "alpha3 externalization reservation exceeds operational bound".into(),
        )
    })?;
    let prompt_contract = "alpha3.externalization.v2".to_owned();
    Ok(ExternalizationClaimV1 {
        claim_token: claim.claim_token,
        intention_id: *intention_id,
        attempt_no: claim.attempt_no,
        max_tokens,
        prompt_contract_digest: wire::domain_hash(
            b"ae.alpha3.externalization-v2.prompt-contract",
            &[prompt_contract.as_bytes()],
        ),
        prompt_contract,
        relation_policy_revision: claim.policy_revision,
        target_binding_digest: target.binding_digest,
        capability_snapshot_digest: claim.capability_snapshot_digest,
        frozen_input_digest: wire::domain_hash(
            b"ae.alpha3.externalization-v2.operational",
            &[&claim.claim_token, &claim.intention_public_ref],
        ),
        caller_incarnation,
        lease_deadline_utc_ms: claim.lease_deadline_utc_ms,
    })
}

fn operational_dispatch_claim(
    _conn: &Connection,
    outbound: &OutboundAttemptV1,
    raw_caller: Vec<u8>,
    body: String,
) -> Result<DispatchClaimV1, StoreError> {
    if let Ok(claim) = serde_json::from_str::<DispatchClaimV1>(&body) {
        return Ok(claim);
    }
    let claim: DispatchClaimV2 = parse(body)?;
    let caller_incarnation: Digest = raw_caller
        .try_into()
        .map_err(|_| StoreError::AutonomyConflict("invalid alpha3 dispatch caller".into()))?;
    let relation_scope = wire::persona_scope_digest(
        &outbound.target.bot_token,
        &outbound.target.persona_token,
        Some(&outbound.target.relation_token),
    );
    let current_locator =
        encode_execution_scoped_locator_v1(&relation_scope, &outbound.outbound_id);
    if (claim.outbound_public_ref != current_locator
        && !legacy_v2_execution_public_ref_matches(
            &relation_scope,
            &outbound.outbound_id,
            &claim.outbound_public_ref,
        ))
        || claim.target_binding_digest != outbound.target.binding_digest
    {
        return Err(StoreError::AutonomyConflict(
            "alpha3 dispatch claim authority mismatch".into(),
        ));
    }
    Ok(DispatchClaimV1 {
        claim_token: claim.claim_token,
        outbound_id: outbound.outbound_id,
        target: outbound.target.clone(),
        candidate_ciphertext: outbound.candidate_ciphertext.clone(),
        preflight_digest: wire::domain_hash(
            b"ae.alpha3.dispatch-v2.preflight",
            &[&claim.claim_token, &claim.outbound_public_ref],
        ),
        relation_policy_revision: claim.policy_revision,
        capability_snapshot_digest: claim.capability_snapshot_digest,
        frozen_input_digest: wire::domain_hash(
            b"ae.alpha3.dispatch-v2.operational",
            &[&claim.claim_token, &claim.target_binding_digest],
        ),
        caller_incarnation,
        lease_deadline_utc_ms: claim.lease_deadline_utc_ms,
    })
}

fn read_operational_outbound_and_claim(
    conn: &Connection,
    intention_id: &Id128,
) -> Result<(Option<OutboundAttemptV1>, Option<OperationalClaimV1>), StoreError> {
    let outbound: Option<OutboundAttemptV1> = conn
        .query_row(
            "SELECT body_json FROM outbound_attempt WHERE intention_id=?1",
            params![intention_id.to_vec()],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(parse)
        .transpose()?;
    let externalization_claim = conn
        .query_row(
            "SELECT caller_incarnation,
                    CASE WHEN typeof(body_json)='text'
                               AND length(CAST(body_json AS BLOB))<=?2
                         THEN body_json END
             FROM autonomy_claim
             WHERE claim_kind IN ('externalization','externalization_v2') AND record_id=?1",
            params![intention_id.to_vec(), MAX_AUTONOMY_CLAIM_BODY_BYTES as i64],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?
        .map(|(caller, body)| {
            operational_externalization_claim(
                conn,
                intention_id,
                caller,
                bounded_autonomy_claim_body(body)?,
            )
        })
        .transpose()?;
    let dispatch_claim = if let Some(outbound) = &outbound {
        conn.query_row(
            "SELECT caller_incarnation,
                    CASE WHEN typeof(body_json)='text'
                               AND length(CAST(body_json AS BLOB))<=?2
                         THEN body_json END
             FROM autonomy_claim
             WHERE claim_kind IN ('dispatch','dispatch_v2') AND record_id=?1",
            params![
                outbound.outbound_id.to_vec(),
                MAX_AUTONOMY_CLAIM_BODY_BYTES as i64
            ],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?
        .map(|(caller, body)| {
            operational_dispatch_claim(conn, outbound, caller, bounded_autonomy_claim_body(body)?)
        })
        .transpose()?
    } else {
        None
    };
    if externalization_claim.is_some() && dispatch_claim.is_some() {
        return Err(StoreError::AutonomyConflict(
            "multiple active operational claims for one intention".into(),
        ));
    }
    let active_claim = externalization_claim
        .map(OperationalClaimV1::Externalization)
        .or_else(|| dispatch_claim.map(OperationalClaimV1::Dispatch));
    Ok((outbound, active_claim))
}

fn read_operational_budgets(
    conn: &Connection,
    relation_scope: &Digest,
) -> Result<Vec<OperationalBudgetV1>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT budget_day_start_utc_ms,limit_tokens,reserved_tokens,
                charged_tokens,used_tokens,usage_known,revision
         FROM externalization_budget WHERE relation_scope=?1
         ORDER BY budget_day_start_utc_ms",
    )?;
    let values = statement
        .query_map(params![blob(*relation_scope)], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    values
        .into_iter()
        .map(
            |(day_start, limit, reserved, charged, used, usage_known, revision)| {
                if !matches!(usage_known, 0 | 1) {
                    return Err(StoreError::AutonomyConflict(
                        "invalid operational budget usage-known marker".into(),
                    ));
                }
                let budget = OperationalBudgetV1 {
                    budget_day_start_utc_ms: day_start.try_into().map_err(|_| {
                        StoreError::AutonomyConflict("negative operational budget day".into())
                    })?,
                    limit_tokens: limit.try_into().map_err(|_| {
                        StoreError::AutonomyConflict("invalid operational budget limit".into())
                    })?,
                    reserved_tokens: reserved.try_into().map_err(|_| {
                        StoreError::AutonomyConflict("invalid operational budget tokens".into())
                    })?,
                    charged_tokens: charged.try_into().map_err(|_| {
                        StoreError::AutonomyConflict("invalid operational budget charge".into())
                    })?,
                    used_tokens: used.try_into().map_err(|_| {
                        StoreError::AutonomyConflict("invalid operational budget usage".into())
                    })?,
                    usage_known: usage_known == 1,
                    revision: revision.try_into().map_err(|_| {
                        StoreError::AutonomyConflict("invalid operational budget revision".into())
                    })?,
                };
                let consumed = budget
                    .charged_tokens
                    .checked_add(budget.reserved_tokens)
                    .ok_or_else(|| {
                        StoreError::AutonomyConflict("operational budget token overflow".into())
                    })?;
                if budget.revision == 0
                    || consumed > budget.limit_tokens
                    || budget.used_tokens > budget.charged_tokens
                {
                    return Err(StoreError::AutonomyConflict(
                        "operational budget authority is inconsistent".into(),
                    ));
                }
                Ok(budget)
            },
        )
        .collect()
}

fn capture_operational_delta(
    conn: &Connection,
    event_kind: &str,
    intention_id: &Id128,
) -> Result<OperationalAuthorityDeltaV1, StoreError> {
    let (intention_body, intention_revision): (String, i64) = conn.query_row(
        "SELECT body_json,revision FROM durable_intention WHERE intention_id=?1",
        params![intention_id.to_vec()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let intention: DurableIntentionV1 = parse(intention_body)?;
    let (outbound, active_claim) = read_operational_outbound_and_claim(conn, intention_id)?;
    let (active_claim_budget_day_start_utc_ms, active_claim_unknown_full_charged) =
        if matches!(&active_claim, Some(OperationalClaimV1::Externalization(_))) {
            let (raw_day, migrated): (i64, i64) = conn.query_row(
                "SELECT b.budget_day_start_utc_ms,b.migrated_unknown_full_charge
                 FROM autonomy_claim AS c
                 JOIN externalization_budget_claim AS b ON b.claim_token=c.claim_token
                 WHERE c.record_id=?1
                   AND c.claim_kind IN ('externalization','externalization_v2')",
                params![intention_id.to_vec()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            if !matches!(migrated, 0 | 1) {
                return Err(StoreError::AutonomyConflict(
                    "invalid operational claim settlement marker".into(),
                ));
            }
            (
                Some(raw_day.try_into().map_err(|_| {
                    StoreError::AutonomyConflict("negative operational claim budget day".into())
                })?),
                migrated == 1,
            )
        } else {
            (None, false)
        };
    let budgets = read_operational_budgets(conn, &intention.relation_scope)?;
    let policy = conn
        .query_row(
            "SELECT body_json FROM relation_temporal_policy WHERE relation_scope=?1",
            params![blob(intention.relation_scope)],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(parse)
        .transpose()?;
    Ok(OperationalAuthorityDeltaV1 {
        schema_version: 1,
        event_kind: event_kind.to_owned(),
        persona_scope: intention.persona_scope,
        relation_scope: intention.relation_scope,
        intention_id: *intention_id,
        intention,
        intention_revision: intention_revision.try_into().map_err(|_| {
            StoreError::AutonomyConflict("negative operational intention revision".into())
        })?,
        outbound,
        active_claim,
        active_claim_budget_day_start_utc_ms,
        active_claim_unknown_full_charged,
        budgets,
        policy,
        recorded_at_utc_ms: now_ms(),
    })
}

// Preserve the established storage/API shape in this compatibility boundary.
#[allow(clippy::type_complexity)]
fn terminalize_revoked_alpha3_work(
    tx: &Transaction<'_>,
    event_kind: &str,
    intention_id: &Id128,
) -> Result<(), StoreError> {
    let mut terminal_state = match event_kind {
        "contact_intention_expired" => IntentionStateV1::Expired,
        "contact_intention_suppressed" | "contact_basis_revoked_after_boundary" => {
            IntentionStateV1::Suppressed
        }
        _ => return Ok(()),
    };
    let (intention_body, intention_revision): (String, i64) = tx.query_row(
        "SELECT body_json,revision FROM durable_intention WHERE intention_id=?1",
        params![intention_id.to_vec()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let mut intention: DurableIntentionV1 = parse(intention_body)?;
    let original_intention = intention.clone();
    let mut wake_terminalization = false;
    if event_kind == "contact_basis_revoked_after_boundary" {
        let basis_body: String = tx.query_row(
            "SELECT body_json FROM contact_intention_basis WHERE intention_id=?1",
            params![intention_id.to_vec()],
            |row| row.get(0),
        )?;
        let basis: ContactIntentionBasisV1 = parse(basis_body)?;
        let runtime_body: String = tx.query_row(
            "SELECT body_json FROM autonomous_runtime_state WHERE persona_scope=?1",
            params![blob(intention.persona_scope)],
            |row| row.get(0),
        )?;
        let runtime: AutonomousRuntimeStateV1 = parse(runtime_body)?;
        if basis.intention_id != *intention_id
            || basis.relation_scope != intention.relation_scope
            || runtime.persona_scope != intention.persona_scope
        {
            return Err(StoreError::AutonomyConflict(
                "revoked-work expiry authority is inconsistent".into(),
            ));
        }
        if intention.expires_at_utc_ms <= runtime.last_advanced_at_utc_ms
            || basis.expires_at_utc_ms <= runtime.last_advanced_at_utc_ms
        {
            terminal_state = IntentionStateV1::Expired;
        }
        let mut statement = tx.prepare(
            "SELECT CASE WHEN typeof(body_json)='text'
                               AND length(CAST(body_json AS BLOB))<=?1
                         THEN body_json END
             FROM autonomy_claim WHERE claim_kind='wake_v2'",
        )?;
        let wake_bodies = statement
            .query_map(params![MAX_AUTONOMY_CLAIM_BODY_BYTES as i64], |row| {
                row.get::<_, Option<String>>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        for body in wake_bodies {
            let body = bounded_autonomy_claim_body(body)?;
            let claim: WakeClaimV2 = parse(body)?;
            if claim.proposal.legacy.state.persona_scope == intention.persona_scope
                && claim.proposal.legacy.state.generation == runtime.generation
                && claim.event.frozen.effective_now_utc_ms == runtime.last_advanced_at_utc_ms
            {
                wake_terminalization = true;
                break;
            }
        }
    }
    let consent_body: Option<String> = tx
        .query_row(
            "SELECT c.body_json FROM relation_consent_head AS h
             JOIN relation_consent AS c
               ON c.relation_scope=h.relation_scope
              AND c.consent_epoch=h.consent_epoch AND c.revision=h.revision
             WHERE h.relation_scope=?1",
            params![blob(intention.relation_scope)],
            |row| row.get(0),
        )
        .optional()?;
    let consent = consent_body.map(parse::<RelationConsentV1>).transpose()?;
    if !wake_terminalization
        && !consent.is_some_and(|value| {
            matches!(
                value.state,
                RelationConsentStateV1::Paused | RelationConsentStateV1::Ended
            )
        })
    {
        return Ok(());
    }

    let externalization: Option<(Vec<u8>, Vec<u8>, Vec<u8>, i64, i64, i64, Option<String>)> = tx
        .query_row(
            "SELECT c.claim_token,c.caller_incarnation,b.relation_scope,
                    b.budget_day_start_utc_ms,b.reserved_tokens,
                    b.migrated_unknown_full_charge,
                    CASE WHEN typeof(c.body_json)='text'
                               AND length(CAST(c.body_json AS BLOB))<=?2
                         THEN c.body_json END
             FROM autonomy_claim AS c
             JOIN externalization_budget_claim AS b ON b.claim_token=c.claim_token
             WHERE c.claim_kind IN ('externalization','externalization_v2')
               AND c.record_id=?1",
            params![intention_id.to_vec(), MAX_AUTONOMY_CLAIM_BODY_BYTES as i64],
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
        .optional()?;
    if let Some((token, caller, raw_relation, day_start, reserved, migrated, body)) =
        externalization
    {
        let body = bounded_autonomy_claim_body(body)?;
        let relation_scope: Digest = raw_relation.try_into().map_err(|_| {
            StoreError::AutonomyConflict("invalid revoked-work budget relation".into())
        })?;
        if relation_scope != intention.relation_scope
            || day_start < 0
            || reserved < 0
            || !matches!(migrated, 0 | 1)
        {
            return Err(StoreError::AutonomyConflict(
                "revoked externalization budget authority is inconsistent".into(),
            ));
        }
        let claim = operational_externalization_claim(tx, intention_id, caller, body)?;
        if claim.claim_token.as_slice() != token.as_slice() {
            return Err(StoreError::AutonomyConflict(
                "revoked externalization claim token mismatch".into(),
            ));
        }
        if migrated == 0 {
            let changed = tx.execute(
                "UPDATE externalization_budget
                 SET reserved_tokens=reserved_tokens-?3,
                     charged_tokens=charged_tokens+?3,
                     usage_known=0,
                     revision=revision+1
                 WHERE relation_scope=?1 AND budget_day_start_utc_ms=?2
                   AND reserved_tokens>=?3
                   AND charged_tokens+reserved_tokens<=limit_tokens
                   AND used_tokens<=charged_tokens",
                params![blob(relation_scope), day_start, reserved],
            )?;
            if changed != 1 {
                return Err(StoreError::AutonomyConflict(
                    "revoked externalization reservation is unavailable".into(),
                ));
            }
        }
        let deleted_budget = tx.execute(
            "DELETE FROM externalization_budget_claim WHERE claim_token=?1",
            params![token.clone()],
        )?;
        let deleted_claim = tx.execute(
            "DELETE FROM autonomy_claim WHERE claim_token=?1
             AND claim_kind IN ('externalization','externalization_v2')",
            params![token],
        )?;
        if deleted_budget != 1 || deleted_claim != 1 {
            return Err(StoreError::AutonomyConflict(
                "revoked externalization claim removal failed".into(),
            ));
        }
        intention.state = terminal_state;
    } else if matches!(
        intention.state,
        IntentionStateV1::Forming
            | IntentionStateV1::Ready
            | IntentionStateV1::Deferred
            | IntentionStateV1::Externalizing
    ) {
        intention.state = terminal_state;
    }

    let outbound_row: Option<(Vec<u8>, String, String)> = tx
        .query_row(
            "SELECT outbound_id,state,body_json FROM outbound_attempt WHERE intention_id=?1",
            params![intention_id.to_vec()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if let Some((raw_outbound_id, stored_state, outbound_body)) = outbound_row {
        let outbound_id: Id128 = raw_outbound_id
            .as_slice()
            .try_into()
            .map_err(|_| StoreError::AutonomyConflict("invalid revoked-work outbound id".into()))?;
        let mut outbound: OutboundAttemptV1 = parse(outbound_body)?;
        if outbound.outbound_id != outbound_id
            || outbound.intention_id != *intention_id
            || stored_state != state_name(outbound.state)?
        {
            return Err(StoreError::AutonomyConflict(
                "revoked-work outbound projection is inconsistent".into(),
            ));
        }
        let dispatch: Option<(Vec<u8>, Vec<u8>, Option<String>)> = tx
            .query_row(
                "SELECT claim_token,caller_incarnation,
                        CASE WHEN typeof(body_json)='text'
                                   AND length(CAST(body_json AS BLOB))<=?2
                             THEN body_json END
                 FROM autonomy_claim
                 WHERE claim_kind IN ('dispatch','dispatch_v2') AND record_id=?1",
                params![outbound_id.to_vec(), MAX_AUTONOMY_CLAIM_BODY_BYTES as i64],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((token, caller, body)) = &dispatch {
            let claim = operational_dispatch_claim(
                tx,
                &outbound,
                caller.clone(),
                bounded_autonomy_claim_body(body.clone())?,
            )?;
            if claim.claim_token.as_slice() != token.as_slice()
                || outbound.state != IntentionStateV1::AdapterCallStarted
            {
                return Err(StoreError::AutonomyConflict(
                    "revoked dispatch claim authority is inconsistent".into(),
                ));
            }
        }
        let original_outbound = outbound.clone();
        if outbound.state == IntentionStateV1::AdapterCallStarted {
            outbound.state = IntentionStateV1::DispatchUnknown;
            outbound.settled_at_utc_ms = Some(now_ms());
            intention.state = IntentionStateV1::DispatchUnknown;
        } else if outbound.state == IntentionStateV1::DispatchPending {
            outbound.state = IntentionStateV1::Terminal;
            outbound.settled_at_utc_ms = Some(now_ms());
            intention.state = terminal_state;
        }
        if outbound != original_outbound {
            let changed = tx.execute(
                "UPDATE outbound_attempt SET state=?2,body_json=?3
                 WHERE outbound_id=?1 AND state=?4",
                params![
                    outbound_id.to_vec(),
                    state_name(outbound.state)?,
                    json(&outbound)?,
                    state_name(original_outbound.state)?,
                ],
            )?;
            if changed != 1 {
                return Err(StoreError::AutonomyConflict(
                    "revoked-work outbound compare-and-swap failed".into(),
                ));
            }
        }
        if let Some((token, _, _)) = dispatch {
            tx.execute(
                "DELETE FROM autonomy_claim WHERE claim_token=?1
                 AND claim_kind IN ('dispatch','dispatch_v2')",
                params![token],
            )?;
        }
    }

    if intention != original_intention {
        let next_revision = intention_revision.checked_add(1).ok_or_else(|| {
            StoreError::AutonomyConflict("revoked-work intention revision overflow".into())
        })?;
        let changed = tx.execute(
            "UPDATE durable_intention SET state=?2,revision=?3,body_json=?4
             WHERE intention_id=?1 AND revision=?5",
            params![
                intention_id.to_vec(),
                state_name(intention.state)?,
                next_revision,
                json(&intention)?,
                intention_revision,
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::AutonomyConflict(
                "revoked-work intention compare-and-swap failed".into(),
            ));
        }
    }
    Ok(())
}

pub(crate) fn append_operational_authority(
    tx: &Transaction<'_>,
    event_kind: &str,
    intention_id: &Id128,
) -> Result<(), StoreError> {
    crate::core_boundary_v9::enforce_retired(
        tx,
        crate::core_boundary_v9::RetiredOperationTagV1::append_operational_authority,
    )?;
    terminalize_revoked_alpha3_work(tx, event_kind, intention_id)?;
    let delta = capture_operational_delta(tx, event_kind, intention_id)?;
    let anchor = tx
        .query_row(
            "SELECT entry_count,head_digest,ever_seen FROM autonomy_operational_authority_head WHERE persona_scope=?1",
            params![blob(delta.persona_scope)],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?;
    let (count, previous, ever_seen) = match anchor {
        Some((count, raw_head, ever_seen)) => {
            let head: Digest = raw_head.try_into().map_err(|_| {
                StoreError::AutonomyConflict("invalid operational anchor digest".into())
            })?;
            (count, head, ever_seen)
        }
        None => {
            let existing: i64 = tx.query_row(
                "SELECT COUNT(*) FROM autonomy_operational_authority WHERE persona_scope=?1",
                params![blob(delta.persona_scope)],
                |row| row.get(0),
            )?;
            if existing != 0 {
                return Err(StoreError::AutonomyConflict(
                    "operational authority anchor is missing".into(),
                ));
            }
            tx.execute(
                "INSERT INTO autonomy_operational_authority_head(persona_scope,entry_count,head_digest,ever_seen) VALUES(?1,0,?2,0)",
                params![blob(delta.persona_scope), blob([0; 32])],
            )?;
            (0, [0; 32], 0)
        }
    };
    if count < 0 || !matches!(ever_seen, 0 | 1) || (count == 0 && ever_seen != 0) {
        return Err(StoreError::AutonomyConflict(
            "invalid operational authority anchor".into(),
        ));
    }
    let actual = tx.query_row(
        "SELECT COUNT(*),COALESCE(MAX(persona_ordinal),0) FROM autonomy_operational_authority WHERE persona_scope=?1",
        params![blob(delta.persona_scope)],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )?;
    if actual != (count, count) {
        return Err(StoreError::AutonomyConflict(
            "operational authority length mismatch".into(),
        ));
    }
    if count > 0 {
        let stored_head: Vec<u8> = tx.query_row(
            "SELECT chain_digest FROM autonomy_operational_authority WHERE persona_scope=?1 AND persona_ordinal=?2",
            params![blob(delta.persona_scope), count],
            |row| row.get(0),
        )?;
        if stored_head != previous.to_vec() {
            return Err(StoreError::AutonomyConflict(
                "operational authority head mismatch".into(),
            ));
        }
    }
    let body = json(&delta)?;
    let chain = operational_chain_digest(&previous, body.as_bytes());
    tx.execute(
        "INSERT INTO autonomy_operational_authority(persona_scope,persona_ordinal,relation_scope,intention_id,event_kind,delta_json,previous_digest,chain_digest,committed_at_utc_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![blob(delta.persona_scope),count+1,blob(delta.relation_scope),delta.intention_id.to_vec(),event_kind,body,blob(previous),blob(chain),delta.recorded_at_utc_ms as i64],
    )?;
    let changed = tx.execute(
        "UPDATE autonomy_operational_authority_head SET entry_count=?2,head_digest=?3,ever_seen=1 WHERE persona_scope=?1 AND entry_count=?4 AND head_digest=?5",
        params![blob(delta.persona_scope),count+1,blob(chain),count,blob(previous)],
    )?;
    if changed != 1 {
        return Err(StoreError::AutonomyConflict(
            "operational authority anchor CAS failed".into(),
        ));
    }
    let ordinal: u64 = (count + 1).try_into().map_err(|_| {
        StoreError::AutonomyConflict("invalid operational authority ordinal".into())
    })?;
    append_operational_checkpoint(tx, ordinal, chain, &delta)?;
    Ok(())
}

fn read_operational_authority(
    conn: &Connection,
    persona_scope: &Digest,
) -> Result<Vec<(u64, OperationalAuthorityDeltaV1)>, StoreError> {
    let anchor = conn
        .query_row(
            "SELECT entry_count,head_digest,ever_seen FROM autonomy_operational_authority_head WHERE persona_scope=?1",
            params![blob(*persona_scope)],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?;
    let mut statement = conn.prepare(
        "SELECT persona_ordinal,persona_scope,relation_scope,intention_id,event_kind,delta_json,previous_digest,chain_digest FROM autonomy_operational_authority WHERE persona_scope=?1 ORDER BY persona_ordinal ASC",
    )?;
    let rows = statement
        .query_map(params![blob(*persona_scope)], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Vec<u8>>(6)?,
                row.get::<_, Vec<u8>>(7)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut previous = [0_u8; 32];
    let mut decoded = Vec::with_capacity(rows.len());
    for (raw_ordinal, stored_persona, stored_relation, stored_id, kind, body, prior, chain) in rows
    {
        let ordinal: u64 = raw_ordinal.try_into().map_err(|_| {
            StoreError::AutonomyConflict("negative operational authority ordinal".into())
        })?;
        let delta: OperationalAuthorityDeltaV1 = parse(body.clone())?;
        let expected_chain = operational_chain_digest(&previous, body.as_bytes());
        if ordinal != decoded.len() as u64 + 1
            || delta.schema_version != 1
            || delta.event_kind != kind
            || stored_persona != delta.persona_scope.to_vec()
            || stored_relation != delta.relation_scope.to_vec()
            || stored_id != delta.intention_id.to_vec()
            || delta.persona_scope != *persona_scope
            || prior != previous.to_vec()
            || chain != expected_chain.to_vec()
            || delta.intention.intention_id != delta.intention_id
            || delta.intention.persona_scope != delta.persona_scope
            || delta.intention.relation_scope != delta.relation_scope
        {
            return Err(StoreError::AutonomyConflict(
                "operational authority chain mismatch".into(),
            ));
        }
        previous = expected_chain;
        decoded.push((ordinal, delta));
    }
    match anchor {
        Some((count, raw_head, ever_seen)) => {
            let head: Digest = raw_head.try_into().map_err(|_| {
                StoreError::AutonomyConflict("invalid operational anchor digest".into())
            })?;
            if count < 0
                || usize::try_from(count).ok() != Some(decoded.len())
                || !matches!(ever_seen, 0 | 1)
                || (ever_seen == 1 && count == 0)
                || (count == 0 && head != [0; 32])
                || (count > 0 && head != previous)
            {
                return Err(StoreError::AutonomyConflict(
                    "operational authority anchor mismatch".into(),
                ));
            }
        }
        None if !decoded.is_empty() => {
            return Err(StoreError::AutonomyConflict(
                "operational authority anchor is missing".into(),
            ));
        }
        None => {}
    }
    Ok(decoded)
}

fn read_operational_checkpoints(
    conn: &Connection,
    persona_scope: &Digest,
) -> Result<Vec<(u64, OperationalCheckpointV1)>, StoreError> {
    let rows = {
        let mut statement = conn.prepare(
            "SELECT logical_revision,base_revision,event_kind,event_bytes,event_digest,receipt_bytes,delta_bytes,chain_digest FROM journal WHERE scope_digest=?1 ORDER BY logical_revision",
        )?;
        let values = statement
            .query_map(params![blob(*persona_scope)], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                    row.get::<_, Vec<u8>>(6)?,
                    row.get::<_, Vec<u8>>(7)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        values
    };
    let mut previous_journal = None::<Digest>;
    let mut previous_operational = [0; 32];
    let mut checkpoints = Vec::new();
    for (index, (raw_revision, raw_base, kind, event, event_digest, receipt, delta, chain)) in
        rows.into_iter().enumerate()
    {
        let revision: u64 = raw_revision.try_into().map_err(|_| {
            StoreError::AutonomyConflict("negative canonical journal revision".into())
        })?;
        let base: u64 = raw_base.try_into().map_err(|_| {
            StoreError::AutonomyConflict("negative canonical journal base revision".into())
        })?;
        if revision != index as u64 + 1 || base != revision.saturating_sub(1) {
            return Err(StoreError::AutonomyConflict(
                "canonical journal revision discontinuity".into(),
            ));
        }
        let stored_chain: Digest = chain.try_into().map_err(|_| {
            StoreError::AutonomyConflict("invalid canonical journal chain digest".into())
        })?;
        if let Some(seed) = previous_journal {
            let expected = if delta.is_empty() {
                ae_continuum::chain_link(&seed, &event, &receipt)
            } else {
                ae_continuum::chain_link_with_delta(&seed, &event, &receipt, &delta)
            };
            if expected != stored_chain {
                return Err(StoreError::AutonomyConflict(
                    "canonical journal chain mismatch".into(),
                ));
            }
        }
        previous_journal = Some(stored_chain);
        if kind != OPERATIONAL_CHECKPOINT_KIND {
            continue;
        }
        if !delta.is_empty() {
            return Err(StoreError::AutonomyConflict(
                "operational checkpoint has unexpected journal delta".into(),
            ));
        }
        let checkpoint: OperationalCheckpointV1 = serde_json::from_slice(&event)
            .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
        let ordinal = checkpoints.len() as u64 + 1;
        let frozen_digest = operational_delta_digest(&checkpoint.delta)?;
        let body = json(&checkpoint.delta)?;
        let expected_head = operational_chain_digest(&previous_operational, body.as_bytes());
        let expected_event = wire::domain_hash(b"ae.operational-checkpoint-event.v1", &[&event]);
        let expected_receipt = wire::domain_hash(
            b"ae.operational-checkpoint-receipt.v1",
            &[&expected_event, &expected_head, &ordinal.to_le_bytes()],
        );
        if checkpoint.schema_version != 1
            || checkpoint.persona_scope != *persona_scope
            || checkpoint.operational_ordinal != ordinal
            || checkpoint.operational_count != ordinal
            || checkpoint.operational_head_digest != expected_head
            || checkpoint.transition_kind != checkpoint.delta.event_kind
            || checkpoint.intention_id != checkpoint.delta.intention_id
            || checkpoint.outbound_id
                != checkpoint
                    .delta
                    .outbound
                    .as_ref()
                    .map(|value| value.outbound_id)
            || checkpoint.frozen_delta_digest != frozen_digest
            || event_digest != expected_event.to_vec()
            || receipt != expected_receipt.to_vec()
        {
            return Err(StoreError::AutonomyConflict(
                "canonical operational checkpoint mismatch".into(),
            ));
        }
        previous_operational = expected_head;
        checkpoints.push((ordinal, checkpoint));
    }
    Ok(checkpoints)
}

pub(crate) fn validate_observe_relation(
    conn: &Connection,
    persona_scope: &Digest,
    relation_scope: Option<&Digest>,
) -> Result<(), StoreError> {
    let Some(relation_scope) = relation_scope else {
        return Ok(());
    };
    let bound: i64 = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM autonomy_scope_binding WHERE work_scope=?1 AND persona_scope=?2)",
        params![blob(*relation_scope), blob(*persona_scope)],
        |row| row.get(0),
    )?;
    if bound == 0 {
        return Err(observe_invalid_request("relation is not bound to persona"));
    }
    Ok(())
}

fn observe_public_event_ref(persona_scope: &Digest, event_id: &Id128) -> String {
    ae_contracts::hex::encode32(&wire::domain_hash(
        b"ae.observe.public-event-ref.v1",
        &[persona_scope, event_id],
    ))
}

fn observe_event_projection(event: InnerEventV1) -> ObserveEventV1 {
    ObserveEventV1 {
        public_event_ref: observe_public_event_ref(&event.persona_scope, &event.event_id),
        source: ObserveEventSourceV1::Inner,
        kind: event.kind,
        committed_at_utc_ms: event.committed_at_utc_ms,
        relation_scope: None,
        authority_status: ObserveAuthorityStatusV1::Committed,
        causal: ObserveEventCausalV1 {
            turn_id: None,
            action_id: None,
            delivery_id: None,
            claim_id: None,
            parent_event_refs: event
                .source_event_ids
                .iter()
                .map(|event_id| observe_public_event_ref(&event.persona_scope, event_id))
                .collect(),
        },
        summary_code: event.summary_code,
        value_before: event.value_before,
        value_after: event.value_after,
    }
}

const OBSERVE_INVALID_REQUEST: &str = "OBSERVE_INVALID_REQUEST";
const OBSERVE_INVALID_CURSOR: &str = "OBSERVE_INVALID_CURSOR";
const OBSERVE_PROJECTION_UNAVAILABLE: &str = "OBSERVE_PROJECTION_UNAVAILABLE";

fn observe_error(code: &str, message: impl AsRef<str>) -> StoreError {
    StoreError::AutonomyConflict(format!("{code}::{}", message.as_ref()))
}

fn observe_invalid_request(message: impl AsRef<str>) -> StoreError {
    observe_error(OBSERVE_INVALID_REQUEST, message)
}

fn observe_invalid_cursor(message: impl AsRef<str>) -> StoreError {
    observe_error(OBSERVE_INVALID_CURSOR, message)
}

fn observe_projection_unavailable(message: impl AsRef<str>) -> StoreError {
    observe_error(OBSERVE_PROJECTION_UNAVAILABLE, message)
}

fn observe_request_i64(value: u64, field: &str) -> Result<i64, StoreError> {
    value
        .try_into()
        .map_err(|_| observe_invalid_request(format!("{field} is out of range")))
}

fn observe_projection_u64(value: i64, field: &str) -> Result<u64, StoreError> {
    value
        .try_into()
        .map_err(|_| observe_projection_unavailable(format!("negative {field}")))
}

pub(crate) struct ObserveStateHead {
    pub(crate) canonical_revision: u64,
    pub(crate) runtime: AutonomousRuntimeStateV1,
    pub(crate) mood_event: Option<InnerEventV1>,
}

pub(crate) fn observe_state_head(
    conn: &Connection,
    persona_scope: &Digest,
) -> Result<ObserveStateHead, StoreError> {
    // `autonomy_snapshot` is the bounded locator for the newest state-bearing
    // revision. The exact journal row remains the canonical source checked
    // below; this intentionally does not replay history from genesis.
    let canonical_revision = observe_projection_u64(
        conn.query_row(
            "SELECT COALESCE(MAX(logical_revision),0) FROM journal WHERE scope_digest=?1",
            params![blob(*persona_scope)],
            |row| row.get::<_, i64>(0),
        )?,
        "canonical revision",
    )?;
    let (raw_generation, raw_state_revision, bounded_runtime_body): (i64, i64, Option<String>) = conn
        .query_row(
            "SELECT generation,state_revision,CASE WHEN length(CAST(body_json AS BLOB))<=?2 THEN body_json END FROM autonomous_runtime_state WHERE persona_scope=?1",
            params![blob(*persona_scope), MAX_RUNTIME_STATE_BYTES as i64],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| observe_projection_unavailable("runtime state is missing"))?;
    let runtime_body = bounded_runtime_body.ok_or_else(|| {
        observe_projection_unavailable("runtime state exceeds its verification bound")
    })?;
    let runtime: AutonomousRuntimeStateV1 = serde_json::from_str(&runtime_body)
        .map_err(|_| observe_projection_unavailable("runtime state body is invalid"))?;
    let generation = observe_projection_u64(raw_generation, "runtime generation")?;
    let state_revision = observe_projection_u64(raw_state_revision, "runtime state revision")?;
    if runtime.persona_scope != *persona_scope
        || runtime.generation != generation
        || runtime.state_revision != state_revision
    {
        return Err(observe_projection_unavailable(
            "runtime state columns differ from its body",
        ));
    }

    let (raw_head_revision, raw_state_digest, bounded_state_bytes):
        (i64, Vec<u8>, Option<Vec<u8>>) = conn
        .query_row(
            "SELECT journal_revision,CASE WHEN length(state_digest)=32 THEN state_digest ELSE zeroblob(0) END,CASE WHEN length(state_bytes)<=?2 THEN state_bytes END FROM autonomy_snapshot WHERE persona_scope=?1 ORDER BY journal_revision DESC LIMIT 1",
            params![blob(*persona_scope), MAX_SNAPSHOT_STATE_BYTES as i64],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| observe_projection_unavailable("state-bearing snapshot head is missing"))?;
    let state_bytes = bounded_state_bytes.ok_or_else(|| {
        observe_projection_unavailable("state head exceeds its verification bound")
    })?;
    let head_revision = observe_projection_u64(raw_head_revision, "state head revision")?;
    if head_revision == 0 || head_revision > canonical_revision {
        return Err(observe_projection_unavailable(
            "state head revision is outside the canonical watermark",
        ));
    }
    let state_digest: Digest = raw_state_digest
        .try_into()
        .map_err(|_| observe_projection_unavailable("state head digest has invalid length"))?;
    let expected_digest = wire::domain_hash(b"ae.autonomy.snapshot.v1", &[&state_bytes]);
    if state_digest != expected_digest {
        return Err(observe_projection_unavailable(
            "state head digest differs from its bytes",
        ));
    }
    let snapshotted_state: AutonomousRuntimeStateV1 = serde_json::from_slice(&state_bytes)
        .map_err(|_| observe_projection_unavailable("state head bytes are invalid"))?;
    let bounded_delta_bytes: Option<Vec<u8>> = conn
        .query_row(
            "SELECT CASE WHEN length(delta_bytes)<=?3 THEN delta_bytes END FROM journal WHERE scope_digest=?1 AND logical_revision=?2",
            params![
                blob(*persona_scope),
                raw_head_revision,
                MAX_AUTONOMY_DELTA_BYTES as i64
            ],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| observe_projection_unavailable("state head canonical delta is missing"))?;
    let delta_bytes = bounded_delta_bytes.ok_or_else(|| {
        observe_projection_unavailable("state head canonical delta exceeds its verification bound")
    })?;
    let delta = observe_bounded_autonomy_delta(&delta_bytes, "state head canonical delta")?;
    let canonical_state = delta
        .state
        .ok_or_else(|| observe_projection_unavailable("state head delta does not carry state"))?;
    let canonical_state_bytes = serde_json::to_vec(&canonical_state)
        .map_err(|_| observe_projection_unavailable("canonical state head cannot be encoded"))?;
    let canonical_state_digest =
        wire::domain_hash(b"ae.autonomy.snapshot.v1", &[&canonical_state_bytes]);
    if state_bytes != canonical_state_bytes
        || state_digest != canonical_state_digest
        || canonical_state.persona_scope != *persona_scope
        || snapshotted_state != canonical_state
        || runtime != canonical_state
    {
        return Err(observe_projection_unavailable(
            "runtime state differs from the canonical state head",
        ));
    }
    let mut mood_event = None;
    for event in delta.inner_events {
        if event.persona_scope != *persona_scope {
            return Err(observe_projection_unavailable(
                "state head event has a different persona scope",
            ));
        }
        if !event.tombstoned
            && mood_event.as_ref().is_none_or(|current: &InnerEventV1| {
                (event.committed_at_utc_ms, event.event_id)
                    > (current.committed_at_utc_ms, current.event_id)
            })
        {
            mood_event = Some(event);
        }
    }
    Ok(ObserveStateHead {
        canonical_revision,
        runtime,
        mood_event,
    })
}

#[derive(Clone)]
struct ObserveStoredEventRow {
    event_id: Id128,
    journal_revision: u64,
    committed_at_utc_ms: u64,
    kind: String,
    tombstoned: bool,
    body: String,
}

fn decode_observe_stored_event_row(
    raw_id: Vec<u8>,
    raw_revision: i64,
    raw_committed_at: i64,
    kind: String,
    raw_tombstoned: i64,
    body: String,
) -> Result<ObserveStoredEventRow, StoreError> {
    let event_id = raw_id
        .try_into()
        .map_err(|_| observe_projection_unavailable("inner event id has invalid length"))?;
    let journal_revision = observe_projection_u64(raw_revision, "inner event journal revision")?;
    let committed_at_utc_ms =
        observe_projection_u64(raw_committed_at, "inner event committed time")?;
    let tombstoned = match raw_tombstoned {
        0 => false,
        1 => true,
        _ => {
            return Err(observe_projection_unavailable(
                "inner event tombstone is not boolean",
            ))
        }
    };
    Ok(ObserveStoredEventRow {
        event_id,
        journal_revision,
        committed_at_utc_ms,
        kind,
        tombstoned,
        body,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ObserveDatabaseEpoch {
    total_changes: u64,
    data_version: i64,
    schema_version: i64,
}

#[derive(Clone)]
struct ObserveCanonicalRevision {
    journal_revision: u64,
    event_bytes: u64,
    delta_bytes: u64,
    chain_digest: Digest,
}

struct ObserveRevisionWitness {
    canonical: ObserveCanonicalRevision,
    event_count: u64,
    event_digest: Digest,
    events: Vec<InnerEventV1>,
}

#[derive(Default)]
pub(crate) struct ObserveWitnessCache {
    epoch: Option<ObserveDatabaseEpoch>,
    revisions: BTreeMap<(Digest, u64), Arc<ObserveRevisionWitness>>,
    #[cfg(test)]
    canonical_parse_count: u64,
    #[cfg(test)]
    witness_lookup_count: u64,
}

impl ObserveWitnessCache {
    fn synchronize(&mut self, epoch: ObserveDatabaseEpoch) {
        if self.epoch.as_ref() != Some(&epoch) {
            self.revisions.clear();
            self.epoch = Some(epoch);
        }
    }
}

pub(crate) struct ObserveQueryOnlyGuard<'a> {
    conn: &'a Connection,
    active: bool,
}

impl<'a> ObserveQueryOnlyGuard<'a> {
    pub(crate) fn enable(conn: &'a Connection) -> Result<Self, StoreError> {
        conn.pragma_update(None, "query_only", "ON")?;
        Ok(Self { conn, active: true })
    }

    pub(crate) fn finish(mut self) -> Result<(), StoreError> {
        self.conn.pragma_update(None, "query_only", "OFF")?;
        self.active = false;
        Ok(())
    }
}

impl Drop for ObserveQueryOnlyGuard<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = self.conn.pragma_update(None, "query_only", "OFF");
        }
    }
}

fn observe_bounded_autonomy_delta(
    bytes: &[u8],
    context: &str,
) -> Result<AutonomyJournalDeltaV1, StoreError> {
    if bytes.len() > MAX_AUTONOMY_DELTA_BYTES {
        return Err(observe_projection_unavailable(format!(
            "{context} exceeds the 1 MiB verification bound"
        )));
    }
    let delta: AutonomyJournalDeltaV1 = serde_json::from_slice(bytes)
        .map_err(|_| observe_projection_unavailable(format!("{context} is invalid")))?;
    if let Some(reason) = autonomy_delta_bound_violation(&delta) {
        return Err(observe_projection_unavailable(format!(
            "{context} exceeds its verification bound: {reason}"
        )));
    }
    Ok(delta)
}

fn observe_database_epoch(conn: &Connection) -> Result<ObserveDatabaseEpoch, StoreError> {
    Ok(ObserveDatabaseEpoch {
        total_changes: conn.total_changes(),
        data_version: conn.query_row("PRAGMA data_version", [], |row| row.get(0))?,
        schema_version: conn.query_row("PRAGMA schema_version", [], |row| row.get(0))?,
    })
}

fn observe_manifest_for_revision(
    conn: &Connection,
    persona_scope: &Digest,
    journal_revision: u64,
) -> Result<(u64, Digest), StoreError> {
    let revision_sql = observe_request_i64(journal_revision, "event manifest revision")?;
    let (raw_count, raw_digest): (i64, Vec<u8>) = conn
        .query_row(
            "SELECT event_count,CASE WHEN length(event_digest)=32 THEN event_digest ELSE zeroblob(0) END FROM inner_event_manifest WHERE persona_scope=?1 AND journal_revision=?2",
            params![blob(*persona_scope), revision_sql],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| observe_projection_unavailable("canonical TimeAdvance manifest is missing"))?;
    let event_count = observe_projection_u64(raw_count, "inner event manifest count")?;
    if event_count > MAX_INNER_EVENTS_PER_DELTA as u64 {
        return Err(observe_projection_unavailable(
            "inner event manifest count exceeds its verification bound",
        ));
    }
    let event_digest = raw_digest.try_into().map_err(|_| {
        observe_projection_unavailable("inner event manifest digest has invalid length")
    })?;
    Ok((event_count, event_digest))
}

fn observe_projection_count(
    conn: &Connection,
    persona_scope: &Digest,
    journal_revision: u64,
) -> Result<u64, StoreError> {
    let (raw_count, raw_bytes): (i64, i64) = conn.query_row(
        "SELECT COUNT(*),COALESCE(SUM(length(CAST(body_json AS BLOB))),0) FROM inner_event WHERE persona_scope=?1 AND journal_revision=?2",
        params![
            blob(*persona_scope),
            observe_request_i64(journal_revision, "inner event revision")?
        ],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let projected_bytes = observe_projection_u64(raw_bytes, "inner event projection byte count")?;
    if projected_bytes > MAX_AUTONOMY_DELTA_BYTES as u64 {
        return Err(observe_projection_unavailable(
            "inner event projection exceeds its per-revision verification bound",
        ));
    }
    observe_projection_u64(raw_count, "inner event projection count")
}

fn verify_observe_projection_leaf(
    row: ObserveStoredEventRow,
    journal_revision: u64,
    canonical: &InnerEventV1,
) -> Result<InnerEventV1, StoreError> {
    let stored: InnerEventV1 = serde_json::from_str(&row.body)
        .map_err(|_| observe_projection_unavailable("inner event projection body is invalid"))?;
    if row.event_id != canonical.event_id
        || row.journal_revision != journal_revision
        || stored != *canonical
        || row.committed_at_utc_ms != canonical.committed_at_utc_ms
        || row.kind != format!("{:?}", canonical.kind)
        || row.kind.len() > MAX_JOURNAL_EVENT_KIND_BYTES
        || row.tombstoned != canonical.tombstoned
    {
        return Err(observe_projection_unavailable(
            "inner event projection differs from its canonical witness",
        ));
    }
    Ok(stored)
}

fn observe_projection_leaf_page(
    conn: &Connection,
    persona_scope: &Digest,
    journal_revision: u64,
    events: &[InnerEventV1],
) -> Result<Vec<InnerEventV1>, StoreError> {
    let Some((first, rest)) = events.split_first() else {
        return Ok(Vec::new());
    };
    let last = rest.last().unwrap_or(first);
    let mut statement = conn.prepare(
        "SELECT CASE WHEN length(event_id)=16 THEN event_id ELSE zeroblob(0) END,journal_revision,committed_at_utc_ms,CASE WHEN length(CAST(kind AS BLOB))<=?5 THEN kind END,tombstoned,CASE WHEN length(CAST(body_json AS BLOB))<=?6 THEN body_json END FROM inner_event WHERE persona_scope=?1 AND journal_revision=?2 AND event_id>=?3 AND event_id<=?4 ORDER BY event_id LIMIT ?7",
    )?;
    let mut rows = statement.query(params![
        blob(*persona_scope),
        observe_request_i64(journal_revision, "inner event revision")?,
        first.event_id.to_vec(),
        last.event_id.to_vec(),
        MAX_JOURNAL_EVENT_KIND_BYTES as i64,
        MAX_AUTONOMY_DELTA_BYTES as i64,
        events.len() as i64 + 1,
    ])?;
    let mut verified = Vec::with_capacity(events.len());
    for event in events {
        let row = rows.next()?.ok_or_else(|| {
            observe_projection_unavailable("canonical inner event projection is missing")
        })?;
        let kind: Option<String> = row.get(3)?;
        let kind = kind.ok_or_else(|| {
            observe_projection_unavailable("inner event kind exceeds its verification bound")
        })?;
        let body: Option<String> = row.get(5)?;
        let body = body.ok_or_else(|| {
            observe_projection_unavailable("inner event projection exceeds its verification bound")
        })?;
        let decoded = decode_observe_stored_event_row(
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            kind,
            row.get(4)?,
            body,
        )?;
        verified.push(verify_observe_projection_leaf(
            decoded,
            journal_revision,
            event,
        )?);
    }
    if rows.next()?.is_some() {
        return Err(observe_projection_unavailable(
            "inner event leaf page contains an unexpected projection row",
        ));
    }
    Ok(verified)
}

fn verify_observe_projection_revision_full(
    conn: &Connection,
    persona_scope: &Digest,
    journal_revision: u64,
    events: &[InnerEventV1],
) -> Result<(), StoreError> {
    let mut statement = conn.prepare(
        "SELECT CASE WHEN length(event_id)=16 THEN event_id ELSE zeroblob(0) END,journal_revision,committed_at_utc_ms,CASE WHEN length(CAST(kind AS BLOB))<=?3 THEN kind END,tombstoned,CASE WHEN length(CAST(body_json AS BLOB))<=?4 THEN body_json END FROM inner_event WHERE persona_scope=?1 AND journal_revision=?2 ORDER BY event_id LIMIT ?5",
    )?;
    let mut rows = statement.query(params![
        blob(*persona_scope),
        observe_request_i64(journal_revision, "inner event revision")?,
        MAX_JOURNAL_EVENT_KIND_BYTES as i64,
        MAX_AUTONOMY_DELTA_BYTES as i64,
        MAX_INNER_EVENTS_PER_DELTA as i64 + 1,
    ])?;
    for event in events {
        let row = rows.next()?.ok_or_else(|| {
            observe_projection_unavailable("canonical inner event projection is missing")
        })?;
        let kind: Option<String> = row.get(3)?;
        let kind = kind.ok_or_else(|| {
            observe_projection_unavailable("inner event kind exceeds its verification bound")
        })?;
        let body: Option<String> = row.get(5)?;
        let body = body.ok_or_else(|| {
            observe_projection_unavailable("inner event projection exceeds its verification bound")
        })?;
        let decoded = decode_observe_stored_event_row(
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            kind,
            row.get(4)?,
            body,
        )?;
        verify_observe_projection_leaf(decoded, journal_revision, event)?;
    }
    if rows.next()?.is_some() {
        return Err(observe_projection_unavailable(
            "inner event projection has rows outside its canonical witness",
        ));
    }
    Ok(())
}

fn build_observe_revision_witness(
    conn: &Connection,
    persona_scope: &Digest,
    canonical: &ObserveCanonicalRevision,
    manifest: (u64, Digest),
) -> Result<ObserveRevisionWitness, StoreError> {
    let bounded_delta: Option<Vec<u8>> = conn.query_row(
        "SELECT CASE WHEN length(event_bytes)<=?3 THEN event_bytes END,CASE WHEN length(delta_bytes)<=?4 THEN delta_bytes END FROM journal WHERE scope_digest=?1 AND logical_revision=?2 AND event_kind='time_advance'",
        params![
            blob(*persona_scope),
            observe_request_i64(canonical.journal_revision, "canonical revision")?,
            MAX_CANONICAL_EVENT_BYTES as i64,
            MAX_AUTONOMY_DELTA_BYTES as i64
        ],
        |row| Ok((row.get::<_, Option<Vec<u8>>>(0)?, row.get::<_, Option<Vec<u8>>>(1)?)),
    ).map(|(event, delta)| {
        event.ok_or_else(|| observe_projection_unavailable("canonical event exceeds its verification bound"))
            .and_then(|event| {
                let actual_event_len: u64 = event.len().try_into().map_err(|_| {
                    observe_projection_unavailable("canonical event length is invalid")
                })?;
                if actual_event_len != canonical.event_bytes {
                    return Err(observe_projection_unavailable("canonical event identity changed"));
                }
                let decoded = wire::decode_event(&event).map_err(|_| {
                    observe_projection_unavailable("canonical TimeAdvance event is invalid")
                })?;
                if !matches!(decoded, CanonicalEvent::TimeAdvance(_)) {
                    return Err(observe_projection_unavailable(
                        "canonical TimeAdvance kind differs from its event bytes",
                    ));
                }
                Ok(delta)
            })
    })??;
    let delta_bytes = bounded_delta.ok_or_else(|| {
        observe_projection_unavailable("canonical TimeAdvance delta exceeds its verification bound")
    })?;
    let actual_len: u64 = delta_bytes
        .len()
        .try_into()
        .map_err(|_| observe_projection_unavailable("canonical delta length is invalid"))?;
    if actual_len != canonical.delta_bytes {
        return Err(observe_projection_unavailable(
            "canonical TimeAdvance delta identity changed",
        ));
    }
    let delta = observe_bounded_autonomy_delta(&delta_bytes, "event page canonical delta")?;
    let mut events = delta.inner_events;
    events.sort_by_key(|event| event.event_id);
    if events
        .iter()
        .any(|event| event.persona_scope != *persona_scope)
    {
        return Err(observe_projection_unavailable(
            "canonical TimeAdvance contains a cross-scope event",
        ));
    }
    let event_digest = inner_event_manifest_digest(&events)
        .map_err(|_| observe_projection_unavailable("canonical inner event set is invalid"))?;
    let event_count: u64 = events
        .len()
        .try_into()
        .map_err(|_| observe_projection_unavailable("canonical event count is invalid"))?;
    if manifest != (event_count, event_digest) {
        return Err(observe_projection_unavailable(
            "inner event manifest differs from its canonical delta",
        ));
    }
    if observe_projection_count(conn, persona_scope, canonical.journal_revision)? != event_count {
        return Err(observe_projection_unavailable(
            "inner event projection count differs from its canonical delta",
        ));
    }
    verify_observe_projection_revision_full(
        conn,
        persona_scope,
        canonical.journal_revision,
        &events,
    )?;
    Ok(ObserveRevisionWitness {
        canonical: canonical.clone(),
        event_count,
        event_digest,
        events,
    })
}

fn observe_validate_non_time_revision(
    conn: &Connection,
    persona_scope: &Digest,
    canonical: &ObserveCanonicalRevision,
    event_kind: &str,
) -> Result<(), StoreError> {
    let (bounded_event, bounded_delta): (Option<Vec<u8>>, Option<Vec<u8>>) = conn.query_row(
        "SELECT CASE WHEN length(event_bytes)<=?3 THEN event_bytes END,CASE WHEN length(delta_bytes)<=?4 THEN delta_bytes END FROM journal WHERE scope_digest=?1 AND logical_revision=?2",
        params![
            blob(*persona_scope),
            observe_request_i64(canonical.journal_revision, "canonical revision")?,
            MAX_CANONICAL_EVENT_BYTES as i64,
            MAX_AUTONOMY_DELTA_BYTES as i64,
        ],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let event_bytes = bounded_event.ok_or_else(|| {
        observe_projection_unavailable("canonical event exceeds its verification bound")
    })?;
    if u64::try_from(event_bytes.len())
        .map_err(|_| observe_projection_unavailable("canonical event length is invalid"))?
        != canonical.event_bytes
    {
        return Err(observe_projection_unavailable(
            "canonical event identity changed",
        ));
    }
    if event_kind != OPERATIONAL_CHECKPOINT_KIND {
        let decoded = wire::decode_event(&event_bytes).map_err(|_| {
            observe_projection_unavailable(format!(
                "canonical event is invalid at revision {} ({event_kind})",
                canonical.journal_revision
            ))
        })?;
        if wire::event_kind_name(&decoded) != event_kind
            || matches!(decoded, CanonicalEvent::TimeAdvance(_))
        {
            return Err(observe_projection_unavailable(
                "canonical event kind differs from its event bytes",
            ));
        }
    }
    let delta_bytes = bounded_delta.ok_or_else(|| {
        observe_projection_unavailable("canonical delta exceeds its verification bound")
    })?;
    if u64::try_from(delta_bytes.len())
        .map_err(|_| observe_projection_unavailable("canonical delta length is invalid"))?
        != canonical.delta_bytes
    {
        return Err(observe_projection_unavailable(
            "canonical delta identity changed",
        ));
    }
    if !delta_bytes.is_empty() {
        let delta =
            observe_bounded_autonomy_delta(&delta_bytes, "canonical non-TimeAdvance delta")?;
        if !delta.inner_events.is_empty() {
            return Err(observe_projection_unavailable(
                "non-TimeAdvance canonical delta contains inner events",
            ));
        }
    }
    let unexpected_projection: i64 = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM inner_event_manifest WHERE persona_scope=?1 AND journal_revision=?2) OR EXISTS(SELECT 1 FROM inner_event WHERE persona_scope=?1 AND journal_revision=?2)",
        params![
            blob(*persona_scope),
            observe_request_i64(canonical.journal_revision, "canonical revision")?
        ],
        |row| row.get(0),
    )?;
    if unexpected_projection != 0 {
        return Err(observe_projection_unavailable(
            "non-TimeAdvance revision has an inner event projection",
        ));
    }
    Ok(())
}

fn observe_revision_witness(
    conn: &Connection,
    cache: &mut ObserveWitnessCache,
    persona_scope: &Digest,
    canonical: &ObserveCanonicalRevision,
) -> Result<Arc<ObserveRevisionWitness>, StoreError> {
    #[cfg(test)]
    {
        cache.witness_lookup_count = cache.witness_lookup_count.saturating_add(1);
    }
    let manifest = observe_manifest_for_revision(conn, persona_scope, canonical.journal_revision)?;
    if let Some(witness) = cache
        .revisions
        .get(&(*persona_scope, canonical.journal_revision))
    {
        if witness.canonical.chain_digest == canonical.chain_digest
            && witness.canonical.event_bytes == canonical.event_bytes
            && witness.canonical.delta_bytes == canonical.delta_bytes
            && manifest == (witness.event_count, witness.event_digest)
        {
            return Ok(Arc::clone(witness));
        }
        cache
            .revisions
            .remove(&(*persona_scope, canonical.journal_revision));
    }
    let witness = Arc::new(build_observe_revision_witness(
        conn,
        persona_scope,
        canonical,
        manifest,
    )?);
    #[cfg(test)]
    {
        cache.canonical_parse_count = cache.canonical_parse_count.saturating_add(1);
    }
    if cache.revisions.len() >= MAX_OBSERVE_WITNESS_CACHE_REVISIONS {
        if let Some(oldest) = cache.revisions.keys().next().copied() {
            cache.revisions.remove(&oldest);
        }
    }
    cache.revisions.insert(
        (*persona_scope, canonical.journal_revision),
        Arc::clone(&witness),
    );
    Ok(witness)
}

fn observe_validate_cursor_position(
    conn: &Connection,
    cache: &mut ObserveWitnessCache,
    persona_scope: &Digest,
    cursor: &ObserveCursorV1,
) -> Result<Option<Arc<ObserveRevisionWitness>>, StoreError> {
    let row: Option<(Option<String>, i64, i64, Vec<u8>)> = conn
        .query_row(
            "SELECT CASE WHEN length(CAST(event_kind AS BLOB))<=?3 THEN event_kind END,length(event_bytes),length(delta_bytes),CASE WHEN length(chain_digest)=32 THEN chain_digest ELSE zeroblob(0) END FROM journal WHERE scope_digest=?1 AND logical_revision=?2",
            params![
                blob(*persona_scope),
                observe_request_i64(cursor.journal_revision, "cursor revision")?,
                MAX_JOURNAL_EVENT_KIND_BYTES as i64,
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let (bounded_kind, raw_event_len, raw_delta_len, raw_chain) =
        row.ok_or_else(|| observe_invalid_cursor("cursor canonical revision is missing"))?;
    let event_kind = bounded_kind.ok_or_else(|| {
        observe_projection_unavailable("cursor canonical event kind exceeds its verification bound")
    })?;
    let event_bytes = observe_projection_u64(raw_event_len, "cursor canonical event byte length")?;
    let delta_bytes = observe_projection_u64(raw_delta_len, "cursor canonical delta byte length")?;
    if event_bytes > MAX_CANONICAL_EVENT_BYTES as u64
        || delta_bytes > MAX_AUTONOMY_DELTA_BYTES as u64
    {
        return Err(observe_projection_unavailable(
            "cursor canonical row exceeds its verification bound",
        ));
    }
    let canonical = ObserveCanonicalRevision {
        journal_revision: cursor.journal_revision,
        event_bytes,
        delta_bytes,
        chain_digest: raw_chain.try_into().map_err(|_| {
            observe_projection_unavailable("cursor canonical chain digest has invalid length")
        })?,
    };
    if event_kind == "time_advance" {
        let witness = observe_revision_witness(conn, cache, persona_scope, &canonical)?;
        match cursor.event_id {
            Some(event_id)
                if witness
                    .events
                    .binary_search_by_key(&event_id, |event| event.event_id)
                    .is_ok() => {}
            Some(_) => {
                return Err(observe_invalid_cursor(
                    "cursor is not a canonical event for its persona",
                ));
            }
            None if witness.events.is_empty() => {}
            None => {
                return Err(observe_invalid_cursor(
                    "completed cursor cannot skip an event-bearing TimeAdvance",
                ));
            }
        }
        Ok(Some(witness))
    } else if cursor.event_id.is_some() {
        Err(observe_invalid_cursor(
            "event cursor does not reference a canonical TimeAdvance",
        ))
    } else {
        observe_validate_non_time_revision(conn, persona_scope, &canonical, &event_kind)?;
        Ok(None)
    }
}

fn restore_operational_authority_from_checkpoints(
    tx: &Transaction<'_>,
    persona_scope: &Digest,
    checkpoints: &[(u64, OperationalCheckpointV1)],
) -> Result<(), StoreError> {
    tx.execute(
        "DELETE FROM autonomy_operational_authority WHERE persona_scope=?1",
        params![blob(*persona_scope)],
    )?;
    tx.execute(
        "DELETE FROM autonomy_operational_authority_head WHERE persona_scope=?1",
        params![blob(*persona_scope)],
    )?;
    let mut previous = [0; 32];
    for (ordinal, checkpoint) in checkpoints {
        let body = json(&checkpoint.delta)?;
        let chain = operational_chain_digest(&previous, body.as_bytes());
        tx.execute(
            "INSERT INTO autonomy_operational_authority(persona_scope,persona_ordinal,relation_scope,intention_id,event_kind,delta_json,previous_digest,chain_digest,committed_at_utc_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![blob(*persona_scope), *ordinal as i64, blob(checkpoint.delta.relation_scope), checkpoint.delta.intention_id.to_vec(), checkpoint.delta.event_kind, body, blob(previous), blob(chain), checkpoint.delta.recorded_at_utc_ms as i64],
        )?;
        previous = chain;
    }
    tx.execute(
        "INSERT INTO autonomy_operational_authority_head(persona_scope,entry_count,head_digest,ever_seen) VALUES(?1,?2,?3,1)",
        params![blob(*persona_scope), checkpoints.len() as i64, blob(previous)],
    )?;
    Ok(())
}

fn reconcile_relation_policies_from_authority(
    tx: &Transaction<'_>,
    history: &[(u64, OperationalAuthorityDeltaV1)],
) -> Result<(), StoreError> {
    let mut latest = BTreeMap::<Digest, RelationTemporalPolicyV1>::new();
    for (_, delta) in history {
        if let Some(policy) = &delta.policy {
            if policy.relation_scope != delta.relation_scope {
                return Err(StoreError::AutonomyConflict(
                    "operational policy relation mismatch".into(),
                ));
            }
            latest.insert(delta.relation_scope, policy.clone());
        }
    }
    for (relation_scope, authoritative) in latest {
        let current = tx
            .query_row(
                "SELECT body_json FROM relation_temporal_policy WHERE relation_scope=?1",
                params![blob(relation_scope)],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .and_then(|raw| parse::<RelationTemporalPolicyV1>(raw).ok());
        let mut repaired = match current {
            Some(value)
                if value.relation_scope == relation_scope
                    && value.schema_version == authoritative.schema_version
                    && value.revision >= authoritative.revision =>
            {
                value
            }
            _ => authoritative.clone(),
        };
        repaired.daily_submitted = authoritative.daily_submitted;
        repaired.consecutive_unanswered = authoritative.consecutive_unanswered;
        repaired.last_inbound_utc_ms = authoritative.last_inbound_utc_ms;
        repaired.last_proactive_submitted_utc_ms = authoritative.last_proactive_submitted_utc_ms;
        tx.execute(
            "INSERT INTO relation_temporal_policy(relation_scope,revision,body_json) VALUES(?1,?2,?3) ON CONFLICT(relation_scope) DO UPDATE SET revision=excluded.revision,body_json=excluded.body_json",
            params![blob(relation_scope), repaired.revision as i64, json(&repaired)?],
        )?;
    }
    Ok(())
}

fn state_name(value: IntentionStateV1) -> Result<String, StoreError> {
    let encoded =
        serde_json::to_string(&value).map_err(|e| StoreError::AutonomyConflict(e.to_string()))?;
    Ok(encoded.trim_matches('"').to_owned())
}

// Preserve the established storage/API shape in this compatibility boundary.
#[allow(clippy::too_many_arguments)]
fn wake_time_settlement_digest_v1(
    claim_token: &Digest,
    persona_scope: &Digest,
    event_id: &Id128,
    event_digest: &Digest,
    canonical_revision: u64,
    semantic_revision: Option<u64>,
    state_generation: u64,
    state_revision: u64,
    state_digest: &Digest,
    result_bytes: &[u8],
) -> Digest {
    let semantic_present = [u8::from(semantic_revision.is_some())];
    let semantic_revision = semantic_revision.unwrap_or(0).to_le_bytes();
    wire::domain_hash(
        b"astr-embodiment/wake-time-settlement-v1",
        &[
            claim_token,
            persona_scope,
            event_id,
            event_digest,
            &canonical_revision.to_le_bytes(),
            &semantic_present,
            &semantic_revision,
            &state_generation.to_le_bytes(),
            &state_revision.to_le_bytes(),
            state_digest,
            result_bytes,
        ],
    )
}

fn read_wake_time_settlement_v1(
    conn: &Connection,
    claim_token: &Digest,
) -> Result<Option<(AutonomousRuntimeStateV1, u64)>, StoreError> {
    type StoredWakeSettlement = (
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        i64,
        Option<i64>,
        i64,
        i64,
        Vec<u8>,
        Option<Vec<u8>>,
        i64,
        Vec<u8>,
    );
    let Some(raw): Option<StoredWakeSettlement> = conn
        .query_row(
            "SELECT persona_scope,event_id,event_digest,canonical_revision,semantic_revision,
                    state_generation,state_revision,state_digest,
                    CASE WHEN typeof(result_bytes)='blob' AND length(result_bytes)<=?2
                         THEN result_bytes END,
                    CASE WHEN typeof(result_bytes)='blob' THEN length(result_bytes) ELSE -1 END,
                    result_digest
             FROM wake_time_settlement_v1 WHERE claim_token=?1",
            params![blob(*claim_token), MAX_RUNTIME_STATE_BYTES as i64],
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
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(None);
    };
    let persona_scope: Digest = raw
        .0
        .try_into()
        .map_err(|_| StoreError::AutonomyConflict("wake settlement persona digest".into()))?;
    let event_id: Id128 = raw
        .1
        .try_into()
        .map_err(|_| StoreError::AutonomyConflict("wake settlement event id".into()))?;
    let event_digest: Digest = raw
        .2
        .try_into()
        .map_err(|_| StoreError::AutonomyConflict("wake settlement event digest".into()))?;
    let canonical_revision = autonomy_u64(raw.3, "wake settlement revision")?;
    let semantic_revision = raw
        .4
        .map(|value| autonomy_u64(value, "wake settlement semantic revision"))
        .transpose()?;
    let state_generation = autonomy_u64(raw.5, "wake settlement generation")?;
    let state_revision = autonomy_u64(raw.6, "wake settlement state revision")?;
    let stored_state_digest: Digest = raw
        .7
        .try_into()
        .map_err(|_| StoreError::AutonomyConflict("wake settlement state digest".into()))?;
    let result_bytes = bounded_typed_value(
        raw.8,
        raw.9,
        MAX_RUNTIME_STATE_BYTES as u64,
        "wake_settlement.result_bytes",
        "wake_settlement_result_type",
    )?;
    let stored_result_digest: Digest = raw
        .10
        .try_into()
        .map_err(|_| StoreError::AutonomyConflict("wake settlement result digest".into()))?;
    let state: AutonomousRuntimeStateV1 = serde_json::from_slice(&result_bytes)
        .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
    let canonical_state_bytes = serde_json::to_vec(&state)
        .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
    let derived_state_digest =
        wire::domain_hash(b"ae.autonomy.snapshot.v1", &[&canonical_state_bytes]);
    let expected_result_digest = wake_time_settlement_digest_v1(
        claim_token,
        &persona_scope,
        &event_id,
        &event_digest,
        canonical_revision,
        semantic_revision,
        state_generation,
        state_revision,
        &stored_state_digest,
        &result_bytes,
    );
    let journal_ok: i64 = conn.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM journal j JOIN applied_events a
             ON a.scope_digest=j.scope_digest AND a.event_digest=j.event_digest
            AND a.revision=j.logical_revision
           WHERE j.scope_digest=?1 AND j.logical_revision<=?2 AND j.event_digest=?3
         )",
        params![
            blob(persona_scope),
            autonomy_sql_i64(canonical_revision, "wake settlement revision")?,
            blob(event_digest),
        ],
        |row| row.get(0),
    )?;
    let semantic_ok = if let Some(revision) = semantic_revision {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM semantic_time_authority
             WHERE persona_scope=?1 AND semantic_revision=?2 AND event_digest=?3)",
            params![
                blob(persona_scope),
                autonomy_sql_i64(revision, "wake settlement semantic revision")?,
                blob(event_digest),
            ],
            |row| row.get::<_, i64>(0),
        )? == 1
    } else {
        true
    };
    if canonical_state_bytes != result_bytes
        || state.persona_scope != persona_scope
        || state.generation != state_generation
        || state.state_revision != state_revision
        || derived_state_digest != stored_state_digest
        || expected_result_digest != stored_result_digest
        || journal_ok != 1
        || !semantic_ok
    {
        return Err(StoreError::AutonomyConflict(
            "wake settlement closure failed".into(),
        ));
    }
    Ok(Some((state, canonical_revision)))
}

pub(crate) fn dispatch_claim_token(claim: &DispatchClaimV1) -> Digest {
    let target = serde_json::to_vec(&claim.target).expect("claim target is serializable");
    wire::domain_hash(
        b"ae.dispatch-claim.v2",
        &[
            &claim.preflight_digest,
            &claim.outbound_id,
            &target,
            &claim.candidate_ciphertext,
            &claim.relation_policy_revision.to_le_bytes(),
            &claim.capability_snapshot_digest,
            &claim.frozen_input_digest,
            &claim.caller_incarnation,
            &claim.lease_deadline_utc_ms.to_le_bytes(),
        ],
    )
}

pub(crate) fn externalization_claim_token(claim: &ExternalizationClaimV1) -> Digest {
    wire::domain_hash(
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
    )
}

pub(crate) fn digest_claim(kind: &[u8], id: &[u8], caller: &Digest) -> Digest {
    wire::domain_hash(b"ae.autonomy.claim.v1", &[kind, id, caller])
}

impl Store {
    // Discover autonomy scopes from canonical event bytes and repair the
    // derived lookup binding. The binding is never an authority source.

    // Read and revalidate an already committed wake result.  This is the
    // retry path after the one-shot claim has been consumed.

    // Settle a wake from the Store-minted claim alone. Receipt identity,
    // semantic graph/counts, journal base and chain seed are sampled only
    // after the Store owns the immediate write transaction.

    // Rebuild canonical state from journal deltas and operational lifecycle
    // projections from the append-only authority chain. Targets and relation
    // configuration remain independent inputs; in-flight claims are resolved
    // conservatively instead of being recreated as callable leases.

    pub fn observe_snapshot_v1(
        &self,
        request: &ObserveSnapshotRequestV1,
    ) -> Result<ObserveSnapshotV1, StoreError> {
        if request.schema_version != 1 || request.mode != ObserveModeV1::CommittedOnly {
            return Err(observe_invalid_request(
                "snapshot requires schema v1 committed_only",
            ));
        }
        let conn = self.connection()?;
        let query_only = ObserveQueryOnlyGuard::enable(conn)?;
        let tx = conn.unchecked_transaction()?;
        validate_observe_relation(&tx, &request.persona_scope, request.relation_scope.as_ref())?;
        let ObserveStateHead {
            canonical_revision,
            runtime,
            mood_event,
        } = observe_state_head(&tx, &request.persona_scope)?;
        let projection = ObserveProjectionStatusV1 {
            head_consistent: true,
        };
        let mood = mood_event.map(|event| {
            let contact_tendency = Fixed::from_raw(
                runtime
                    .affiliation_need
                    .raw()
                    .saturating_add(runtime.unfinished_topic_salience.raw())
                    / 2,
            )
            .clamp(Fixed::ZERO, Fixed::ONE);
            let mood = if runtime.affiliation_need >= Fixed::from_raw(600_000) {
                "想念"
            } else if runtime.arousal >= Fixed::from_raw(600_000) {
                "活跃"
            } else {
                "平静"
            };
            MoodCardV1 {
                mood: mood.to_owned(),
                sleep: runtime.sleep_state,
                event_id: event.event_id,
                contact_tendency,
                as_of_utc_ms: event.committed_at_utc_ms,
            }
        });
        let response = ObserveSnapshotV1 {
            schema_version: 1,
            generated_at_utc_ms: now_ms(),
            persona_scope: request.persona_scope,
            relation_scope: request.relation_scope,
            canonical_revision,
            operational_ordinal: None,
            actor_epoch: None,
            actor_sequence: None,
            projection,
            runtime,
            mood,
            pending: ObservePendingV1 {
                intentions: None,
                outbounds: None,
                active_claims: None,
            },
            budget: ObserveBudgetV1 {
                day_start_utc_ms: None,
                reserved_tokens: None,
                used_tokens: None,
            },
            latest_gate: None,
        };
        tx.commit()?;
        query_only.finish()?;
        Ok(response)
    }

    pub fn observe_events_v1(
        &self,
        request: &ObserveEventsRequestV1,
    ) -> Result<ObserveEventsV1, StoreError> {
        if request.schema_version != 1 || request.mode != ObserveModeV1::CommittedOnly {
            return Err(observe_invalid_request(
                "events requires schema v1 committed_only",
            ));
        }
        if !(1..=64).contains(&request.limit) {
            return Err(observe_invalid_request("events limit must be 1..=64"));
        }
        let conn = self.connection()?;
        let query_only = ObserveQueryOnlyGuard::enable(conn)?;
        let tx = conn.unchecked_transaction()?;
        let epoch = observe_database_epoch(&tx)?;
        let mut witness_cache = self.observe_witness_cache.borrow_mut();
        witness_cache.synchronize(epoch);
        validate_observe_relation(&tx, &request.persona_scope, request.relation_scope.as_ref())?;
        let current_revision = observe_projection_u64(
            tx.query_row(
                "SELECT COALESCE(MAX(logical_revision),0) FROM journal WHERE scope_digest=?1",
                params![blob(request.persona_scope)],
                |row| row.get::<_, i64>(0),
            )?,
            "canonical revision",
        )?;
        let projected_revision = observe_projection_u64(
            tx.query_row(
                "SELECT MAX(COALESCE((SELECT MAX(journal_revision) FROM inner_event_manifest WHERE persona_scope=?1),0),COALESCE((SELECT MAX(journal_revision) FROM inner_event WHERE persona_scope=?1),0))",
                params![blob(request.persona_scope)],
                |row| row.get::<_, i64>(0),
            )?,
            "inner event projection revision",
        )?;
        if projected_revision > current_revision {
            return Err(observe_projection_unavailable(
                "inner event projection extends past the canonical head",
            ));
        }
        let through_revision = request.through_revision.unwrap_or(current_revision);
        if through_revision > current_revision {
            return Err(observe_invalid_request(
                "through_revision exceeds the current canonical head",
            ));
        }
        let through_revision_sql = observe_request_i64(through_revision, "through_revision")?;
        let after_revision = match &request.after {
            Some(cursor) => {
                if cursor.schema_version != 1 || cursor.persona_scope != request.persona_scope {
                    return Err(observe_invalid_cursor(
                        "cursor schema or persona scope mismatch",
                    ));
                }
                if cursor.journal_revision == 0 || cursor.journal_revision > through_revision {
                    return Err(observe_invalid_cursor(
                        "cursor position is outside its requested watermark",
                    ));
                }
                cursor.journal_revision
            }
            None => 0,
        };
        let mut cursor_witness = if let Some(cursor) = &request.after {
            observe_validate_cursor_position(
                &tx,
                &mut witness_cache,
                &request.persona_scope,
                cursor,
            )?
        } else {
            None
        };
        let first_revision = match request.after.as_ref().and_then(|cursor| cursor.event_id) {
            Some(_) => Some(after_revision),
            None if request.after.is_some() => after_revision.checked_add(1),
            None => Some(1),
        };
        let raw_canonical = if first_revision.is_some_and(|revision| revision <= through_revision) {
            let mut statement = tx.prepare(
                "SELECT logical_revision,CASE WHEN length(CAST(event_kind AS BLOB))<=?4 THEN event_kind END,length(event_bytes),length(delta_bytes),CASE WHEN length(chain_digest)=32 THEN chain_digest ELSE zeroblob(0) END FROM journal WHERE scope_digest=?1 AND logical_revision>=?2 AND logical_revision<=?3 ORDER BY logical_revision LIMIT ?5",
            )?;
            let rows = statement
                .query_map(
                    params![
                        blob(request.persona_scope),
                        observe_request_i64(first_revision.unwrap(), "first cursor revision")?,
                        through_revision_sql,
                        MAX_JOURNAL_EVENT_KIND_BYTES as i64,
                        MAX_OBSERVE_MANIFESTS_PER_PAGE as i64,
                    ],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, Vec<u8>>(4)?,
                        ))
                    },
                )?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        } else {
            Vec::new()
        };
        let canonical_batch_exhausted = raw_canonical.len() < MAX_OBSERVE_MANIFESTS_PER_PAGE;
        let mut page = Vec::with_capacity(usize::from(request.limit));
        let mut next = request.after.clone();
        let mut page_full = false;
        let mut expected_revision = first_revision;
        'revisions: for (raw_revision, bounded_kind, raw_event_len, raw_delta_len, raw_chain) in
            raw_canonical
        {
            let journal_revision =
                observe_projection_u64(raw_revision, "canonical journal revision")?;
            if expected_revision != Some(journal_revision) {
                return Err(observe_projection_unavailable(
                    "canonical journal scan contains a revision gap",
                ));
            }
            expected_revision = journal_revision.checked_add(1);
            let event_kind = bounded_kind.ok_or_else(|| {
                observe_projection_unavailable(
                    "canonical event kind exceeds its verification bound",
                )
            })?;
            let event_bytes = observe_projection_u64(raw_event_len, "canonical event byte length")?;
            let delta_bytes = observe_projection_u64(raw_delta_len, "canonical delta byte length")?;
            if event_bytes > MAX_CANONICAL_EVENT_BYTES as u64
                || delta_bytes > MAX_AUTONOMY_DELTA_BYTES as u64
            {
                return Err(observe_projection_unavailable(
                    "canonical journal row exceeds its verification bound",
                ));
            }
            let chain_digest: Digest = raw_chain.try_into().map_err(|_| {
                observe_projection_unavailable("canonical chain digest has invalid length")
            })?;
            let canonical = ObserveCanonicalRevision {
                journal_revision,
                event_bytes,
                delta_bytes,
                chain_digest,
            };
            if event_kind != "time_advance" {
                observe_validate_non_time_revision(
                    &tx,
                    &request.persona_scope,
                    &canonical,
                    &event_kind,
                )?;
                if page.is_empty() {
                    next = Some(ObserveCursorV1 {
                        schema_version: 1,
                        persona_scope: request.persona_scope,
                        journal_revision,
                        event_id: None,
                    });
                }
                continue;
            }
            let witness = match cursor_witness.take() {
                Some(witness)
                    if witness.canonical.journal_revision == canonical.journal_revision
                        && witness.canonical.chain_digest == canonical.chain_digest
                        && witness.canonical.event_bytes == canonical.event_bytes
                        && witness.canonical.delta_bytes == canonical.delta_bytes =>
                {
                    witness
                }
                _ => observe_revision_witness(
                    &tx,
                    &mut witness_cache,
                    &request.persona_scope,
                    &canonical,
                )?,
            };
            let start_index = if request.after.as_ref().is_some_and(|cursor| {
                cursor.journal_revision == journal_revision && cursor.event_id.is_some()
            }) {
                let cursor_id = request
                    .after
                    .as_ref()
                    .and_then(|cursor| cursor.event_id)
                    .unwrap();
                witness
                    .events
                    .binary_search_by_key(&cursor_id, |event| event.event_id)
                    .map_err(|_| {
                        observe_invalid_cursor("cursor is not a canonical event for its persona")
                    })?
                    + 1
            } else {
                0
            };
            let page_len_before_revision = page.len();
            let remaining = usize::from(request.limit) - page.len();
            let end_index = start_index
                .saturating_add(remaining)
                .min(witness.events.len());
            for event in observe_projection_leaf_page(
                &tx,
                &request.persona_scope,
                journal_revision,
                &witness.events[start_index..end_index],
            )? {
                let event_id = event.event_id;
                page.push(event);
                next = Some(ObserveCursorV1 {
                    schema_version: 1,
                    persona_scope: request.persona_scope,
                    journal_revision,
                    event_id: Some(event_id),
                });
                if page.len() == usize::from(request.limit) {
                    page_full = true;
                    break 'revisions;
                }
            }
            if page.is_empty()
                && page.len() == page_len_before_revision
                && witness.events.is_empty()
            {
                next = Some(ObserveCursorV1 {
                    schema_version: 1,
                    persona_scope: request.persona_scope,
                    journal_revision,
                    event_id: None,
                });
            }
        }
        if !page_full
            && canonical_batch_exhausted
            && expected_revision.is_some_and(|revision| revision <= through_revision)
        {
            return Err(observe_projection_unavailable(
                "canonical journal scan ended before its watermark",
            ));
        }
        if !page_full
            && canonical_batch_exhausted
            && through_revision > 0
            && next
                .as_ref()
                .is_some_and(|cursor| cursor.event_id.is_none())
        {
            next = Some(ObserveCursorV1 {
                schema_version: 1,
                persona_scope: request.persona_scope,
                journal_revision: through_revision,
                event_id: None,
            });
        }
        let items = page
            .into_iter()
            .filter(|event| !event.tombstoned)
            .map(observe_event_projection)
            .collect();
        let response = ObserveEventsV1 {
            schema_version: 1,
            items,
            next,
            high_water: ObserveHighWaterV1 {
                through_revision,
                operational_ordinal: None,
            },
        };
        drop(witness_cache);
        tx.commit()?;
        query_only.finish()?;
        Ok(response)
    }

    pub fn query_inner_events(
        &self,
        query: &InnerEventQueryV1,
    ) -> Result<InnerEventPageV1, StoreError> {
        if !(1..=100).contains(&query.limit) {
            return Err(StoreError::AutonomyConflict("limit must be 1..=100".into()));
        }
        let conn = self.connection()?;
        let after: Option<(i64, Vec<u8>)> = query
            .after_event_id
            .map(|id| {
                conn.query_row(
                    "SELECT committed_at_utc_ms,event_id FROM inner_event WHERE event_id=?1",
                    params![id.to_vec()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .transpose()?;
        let (after_ms, after_id) = after.unwrap_or((-1, vec![]));
        let mut stmt=conn.prepare("SELECT body_json FROM inner_event WHERE persona_scope=?1 AND tombstoned=0 AND (committed_at_utc_ms>?2 OR (committed_at_utc_ms=?2 AND event_id>?3)) ORDER BY committed_at_utc_ms,event_id LIMIT ?4")?;
        let items = stmt
            .query_map(
                params![
                    blob(query.persona_scope),
                    after_ms,
                    after_id,
                    query.limit as i64
                ],
                |r| r.get::<_, String>(0),
            )?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(parse)
            .collect::<Result<Vec<InnerEventV1>, _>>()?;
        let next_after_event_id = if items.len() == query.limit as usize {
            items.last().map(|i| i.event_id)
        } else {
            None
        };
        Ok(InnerEventPageV1 {
            items,
            next_after_event_id,
        })
    }

    pub fn latest_inner_event(
        &self,
        persona_scope: &Digest,
    ) -> Result<Option<InnerEventV1>, StoreError> {
        self.connection()?.query_row(
            "SELECT body_json FROM inner_event WHERE persona_scope=?1 AND tombstoned=0 ORDER BY committed_at_utc_ms DESC,event_id DESC LIMIT 1",
            params![blob(*persona_scope)],
            |row| row.get::<_, String>(0),
        ).optional()?.map(parse).transpose()
    }
}
