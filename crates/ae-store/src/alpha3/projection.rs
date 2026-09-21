use crate::semantic::AttestedAffectProjectionInputV1;
use crate::{blob, Store, StoreError};
use ae_contracts::*;
use ae_semantic_core::potential_region_projection_v1;
use rusqlite::{params, Connection, OptionalExtension};

const MAX_ALPHA3_BODY_BYTES: usize = 256 * 1024;
// Retained historical verification data; no active executor is restored.
#[allow(dead_code)]
const CLAIM_LEASE_MS: u64 = 120_000;
// Retained historical verification data; no active executor is restored.
#[allow(dead_code)]
const MAX_ALPHA3_OBSERVER_SCAN_ROWS: usize = 128;

fn conflict(code: &str, message: impl AsRef<str>) -> StoreError {
    StoreError::AutonomyConflict(format!("ALPHA3_{code}::{}", message.as_ref()))
}

fn parse<T: serde::de::DeserializeOwned>(body: String) -> Result<T, StoreError> {
    if body.len() > MAX_ALPHA3_BODY_BYTES {
        return Err(conflict(
            "PROJECTION_INCOMPLETE",
            "alpha3 authority body exceeds 256 KiB",
        ));
    }
    serde_json::from_str(&body)
        .map_err(|error| conflict("PROJECTION_INCOMPLETE", error.to_string()))
}

fn rust_u64(value: i64, field: &str) -> Result<u64, StoreError> {
    value
        .try_into()
        .map_err(|_| conflict("PROJECTION_INCOMPLETE", format!("negative {field}")))
}

fn state_name<T: serde::Serialize>(value: T) -> Result<String, StoreError> {
    let encoded = serde_json::to_string(&value)
        .map_err(|error| conflict("SCHEMA_UNSUPPORTED", error.to_string()))?;
    Ok(encoded.trim_matches('"').to_owned())
}

// Retained historical verification data; no active executor is restored.
#[allow(dead_code)]
struct GateAuthority {
    relation_scope: Digest,
    intention: DurableIntentionV1,
    intention_revision: u64,
    basis: ContactIntentionBasisV1,
    consent: RelationConsentV1,
    contact: RelationContactProcessV1,
    temporal_policy: RelationTemporalPolicyV1,
    budget_policy: RelationBudgetPolicyV1,
    ledger: RelationBudgetLedgerV1,
    target: Option<OutboundTargetEnvelopeV1>,
    state: AutonomousRuntimeStateV1,
    unsettled_usage: bool,
}

#[derive(Clone, Copy, serde::Serialize)]
// Retained historical verification data; no active executor is restored.
#[allow(dead_code)]
struct GateAuthorityDeadlinesV1 {
    evaluated_at_utc_ms: u64,
    projection_ttl_expires_at_utc_ms: u64,
    intention_expires_at_utc_ms: u64,
    basis_expires_at_utc_ms: u64,
    consent_expires_at_utc_ms: Option<u64>,
    budget_expires_at_utc_ms: u64,
    retry_at_utc_ms: Option<u64>,
    authority_expires_at_utc_ms: u64,
}

#[derive(serde::Serialize)]
// Retained historical verification data; no active executor is restored.
#[allow(dead_code)]
struct GateAuthoritySnapshotV1 {
    schema_version: u16,
    phase: GatePhaseV1,
    relation_scope: Digest,
    intention_id: Id128,
    intention_revision: u64,
    intention_state: IntentionStateV1,
    intention_not_before_utc_ms: u64,
    intention_expires_at_utc_ms: u64,
    deadlines: GateAuthorityDeadlinesV1,
    basis_commitment: Digest,
    consent_commitment: Digest,
    contact_commitment: Digest,
    temporal_policy_commitment: Digest,
    budget_policy_commitment: Digest,
    budget_ledger_commitment: Digest,
    target_binding_digest: Option<Digest>,
    target_generation: Option<u64>,
    runtime_generation: u64,
    runtime_state_revision: u64,
    runtime_sleep_state: SleepStateV1,
    readiness_commitment: Digest,
    capability_snapshot_digest: Digest,
    outbound_id: Option<Id128>,
    outbound_state: Option<IntentionStateV1>,
    outbound_target_digest: Option<Digest>,
    active_claim_count: u64,
}

