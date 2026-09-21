#![allow(dead_code)]

use crate::{Store, StoreError};
use ae_continuum::JournalRow;
use ae_contracts::{
    wire, AdminAction, AutonomousRuntimeStateV1, AutonomyJournalDeltaV1, CanonicalEvent,
    CommitStatus, Digest, DreamResidueStateV1, DreamResidueV1, DreamReviewActionV1,
    DreamReviewRequestV1, EcosystemCapabilityGrantV1, EcosystemProposalV1, InnerEventKindV1,
    InnerEventV1, InteractionFactV1, InteractionSourceAuthorityV1, LivedDayStateV1,
    LivedNodeImportanceV1, LivedSegmentStateV1, ScopeRef, SleepStateV1, TimeAdvanceV1,
    TransitionReceipt, WorldAnchorV1, WorldLayerV1,
};
use ae_fixed::Fixed;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

const MAX_BODY_BYTES: usize = 256 * 1024;
const MAX_CONTEXT_BYTES: usize = 4_096;
const MAX_DELTA_BYTES: usize = 1024 * 1024;
const DREAM_REVIEW_REJECT_OPERATION: &str = "lived_dream_review_reject_v1";
const DREAM_REVIEW_RETAIN_OPERATION: &str = "lived_dream_review_retain_non_fact_v1";
const DREAM_REVIEW_EXPIRE_OPERATION: &str = "lived_dream_review_expire_v1";

fn json<T: serde::Serialize>(value: &T) -> Result<String, StoreError> {
    let body = serde_json::to_string(value)
        .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
    if body.len() > MAX_BODY_BYTES {
        return Err(StoreError::AutonomyConflict(
            "alpha3 lived-world body exceeds 256 KiB".into(),
        ));
    }
    Ok(body)
}

fn parse<T: serde::de::DeserializeOwned>(body: String) -> Result<T, StoreError> {
    if body.len() > MAX_BODY_BYTES {
        return Err(StoreError::AutonomyConflict(
            "alpha3 lived-world body exceeds 256 KiB".into(),
        ));
    }
    serde_json::from_str(&body).map_err(|error| StoreError::AutonomyConflict(error.to_string()))
}

fn autonomous_runtime_state_from(
    conn: &Connection,
    persona_scope: &Digest,
) -> Result<(AutonomousRuntimeStateV1, Digest), StoreError> {
    let raw: Option<(i64, i64, Option<String>)> = conn
        .query_row(
            "SELECT generation,state_revision,
                    CASE WHEN length(CAST(body_json AS BLOB))<=?2 THEN body_json END
             FROM autonomous_runtime_state WHERE persona_scope=?1",
            params![persona_scope.to_vec(), MAX_BODY_BYTES as i64],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let (raw_generation, raw_revision, body) = raw.ok_or(StoreError::AutonomyNotFound(
        "autonomous runtime state for dream review",
    ))?;
    let state: AutonomousRuntimeStateV1 = parse(body.ok_or_else(|| {
        StoreError::AutonomyConflict("autonomous runtime state exceeds its byte bound".into())
    })?)?;
    let generation = u64::try_from(raw_generation)
        .map_err(|_| StoreError::AutonomyConflict("negative autonomous generation".into()))?;
    let state_revision = u64::try_from(raw_revision)
        .map_err(|_| StoreError::AutonomyConflict("negative autonomous state revision".into()))?;
    if state.persona_scope != *persona_scope
        || state.generation != generation
        || state.state_revision != state_revision
    {
        return Err(StoreError::AutonomyConflict(
            "autonomous runtime state columns or scope differ from its body".into(),
        ));
    }
    let state_bytes = serde_json::to_vec(&state)
        .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
    if state_bytes.len() > MAX_BODY_BYTES {
        return Err(StoreError::AutonomyConflict(
            "autonomous runtime state exceeds its byte bound".into(),
        ));
    }
    let state_digest = wire::domain_hash(b"ae.autonomy.snapshot.v1", &[&state_bytes]);
    Ok((state, state_digest))
}

fn sql_u64(value: u64, field: &str) -> Result<i64, StoreError> {
    value
        .try_into()
        .map_err(|_| StoreError::AutonomyConflict(format!("{field} is out of SQLite range")))
}

fn projection_body_digest<T: serde::Serialize>(
    domain: &[u8],
    value: &T,
) -> Result<Digest, StoreError> {
    let body = json(value)?;
    Ok(wire::domain_hash(domain, &[body.as_bytes()]))
}

fn lived_world_projection_digest(
    anchor: &WorldAnchorV1,
    lived: &LivedDayStateV1,
    dreams: &[DreamResidueV1],
) -> Result<Digest, StoreError> {
    anchor
        .validate()
        .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
    lived
        .validate()
        .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
    if anchor.persona_scope != lived.persona_scope
        || anchor.world_anchor_id != lived.world_anchor_id
    {
        return Err(StoreError::AutonomyConflict(
            "lived-world commitment scope or anchor mismatch".into(),
        ));
    }
    let anchor_digest = projection_body_digest(b"ae.lived-world.anchor-projection.v1", anchor)?;
    let lived_digest = projection_body_digest(b"ae.lived-world.day-projection.v1", lived)?;
    let mut ordered_dreams = dreams.iter().collect::<Vec<_>>();
    ordered_dreams.sort_by_key(|dream| (dream.created_at_utc_ms, dream.residue_id));
    let mut dream_digest = wire::domain_hash(b"ae.lived-world.dream-projection.empty.v1", &[]);
    for dream in ordered_dreams {
        dream
            .validate()
            .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
        if dream.persona_scope != lived.persona_scope {
            return Err(StoreError::AutonomyConflict(
                "lived-world dream commitment scope mismatch".into(),
            ));
        }
        let row_digest = projection_body_digest(b"ae.lived-world.dream-row.v1", dream)?;
        dream_digest = wire::domain_hash(
            b"ae.lived-world.dream-projection.fold.v1",
            &[&dream_digest, &row_digest],
        );
    }
    Ok(wire::domain_hash(
        b"ae.lived-world.projection-commitment.v1",
        &[&anchor_digest, &lived_digest, &dream_digest],
    ))
}

fn dream_review_operation(action: DreamReviewActionV1) -> &'static str {
    match action {
        DreamReviewActionV1::Reject => DREAM_REVIEW_REJECT_OPERATION,
        DreamReviewActionV1::RetainNonFact => DREAM_REVIEW_RETAIN_OPERATION,
        DreamReviewActionV1::Expire => DREAM_REVIEW_EXPIRE_OPERATION,
    }
}