impl Store {}

#[cfg(test)]
mod bounded_observer_scan_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn scoped_id(tag: u8, value: u64) -> Id128 {
        let mut id = [tag; 16];
        id[8..].copy_from_slice(&value.to_be_bytes());
        id
    }

    fn scoped_digest(tag: u8, value: u64) -> Digest {
        let mut digest = [tag; 32];
        digest[24..].copy_from_slice(&value.to_be_bytes());
        digest
    }

    fn measured_public_claims(
        conn: &Connection,
        persona_scope: &Digest,
        relation_scope: &Digest,
    ) -> ((Vec<PublicClaimV2>, bool), usize) {
        let counter = Arc::new(AtomicUsize::new(0));
        let progress_counter = Arc::clone(&counter);
        conn.progress_handler(
            1,
            Some(move || {
                progress_counter.fetch_add(1, Ordering::Relaxed);
                false
            }),
        );
        let result = public_claims(conn, persona_scope, relation_scope);
        conn.progress_handler(0, None::<fn() -> bool>);
        (result.unwrap(), counter.load(Ordering::Relaxed))
    }

    fn insert_mismatched_claim_candidates(
        conn: &Connection,
        start: usize,
        end: usize,
        wrong_persona_scope: &Digest,
        relation_scope: &Digest,
    ) {
        if start == end {
            return;
        }
        conn.execute(
            "WITH RECURSIVE seq(value) AS (
                 SELECT ?1 WHERE ?1 < ?2
                 UNION ALL
                 SELECT value+1 FROM seq WHERE value+1 < ?2
             )
             INSERT INTO durable_intention(intention_id,persona_scope,relation_scope)
             SELECT CAST(printf('%016x',value) AS BLOB),?3,?4 FROM seq",
            params![
                start as i64,
                end as i64,
                blob(*wrong_persona_scope),
                blob(*relation_scope),
            ],
        )
        .unwrap();
        conn.execute(
            "WITH RECURSIVE seq(value) AS (
                 SELECT ?1 WHERE ?1 < ?2
                 UNION ALL
                 SELECT value+1 FROM seq WHERE value+1 < ?2
             )
             INSERT INTO contact_intention_basis(
                 intention_id,relation_scope,cause_digest,live
             )
             SELECT CAST(printf('%016x',value) AS BLOB),?3,
                    CAST(printf('%032x',value) AS BLOB),1
             FROM seq",
            params![start as i64, end as i64, blob(*relation_scope)],
        )
        .unwrap();
    }

    #[test]
    fn observer_windows_are_indexed_bounded_and_fail_closed() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE inner_event(
                 event_id BLOB PRIMARY KEY, persona_scope BLOB NOT NULL,
                 journal_revision INTEGER NOT NULL, committed_at_utc_ms INTEGER NOT NULL,
                 kind TEXT NOT NULL, tombstoned INTEGER NOT NULL, body_json TEXT NOT NULL
             );
             CREATE INDEX inner_event_scope_order
                 ON inner_event(persona_scope,committed_at_utc_ms,event_id);
             CREATE TABLE durable_intention(
                 intention_id BLOB PRIMARY KEY, persona_scope BLOB NOT NULL,
                 relation_scope BLOB NOT NULL
             );
             CREATE TABLE contact_intention_basis(
                 intention_id BLOB PRIMARY KEY, relation_scope BLOB NOT NULL,
                 cause_digest BLOB NOT NULL, live INTEGER NOT NULL
             );
             CREATE UNIQUE INDEX contact_intention_live_cause
                 ON contact_intention_basis(relation_scope,cause_digest) WHERE live=1;
             CREATE TABLE outbound_attempt(
                 outbound_id BLOB PRIMARY KEY, intention_id BLOB NOT NULL UNIQUE
             );
             CREATE TABLE autonomy_claim(
                 claim_token BLOB PRIMARY KEY, claim_kind TEXT NOT NULL,
                 record_id BLOB NOT NULL, lease_deadline_utc_ms INTEGER NOT NULL,
                 UNIQUE(claim_kind,record_id)
             );",
        )
        .unwrap();
        let persona_scope = [0x11; 32];
        let relation_scope = [0x12; 32];
        for value in 0..=MAX_ALPHA3_OBSERVER_SCAN_ROWS {
            let event = InnerEventV1 {
                schema_version: AUTONOMY_SCHEMA_VERSION,
                event_id: scoped_id(0x21, value as u64),
                persona_scope,
                kind: InnerEventKindV1::HomeostasisChanged,
                committed_at_utc_ms: 1_700_000_000_000 + value as u64,
                summary_code: "bounded_observer_probe".into(),
                value_before: None,
                value_after: None,
                source_event_ids: Vec::new(),
                tombstoned: true,
            };
            conn.execute(
                "INSERT INTO inner_event(
                     event_id,persona_scope,journal_revision,committed_at_utc_ms,
                     kind,tombstoned,body_json
                 ) VALUES(?1,?2,?3,?4,'homeostasis_changed',1,?5)",
                params![
                    blob(event.event_id),
                    blob(persona_scope),
                    value as i64 + 1,
                    event.committed_at_utc_ms as i64,
                    serde_json::to_string(&event).unwrap(),
                ],
            )
            .unwrap();

            if value == 0 {
                let intention_id = scoped_id(0x31, value as u64);
                conn.execute(
                    "INSERT INTO durable_intention(intention_id,persona_scope,relation_scope)
                     VALUES(?1,?2,?3)",
                    params![
                        blob(intention_id),
                        blob(persona_scope),
                        blob(relation_scope)
                    ],
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO contact_intention_basis(
                         intention_id,relation_scope,cause_digest,live
                     ) VALUES(?1,?2,?3,1)",
                    params![
                        blob(intention_id),
                        blob(relation_scope),
                        blob(scoped_digest(0x32, value as u64)),
                    ],
                )
                .unwrap();
            }
        }

        assert_eq!(
            notable_nodes(&conn, &persona_scope, None).unwrap(),
            ProjectionFieldV1::Inconsistent
        );
        let wrong_persona_scope = [0x99; 32];
        let mut sqlite_steps = Vec::new();
        let (claims, steps) = measured_public_claims(&conn, &persona_scope, &relation_scope);
        assert_eq!(claims, (Vec::new(), false));
        sqlite_steps.push((0_usize, steps));
        let mut inserted = 0_usize;
        for candidate_count in [1_000_usize, 10_000, 50_000] {
            insert_mismatched_claim_candidates(
                &conn,
                inserted,
                candidate_count,
                &wrong_persona_scope,
                &relation_scope,
            );
            inserted = candidate_count;
            let (claims, steps) = measured_public_claims(&conn, &persona_scope, &relation_scope);
            assert_eq!(claims, (Vec::new(), true));
            sqlite_steps.push((candidate_count, steps));
        }
        let maximum_steps = sqlite_steps.iter().map(|(_, steps)| *steps).max().unwrap();
        assert!(
            maximum_steps < 10_000,
            "claim candidate window VM steps grew with skipped candidates: {sqlite_steps:?}"
        );
        let nonzero_steps = sqlite_steps
            .iter()
            .filter_map(|(candidates, steps)| (*candidates != 0).then_some(*steps))
            .collect::<Vec<_>>();
        assert!(
            nonzero_steps.iter().max().unwrap() - nonzero_steps.iter().min().unwrap() <= 512,
            "claim candidate window changed with table cardinality: {sqlite_steps:?}"
        );

        let mismatched_relation = [0x13; 32];
        let mismatched_id = scoped_id(0x41, 1);
        conn.execute(
            "INSERT INTO durable_intention(intention_id,persona_scope,relation_scope)
             VALUES(?1,?2,?3)",
            params![
                blob(mismatched_id),
                blob(wrong_persona_scope),
                blob(mismatched_relation)
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO contact_intention_basis(intention_id,relation_scope,cause_digest,live)
             VALUES(?1,?2,?3,1)",
            params![
                blob(mismatched_id),
                blob(mismatched_relation),
                blob(scoped_digest(0x42, 1)),
            ],
        )
        .unwrap();
        assert_eq!(
            public_claims(&conn, &persona_scope, &mismatched_relation).unwrap(),
            (Vec::new(), true)
        );

        let corrupt_relation = [0x14; 32];
        conn.execute(
            "INSERT INTO contact_intention_basis(intention_id,relation_scope,cause_digest,live)
             VALUES(zeroblob(1048577),?1,?2,1)",
            params![blob(corrupt_relation), blob(scoped_digest(0x43, 1))],
        )
        .unwrap();
        assert_eq!(
            public_claims(&conn, &persona_scope, &corrupt_relation).unwrap(),
            (Vec::new(), true)
        );

        let notable_plan = conn
            .prepare(
                "EXPLAIN QUERY PLAN
                 SELECT event_id,committed_at_utc_ms,body_json,tombstoned
                 FROM inner_event INDEXED BY inner_event_scope_order
                 WHERE persona_scope=?1
                 ORDER BY committed_at_utc_ms DESC,event_id DESC LIMIT ?2",
            )
            .unwrap()
            .query_map(
                params![
                    blob(persona_scope),
                    (MAX_ALPHA3_OBSERVER_SCAN_ROWS + 1) as i64
                ],
                |row| row.get::<_, String>(3),
            )
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .join("\n")
            .to_ascii_uppercase();
        assert!(notable_plan.contains("INNER_EVENT_SCOPE_ORDER"));
        assert!(!notable_plan.contains("TEMP B-TREE"));

        let claims_plan = conn
            .prepare(
                "EXPLAIN QUERY PLAN
                 SELECT b.intention_id
                 FROM contact_intention_basis AS b INDEXED BY contact_intention_live_cause
                 WHERE b.relation_scope=?1 AND b.live=1
                 ORDER BY b.cause_digest LIMIT ?2",
            )
            .unwrap()
            .query_map(
                params![
                    blob(relation_scope),
                    (MAX_ALPHA3_OBSERVER_SCAN_ROWS + 1) as i64
                ],
                |row| row.get::<_, String>(3),
            )
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .join("\n")
            .to_ascii_uppercase();
        assert!(claims_plan.contains("CONTACT_INTENTION_LIVE_CAUSE"));
        assert!(!claims_plan.contains("DURABLE_INTENTION"));
        assert!(!claims_plan.contains("TEMP B-TREE"));
        assert!(!claims_plan.contains("MULTI-INDEX OR"));
    }
}