fn dream_review_state_for_operation(operation: &str) -> Option<DreamResidueStateV1> {
    match operation {
        DREAM_REVIEW_REJECT_OPERATION => Some(DreamResidueStateV1::Rejected),
        DREAM_REVIEW_RETAIN_OPERATION => Some(DreamResidueStateV1::RetainedNonFact),
        DREAM_REVIEW_EXPIRE_OPERATION => Some(DreamResidueStateV1::Expired),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn dream_review_authority_digest(
    persona_scope: &Digest,
    operation: &str,
    review_event_id: &[u8; 16],
    reviewed_at_utc_ms: u64,
    pre: &DreamResidueV1,
    post: &DreamResidueV1,
    pre_operational_revision: u64,
    post_operational_revision: u64,
    canonical_base_revision: u64,
    canonical_next_revision: u64,
    autonomous_state_digest: &Digest,
    autonomous_state_revision: u64,
    projection_commitment: &Digest,
) -> Result<Digest, StoreError> {
    let pre_digest = projection_body_digest(b"ae.dream-review.pre-row.v1", pre)?;
    let post_digest = projection_body_digest(b"ae.dream-review.post-row.v1", post)?;
    let reviewed_at = reviewed_at_utc_ms.to_le_bytes();
    let pre_revision = pre_operational_revision.to_le_bytes();
    let post_revision = post_operational_revision.to_le_bytes();
    let canonical_base = canonical_base_revision.to_le_bytes();
    let canonical_next = canonical_next_revision.to_le_bytes();
    let state_revision = autonomous_state_revision.to_le_bytes();
    Ok(wire::domain_hash(
        b"ae.lived-world.dream-review-authority.v2",
        &[
            persona_scope,
            &post.residue_id,
            review_event_id,
            operation.as_bytes(),
            b"awake_explicit_review",
            &reviewed_at,
            &pre_digest,
            &post_digest,
            &pre_revision,
            &post_revision,
            &canonical_base,
            &canonical_next,
            autonomous_state_digest,
            &state_revision,
            projection_commitment,
        ],
    ))
}

fn journal_rows_from(
    conn: &Connection,
    persona_scope: &Digest,
) -> Result<Vec<JournalRow>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT logical_revision,base_revision,event_kind,
                CASE WHEN length(event_bytes)<=?2 THEN event_bytes END,
                CASE WHEN length(event_digest)=32 THEN event_digest END,
                CASE WHEN length(receipt_bytes)<=?2 THEN receipt_bytes END,
                CASE WHEN length(delta_bytes)<=?2 THEN delta_bytes END,
                CASE WHEN length(chain_digest)=32 THEN chain_digest END
         FROM journal WHERE scope_digest=?1 ORDER BY logical_revision",
    )?;
    let raw = statement
        .query_map(
            params![persona_scope.to_vec(), MAX_DELTA_BYTES as i64],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<Vec<u8>>>(3)?,
                    row.get::<_, Option<Vec<u8>>>(4)?,
                    row.get::<_, Option<Vec<u8>>>(5)?,
                    row.get::<_, Option<Vec<u8>>>(6)?,
                    row.get::<_, Option<Vec<u8>>>(7)?,
                ))
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    raw.into_iter()
        .map(
            |(
                revision,
                base_revision,
                event_kind,
                event_bytes,
                event_digest,
                receipt_bytes,
                delta_bytes,
                chain_digest,
            )| {
                let revision = u64::try_from(revision).map_err(|_| {
                    StoreError::AutonomyConflict("negative lived-world journal revision".into())
                })?;
                let base_revision = u64::try_from(base_revision).map_err(|_| {
                    StoreError::AutonomyConflict(
                        "negative lived-world journal base revision".into(),
                    )
                })?;
                let event_digest: Digest = event_digest
                    .ok_or_else(|| {
                        StoreError::AutonomyConflict(
                            "lived-world journal event digest is invalid".into(),
                        )
                    })?
                    .try_into()
                    .map_err(|_| {
                        StoreError::AutonomyConflict(
                            "lived-world journal event digest is invalid".into(),
                        )
                    })?;
                let chain_digest: Digest = chain_digest
                    .ok_or_else(|| {
                        StoreError::AutonomyConflict(
                            "lived-world journal chain digest is invalid".into(),
                        )
                    })?
                    .try_into()
                    .map_err(|_| {
                        StoreError::AutonomyConflict(
                            "lived-world journal chain digest is invalid".into(),
                        )
                    })?;
                Ok(JournalRow {
                    revision,
                    scope_digest: *persona_scope,
                    base_revision,
                    event_kind,
                    event_bytes: event_bytes.ok_or_else(|| {
                        StoreError::AutonomyConflict(
                            "lived-world journal event exceeds 1 MiB".into(),
                        )
                    })?,
                    event_digest,
                    receipt_bytes: receipt_bytes.ok_or_else(|| {
                        StoreError::AutonomyConflict(
                            "lived-world journal receipt exceeds 1 MiB".into(),
                        )
                    })?,
                    delta_bytes: delta_bytes.ok_or_else(|| {
                        StoreError::AutonomyConflict(
                            "lived-world journal delta exceeds 1 MiB".into(),
                        )
                    })?,
                    chain_digest,
                })
            },
        )
        .collect()
}

fn has_lived_world_authority(
    conn: &Connection,
    persona_scope: &Digest,
) -> Result<bool, StoreError> {
    for row in journal_rows_from(conn, persona_scope)? {
        let delta = if row.delta_bytes.is_empty() {
            AutonomyJournalDeltaV1 {
                state: None,
                inner_events: Vec::new(),
                intention: None,
            }
        } else {
            serde_json::from_slice::<AutonomyJournalDeltaV1>(&row.delta_bytes)
                .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?
        };
        if delta
            .inner_events
            .iter()
            .any(|event| event.summary_code == "lived_world_committed")
        {
            return Ok(true);
        }
        if row.event_kind == "admin_action" {
            let decoded = wire::decode_event(&row.event_bytes)
                .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
            if matches!(
                decoded,
                CanonicalEvent::AdminAction(ref review)
                    if dream_review_state_for_operation(&review.operation).is_some()
            ) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn lived_world_commitment_event_for_projection(
    persona_scope: Digest,
    committed_at_utc_ms: u64,
    anchor: &WorldAnchorV1,
    lived: &LivedDayStateV1,
    dreams: &[DreamResidueV1],
) -> Result<InnerEventV1, StoreError> {
    if persona_scope != lived.persona_scope {
        return Err(StoreError::AutonomyConflict(
            "lived-world commitment persona scope mismatch".into(),
        ));
    }
    let commitment = lived_world_projection_digest(anchor, lived, dreams)?;
    let event_id = commitment[..16]
        .try_into()
        .expect("commitment digest prefix");
    let value_before = i64::from_le_bytes(
        commitment[16..24]
            .try_into()
            .expect("commitment digest middle"),
    );
    let value_after = i64::from_le_bytes(
        commitment[24..32]
            .try_into()
            .expect("commitment digest suffix"),
    );
    Ok(InnerEventV1 {
        schema_version: 1,
        event_id,
        persona_scope,
        kind: InnerEventKindV1::HomeostasisChanged,
        committed_at_utc_ms,
        summary_code: "lived_world_committed".into(),
        value_before: Some(Fixed::from_raw(value_before)),
        value_after: Some(Fixed::from_raw(value_after)),
        source_event_ids: Vec::new(),
        tombstoned: false,
    })
}

fn world_anchor_from(
    conn: &Connection,
    persona_scope: &Digest,
) -> Result<Option<WorldAnchorV1>, StoreError> {
    conn.query_row(
        "SELECT CASE WHEN length(CAST(body_json AS BLOB))<=?2 THEN body_json END
         FROM world_anchor WHERE persona_scope=?1",
        params![persona_scope.to_vec(), MAX_BODY_BYTES as i64],
        |row| row.get::<_, Option<String>>(0),
    )
    .optional()?
    .map(|body| {
        parse(body.ok_or_else(|| {
            StoreError::AutonomyConflict("world anchor body exceeds 256 KiB".into())
        })?)
    })
    .transpose()
}

fn lived_day_from(
    conn: &Connection,
    persona_scope: &Digest,
) -> Result<Option<LivedDayStateV1>, StoreError> {
    conn.query_row(
        "SELECT CASE WHEN length(CAST(body_json AS BLOB))<=?2 THEN body_json END
         FROM lived_day_state WHERE persona_scope=?1",
        params![persona_scope.to_vec(), MAX_BODY_BYTES as i64],
        |row| row.get::<_, Option<String>>(0),
    )
    .optional()?
    .map(|body| {
        parse(
            body.ok_or_else(|| {
                StoreError::AutonomyConflict("lived day body exceeds 256 KiB".into())
            })?,
        )
    })
    .transpose()
}

fn dream_from(
    conn: &Connection,
    residue_id: &[u8; 16],
) -> Result<Option<(u64, DreamResidueV1)>, StoreError> {
    conn.query_row(
        "SELECT revision,CASE WHEN length(CAST(body_json AS BLOB))<=?2 THEN body_json END
         FROM dream_residue WHERE residue_id=?1",
        params![residue_id.to_vec(), MAX_BODY_BYTES as i64],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
    )
    .optional()?
    .map(|(revision, body)| {
        let revision = u64::try_from(revision)
            .map_err(|_| StoreError::AutonomyConflict("negative dream residue revision".into()))?;
        let dream = parse(body.ok_or_else(|| {
            StoreError::AutonomyConflict("dream residue body exceeds 256 KiB".into())
        })?)?;
        Ok((revision, dream))
    })
    .transpose()
}

fn dream_residues_from(
    conn: &Connection,
    persona_scope: &Digest,
) -> Result<Vec<DreamResidueV1>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT CASE WHEN length(CAST(body_json AS BLOB))<=?2 THEN body_json END
         FROM dream_residue WHERE persona_scope=?1
         ORDER BY revision,residue_id",
    )?;
    let rows = statement
        .query_map(
            params![persona_scope.to_vec(), MAX_BODY_BYTES as i64],
            |row| row.get::<_, Option<String>>(0),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let mut dreams = Vec::with_capacity(rows.len());
    for body in rows {
        let dream: DreamResidueV1 = parse(body.ok_or_else(|| {
            StoreError::AutonomyConflict("dream residue body exceeds 256 KiB".into())
        })?)?;
        if dream.persona_scope != *persona_scope {
            return Err(StoreError::AutonomyConflict(
                "dream residue scope differs from query".into(),
            ));
        }
        dreams.push(dream);
    }
    dreams.sort_by_key(|dream| (dream.created_at_utc_ms, dream.residue_id));
    Ok(dreams)
}

fn dream_state_name(state: DreamResidueStateV1) -> &'static str {
    match state {
        DreamResidueStateV1::PendingWakingReview => "pending_waking_review",
        DreamResidueStateV1::Rejected => "rejected",
        DreamResidueStateV1::RetainedNonFact => "retained_non_fact",
        DreamResidueStateV1::Expired => "expired",
    }
}

fn proposal_capability_is_live(
    conn: &Connection,
    proposal: &EcosystemProposalV1,
    now_utc_ms: u64,
) -> Result<bool, StoreError> {
    let mut statement = conn.prepare(
        "SELECT CASE WHEN length(CAST(body_json AS BLOB))<=?3 THEN body_json END
         FROM ecosystem_capability_grant
         WHERE plugin_instance_digest=?1 AND capability_digest=?2 AND state='active'
         ORDER BY revision DESC LIMIT 1",
    )?;
    let body = statement
        .query_row(
            params![
                proposal.plugin_instance_digest.to_vec(),
                proposal.capability_digest.to_vec(),
                MAX_BODY_BYTES as i64
            ],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?;
    let Some(body) = body else {
        return Ok(false);
    };
    let grant: EcosystemCapabilityGrantV1 = parse(body.ok_or_else(|| {
        StoreError::AutonomyConflict("ecosystem capability body exceeds 256 KiB".into())
    })?)?;
    Ok(
        grant.plugin_instance_digest == proposal.plugin_instance_digest
            && grant.capability_digest == proposal.capability_digest
            && grant
                .valid_until_utc_ms
                .is_none_or(|expires| now_utc_ms < expires),
    )
}

fn source_is_attested(
    conn: &Connection,
    persona_scope: &Digest,
    source_id: &[u8; 16],
    now_utc_ms: u64,
) -> Result<bool, StoreError> {
    let interaction = conn
        .query_row(
            "SELECT CASE WHEN length(CAST(body_json AS BLOB))<=?3 THEN body_json END
             FROM interaction_fact WHERE persona_scope=?1 AND fact_id=?2",
            params![
                persona_scope.to_vec(),
                source_id.to_vec(),
                MAX_BODY_BYTES as i64
            ],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?;
    if let Some(body) = interaction {
        let fact: InteractionFactV1 = parse(body.ok_or_else(|| {
            StoreError::AutonomyConflict("interaction fact body exceeds 256 KiB".into())
        })?)?;
        return Ok(matches!(
            fact.source_authority,
            InteractionSourceAuthorityV1::ExplicitControl
                | InteractionSourceAuthorityV1::AstrbotMetadata
                | InteractionSourceAuthorityV1::DeterministicRule
        ) && fact
            .expires_at_utc_ms
            .is_some_and(|expires| now_utc_ms < expires));
    }

    let proposal = conn
        .query_row(
            "SELECT CASE WHEN length(CAST(body_json AS BLOB))<=?3 THEN body_json END
             FROM ecosystem_proposal
             WHERE persona_scope=?1 AND proposal_id=?2 AND decision='accepted'",
            params![
                persona_scope.to_vec(),
                source_id.to_vec(),
                MAX_BODY_BYTES as i64
            ],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?;
    let Some(body) = proposal else {
        return Ok(false);
    };
    let proposal: EcosystemProposalV1 = parse(body.ok_or_else(|| {
        StoreError::AutonomyConflict("ecosystem proposal body exceeds 256 KiB".into())
    })?)?;
    Ok(proposal.valid_from_utc_ms <= now_utc_ms
        && now_utc_ms < proposal.expires_at_utc_ms
        && proposal_capability_is_live(conn, &proposal, now_utc_ms)?)
}

fn validate_external_layer(
    tx: &Transaction<'_>,
    persona_scope: &Digest,
    layer: WorldLayerV1,
    source_event_ids: &[[u8; 16]],
    now_utc_ms: u64,
) -> Result<(), StoreError> {
    if layer != WorldLayerV1::ExternalObserved {
        return Ok(());
    }
    if source_event_ids.is_empty() {
        return Err(StoreError::AutonomyConflict(
            "world_layer_forbidden: external_observed requires an attested source".into(),
        ));
    }
    for source in source_event_ids {
        if !source_is_attested(tx, persona_scope, source, now_utc_ms)? {
            return Err(StoreError::AutonomyConflict(
                "source_untrusted: external_observed source is absent, stale, or unattested".into(),
            ));
        }
    }
    Ok(())
}

fn validate_lived_day(
    tx: &Transaction<'_>,
    lived: &LivedDayStateV1,
    anchor: &WorldAnchorV1,
    now_utc_ms: u64,
) -> Result<(), StoreError> {
    lived
        .validate()
        .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
    if lived.persona_scope != anchor.persona_scope
        || lived.world_anchor_id != anchor.world_anchor_id
        || lived.current_segment.state != LivedSegmentStateV1::Active
        || lived.current_segment.starts_at_utc_ms > now_utc_ms
        || now_utc_ms >= lived.current_segment.ends_at_utc_ms
        || lived.next_transition_utc_ms != lived.current_segment.ends_at_utc_ms
        || !anchor
            .allowed_layers
            .contains(&lived.current_segment.world_layer)
        || lived
            .active_goal
            .as_ref()
            .is_some_and(|goal| !anchor.allowed_layers.contains(&goal.world_layer))
        || lived.routine_formula_digest == [0; 32]
    {
        return Err(StoreError::AutonomyConflict(
            "lived day scope, layer, time, or formula is invalid".into(),
        ));
    }
    validate_external_layer(
        tx,
        &lived.persona_scope,
        lived.current_segment.world_layer,
        &lived.current_segment.source_event_ids,
        now_utc_ms,
    )?;
    if let Some(goal) = &lived.active_goal {
        validate_external_layer(
            tx,
            &lived.persona_scope,
            goal.world_layer,
            &goal.source_event_ids,
            now_utc_ms,
        )?;
    }
    Ok(())
}

fn ensure_anchor_tx(
    tx: &Transaction<'_>,
    proposed: &WorldAnchorV1,
) -> Result<WorldAnchorV1, StoreError> {
    proposed
        .validate()
        .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
    if let Some(existing) = world_anchor_from(tx, &proposed.persona_scope)? {
        if existing != *proposed {
            return Err(StoreError::AutonomyConflict(
                "world_anchor_immutable: existing anchor differs from proposal".into(),
            ));
        }
        return Ok(existing);
    }
    let inserted = tx.execute(
        "INSERT INTO world_anchor(world_anchor_id,persona_scope,mode,revision,body_json)
         VALUES(?1,?2,'mixed_main_world',1,?3)",
        params![
            proposed.world_anchor_id.to_vec(),
            proposed.persona_scope.to_vec(),
            json(proposed)?
        ],
    )?;
    if inserted != 1 {
        return Err(StoreError::AutonomyConflict(
            "world anchor bootstrap did not insert exactly one row".into(),
        ));
    }
    let stored = world_anchor_from(tx, &proposed.persona_scope)?
        .ok_or(StoreError::AutonomyNotFound("world anchor after bootstrap"))?;
    if stored != *proposed {
        return Err(StoreError::AutonomyConflict(
            "world anchor bootstrap readback mismatch".into(),
        ));
    }
    Ok(stored)
}

fn persist_lived_day_tx(
    tx: &Transaction<'_>,
    lived: &LivedDayStateV1,
    anchor: &WorldAnchorV1,
    now_utc_ms: u64,
) -> Result<(), StoreError> {
    validate_lived_day(tx, lived, anchor, now_utc_ms)?;
    match lived_day_from(tx, &lived.persona_scope)? {
        None => {
            if lived.revision != 1 {
                return Err(StoreError::AutonomyConflict(
                    "initial lived day revision must be one".into(),
                ));
            }
            tx.execute(
                "INSERT INTO lived_day_state(
                     persona_scope,world_anchor_id,persona_day_ordinal,revision,body_json
                 ) VALUES(?1,?2,?3,?4,?5)",
                params![
                    lived.persona_scope.to_vec(),
                    lived.world_anchor_id.to_vec(),
                    i64::from(lived.persona_day_ordinal),
                    sql_u64(lived.revision, "lived day revision")?,
                    json(lived)?
                ],
            )?;
        }
        Some(previous) => {
            if lived.revision != previous.revision.saturating_add(1) {
                return Err(StoreError::AutonomyConflict(
                    "lived day compare-and-swap revision is stale".into(),
                ));
            }
            let changed = tx.execute(
                "UPDATE lived_day_state
                 SET world_anchor_id=?2,persona_day_ordinal=?3,revision=?4,body_json=?5
                 WHERE persona_scope=?1 AND revision=?6",
                params![
                    lived.persona_scope.to_vec(),
                    lived.world_anchor_id.to_vec(),
                    i64::from(lived.persona_day_ordinal),
                    sql_u64(lived.revision, "lived day revision")?,
                    json(lived)?,
                    sql_u64(previous.revision, "previous lived day revision")?
                ],
            )?;
            if changed != 1 {
                return Err(StoreError::AutonomyConflict(
                    "lived day compare-and-swap failed".into(),
                ));
            }
        }
    }
    if lived_day_from(tx, &lived.persona_scope)?.as_ref() != Some(lived) {
        return Err(StoreError::AutonomyConflict(
            "lived day readback mismatch".into(),
        ));
    }
    Ok(())
}

fn expected_dream_transition(
    current: &DreamResidueV1,
    next: &DreamResidueV1,
) -> Result<(), StoreError> {
    if current.state != DreamResidueStateV1::PendingWakingReview
        || !matches!(
            next.state,
            DreamResidueStateV1::Rejected
                | DreamResidueStateV1::RetainedNonFact
                | DreamResidueStateV1::Expired
        )
        || next.reviewed_at_utc_ms.is_none()
        || next.review_event_id.is_none()
    {
        return Err(StoreError::AutonomyConflict(
            "dream_not_reviewed: invalid dream review transition".into(),
        ));
    }
    let mut expected = current.clone();
    expected.state = next.state;
    expected.reviewed_at_utc_ms = next.reviewed_at_utc_ms;
    expected.review_event_id = next.review_event_id;
    if expected != *next {
        return Err(StoreError::AutonomyConflict(
            "dream review attempted to alter non-review authority".into(),
        ));
    }
    next.validate()
        .map_err(|error| StoreError::AutonomyConflict(error.to_string()))
}

fn persist_dream_transition_tx(
    tx: &Transaction<'_>,
    next: &DreamResidueV1,
) -> Result<(), StoreError> {
    let (revision, current) = dream_from(tx, &next.residue_id)?
        .ok_or(StoreError::AutonomyNotFound("pending dream residue"))?;
    expected_dream_transition(&current, next)?;
    let changed = tx.execute(
        "UPDATE dream_residue SET revision=?2,state=?3,body_json=?4
         WHERE residue_id=?1 AND revision=?5 AND state='pending_waking_review'",
        params![
            next.residue_id.to_vec(),
            sql_u64(revision.saturating_add(1), "dream residue revision")?,
            dream_state_name(next.state),
            json(next)?,
            sql_u64(revision, "previous dream residue revision")?
        ],
    )?;
    if changed != 1 {
        return Err(StoreError::AutonomyConflict(
            "dream residue compare-and-swap failed".into(),
        ));
    }
    let (_, stored) = dream_from(tx, &next.residue_id)?
        .ok_or(StoreError::AutonomyNotFound("dream residue after review"))?;
    if stored != *next {
        return Err(StoreError::AutonomyConflict(
            "dream residue readback mismatch".into(),
        ));
    }
    Ok(())
}

// Called by Task 2's wake materializer while its existing BEGIN IMMEDIATE is
// still open. This function never starts or commits a transaction.

fn notable_importance(event: &InnerEventV1) -> Option<LivedNodeImportanceV1> {
    match event.summary_code.as_str() {
        "contact_granted" | "contact_paused" | "relation_ended" => {
            Some(LivedNodeImportanceV1::SafetyCritical)
        }
        "follow_up_requested"
        | "follow_up_resolved"
        | "inbound_observed"
        | "lived_day_changed"
        | "lived_goal_changed"
        | "lived_proposal_changed"
        | "lived_sleep_changed"
        | "lived_day_catch_up" => Some(LivedNodeImportanceV1::Notable),
        _ if event.kind == InnerEventKindV1::SleepTransition => {
            Some(LivedNodeImportanceV1::Notable)
        }
        _ => None,
    }
}

enum LivedProjectionAuthorityV1 {
    Wake(TimeAdvanceV1, InnerEventV1),
    DreamReview(AdminAction, TransitionReceipt),
}

fn verify_lived_world_replay_from(
    conn: &Connection,
    persona_scope: &Digest,
) -> Result<(), StoreError> {
    let rows = journal_rows_from(conn, persona_scope)?;
    let mut latest = None;
    let mut lived_initialized = false;
    for (index, row) in rows.iter().enumerate() {
        let delta = if row.delta_bytes.is_empty() {
            AutonomyJournalDeltaV1 {
                state: None,
                inner_events: Vec::new(),
                intention: None,
            }
        } else {
            serde_json::from_slice::<AutonomyJournalDeltaV1>(&row.delta_bytes)
                .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?
        };
        let commitments = delta
            .inner_events
            .iter()
            .filter(|event| event.summary_code == "lived_world_committed")
            .collect::<Vec<_>>();
        if commitments.len() > 1 {
            return Err(StoreError::AutonomyConflict(
                "multiple lived-world commitments in one wake delta".into(),
            ));
        }
        if row.event_kind == "time_advance" && lived_initialized && commitments.len() != 1 {
            return Err(StoreError::AutonomyConflict(
                "initialized lived-world wake lacks exactly one commitment".into(),
            ));
        }
        if let Some(committed) = commitments.first().copied() {
            let decoded = wire::decode_event(&row.event_bytes)
                .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
            let CanonicalEvent::TimeAdvance(wake) = decoded else {
                return Err(StoreError::AutonomyConflict(
                    "lived-world commitment is not bound to a wake event".into(),
                ));
            };
            let expected_scope = wire::persona_scope_digest(
                &wake.scope.bot_token,
                &wake.scope.persona_token,
                wake.scope.relation_token.as_ref(),
            );
            if wake.scope.relation_token.is_some()
                || expected_scope != *persona_scope
                || committed.persona_scope != *persona_scope
                || committed.committed_at_utc_ms != wake.frozen.effective_now_utc_ms
            {
                return Err(StoreError::AutonomyConflict(
                    "lived-world commitment wake scope or time mismatch".into(),
                ));
            }
            let expected_manifest =
                crate::autonomy::inner_event_manifest_digest(&delta.inner_events)?;
            let manifest: Option<(i64, Vec<u8>)> = conn
                .query_row(
                    "SELECT event_count,event_digest FROM inner_event_manifest
                     WHERE persona_scope=?1 AND journal_revision=?2",
                    params![
                        persona_scope.to_vec(),
                        sql_u64(row.revision, "lived-world manifest revision")?
                    ],
                    |manifest_row| Ok((manifest_row.get(0)?, manifest_row.get(1)?)),
                )
                .optional()?;
            let Some((event_count, manifest_digest)) = manifest else {
                return Err(StoreError::AutonomyConflict(
                    "lived-world replay manifest is absent".into(),
                ));
            };
            if usize::try_from(event_count).ok() != Some(delta.inner_events.len())
                || manifest_digest != expected_manifest
            {
                return Err(StoreError::AutonomyConflict(
                    "lived-world replay manifest mismatch".into(),
                ));
            }
            let projected_body: Option<String> = conn
                .query_row(
                    "SELECT CASE WHEN length(CAST(body_json AS BLOB))<=?4 THEN body_json END
                     FROM inner_event
                     WHERE event_id=?1 AND persona_scope=?2 AND journal_revision=?3",
                    params![
                        committed.event_id.to_vec(),
                        persona_scope.to_vec(),
                        sql_u64(row.revision, "lived-world event revision")?,
                        MAX_BODY_BYTES as i64
                    ],
                    |event_row| event_row.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten();
            let projected: InnerEventV1 = parse(projected_body.ok_or_else(|| {
                StoreError::AutonomyConflict(
                    "lived-world commitment event projection is absent or oversized".into(),
                )
            })?)?;
            if projected != *committed {
                return Err(StoreError::AutonomyConflict(
                    "lived-world commitment event projection mismatch".into(),
                ));
            }
            lived_initialized = true;
            latest = Some(LivedProjectionAuthorityV1::Wake(wake, committed.clone()));
        }

        if row.event_kind != "admin_action" {
            continue;
        }
        let decoded = wire::decode_event(&row.event_bytes)
            .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
        let CanonicalEvent::AdminAction(review) = decoded else {
            return Err(StoreError::AutonomyConflict(
                "dream review authority event kind mismatch".into(),
            ));
        };
        if dream_review_state_for_operation(&review.operation).is_none() {
            continue;
        }
        lived_initialized = true;
        if delta.state.is_some() || !delta.inner_events.is_empty() || delta.intention.is_some() {
            return Err(StoreError::AutonomyConflict(
                "dream review authority delta is not empty".into(),
            ));
        }
        let receipt = wire::decode_transition_receipt(&row.receipt_bytes)
            .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
        let derived_scope = wire::persona_scope_digest(
            &review.scope.bot_token,
            &review.scope.persona_token,
            review.scope.relation_token.as_ref(),
        );
        if review.scope.relation_token.is_some()
            || derived_scope != *persona_scope
            || row.event_digest != wire::event_digest(&CanonicalEvent::AdminAction(review.clone()))
            || receipt.schema_version != 1
            || receipt.scope_digest != *persona_scope
            || receipt.event_digest != row.event_digest
            || receipt.authority_digest
                != ae_authority::authority_projection_digest(&CanonicalEvent::AdminAction(
                    review.clone(),
                ))
            || receipt.base_revision != row.base_revision
            || receipt.next_revision != row.revision
            || row.base_revision.saturating_add(1) != row.revision
            || receipt.state_before != receipt.state_after
            || receipt.action_contract.is_some()
            || receipt.status != CommitStatus::Committed
        {
            return Err(StoreError::AutonomyConflict(
                "dream review authority receipt mismatch".into(),
            ));
        }
        let previous = index
            .checked_sub(1)
            .and_then(|value| rows.get(value))
            .ok_or_else(|| {
                StoreError::AutonomyConflict(
                    "dream review authority lacks a chain predecessor".into(),
                )
            })?;
        let expected_chain = if row.delta_bytes.is_empty() {
            ae_continuum::chain_link(&previous.chain_digest, &row.event_bytes, &row.receipt_bytes)
        } else {
            ae_continuum::chain_link_with_delta(
                &previous.chain_digest,
                &row.event_bytes,
                &row.receipt_bytes,
                &row.delta_bytes,
            )
        };
        let applied_revision: Option<i64> = conn
            .query_row(
                "SELECT revision FROM applied_events WHERE scope_digest=?1 AND event_digest=?2",
                params![persona_scope.to_vec(), row.event_digest.to_vec()],
                |applied| applied.get(0),
            )
            .optional()?;
        if expected_chain != row.chain_digest
            || applied_revision.and_then(|value| u64::try_from(value).ok()) != Some(row.revision)
        {
            return Err(StoreError::AutonomyConflict(
                "dream review authority chain or projection mismatch".into(),
            ));
        }
        latest = Some(LivedProjectionAuthorityV1::DreamReview(review, receipt));
    }

    let Some(latest) = latest else {
        if lived_day_from(conn, persona_scope)?.is_none()
            && dream_residues_from(conn, persona_scope)?.is_empty()
        {
            // Schema migration may have backfilled the immutable fixed-world
            // anchor before the first lived wake. That anchor-only state is the
            // sole projection-compatible pre-authority exception.
            return Ok(());
        }
        return Err(StoreError::AutonomyConflict(
            "lived-world replay commitment is absent".into(),
        ));
    };
    let anchor = world_anchor_from(conn, persona_scope)?
        .ok_or(StoreError::AutonomyNotFound("world anchor for replay"))?;
    let lived = lived_day_from(conn, persona_scope)?
        .ok_or(StoreError::AutonomyNotFound("lived day for replay"))?;
    let dreams = dream_residues_from(conn, persona_scope)?;
    match latest {
        LivedProjectionAuthorityV1::Wake(wake, committed) => {
            let expected = lived_world_commitment_event_for_projection(
                *persona_scope,
                wake.frozen.effective_now_utc_ms,
                &anchor,
                &lived,
                &dreams,
            )?;
            if expected != committed {
                return Err(StoreError::AutonomyConflict(
                    "lived-world replay commitment mismatch".into(),
                ));
            }
        }
        LivedProjectionAuthorityV1::DreamReview(review, receipt) => {
            let (autonomous_state, autonomous_state_digest) =
                autonomous_runtime_state_from(conn, persona_scope)?;
            if autonomous_state.sleep_state != SleepStateV1::Awake
                || receipt.state_before != autonomous_state_digest
                || receipt.state_after != autonomous_state_digest
            {
                return Err(StoreError::AutonomyConflict(
                    "lived-world dream review state authority mismatch".into(),
                ));
            }
            let matching = dreams
                .iter()
                .filter(|dream| dream.review_event_id == Some(review.event_id))
                .collect::<Vec<_>>();
            if matching.len() != 1 {
                return Err(StoreError::AutonomyConflict(
                    "lived-world replay commitment mismatch".into(),
                ));
            }
            let post = matching[0];
            if dream_review_state_for_operation(&review.operation) != Some(post.state) {
                return Err(StoreError::AutonomyConflict(
                    "lived-world replay commitment mismatch".into(),
                ));
            }
            let (post_revision, stored_post) = dream_from(conn, &post.residue_id)?
                .ok_or(StoreError::AutonomyNotFound("reviewed dream residue"))?;
            if stored_post != *post || post_revision == 0 {
                return Err(StoreError::AutonomyConflict(
                    "lived-world replay commitment mismatch".into(),
                ));
            }
            let mut pre = post.clone();
            pre.state = DreamResidueStateV1::PendingWakingReview;
            pre.reviewed_at_utc_ms = None;
            pre.review_event_id = None;
            expected_dream_transition(&pre, post)?;
            let projection_commitment = lived_world_projection_digest(&anchor, &lived, &dreams)?;
            let expected_nonce = dream_review_authority_digest(
                persona_scope,
                &review.operation,
                &review.event_id,
                post.reviewed_at_utc_ms.ok_or_else(|| {
                    StoreError::AutonomyConflict("reviewed dream lacks review time".into())
                })?,
                &pre,
                post,
                post_revision - 1,
                post_revision,
                receipt.base_revision,
                receipt.next_revision,
                &autonomous_state_digest,
                autonomous_state.state_revision,
                &projection_commitment,
            )?;
            if review.nonce_digest != expected_nonce {
                return Err(StoreError::AutonomyConflict(
                    "lived-world replay commitment mismatch".into(),
                ));
            }
        }
    }
    Ok(())
}

// Preserve the established storage/API shape in this compatibility boundary.
#[allow(clippy::too_many_arguments)]
fn append_dream_review_authority_tx(
    tx: &Transaction<'_>,
    persona_scope: &Digest,
    request: &DreamReviewRequestV1,
    pre: &DreamResidueV1,
    post: &DreamResidueV1,
    pre_operational_revision: u64,
    post_operational_revision: u64,
    autonomous_state: &AutonomousRuntimeStateV1,
    autonomous_state_digest: &Digest,
) -> Result<u64, StoreError> {
    let anchor = world_anchor_from(tx, persona_scope)?.ok_or(StoreError::AutonomyNotFound(
        "world anchor for dream review",
    ))?;
    let lived = lived_day_from(tx, persona_scope)?
        .ok_or(StoreError::AutonomyNotFound("lived day for dream review"))?;
    let dreams = dream_residues_from(tx, persona_scope)?;
    let projection_commitment = lived_world_projection_digest(&anchor, &lived, &dreams)?;
    let raw_current: i64 = tx.query_row(
        "SELECT COALESCE(MAX(logical_revision),0) FROM journal WHERE scope_digest=?1",
        params![persona_scope.to_vec()],
        |row| row.get(0),
    )?;
    let canonical_base_revision = u64::try_from(raw_current).map_err(|_| {
        StoreError::AutonomyConflict("negative dream review canonical revision".into())
    })?;
    if canonical_base_revision == 0 {
        return Err(StoreError::AutonomyConflict(
            "dream review authority requires a committed wake".into(),
        ));
    }
    let canonical_next_revision = canonical_base_revision.checked_add(1).ok_or_else(|| {
        StoreError::AutonomyConflict("dream review canonical revision overflow".into())
    })?;
    let last_chain_raw: Vec<u8> = tx.query_row(
        "SELECT chain_digest FROM journal WHERE scope_digest=?1 AND logical_revision=?2",
        params![
            persona_scope.to_vec(),
            sql_u64(canonical_base_revision, "dream review base revision")?
        ],
        |row| row.get(0),
    )?;
    let last_chain: Digest = last_chain_raw.try_into().map_err(|_| {
        StoreError::AutonomyConflict("dream review predecessor chain digest is invalid".into())
    })?;
    let basis_receipt_bytes: Vec<u8> = tx.query_row(
        "SELECT receipt_bytes FROM journal
         WHERE scope_digest=?1 AND event_kind!='operational_checkpoint_v1'
         ORDER BY logical_revision DESC LIMIT 1",
        params![persona_scope.to_vec()],
        |row| row.get(0),
    )?;
    let basis_receipt = wire::decode_transition_receipt(&basis_receipt_bytes)
        .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
    if basis_receipt.scope_digest != *persona_scope
        || basis_receipt.status != CommitStatus::Committed
        || basis_receipt.state_after != *autonomous_state_digest
        || autonomous_state.persona_scope != *persona_scope
        || autonomous_state.sleep_state != SleepStateV1::Awake
    {
        return Err(StoreError::AutonomyConflict(
            "dream review canonical state basis is invalid".into(),
        ));
    }
    let wake_event_bytes: Vec<u8> = tx.query_row(
        "SELECT event_bytes FROM journal
         WHERE scope_digest=?1 AND event_kind='time_advance'
         ORDER BY logical_revision DESC LIMIT 1",
        params![persona_scope.to_vec()],
        |row| row.get(0),
    )?;
    let CanonicalEvent::TimeAdvance(wake) = wire::decode_event(&wake_event_bytes)
        .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?
    else {
        return Err(StoreError::AutonomyConflict(
            "dream review wake scope source is invalid".into(),
        ));
    };
    let scope = ScopeRef {
        relation_token: None,
        ..wake.scope
    };
    if wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None) != *persona_scope {
        return Err(StoreError::AutonomyConflict(
            "dream review wake scope source mismatch".into(),
        ));
    }
    let operation = dream_review_operation(request.action);
    let nonce_digest = dream_review_authority_digest(
        persona_scope,
        operation,
        &request.review_event_id,
        request.reviewed_at_utc_ms,
        pre,
        post,
        pre_operational_revision,
        post_operational_revision,
        canonical_base_revision,
        canonical_next_revision,
        autonomous_state_digest,
        autonomous_state.state_revision,
        &projection_commitment,
    )?;
    let canonical = CanonicalEvent::AdminAction(AdminAction {
        event_id: request.review_event_id,
        scope,
        operation: operation.into(),
        nonce_digest,
    });
    let event_bytes = wire::encode_event(&canonical);
    let event_digest = wire::event_digest(&canonical);
    let receipt = TransitionReceipt {
        schema_version: 1,
        formula_digest: basis_receipt.formula_digest,
        scope_digest: *persona_scope,
        event_digest,
        authority_digest: ae_authority::authority_projection_digest(&canonical),
        base_revision: canonical_base_revision,
        next_revision: canonical_next_revision,
        state_before: *autonomous_state_digest,
        state_after: *autonomous_state_digest,
        graph_after: basis_receipt.graph_after,
        action_contract: None,
        active_nodes: basis_receipt.active_nodes,
        active_edges: basis_receipt.active_edges,
        residuals: Default::default(),
        status: CommitStatus::Committed,
    };
    let receipt_bytes = wire::encode_transition_receipt(&receipt);
    let delta_bytes = serde_json::to_vec(&AutonomyJournalDeltaV1 {
        state: None,
        inner_events: Vec::new(),
        intention: None,
    })
    .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
    let chain_digest = ae_continuum::chain_link_with_delta(
        &last_chain,
        &event_bytes,
        &receipt_bytes,
        &delta_bytes,
    );
    let duplicate: Option<i64> = tx
        .query_row(
            "SELECT revision FROM applied_events WHERE scope_digest=?1 AND event_digest=?2",
            params![persona_scope.to_vec(), event_digest.to_vec()],
            |row| row.get(0),
        )
        .optional()?;
    if duplicate.is_some() {
        return Err(StoreError::AutonomyConflict(
            "dream review authority event already exists".into(),
        ));
    }
    let inserted = tx.execute(
        "INSERT INTO journal(logical_revision,scope_digest,base_revision,event_kind,
         event_bytes,event_digest,receipt_bytes,delta_bytes,chain_digest,committed_at_ms)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            sql_u64(canonical_next_revision, "dream review next revision")?,
            persona_scope.to_vec(),
            sql_u64(canonical_base_revision, "dream review base revision")?,
            wire::event_kind_name(&canonical),
            event_bytes,
            event_digest.to_vec(),
            receipt_bytes,
            delta_bytes,
            chain_digest.to_vec(),
            sql_u64(request.reviewed_at_utc_ms, "dream review committed time")?
        ],
    )?;
    let applied = tx.execute(
        "INSERT INTO applied_events(scope_digest,event_digest,revision) VALUES(?1,?2,?3)",
        params![
            persona_scope.to_vec(),
            event_digest.to_vec(),
            sql_u64(canonical_next_revision, "dream review applied revision")?
        ],
    )?;
    if inserted != 1 || applied != 1 {
        return Err(StoreError::AutonomyConflict(
            "dream review authority append failed".into(),
        ));
    }
    verify_lived_world_replay_from(tx, persona_scope)?;
    Ok(canonical_next_revision)
}

impl Store {
    pub(crate) fn lived_world_commitment_event_v1(
        wake: &TimeAdvanceV1,
        anchor: &WorldAnchorV1,
        lived: &LivedDayStateV1,
        dreams: &[DreamResidueV1],
    ) -> Result<InnerEventV1, StoreError> {
        let persona_scope = wire::persona_scope_digest(
            &wake.scope.bot_token,
            &wake.scope.persona_token,
            wake.scope.relation_token.as_ref(),
        );
        if wake.scope.relation_token.is_some() || persona_scope != lived.persona_scope {
            return Err(StoreError::AutonomyConflict(
                "lived-world commitment requires a persona wake scope".into(),
            ));
        }
        lived_world_commitment_event_for_projection(
            persona_scope,
            wake.frozen.effective_now_utc_ms,
            anchor,
            lived,
            dreams,
        )
    }

    // Cross-validates the exact side-table projection at the latest settled
    // wake or waking dream review against its chain-bound authority record.

    /// Test-fixture ingress only. Release builds expose no dream creation path;
    /// production wake/review code can only transition an existing residue.
    #[cfg(debug_assertions)]
    pub(crate) fn insert_pending_dream_fixture_for_test(
        &mut self,
        dream: &DreamResidueV1,
    ) -> Result<(), StoreError> {
        dream
            .validate()
            .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
        if dream.state != DreamResidueStateV1::PendingWakingReview
            || dream.reviewed_at_utc_ms.is_some()
            || dream.review_event_id.is_some()
        {
            return Err(StoreError::AutonomyConflict(
                "test dream fixture must be pending and unreviewed".into(),
            ));
        }
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let inserted = tx.execute(
            "INSERT INTO dream_residue(residue_id,persona_scope,revision,state,body_json)
             VALUES(?1,?2,1,'pending_waking_review',?3)",
            params![
                dream.residue_id.to_vec(),
                dream.persona_scope.to_vec(),
                json(dream)?
            ],
        )?;
        if inserted != 1 {
            return Err(StoreError::AutonomyConflict(
                "test dream fixture did not insert exactly one row".into(),
            ));
        }
        let (_, stored) = dream_from(&tx, &dream.residue_id)?
            .ok_or(StoreError::AutonomyNotFound("test dream fixture"))?;
        if stored != *dream {
            return Err(StoreError::AutonomyConflict(
                "test dream fixture readback mismatch".into(),
            ));
        }
        tx.commit()?;
        Ok(())
    }
}