// Retained historical verification data; no active executor is restored.
#[allow(dead_code)]
const BODY_PROJECTION_TTL_MS: u64 = 60_000;
// Retained historical verification data; no active executor is restored.
#[allow(dead_code)]
const BUDGET_DAY_MS: u64 = 86_400_000;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
// Retained historical verification data; no active executor is restored.
#[allow(dead_code)]
struct BodyRuntimeProjectionRow {
    schema_version: u16,
    persona_scope: String,
    generation: u64,
    state_revision: u64,
    next_wake_at_utc_ms: u64,
    sleep_state: SleepStateV1,
    arousal: ae_fixed::Fixed,
    social_energy: ae_fixed::Fixed,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
// Retained historical verification data; no active executor is restored.
#[allow(dead_code)]
struct IntentionProjectionBody {
    schema_version: u16,
    intention_id: String,
    persona_scope: String,
    relation_scope: String,
    state: IntentionStateV1,
    not_before_utc_ms: u64,
    expires_at_utc_ms: u64,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
// Retained historical verification data; no active executor is restored.
#[allow(dead_code)]
struct OutboundProjectionBody {
    outbound_id: String,
    intention_id: String,
    state: IntentionStateV1,
    target_binding_digest: String,
    created_at_utc_ms: u64,
    settled_at_utc_ms: Option<u64>,
}

#[derive(Clone, Copy)]
// Retained historical verification data; no active executor is restored.
#[allow(dead_code)]
struct IntentionProjectionRow {
    intention_id: Id128,
    persona_scope: Digest,
    relation_scope: Digest,
    state: IntentionStateV1,
    revision: u64,
    not_before_utc_ms: u64,
    expires_at_utc_ms: u64,
}

#[derive(Clone, Copy)]
// Retained historical verification data; no active executor is restored.
#[allow(dead_code)]
struct OutboundProjectionRow {
    outbound_id: Id128,
    intention_id: Id128,
    state: IntentionStateV1,
    target_binding_digest: Digest,
    created_at_utc_ms: u64,
    settled_at_utc_ms: Option<u64>,
}

impl Store {}

#[allow(dead_code)]
fn load_world_anchor(
    conn: &Connection,
    persona_scope: &Digest,
) -> Result<Option<WorldAnchorV1>, StoreError> {
    let row = conn
        .query_row(
            "SELECT revision,body_json FROM world_anchor WHERE persona_scope=?1",
            params![blob(*persona_scope)],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((raw_revision, body)) = row else {
        return Ok(None);
    };
    let anchor: WorldAnchorV1 = parse(body)?;
    if anchor.persona_scope != *persona_scope
        || anchor.revision != rust_u64(raw_revision, "world anchor revision")?
    {
        return Err(conflict(
            "PROJECTION_INCOMPLETE",
            "world anchor columns differ from its body",
        ));
    }
    anchor
        .validate()
        .map_err(|error| conflict("PROJECTION_INCOMPLETE", error.to_string()))?;
    Ok(Some(anchor))
}

#[allow(dead_code)]
fn load_lived_day(
    conn: &Connection,
    persona_scope: &Digest,
) -> Result<Option<LivedDayStateV1>, StoreError> {
    let row = conn
        .query_row(
            "SELECT persona_day_ordinal,revision,body_json
             FROM lived_day_state WHERE persona_scope=?1",
            params![blob(*persona_scope)],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((raw_day, raw_revision, body)) = row else {
        return Ok(None);
    };
    let day: LivedDayStateV1 = parse(body)?;
    if day.persona_scope != *persona_scope
        || i64::from(day.persona_day_ordinal) != raw_day
        || day.revision != rust_u64(raw_revision, "lived day revision")?
    {
        return Err(conflict(
            "PROJECTION_INCOMPLETE",
            "lived day columns differ from its body",
        ));
    }
    day.validate()
        .map_err(|error| conflict("PROJECTION_INCOMPLETE", error.to_string()))?;
    Ok(Some(day))
}

#[derive(Clone)]
pub(crate) struct AffectObserverFieldsV1 {
    affect: ProjectionFieldV1<AffectProjectionV1>,
    // Retained historical verification data; no active executor is restored.
    #[allow(dead_code)]
    semantic_health: ProjectionFieldV1<SemanticProjectionHealthV1>,
}

pub(crate) fn build_affect_observer_fields_v1(
    input: AttestedAffectProjectionInputV1,
) -> Result<AffectObserverFieldsV1, StoreError> {
    let regions = potential_region_projection_v1(&input.previous_field, &input.current_field)
        .map_err(|_| conflict("PROJECTION_INCOMPLETE", "matrix region reduction failed"))?;
    let pyramid =
        ae_renorm::restrict(&input.current_field, &input.formula_digest).map_err(|_| {
            conflict(
                "PROJECTION_INCOMPLETE",
                "matrix renormalization unavailable",
            )
        })?;
    let observer_residual = pyramid.consistency_residual.raw();
    if !(0..=AFFECT_FXP6_SCALE_V1).contains(&observer_residual)
        || !(0..=AFFECT_FXP6_SCALE_V1)
            .contains(&input.semantic_dynamics_renormalization_residual_fxp6)
    {
        return Err(conflict(
            "PROJECTION_INCOMPLETE",
            "matrix renormalization residual is unavailable",
        ));
    }
    let delta_sum = regions
        .delta_mean_fxp6
        .iter()
        .try_fold(0_i128, |sum, value| sum.checked_add(i128::from(*value)))
        .ok_or_else(|| conflict("PROJECTION_INCOMPLETE", "matrix trend overflow"))?;
    let affect = AffectProjectionV1 {
        semantic_revision: input.semantic_revision,
        personality_revision: input.personality_revision,
        state_digest: input.state_digest,
        formula_digest: input.formula_digest,
        confidence_fxp6: input.confidence_fxp6,
        region_mean_fxp6: regions.current_mean_fxp6,
        region_delta_fxp6: regions.delta_mean_fxp6,
        trend: match delta_sum.cmp(&0) {
            std::cmp::Ordering::Less => AffectTrendV1::Falling,
            std::cmp::Ordering::Equal => AffectTrendV1::Stable,
            std::cmp::Ordering::Greater => AffectTrendV1::Rising,
        },
    };
    let semantic_health = SemanticProjectionHealthV1 {
        semantic_revision: input.semantic_revision,
        active_node_count: input.current_field.active_node_count(),
        active_edge_count: u32::try_from(input.graph.edges.len())
            .map_err(|_| conflict("PROJECTION_INCOMPLETE", "semantic edge count overflow"))?,
        graph_digest: input.graph_digest,
        renorm_mapping_digest: pyramid.mapping_digest,
        semantic_dynamics_renormalization_residual_fxp6: input
            .semantic_dynamics_renormalization_residual_fxp6,
        observer_pyramid_consistency_residual_fxp6: observer_residual,
    };
    affect
        .validate()
        .map_err(|error| conflict("PROJECTION_INCOMPLETE", error.to_string()))?;
    semantic_health
        .validate()
        .map_err(|error| conflict("PROJECTION_INCOMPLETE", error.to_string()))?;
    Ok(AffectObserverFieldsV1 {
        affect: ProjectionFieldV1::Available(affect),
        semantic_health: ProjectionFieldV1::Available(semantic_health),
    })
}

pub(crate) fn build_reply_affect_v1(
    input: AttestedAffectProjectionInputV1,
) -> Result<ae_contracts::ReplyAffectV1, StoreError> {
    let fields = build_affect_observer_fields_v1(input)?;
    let ProjectionFieldV1::Available(affect) = fields.affect else {
        return Err(conflict(
            "PROJECTION_INCOMPLETE",
            "reply affect is unavailable",
        ));
    };
    let reply = ae_contracts::ReplyAffectV1 {
        semantic_revision: affect.semantic_revision,
        personality_revision: affect.personality_revision,
        confidence_fxp6: affect.confidence_fxp6,
        region_mean_fxp6: affect.region_mean_fxp6,
        region_delta_fxp6: affect.region_delta_fxp6,
        trend: affect.trend,
    };
    if !reply.validate_v1() {
        return Err(conflict(
            "PROJECTION_INCOMPLETE",
            "reply affect failed its closed contract",
        ));
    }
    Ok(reply)
}

#[allow(dead_code)]
fn sanitized_dream_ref(persona_scope: &Digest, domain: &[u8], value: &Id128) -> Id128 {
    wire::domain_hash(domain, &[persona_scope, value])[..16]
        .try_into()
        .expect("digest prefix")
}

#[allow(dead_code)]
fn private_dreams(
    conn: &Connection,
    persona_scope: &Digest,
) -> Result<Vec<DreamResidueV1>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT residue_id,revision,state,body_json FROM dream_residue
         WHERE persona_scope=?1 ORDER BY revision,residue_id LIMIT ?2",
    )?;
    let rows = statement
        .query_map(
            params![blob(*persona_scope), MAX_ALPHA3_PUBLIC_RECORDS as i64],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let mut dreams = Vec::with_capacity(rows.len());
    for (raw_residue_id, raw_revision, stored_state, body) in rows {
        let residue_id: Id128 = raw_residue_id.try_into().map_err(|_| {
            conflict(
                "PROJECTION_INCOMPLETE",
                "dream residue identifier is invalid",
            )
        })?;
        let mut dream: DreamResidueV1 = parse(body)?;
        if dream.residue_id != residue_id
            || dream.persona_scope != *persona_scope
            || stored_state != state_name(dream.state)?
            || rust_u64(raw_revision, "dream revision")? == 0
            || !dream.non_fact
        {
            return Err(conflict(
                "PROJECTION_INCOMPLETE",
                "dream residue columns differ from non-fact authority",
            ));
        }
        dream
            .validate()
            .map_err(|error| conflict("PROJECTION_INCOMPLETE", error.to_string()))?;
        dream.residue_id = sanitized_dream_ref(
            persona_scope,
            b"ae.alpha3.private-dream-ref.v2",
            &dream.residue_id,
        );
        dream.source_event_ids = dream
            .source_event_ids
            .iter()
            .map(|event_id| {
                sanitized_dream_ref(
                    persona_scope,
                    b"ae.alpha3.private-dream-source-ref.v2",
                    event_id,
                )
            })
            .collect();
        dream.review_event_id = dream.review_event_id.map(|event_id| {
            sanitized_dream_ref(
                persona_scope,
                b"ae.alpha3.private-dream-review-ref.v2",
                &event_id,
            )
        });
        dreams.push(dream);
    }
    Ok(dreams)
}

impl Store {}

fn legacy_dispatch_claim_token(claim: &DispatchClaimV2, caller_incarnation: &Digest) -> Digest {
    wire::domain_hash(
        b"ae.alpha3.dispatch-claim.v2",
        &[
            &claim.outbound_public_ref,
            &claim.consent_epoch.to_le_bytes(),
            &claim.consent_revision.to_le_bytes(),
            &claim.policy_revision.to_le_bytes(),
            &claim.target_binding_digest,
            &claim.capability_snapshot_digest,
            &claim.lease_deadline_utc_ms.to_le_bytes(),
            caller_incarnation,
        ],
    )
}

pub(crate) fn dispatch_claim_token(claim: &DispatchClaimV2, caller_incarnation: &Digest) -> Digest {
    let Some(decision) = claim.gate_decision_snapshot.as_ref() else {
        return legacy_dispatch_claim_token(claim, caller_incarnation);
    };
    let canonical_decision = serde_json::to_vec(decision)
        .expect("closed dispatch gate decision JSON serializes canonically");
    let decision_commitment = wire::domain_hash(
        b"ae.alpha3.dispatch-claim-decision.v1",
        &[&canonical_decision],
    );
    wire::domain_hash(
        b"ae.alpha3.dispatch-claim.v3",
        &[
            &claim.outbound_public_ref,
            &claim.consent_epoch.to_le_bytes(),
            &claim.consent_revision.to_le_bytes(),
            &claim.policy_revision.to_le_bytes(),
            &claim.target_binding_digest,
            &claim.capability_snapshot_digest,
            &claim.lease_deadline_utc_ms.to_le_bytes(),
            &decision_commitment,
            caller_incarnation,
        ],
    )
}

impl Store {}
