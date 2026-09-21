//! Bounded v9 persona inventory and deterministic embodied time authority.
use crate::core_ingress::{check_core, decode, digest, encode, invalid, journal_scope};
use crate::{blob, Store, StoreError};
use ae_contracts::*;
use ae_fixed::Fixed;
use ae_neurofield::{graph_digest, state_digest, NeuralField, SparseGraph};
use rusqlite::types::Value;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::collections::BTreeMap;

fn binding(conn: &Connection, scope: &PersonaScopeRef) -> Result<(Digest, u64), StoreError> {
    let (i,r):(Vec<u8>,u64)=conn.query_row("SELECT incarnation_id,revision FROM active_bindings WHERE bot_token=?1 AND persona_token=?2",params![blob(scope.bot_token),blob(scope.persona_token)],|row|Ok((row.get(0)?,row.get(1)?)))?;
    if r == 0 {
        return Err(invalid("PERSONA_BINDING_INVALID"));
    }
    Ok((
        i.try_into()
            .map_err(|_| invalid("PERSONA_BINDING_INVALID"))?,
        r,
    ))
}
// Preserve the established storage/API shape in this compatibility boundary.
#[allow(clippy::type_complexity)]
fn anchor(
    conn: &Connection,
    scope: &PersonaScopeRef,
) -> Result<Option<(EmbodimentPersonaAnchorReceiptV1, Digest)>, StoreError> {
    let p = core_persona_digest(scope);
    let raw:Option<(Vec<u8>,Vec<u8>,u64,Vec<u8>,Vec<u8>)>=conn.query_row("SELECT CASE WHEN length(receipt_bytes)<=65536 THEN receipt_bytes ELSE zeroblob(0) END,receipt_digest,initial_semantic_revision,incarnation_digest,create_operation_id FROM embodiment_persona_anchor_v1 WHERE persona_scope=?1",params![blob(p)],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
    let Some((bytes, hash, revision, incarnation, operation)) = raw else {
        return Ok(None);
    };
    let receipt: EmbodimentPersonaAnchorReceiptV1 = decode(&bytes)?;
    let expected = digest(b"ae.embodiment.persona-anchor.v1", &bytes);
    if receipt.schema_version != 1
        || expected.as_slice() != hash
        || receipt.scope != *scope
        || receipt.initial_semantic_revision != revision
        || receipt.incarnation_digest.as_slice() != incarnation
        || receipt.operation_id.as_slice() != operation
    {
        return Err(invalid("PERSONA_ANCHOR_INVALID"));
    }
    let bound:bool=conn.query_row("SELECT bot_token=?2 AND persona_token=?3 AND binding_revision=?4 AND legacy_sleep_anchor_digest=?5 AND profile_digest=?6 AND schedule_digest=?7 AND initial_state_digest=?8 AND anchored_at_utc_ms=?9 AND request_digest=?10 FROM embodiment_persona_anchor_v1 WHERE persona_scope=?1",params![blob(p),blob(scope.bot_token),blob(scope.persona_token),receipt.binding_revision,blob(receipt.legacy_sleep_anchor_digest),blob(receipt.profile_digest),blob(receipt.schedule_digest),blob(receipt.initial_state_digest),receipt.anchored_at_utc_ms,blob(receipt.request_digest)],|r|r.get(0))?;
    if !bound {
        return Err(invalid("PERSONA_ANCHOR_INVALID"));
    }
    let (current, current_revision) = binding(conn, scope)?;
    if current != receipt.incarnation_digest || current_revision != receipt.binding_revision {
        return Err(invalid("PERSONA_INCARNATION_CONFLICT"));
    }
    Ok(Some((receipt, expected)))
}
fn number(row: &BTreeMap<String, Value>, name: &str) -> Result<u64, StoreError> {
    match row.get(name) {
        Some(Value::Integer(v)) if *v >= 0 => Ok(*v as u64),
        _ => Err(invalid("CLOCK_HEAD_INVALID")),
    }
}
fn bytes(row: &BTreeMap<String, Value>, name: &str) -> Result<Vec<u8>, StoreError> {
    match row.get(name) {
        Some(Value::Blob(v)) => Ok(v.clone()),
        _ => Err(invalid("CLOCK_HEAD_INVALID")),
    }
}
fn hash(row: &BTreeMap<String, Value>, name: &str) -> Result<Digest, StoreError> {
    bytes(row, name)?
        .try_into()
        .map_err(|_| invalid("CLOCK_HEAD_INVALID"))
}
fn text(row: &BTreeMap<String, Value>, name: &str) -> Result<String, StoreError> {
    match row.get(name) {
        Some(Value::Text(v)) => Ok(v.clone()),
        _ => Err(invalid("CLOCK_HEAD_INVALID")),
    }
}
fn sleep_code(s: SleepStateV1) -> &'static str {
    match s {
        SleepStateV1::Awake => "awake",
        SleepStateV1::Drowsy => "drowsy",
        SleepStateV1::Asleep => "asleep",
    }
}
fn clock_state_digest(s: &EmbodimentClockStateV1) -> Result<Digest, StoreError> {
    Ok(digest(b"ae.embodiment.clock-state.v1", &encode(s)?))
}
fn epoch_digest(e: &MatrixTimeEpochV1) -> Result<Digest, StoreError> {
    Ok(digest(b"ae.embodiment.matrix-epoch.v1", &encode(e)?))
}

fn leaf_digest(r: &EmbodimentClockResultCoreV1, core: Digest) -> Digest {
    let mut b = Vec::new();
    b.extend(r.sequence.to_le_bytes());
    b.push(r.mutation_kind.code());
    b.extend(r.operation_id);
    b.extend(r.event_id);
    b.extend(r.request_digest);
    b.extend(r.frozen_now_utc_ms.to_le_bytes());
    b.extend(r.raw_elapsed_ms.to_le_bytes());
    b.extend(r.applied_elapsed_ms.to_le_bytes());
    b.push(u8::from(r.capped_gap));
    b.extend(r.discrete_event_mask.to_le_bytes());
    b.extend(core);
    b.extend(r.resulting_state_digest);
    digest(b"ae.embodiment.clock-receipt-leaf.v1", &b)
}
fn chain_digest(prior: Digest, sequence: u64, leaf: Digest) -> Digest {
    wire::domain_hash(
        b"ae.embodiment.clock-chain-step.v1",
        &[&prior, &sequence.to_le_bytes(), &leaf],
    )
}
fn commit_digest(r: &EmbodimentClockCommitReceiptV1, bytes: &[u8]) -> Digest {
    wire::domain_hash(
        b"ae.embodiment.clock-commit-receipt.v1",
        &[
            &r.result_core_digest,
            &r.leaf_digest,
            &r.chain_digest,
            &r.head_digest,
            &(bytes.len() as u64).to_le_bytes(),
            bytes,
        ],
    )
}

fn rolling_receipts(
    conn: &Connection,
    h: &EmbodimentClockHeadV1,
) -> Result<Vec<EmbodimentClockCommitReceiptV1>, StoreError> {
    let mut stmt=conn.prepare("SELECT sequence,mutation_kind,operation_id,event_id,request_digest,frozen_now_utc_ms,raw_elapsed_ms,applied_elapsed_ms,capped_gap,discrete_event_mask,prior_chain_digest,leaf_digest,CASE WHEN length(result_core_bytes)<=65536 THEN result_core_bytes ELSE zeroblob(0) END AS result_core_bytes,result_core_digest,resulting_state_digest,CASE WHEN length(commit_receipt_bytes)<=65536 THEN commit_receipt_bytes ELSE zeroblob(0) END AS commit_receipt_bytes,commit_receipt_digest FROM embodiment_clock_receipt_v1 WHERE persona_scope=?1 ORDER BY sequence LIMIT 65")?;
    let names = stmt
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut rows = stmt.query(params![blob(h.persona_scope)])?;
    let mut result = Vec::new();
    let mut chain = h.compacted_chain_digest;
    while let Some(row) = rows.next()? {
        let row: BTreeMap<String, Value> = names
            .iter()
            .enumerate()
            .map(|(i, k)| Ok((k.clone(), row.get(i)?)))
            .collect::<Result<_, rusqlite::Error>>()?;
        let raw = bytes(&row, "commit_receipt_bytes")?;
        let receipt: EmbodimentClockCommitReceiptV1 = decode(&raw)?;
        let r = &receipt.result;
        let core_bytes = bytes(&row, "result_core_bytes")?;
        let core: EmbodimentClockResultCoreV1 = decode(&core_bytes)?;
        let sequence = h
            .compacted_count
            .checked_add(result.len() as u64 + 1)
            .ok_or(invalid("CLOCK_ROLLING_CLOSURE_INVALID"))?;
        if r != &core
            || core.schema_version != 1
            || core_persona_digest(&core.scope) != h.persona_scope
            || r.sequence != sequence
            || number(&row, "sequence")? != sequence
            || number(&row, "mutation_kind")? != u64::from(r.mutation_kind.code())
            || bytes(&row, "operation_id")? != r.operation_id
            || bytes(&row, "event_id")? != r.event_id
            || hash(&row, "request_digest")? != r.request_digest
            || number(&row, "frozen_now_utc_ms")? != r.frozen_now_utc_ms
            || number(&row, "raw_elapsed_ms")? != r.raw_elapsed_ms
            || number(&row, "applied_elapsed_ms")? != r.applied_elapsed_ms
            || number(&row, "capped_gap")? != u64::from(r.capped_gap)
            || number(&row, "discrete_event_mask")? != u64::from(r.discrete_event_mask)
            || hash(&row, "prior_chain_digest")? != chain
            || digest(b"ae.embodiment.clock-result-core.v1", &core_bytes)
                != receipt.result_core_digest
            || hash(&row, "result_core_digest")? != receipt.result_core_digest
            || hash(&row, "resulting_state_digest")? != r.resulting_state_digest
            || leaf_digest(r, receipt.result_core_digest) != receipt.leaf_digest
            || hash(&row, "leaf_digest")? != receipt.leaf_digest
            || chain_digest(chain, sequence, receipt.leaf_digest) != receipt.chain_digest
            || commit_digest(&receipt, &raw) != hash(&row, "commit_receipt_digest")?
        {
            return Err(invalid("CLOCK_ROLLING_CLOSURE_INVALID"));
        }
        chain = receipt.chain_digest;
        result.push(receipt);
    }
    if result.len() != usize::from(h.recent_count) || chain != h.recent_chain_digest {
        return Err(invalid("CLOCK_ROLLING_CLOSURE_INVALID"));
    }
    if let Some(last) = result.last() {
        if last.head_digest != h.head_digest
            || last.result.resulting_state_digest != clock_state_digest(&h.state)?
            || last.result.time_revision != h.time_revision
            || last.result.state_revision != h.state_revision
            || last.result.profile_revision != h.profile_revision
            || last.result.schedule_revision != h.schedule_revision
            || last.result.frozen_now_utc_ms != h.last_now_utc_ms
            || last.result.next_due_at_utc_ms != h.next_due_at_utc_ms
            || last.result.sleep_state != h.sleep_state
        {
            return Err(invalid("CLOCK_ROLLING_CLOSURE_INVALID"));
        }
    }
    Ok(result)
}

fn read_head(conn: &Connection, p: Digest) -> Result<Option<EmbodimentClockHeadV1>, StoreError> {
    let bounded:Option<bool>=conn.query_row("SELECT length(state_bytes) BETWEEN 1 AND 262144 AND length(matrix_epoch_bytes) BETWEEN 1 AND 4096 AND length(tzdb_release) BETWEEN 1 AND 32 FROM embodiment_clock_head_v1 WHERE persona_scope=?1",params![blob(p)],|r|r.get(0)).optional()?;
    match bounded {
        None => return Ok(None),
        Some(false) => return Err(invalid("CLOCK_HEAD_INVALID")),
        Some(true) => {}
    }
    let mut stmt = conn.prepare("SELECT * FROM embodiment_clock_head_v1 WHERE persona_scope=?1")?;
    let names: Vec<String> = stmt.column_names().into_iter().map(str::to_owned).collect();
    let row: BTreeMap<String, Value> = stmt.query_row(params![blob(p)], |r| {
        names
            .iter()
            .enumerate()
            .map(|(i, k)| Ok((k.clone(), r.get(i)?)))
            .collect()
    })?;
    let epoch: MatrixTimeEpochV1 = decode(&bytes(&row, "matrix_epoch_bytes")?)?;
    let state: EmbodimentClockStateV1 = decode(&bytes(&row, "state_bytes")?)?;
    if epoch.anchor_semantic_revision != number(&row, "matrix_anchor_semantic_revision")?
        || epoch.anchor_state_digest != hash(&row, "matrix_anchor_state_digest")?
        || epoch_digest(&epoch)? != hash(&row, "matrix_epoch_digest")?
        || clock_state_digest(&state)? != hash(&row, "state_digest")?
    {
        return Err(invalid("CLOCK_HEAD_INVALID"));
    }
    let sleep = match text(&row, "sleep_state")?.as_str() {
        "awake" => SleepStateV1::Awake,
        "drowsy" => SleepStateV1::Drowsy,
        "asleep" => SleepStateV1::Asleep,
        _ => return Err(invalid("CLOCK_HEAD_INVALID")),
    };
    let h = EmbodimentClockHeadV1 {
        schema_version: 1,
        persona_scope: hash(&row, "persona_scope")?,
        sequence: number(&row, "sequence")?,
        compacted_count: number(&row, "compacted_count")?,
        recent_count: number(&row, "recent_count")?
            .try_into()
            .map_err(|_| invalid("CLOCK_HEAD_INVALID"))?,
        time_revision: number(&row, "time_revision")?,
        state_revision: number(&row, "state_revision")?,
        profile_revision: number(&row, "profile_revision")?,
        schedule_revision: number(&row, "schedule_revision")?,
        tzdb_release: text(&row, "tzdb_release")?,
        tzdb_content_sha256: hash(&row, "tzdb_content_sha256")?,
        last_now_utc_ms: number(&row, "last_now_utc_ms")?,
        next_due_at_utc_ms: number(&row, "next_due_at_utc_ms")?,
        compacted_through_now_utc_ms: number(&row, "compacted_through_now_utc_ms")?,
        sleep_state: sleep,
        sleep_episode_ordinal: number(&row, "sleep_episode_ordinal")?,
        dream_consolidated_episode_ordinal: number(&row, "dream_consolidated_episode_ordinal")?,
        endogenous_phase_code: number(&row, "endogenous_phase_code")?
            .try_into()
            .map_err(|_| invalid("CLOCK_HEAD_INVALID"))?,
        matrix_epoch: epoch,
        matrix_anchor_graph_digest: hash(&row, "matrix_anchor_graph_digest")?,
        formula_digest: hash(&row, "formula_digest")?,
        state,
        compacted_chain_digest: hash(&row, "compacted_chain_digest")?,
        recent_chain_digest: hash(&row, "recent_chain_digest")?,
        head_digest: hash(&row, "head_digest")?,
    };
    if h.persona_scope != p
        || h.sequence
            != h.compacted_count
                .checked_add(u64::from(h.recent_count))
                .ok_or(invalid("CLOCK_HEAD_INVALID"))?
        || h.recent_count > 64
        || h.time_revision == 0
        || h.state_revision == 0
        || h.last_now_utc_ms == 0
        || h.next_due_at_utc_ms <= h.last_now_utc_ms
        || h.compacted_through_now_utc_ms > h.last_now_utc_ms
        || h.dream_consolidated_episode_ordinal > h.sleep_episode_ordinal
        || h.head_digest != h.digest_v1().map_err(|_| invalid("CLOCK_HEAD_INVALID"))?
    {
        return Err(invalid("CLOCK_HEAD_INVALID"));
    }
    rolling_receipts(conn, &h)?;
    Ok(Some(h))
}
const HEAD_COLUMNS: &[&str] = &[
    "persona_scope",
    "sequence",
    "compacted_count",
    "recent_count",
    "time_revision",
    "state_revision",
    "profile_revision",
    "schedule_revision",
    "tzdb_release",
    "tzdb_content_sha256",
    "last_now_utc_ms",
    "next_due_at_utc_ms",
    "compacted_through_now_utc_ms",
    "sleep_state",
    "sleep_episode_ordinal",
    "dream_consolidated_episode_ordinal",
    "endogenous_phase_code",
    "matrix_anchor_semantic_revision",
    "matrix_anchor_state_digest",
    "matrix_anchor_graph_digest",
    "matrix_epoch_bytes",
    "matrix_epoch_digest",
    "formula_digest",
    "state_bytes",
    "state_digest",
    "compacted_chain_digest",
    "recent_chain_digest",
    "head_digest",
];
fn head_values(h: &EmbodimentClockHeadV1) -> Result<Vec<Value>, StoreError> {
    fn n(v: u64) -> Result<Value, StoreError> {
        Ok(Value::Integer(
            v.try_into()
                .map_err(|_| invalid("CLOCK_REVISION_OVERFLOW"))?,
        ))
    }
    Ok(vec![
        Value::Blob(h.persona_scope.to_vec()),
        n(h.sequence)?,
        n(h.compacted_count)?,
        n(u64::from(h.recent_count))?,
        n(h.time_revision)?,
        n(h.state_revision)?,
        n(h.profile_revision)?,
        n(h.schedule_revision)?,
        Value::Text(h.tzdb_release.clone()),
        Value::Blob(h.tzdb_content_sha256.to_vec()),
        n(h.last_now_utc_ms)?,
        n(h.next_due_at_utc_ms)?,
        n(h.compacted_through_now_utc_ms)?,
        Value::Text(sleep_code(h.sleep_state).into()),
        n(h.sleep_episode_ordinal)?,
        n(h.dream_consolidated_episode_ordinal)?,
        n(u64::from(h.endogenous_phase_code))?,
        n(h.matrix_epoch.anchor_semantic_revision)?,
        Value::Blob(h.matrix_epoch.anchor_state_digest.to_vec()),
        Value::Blob(h.matrix_anchor_graph_digest.to_vec()),
        Value::Blob(encode(&h.matrix_epoch)?),
        Value::Blob(epoch_digest(&h.matrix_epoch)?.to_vec()),
        Value::Blob(h.formula_digest.to_vec()),
        Value::Blob(encode(&h.state)?),
        Value::Blob(clock_state_digest(&h.state)?.to_vec()),
        Value::Blob(h.compacted_chain_digest.to_vec()),
        Value::Blob(h.recent_chain_digest.to_vec()),
        Value::Blob(h.head_digest.to_vec()),
    ])
}
fn insert_head(conn: &Connection, h: &EmbodimentClockHeadV1) -> Result<(), StoreError> {
    let values = head_values(h)?;
    let marks = (1..=values.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");
    conn.execute(
        &format!(
            "INSERT INTO embodiment_clock_head_v1({}) VALUES({marks})",
            HEAD_COLUMNS.join(",")
        ),
        rusqlite::params_from_iter(values),
    )?;
    Ok(())
}
fn update_head(
    conn: &Connection,
    h: &EmbodimentClockHeadV1,
    previous_sequence: u64,
) -> Result<(), StoreError> {
    let mut values = head_values(h)?;
    values.push(Value::Integer(
        previous_sequence
            .try_into()
            .map_err(|_| invalid("CLOCK_REVISION_OVERFLOW"))?,
    ));
    let assignments = HEAD_COLUMNS
        .iter()
        .enumerate()
        .skip(1)
        .map(|(i, n)| format!("{n}=?{}", i + 1))
        .collect::<Vec<_>>()
        .join(",");
    let changed=conn.execute(&format!("UPDATE embodiment_clock_head_v1 SET {assignments} WHERE persona_scope=?1 AND sequence=?{}",values.len()),rusqlite::params_from_iter(values))?;
    if changed != 1 {
        return Err(invalid("CLOCK_ROLLING_CLOSURE_INVALID"));
    }
    Ok(())
}

fn formula(
    profile: &EmbodimentTemporalProfileV1,
    schedule: &EmbodimentSleepScheduleV1,
    semantic_formula: Digest,
) -> Result<Digest, StoreError> {
    Ok(wire::domain_hash(
        b"ae.embodiment.clock-formula.v1",
        &[
            &ae_semantic_core::matrix_time_formula_digest_v1(&semantic_formula),
            &encode(profile)?,
            &encode(schedule)?,
            b"sleep-window-homeostasis-fxp6-v1;dream=5400000;phase=250000,750000",
        ],
    ))
}
fn sleep_at(schedule: &EmbodimentSleepScheduleV1, minute: u16) -> SleepStateV1 {
    let start = schedule.preferred_sleep_local_minute;
    let end = schedule.preferred_wake_local_minute;
    let asleep = if start < end {
        minute >= start && minute < end
    } else {
        minute >= start || minute < end
    };
    if asleep {
        SleepStateV1::Asleep
    } else if (start + 1440 - minute) % 1440 <= 30 {
        SleepStateV1::Drowsy
    } else {
        SleepStateV1::Awake
    }
}
fn next_due(
    f: &FrozenPersonaTimeV1,
    schedule: &EmbodimentSleepScheduleV1,
) -> Result<u64, StoreError> {
    let local_ms = (f.now_utc_ms as i128 + i128::from(f.persona_utc_offset_seconds) * 1000)
        .rem_euclid(86400000) as u64;
    let boundaries = [
        schedule.preferred_sleep_local_minute,
        schedule.preferred_wake_local_minute,
        (schedule.preferred_sleep_local_minute + 1440 - 30) % 1440,
    ];
    let delta = boundaries
        .iter()
        .map(|m| {
            let d = (u64::from(*m) * 60000 + 86400000 - local_ms) % 86400000;
            if d == 0 {
                86400000
            } else {
                d
            }
        })
        .min()
        .unwrap_or(EMBODIMENT_MAX_HORIZON_MS);
    let mut next = f
        .now_utc_ms
        .checked_add(delta.min(EMBODIMENT_MAX_HORIZON_MS))
        .ok_or(invalid("CLOCK_TIME_OVERFLOW"))?;
    if let Some(t) = f.next_timezone_transition_utc_ms {
        next = next.min(t);
    }
    Ok(next)
}
fn current_matrix(
    conn: &Connection,
    scope: &PersonaScopeRef,
) -> Result<(u64, MatrixTimeEpochV1, NeuralField, SparseGraph, Digest), StoreError> {
    let p = core_persona_digest(scope);
    let identity =
        crate::semantic::active_identity_for_scope_tx(conn, &journal_scope(scope, [0; 16]))?;
    let formula = phase0_canonical_formula_digest_v1(&identity.formula_digest);
    let (revision, field, graph) = if let Some(origin) = crate::semantic::stored_origin(conn, p)? {
        crate::semantic::semantic_state_for_derivation_tx(conn, p, &origin, &identity)?
    } else {
        (0, identity.baseline_field, identity.baseline_graph)
    };
    let (_, epoch) = crate::semantic::semantic_cursor_epoch_v1(
        conn,
        p,
        revision,
        state_digest(&field, &formula),
    )?;
    Ok((revision, epoch, field, graph, formula))
}

fn matrix_anchor(
    conn: &Connection,
    scope: &PersonaScopeRef,
    epoch: &MatrixTimeEpochV1,
) -> Result<(NeuralField, NeuralField, SparseGraph, Digest), StoreError> {
    let identity =
        crate::semantic::active_identity_for_scope_tx(conn, &journal_scope(scope, [0; 16]))?;
    let semantic_formula = phase0_canonical_formula_digest_v1(&identity.formula_digest);
    let (field, graph) =
        if let Some(origin) = crate::semantic::stored_origin(conn, core_persona_digest(scope))? {
            crate::semantic::semantic_time_anchor_state_v1(
                conn,
                core_persona_digest(scope),
                epoch,
                &origin,
                &identity,
            )?
        } else {
            if epoch.anchor_semantic_revision != 0 {
                return Err(invalid("CLOCK_MATRIX_ANCHOR_INVALID"));
            }
            (
                identity.baseline_field.clone(),
                identity.baseline_graph.clone(),
            )
        };
    if state_digest(&field, &semantic_formula) != epoch.anchor_state_digest {
        return Err(invalid("CLOCK_MATRIX_ANCHOR_INVALID"));
    }
    Ok((field, identity.baseline_field, graph, semantic_formula))
}
fn matrix_projection(
    conn: &Connection,
    scope: &PersonaScopeRef,
    h: &EmbodimentClockHeadV1,
) -> Result<(NeuralField, SparseGraph, Digest), StoreError> {
    let (anchor, baseline, graph, semantic_formula) = matrix_anchor(conn, scope, &h.matrix_epoch)?;
    let projection =
        ae_semantic_core::advance_matrix_time_v1(ae_semantic_core::MatrixTimeInputV1 {
            anchor_field: &anchor,
            genesis_baseline: &baseline,
            epoch: &h.matrix_epoch,
            elapsed_ms: 0,
            phase: MatrixSleepPhaseV1::Awake,
            semantic_formula_digest: semantic_formula,
        })
        .map_err(|_| invalid("CLOCK_MATRIX_EPOCH_INVALID"))?;
    if graph_digest(&graph) != h.matrix_anchor_graph_digest
        || state_digest(&projection.field, &semantic_formula) != h.state.matrix_state_digest
    {
        return Err(invalid("CLOCK_MATRIX_PROJECTION_INVALID"));
    }
    Ok((projection.field, graph, semantic_formula))
}
fn authenticated_clock(
    conn: &Connection,
    scope: &PersonaScopeRef,
) -> Result<(EmbodimentClockHeadV1, EmbodimentProfileReadV1), StoreError> {
    let (anchor, anchor_digest) = anchor(conn, scope)?.ok_or(invalid("CLOCK_PERSONA_MISSING"))?;
    let h = read_head(conn, core_persona_digest(scope))?.ok_or(invalid("CLOCK_HEAD_MISSING"))?;
    let profile = profile_read(conn, scope)?;
    let (_, _, semantic_formula) = matrix_projection(conn, scope, &h)?;
    if h.formula_digest != formula(&profile.profile, &profile.schedule, semantic_formula)?
        || profile.profile_revision != h.profile_revision
        || profile.schedule_revision != h.schedule_revision
        || profile.profile.tzdb_release != h.tzdb_release
        || profile.profile.tzdb_content_sha256 != h.tzdb_content_sha256
    {
        return Err(invalid("CLOCK_PROFILE_CLOSURE_INVALID"));
    }
    if h.compacted_count == 0
        && h.compacted_chain_digest
            != wire::domain_hash(
                b"ae.embodiment.clock-chain-root.v1",
                &[&h.persona_scope, &anchor_digest],
            )
    {
        return Err(invalid("CLOCK_ROLLING_CLOSURE_INVALID"));
    }
    if h.sequence == 0
        && (h.last_now_utc_ms != anchor.anchored_at_utc_ms
            || clock_state_digest(&h.state)? != anchor.initial_state_digest
            || h.matrix_epoch.anchor_semantic_revision > anchor.initial_semantic_revision)
    {
        return Err(invalid("CLOCK_PERSONA_ANCHOR_INVALID"));
    }
    Ok((h, profile))
}

fn has_anchor(conn: &Connection, scope: &PersonaScopeRef) -> Result<bool, StoreError> {
    let installed: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='embodiment_persona_anchor_v1' AND type='table')", [], |r| r.get(0))?;
    if !installed {
        return Ok(false);
    }
    Ok(anchor(conn, scope)?.is_some())
}

pub(crate) fn semantic_input(
    conn: &Connection,
    scope: &PersonaScopeRef,
) -> Result<Option<(NeuralField, SparseGraph)>, StoreError> {
    Ok(core_clock_projection(conn, scope)?.map(|(_, field, graph)| (field, graph)))
}

pub(crate) fn core_clock_projection(
    conn: &Connection,
    scope: &PersonaScopeRef,
) -> Result<Option<(Digest, NeuralField, SparseGraph)>, StoreError> {
    if !has_anchor(conn, scope)? {
        return Ok(None);
    }
    let (h, _) = authenticated_clock(conn, scope)?;
    let (field, graph, _) = matrix_projection(conn, scope, &h)?;
    Ok(Some((h.head_digest, field, graph)))
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SemanticClockProof {
    schema_version: u16,
    scope: PersonaScopeRef,
    semantic_revision: u64,
    semantic_commitment_digest: Digest,
    old_head: EmbodimentClockHeadV1,
    profile: EmbodimentTemporalProfileV1,
    schedule: EmbodimentSleepScheduleV1,
    projected_state_digest: Digest,
}

/// Runs only within the successful semantic writer transaction. The proof is
/// independent of the bounded receipt window and does not contain a field.
pub(crate) fn commit_semantic_anchor(
    conn: &Connection,
    scope: &PersonaScopeRef,
    committed: &crate::semantic::CommittedSemanticV1,
) -> Result<(), StoreError> {
    if !has_anchor(conn, scope)? {
        return Ok(());
    }
    let semantic_receipt = wire::decode_transition_receipt(&committed.journal.receipt_bytes)
        .map_err(|_| invalid("SEMANTIC_CLOCK_INPUT_INVALID"))?;
    let (old, profile) = authenticated_clock(conn, scope)?;
    if committed.semantic_revision <= old.matrix_epoch.anchor_semantic_revision
        || committed.graph_before != old.matrix_anchor_graph_digest
        || semantic_receipt.state_before != old.state.matrix_state_digest
    {
        return Err(invalid("SEMANTIC_CLOCK_INPUT_INVALID"));
    }
    let proof = SemanticClockProof {
        schema_version: 1,
        scope: scope.clone(),
        semantic_revision: committed.semantic_revision,
        semantic_commitment_digest: committed.commitment_digest,
        projected_state_digest: old.state.matrix_state_digest,
        old_head: old.clone(),
        profile: profile.profile,
        schedule: profile.schedule,
    };
    let bytes = encode(&proof)?;
    if bytes.len() > 524288 {
        return Err(invalid("SEMANTIC_CLOCK_PROOF_TOO_LARGE"));
    }
    let proof_digest = digest(b"ae.embodiment.semantic-clock-proof.v1", &bytes);
    conn.execute("INSERT INTO semantic_clock_anchor_proof_v1(persona_scope,semantic_revision,proof_version,semantic_commitment_digest,proof_bytes,proof_digest) VALUES(?1,?2,1,?3,?4,?5)", params![blob(committed.persona_scope), committed.semantic_revision, blob(committed.commitment_digest), bytes, blob(proof_digest)])?;
    let mut h = old.clone();
    h.sequence = increment(old.sequence)?;
    h.state_revision = increment(old.state_revision)?;
    h.matrix_epoch = MatrixTimeEpochV1 {
        schema_version: 1,
        anchor_semantic_revision: committed.semantic_revision,
        anchor_state_digest: committed.state_digest,
        awake_ticks: 0,
        drowsy_ticks: 0,
        asleep_ticks: 0,
        awake_remainder_ms: 0,
        drowsy_remainder_ms: 0,
        asleep_remainder_ms: 0,
    };
    h.matrix_anchor_graph_digest = committed.graph_digest;
    h.state.matrix_state_digest = committed.state_digest;
    let op = core_id(
        b"ae.embodiment.semantic-anchor.operation-id.v1",
        &[
            &h.persona_scope,
            &committed.semantic_revision.to_le_bytes(),
            &committed.commitment_digest,
        ],
    );
    let event = core_id(
        b"ae.embodiment.semantic-anchor.event-id.v1",
        &[&op, &proof_digest],
    );
    let mut result = result_core(
        scope,
        &h,
        EmbodimentClockMutationKindV1::SemanticAnchor,
        op,
        event,
        proof_digest,
        0,
        0,
        0,
    )?;
    result.semantic_proof_digest = Some(proof_digest);
    result.semantic_commitment_digest = Some(committed.commitment_digest);
    append_clock(conn, &old, &mut h, result)?;
    Ok(())
}

/// Reconstruct before checking semantic state_before, even for a zero epoch.
pub(crate) fn semantic_proof_input(
    conn: &Connection,
    scope: &PersonaScopeRef,
    committed: &crate::semantic::CommittedSemanticV1,
) -> Result<Option<(NeuralField, SparseGraph)>, StoreError> {
    if !has_anchor(conn, scope)? {
        return Ok(None);
    }
    let (initial, _) = anchor(conn, scope)?.ok_or(invalid("PERSONA_ANCHOR_INVALID"))?;
    if committed.semantic_revision <= initial.initial_semantic_revision {
        return Ok(None);
    }
    let row: Option<(Vec<u8>, Vec<u8>, Vec<u8>)> = conn.query_row("SELECT CASE WHEN length(proof_bytes)<=524288 THEN proof_bytes ELSE zeroblob(0) END,proof_digest,semantic_commitment_digest FROM semantic_clock_anchor_proof_v1 WHERE persona_scope=?1 AND semantic_revision=?2 AND proof_version=1", params![blob(committed.persona_scope), committed.semantic_revision], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let (bytes, stored_digest, commitment) = row.ok_or(invalid("SEMANTIC_CLOCK_PROOF_MISSING"))?;
    let proof: SemanticClockProof = decode(&bytes)?;
    let semantic_receipt = wire::decode_transition_receipt(&committed.journal.receipt_bytes)
        .map_err(|_| invalid("SEMANTIC_CLOCK_INPUT_INVALID"))?;
    let h = &proof.old_head;
    // Before the first rolling semantic anchor, legacy Time rows may sit
    // between the immutable matrix anchor and the creation semantic head.
    let predecessor = if h.matrix_epoch.anchor_semantic_revision
        <= initial.initial_semantic_revision
    {
        if h.matrix_epoch.anchor_semantic_revision < initial.initial_semantic_revision {
            let creation_head = crate::semantic::read_semantic_commit(
                conn,
                committed.persona_scope,
                initial.initial_semantic_revision,
            )?;
            if creation_head.transition_kind != SemanticTransitionKindV1::Time {
                return Err(invalid("SEMANTIC_CLOCK_INITIAL_ANCHOR_INVALID"));
            }
            let epoch = ae_semantic_core::decode_time_snapshot_v1(&creation_head.snapshot_bytes)
                .map_err(|_| invalid("SEMANTIC_CLOCK_INITIAL_ANCHOR_INVALID"))?
                .epoch_after;
            let old = &h.matrix_epoch;
            let total =
                |ticks: u64, remainder: u64| u128::from(ticks) * 600000 + u128::from(remainder);
            if old.anchor_semantic_revision != epoch.anchor_semantic_revision
                || old.anchor_state_digest != epoch.anchor_state_digest
                || h.matrix_anchor_graph_digest != creation_head.graph_digest
                || total(old.awake_ticks, old.awake_remainder_ms)
                    < total(epoch.awake_ticks, epoch.awake_remainder_ms)
                || total(old.drowsy_ticks, old.drowsy_remainder_ms)
                    < total(epoch.drowsy_ticks, epoch.drowsy_remainder_ms)
                || total(old.asleep_ticks, old.asleep_remainder_ms)
                    < total(epoch.asleep_ticks, epoch.asleep_remainder_ms)
            {
                return Err(invalid("SEMANTIC_CLOCK_INITIAL_ANCHOR_INVALID"));
            }
        }
        initial.initial_semantic_revision
    } else {
        h.matrix_epoch.anchor_semantic_revision
    };
    if digest(b"ae.embodiment.semantic-clock-proof.v1", &bytes).as_slice() != stored_digest
        || commitment != committed.commitment_digest
        || proof.schema_version != 1
        || proof.scope != *scope
        || proof.semantic_revision != committed.semantic_revision
        || proof.semantic_commitment_digest != committed.commitment_digest
        || h.persona_scope != committed.persona_scope
        || h.head_digest != h.digest_v1().map_err(|_| invalid("CLOCK_HEAD_INVALID"))?
        || predecessor.checked_add(1) != Some(committed.semantic_revision)
        || proof.projected_state_digest != h.state.matrix_state_digest
        || proof.projected_state_digest != semantic_receipt.state_before
        || h.matrix_anchor_graph_digest != committed.graph_before
    {
        return Err(invalid("SEMANTIC_CLOCK_PROOF_INVALID"));
    }
    let (field, graph, sf) = matrix_projection(conn, scope, h)?;
    if !proof.profile.validate_v1()
        || !proof.schedule.validate_v1()
        || h.formula_digest != formula(&proof.profile, &proof.schedule, sf)?
    {
        return Err(invalid("SEMANTIC_CLOCK_PROOF_INVALID"));
    }
    Ok(Some((field, graph)))
}
fn cadence(h: &EmbodimentClockHeadV1) -> EmbodimentCadenceV1 {
    if h.sleep_state == SleepStateV1::Asleep {
        EmbodimentCadenceV1::Asleep
    } else if h.state.arousal.raw() >= 650000 {
        EmbodimentCadenceV1::Active
    } else {
        EmbodimentCadenceV1::Calm
    }
}
fn validate_frozen(
    profile: &EmbodimentTemporalProfileV1,
    f: &FrozenPersonaTimeV1,
) -> Result<(), StoreError> {
    if !f.validate_v1()
        || f.persona_tzid != profile.persona_tzid
        || f.tzdb_release != profile.tzdb_release
        || f.tzdb_content_sha256 != profile.tzdb_content_sha256
    {
        Err(invalid("CLOCK_FROZEN_TIME_INVALID"))
    } else {
        Ok(())
    }
}
fn shifted_schedule(s: &EmbodimentSleepScheduleV1, shift: i64) -> EmbodimentSleepScheduleV1 {
    let mut value = s.clone();
    value.preferred_sleep_local_minute =
        (i64::from(s.preferred_sleep_local_minute) + shift / 1000).rem_euclid(1440) as u16;
    value.preferred_wake_local_minute =
        (i64::from(s.preferred_wake_local_minute) + shift / 1000).rem_euclid(1440) as u16;
    value
}
fn increment(v: u64) -> Result<u64, StoreError> {
    v.checked_add(1)
        .filter(|v| *v <= i64::MAX as u64)
        .ok_or(invalid("CLOCK_REVISION_OVERFLOW"))
}

fn project_clock(
    conn: &Connection,
    scope: &PersonaScopeRef,
    old: &EmbodimentClockHeadV1,
    profile: &EmbodimentTemporalProfileV1,
    schedule: &EmbodimentSleepScheduleV1,
    f: &FrozenPersonaTimeV1,
) -> Result<(EmbodimentClockHeadV1, u64, u64, u32), StoreError> {
    if f.now_utc_ms <= old.compacted_through_now_utc_ms {
        return Err(invalid("COMPACTED_OR_STALE"));
    }
    let raw = f
        .now_utc_ms
        .checked_sub(old.last_now_utc_ms)
        .filter(|v| *v > 0)
        .ok_or(invalid("CLOCK_NOT_MONOTONIC"))?;
    let elapsed = raw.min(EMBODIMENT_MAX_HORIZON_MS);
    let mut h = old.clone();
    let effective = shifted_schedule(schedule, h.state.entrainment_shift_milliminutes);
    // Integrate only schedule/zone boundaries within the one capped interval, never
    // replay scheduler wakes. Matrix rounding happens once from the immutable anchor.
    let mut cursor = f.now_utc_ms - elapsed;
    let mut durations = [0u64; 3];
    let mut transitions = 0;
    while cursor < f.now_utc_ms {
        transitions += 1;
        if transitions > 64 {
            return Err(invalid("CLOCK_BOUNDARY_LIMIT"));
        }
        let local = freeze_embodiment_time_v1(&profile.persona_tzid, cursor)
            .map_err(|_| invalid("CLOCK_FROZEN_TIME_INVALID"))?;
        let phase = sleep_at(&effective, local.persona_local_minute);
        let end = next_due(&local, &effective)?.min(f.now_utc_ms);
        let duration = end - cursor;
        durations[match phase {
            SleepStateV1::Awake => 0,
            SleepStateV1::Drowsy => 1,
            SleepStateV1::Asleep => 2,
        }] += duration;
        let rate = if phase == SleepStateV1::Asleep {
            -profile.homeostatic_asleep_decay_per_hour.raw()
        } else {
            profile.homeostatic_awake_gain_per_hour.raw()
        };
        let scaled =
            i128::from(rate) * i128::from(duration) + i128::from(h.state.homeostatic_remainder);
        let value = i128::from(h.state.process_s.raw()) + scaled / 3600000;
        h.state.process_s = Fixed::from_raw(value.clamp(0, 1000000) as i64);
        h.state.homeostatic_remainder = if value <= 0 || value >= 1000000 {
            0
        } else {
            (scaled % 3600000) as i64
        };
        cursor = end;
    }
    let (anchor, baseline, graph, semantic_formula) =
        matrix_anchor(conn, scope, &old.matrix_epoch)?;
    let mut epoch = old.matrix_epoch.clone();
    for (phase, duration) in [
        MatrixSleepPhaseV1::Awake,
        MatrixSleepPhaseV1::Drowsy,
        MatrixSleepPhaseV1::Asleep,
    ]
    .into_iter()
    .zip(durations)
    {
        let (ticks, remainder) = match phase {
            MatrixSleepPhaseV1::Awake => (&mut epoch.awake_ticks, &mut epoch.awake_remainder_ms),
            MatrixSleepPhaseV1::Drowsy => (&mut epoch.drowsy_ticks, &mut epoch.drowsy_remainder_ms),
            MatrixSleepPhaseV1::Asleep => (&mut epoch.asleep_ticks, &mut epoch.asleep_remainder_ms),
        };
        let sum = remainder
            .checked_add(duration)
            .ok_or(invalid("CLOCK_EPOCH_OVERFLOW"))?;
        *ticks = ticks
            .checked_add(sum / 600000)
            .ok_or(invalid("CLOCK_EPOCH_OVERFLOW"))?;
        *remainder = sum % 600000;
    }
    let projected = ae_semantic_core::advance_matrix_time_v1(ae_semantic_core::MatrixTimeInputV1 {
        anchor_field: &anchor,
        genesis_baseline: &baseline,
        epoch: &epoch,
        elapsed_ms: 0,
        phase: MatrixSleepPhaseV1::Awake,
        semantic_formula_digest: semantic_formula,
    })
    .map_err(|_| invalid("CLOCK_MATRIX_EPOCH_INVALID"))?;
    h.matrix_epoch = projected.epoch;
    h.state.matrix_state_digest = state_digest(&projected.field, &semantic_formula);
    h.matrix_anchor_graph_digest = graph_digest(&graph);
    if schedule.mode == SleepScheduleModeV1::Auto {
        let desired: i64 = match schedule.chronotype {
            ChronotypeV1::Morning => -60,
            ChronotypeV1::Intermediate => 0,
            ChronotypeV1::NightOwl => 60,
        };
        let target = desired.clamp(
            -i64::from(schedule.sleep_flex_minutes),
            i64::from(schedule.sleep_flex_minutes),
        ) * 1000;
        let step =
            (u128::from(schedule.entrainment_rate_minutes_per_day) * 1000 * u128::from(elapsed)
                / 86400000) as i64;
        h.state.entrainment_shift_milliminutes +=
            (target - h.state.entrainment_shift_milliminutes).clamp(-step, step);
    } else {
        h.state.entrainment_shift_milliminutes = 0;
    }
    let effective = shifted_schedule(schedule, h.state.entrainment_shift_milliminutes);
    let mut sleep = sleep_at(&effective, f.persona_local_minute);
    if sleep == SleepStateV1::Awake
        && (h.state.process_s >= profile.drowsy_enter_threshold
            || old.sleep_state == SleepStateV1::Drowsy
                && h.state.process_s > profile.drowsy_exit_threshold)
    {
        sleep = SleepStateV1::Drowsy;
    }
    let mut mask = 0;
    if sleep != old.sleep_state {
        let rank = |s| match s {
            SleepStateV1::Awake => 0,
            SleepStateV1::Drowsy => 1,
            SleepStateV1::Asleep => 2,
        };
        mask |= if rank(sleep) > rank(old.sleep_state) {
            1
        } else {
            2
        };
    }
    if sleep == SleepStateV1::Asleep {
        if old.sleep_state != SleepStateV1::Asleep || raw >= 86400000 {
            h.sleep_episode_ordinal = increment(h.sleep_episode_ordinal)?;
            let since = (u64::from(f.persona_local_minute) + 1440
                - u64::from(effective.preferred_sleep_local_minute))
                % 1440
                * 60000;
            h.state.sleep_started_at_utc_ms = f.now_utc_ms.saturating_sub(since);
        }
        if h.dream_consolidated_episode_ordinal < h.sleep_episode_ordinal
            && f.now_utc_ms.saturating_sub(h.state.sleep_started_at_utc_ms) >= 5400000
        {
            h.dream_consolidated_episode_ordinal = h.sleep_episode_ordinal;
            mask |= 4;
        }
    } else {
        h.state.sleep_started_at_utc_ms = 0;
    }
    let phase = (i128::from(f.now_utc_ms) + i128::from(f.persona_utc_offset_seconds) * 1000)
        .rem_euclid(i128::from(profile.circadian_period_millis));
    let normalized = phase * 1000000 / i128::from(profile.circadian_period_millis);
    let triangle = 1000000 - (normalized * 2 - 1000000).abs();
    h.state.process_c = Fixed::from_raw(triangle as i64);
    h.state.circadian_phase_minutes =
        Fixed::from_raw((phase * 1440000000 / i128::from(profile.circadian_period_millis)) as i64);
    h.state.arousal = Fixed::from_raw(
        ((1000000 - h.state.process_s.raw() + h.state.process_c.raw()) / 2).clamp(0, 1000000),
    );
    let value = h.state.arousal.raw();
    let hysteresis = profile.endogenous_phase_hysteresis.raw();
    let phase_code = match old.endogenous_phase_code {
        0 if value >= 250000 + hysteresis => 1,
        1 if value < 250000 - hysteresis => 0,
        1 if value >= 750000 + hysteresis => 2,
        2 if value < 750000 - hysteresis => 1,
        v => v,
    };
    if phase_code != old.endogenous_phase_code {
        mask |= 8;
    }
    h.endogenous_phase_code = phase_code;
    h.sleep_state = sleep;
    h.last_now_utc_ms = f.now_utc_ms;
    h.next_due_at_utc_ms = next_due(f, &effective)?;
    if sleep == SleepStateV1::Asleep
        && h.dream_consolidated_episode_ordinal < h.sleep_episode_ordinal
    {
        h.next_due_at_utc_ms = h.next_due_at_utc_ms.min(
            h.state
                .sleep_started_at_utc_ms
                .saturating_add(5400000)
                .max(f.now_utc_ms + 1),
        );
    }
    h.time_revision = increment(old.time_revision)?;
    h.state_revision = increment(old.state_revision)?;
    h.sequence = increment(old.sequence)?;
    Ok((h, raw, elapsed, mask))
}

fn replay_clock(
    conn: &Connection,
    h: &EmbodimentClockHeadV1,
    operation: Id128,
    request: Digest,
) -> Result<Option<EmbodimentClockCommitOutcomeV1>, StoreError> {
    for receipt in rolling_receipts(conn, h)? {
        if receipt.result.operation_id == operation {
            if receipt.result.request_digest != request {
                return Err(invalid("IDEMPOTENCY_CONFLICT"));
            }
            return Ok(Some(EmbodimentClockCommitOutcomeV1 {
                commit_status: CoreCommitStatusV1::Existing,
                receipt,
            }));
        }
    }
    Ok(None)
}
fn append_clock(
    conn: &Connection,
    old: &EmbodimentClockHeadV1,
    h: &mut EmbodimentClockHeadV1,
    result: EmbodimentClockResultCoreV1,
) -> Result<EmbodimentClockCommitReceiptV1, StoreError> {
    let rows = rolling_receipts(conn, old)?;
    let result_bytes = encode(&result)?;
    let core_digest = digest(b"ae.embodiment.clock-result-core.v1", &result_bytes);
    if old.recent_count == 64 {
        let oldest = rows
            .first()
            .ok_or(invalid("CLOCK_ROLLING_CLOSURE_INVALID"))?;
        h.compacted_count = increment(old.compacted_count)?;
        h.compacted_chain_digest = oldest.chain_digest;
        h.compacted_through_now_utc_ms = oldest.result.frozen_now_utc_ms;
        let deleted = conn.execute(
            "DELETE FROM embodiment_clock_receipt_v1 WHERE persona_scope=?1 AND sequence=?2",
            params![blob(old.persona_scope), h.compacted_count],
        )?;
        if deleted != 1 {
            return Err(invalid("CLOCK_ROLLING_CLOSURE_INVALID"));
        }
        h.recent_count = 64;
    } else {
        h.recent_count = old.recent_count + 1;
    }
    let leaf = leaf_digest(&result, core_digest);
    let chain = chain_digest(old.recent_chain_digest, h.sequence, leaf);
    h.recent_chain_digest = chain;
    h.head_digest = h.digest_v1().map_err(|_| invalid("CLOCK_HEAD_INVALID"))?;
    let receipt = EmbodimentClockCommitReceiptV1 {
        result,
        result_core_digest: core_digest,
        leaf_digest: leaf,
        chain_digest: chain,
        head_digest: h.head_digest,
    };
    let raw = encode(&receipt)?;
    let r = &receipt.result;
    conn.execute("INSERT INTO embodiment_clock_receipt_v1(persona_scope,sequence,mutation_kind,operation_id,event_id,request_digest,frozen_now_utc_ms,raw_elapsed_ms,applied_elapsed_ms,capped_gap,discrete_event_mask,prior_chain_digest,leaf_digest,result_core_bytes,result_core_digest,resulting_state_digest,commit_receipt_bytes,commit_receipt_digest) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",params![blob(h.persona_scope),h.sequence,r.mutation_kind.code(),blob(r.operation_id),blob(r.event_id),blob(r.request_digest),r.frozen_now_utc_ms,r.raw_elapsed_ms,r.applied_elapsed_ms,r.capped_gap,r.discrete_event_mask,blob(old.recent_chain_digest),blob(leaf),result_bytes,blob(core_digest),blob(r.resulting_state_digest),raw,blob(commit_digest(&receipt,&encode(&receipt)?))])?;
    update_head(conn, h, old.sequence)?;
    rolling_receipts(conn, h)?;
    Ok(receipt)
}
// Preserve the established storage/API shape in this compatibility boundary.
#[allow(clippy::too_many_arguments)]
fn result_core(
    scope: &PersonaScopeRef,
    h: &EmbodimentClockHeadV1,
    kind: EmbodimentClockMutationKindV1,
    operation: Id128,
    event: Id128,
    request: Digest,
    raw: u64,
    elapsed: u64,
    mask: u32,
) -> Result<EmbodimentClockResultCoreV1, StoreError> {
    Ok(EmbodimentClockResultCoreV1 {
        schema_version: 1,
        mutation_kind: kind,
        scope: scope.clone(),
        operation_id: operation,
        event_id: event,
        request_digest: request,
        sequence: h.sequence,
        time_revision: h.time_revision,
        state_revision: h.state_revision,
        profile_revision: h.profile_revision,
        schedule_revision: h.schedule_revision,
        matrix_semantic_revision: h.matrix_epoch.anchor_semantic_revision,
        frozen_now_utc_ms: h.last_now_utc_ms,
        raw_elapsed_ms: raw,
        applied_elapsed_ms: elapsed,
        capped_gap: raw > elapsed,
        discrete_event_mask: mask,
        sleep_state: h.sleep_state,
        next_due_at_utc_ms: h.next_due_at_utc_ms,
        cadence_class: cadence(h),
        resulting_state_digest: clock_state_digest(&h.state)?,
        semantic_proof_digest: None,
        semantic_commitment_digest: None,
    })
}

impl Store {
    pub fn embodiment_clock_status_v1(
        &mut self,
        r: &EmbodimentClockStatusRequestV1,
    ) -> Result<EmbodimentClockStatusV1, StoreError> {
        if r.schema_version != 1 || !core_scope_valid(&r.scope) {
            return Err(invalid("INVALID_CLOCK_REQUEST"));
        }
        let tx = self
            .conn
            .as_mut()
            .ok_or(StoreError::Closed)?
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        check_core(&tx, &r.scope)?;
        let (h, profile) = authenticated_clock(&tx, &r.scope)?;
        if r.profile_revision != h.profile_revision || r.schedule_revision != h.schedule_revision {
            return Err(invalid("PROFILE_REVISION_CONFLICT"));
        }
        validate_frozen(&profile.profile, &r.frozen)?;
        if r.frozen.now_utc_ms < h.next_due_at_utc_ms {
            return Ok(EmbodimentClockStatusV1::NotDue {
                next_due_at_utc_ms: h.next_due_at_utc_ms,
                cadence_class: cadence(&h),
                clock_head_digest: h.head_digest,
            });
        }
        let mut request = EmbodimentTimeAdvanceRequestV1 {
            schema_version: 1,
            operation_id: [0; 16],
            scope: r.scope.clone(),
            profile_revision: r.profile_revision,
            schedule_revision: r.schedule_revision,
            frozen: r.frozen.clone(),
        };
        request.operation_id = request.expected_operation_id();
        let bytes = request
            .encode_wire_v1()
            .map_err(|_| invalid("CLOCK_WIRE_INVALID"))?;
        Ok(EmbodimentClockStatusV1::Due {
            request_digest: digest(b"ae.embodiment-time.request.v1", &bytes),
            exact_advance_request_bytes: bytes,
            next_due_at_utc_ms: h.next_due_at_utc_ms,
            clock_head_digest: h.head_digest,
        })
    }
    pub fn advance_embodiment_time_v1(
        &mut self,
        bytes: &[u8],
    ) -> Result<EmbodimentClockCommitOutcomeV1, StoreError> {
        let r = EmbodimentTimeAdvanceRequestV1::decode_wire_v1(bytes)
            .map_err(|_| invalid("CLOCK_WIRE_INVALID"))?;
        if r.schema_version != 1
            || !core_scope_valid(&r.scope)
            || r.operation_id != r.expected_operation_id()
            || r.encode_wire_v1()
                .map_err(|_| invalid("CLOCK_WIRE_INVALID"))?
                != bytes
        {
            return Err(invalid("INVALID_CLOCK_REQUEST"));
        }
        let request = digest(b"ae.embodiment-time.request.v1", bytes);
        let p = core_persona_digest(&r.scope);
        let tx = self
            .conn
            .as_mut()
            .ok_or(StoreError::Closed)?
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_core(&tx, &r.scope)?;
        anchor(&tx, &r.scope)?.ok_or(invalid("CLOCK_PERSONA_MISSING"))?;
        let head = read_head(&tx, p)?.ok_or(invalid("CLOCK_HEAD_MISSING"))?;
        if let Some(outcome) = replay_clock(&tx, &head, r.operation_id, request)? {
            return Ok(outcome);
        }
        let (old, profile) = authenticated_clock(&tx, &r.scope)?;
        if r.profile_revision != old.profile_revision
            || r.schedule_revision != old.schedule_revision
        {
            return Err(invalid("PROFILE_REVISION_CONFLICT"));
        }
        validate_frozen(&profile.profile, &r.frozen)?;
        let (mut h, raw, elapsed, mask) = project_clock(
            &tx,
            &r.scope,
            &old,
            &profile.profile,
            &profile.schedule,
            &r.frozen,
        )?;
        let event = core_id(
            b"ae.embodiment-time.event-id.v1",
            &[&p, &r.operation_id, &request],
        );
        let result = result_core(
            &r.scope,
            &h,
            EmbodimentClockMutationKindV1::Advance,
            r.operation_id,
            event,
            request,
            raw,
            elapsed,
            mask,
        )?;
        let receipt = append_clock(&tx, &old, &mut h, result)?;
        tx.commit()?;
        Ok(EmbodimentClockCommitOutcomeV1 {
            commit_status: CoreCommitStatusV1::Committed,
            receipt,
        })
    }
    pub fn compare_and_swap_embodiment_profile_v1(
        &mut self,
        r: &CompareAndSwapEmbodimentProfileV1,
    ) -> Result<EmbodimentClockCommitOutcomeV1, StoreError> {
        let p = core_persona_digest(&r.scope);
        let profile_bytes = encode(&r.replacement_profile)?;
        let schedule_bytes = encode(&r.replacement_schedule)?;
        let pd = digest(b"ae.embodiment.profile.v1", &profile_bytes);
        let sd = digest(b"ae.embodiment.sleep-schedule.v1", &schedule_bytes);
        let op = core_id(
            b"ae.embodiment.profile-cas.operation-id.v1",
            &[
                &p,
                &r.expected_profile_revision.to_le_bytes(),
                &r.expected_schedule_revision.to_le_bytes(),
                &pd,
                &sd,
            ],
        );
        if r.schema_version != 1 || !core_scope_valid(&r.scope) || op != r.operation_id {
            return Err(invalid("INVALID_PROFILE_CAS"));
        }
        let request = digest(b"ae.embodiment.profile-cas.request.v1", &encode(r)?);
        let tx = self
            .conn
            .as_mut()
            .ok_or(StoreError::Closed)?
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_core(&tx, &r.scope)?;
        anchor(&tx, &r.scope)?.ok_or(invalid("CLOCK_PERSONA_MISSING"))?;
        let head = read_head(&tx, p)?.ok_or(invalid("CLOCK_HEAD_MISSING"))?;
        if let Some(outcome) = replay_clock(&tx, &head, op, request)? {
            return Ok(outcome);
        }
        let (old, profile) = authenticated_clock(&tx, &r.scope)?;
        if old.profile_revision != r.expected_profile_revision
            || old.schedule_revision != r.expected_schedule_revision
        {
            return Err(invalid("PROFILE_REVISION_CONFLICT"));
        }
        if !r.replacement_profile.validate_v1() || !r.replacement_schedule.validate_v1() {
            return Err(invalid("INVALID_EMBODIMENT_PROFILE"));
        }
        validate_frozen(&r.replacement_profile, &r.frozen)?;
        // Elapsed life belongs to the old profile. The new zone takes effect at
        // the CAS instant, with no retroactive reinterpretation of past sleep.
        let old_frozen =
            freeze_embodiment_time_v1(&profile.profile.persona_tzid, r.frozen.now_utc_ms)
                .map_err(|_| invalid("CLOCK_FROZEN_TIME_INVALID"))?;
        let (mut h, raw, elapsed, mut mask) = project_clock(
            &tx,
            &r.scope,
            &old,
            &profile.profile,
            &profile.schedule,
            &old_frozen,
        )?;
        h.profile_revision = increment(old.profile_revision)?;
        h.schedule_revision = increment(old.schedule_revision)?;
        h.tzdb_release = r.replacement_profile.tzdb_release.clone();
        h.tzdb_content_sha256 = r.replacement_profile.tzdb_content_sha256;
        let (_, _, sf) = matrix_projection(&tx, &r.scope, &h)?;
        h.formula_digest = formula(&r.replacement_profile, &r.replacement_schedule, sf)?;
        h.next_due_at_utc_ms = next_due(
            &r.frozen,
            &shifted_schedule(
                &r.replacement_schedule,
                h.state.entrainment_shift_milliminutes,
            ),
        )?;
        mask |= 16;
        let changed=tx.execute("UPDATE embodiment_profile_v1 SET revision=?2,persona_tzid=?3,tzdb_release=?4,tzdb_content_sha256=?5,profile_bytes=?6,profile_digest=?7 WHERE persona_scope=?1 AND revision=?8",params![blob(p),h.profile_revision,r.replacement_profile.persona_tzid,r.replacement_profile.tzdb_release,blob(r.replacement_profile.tzdb_content_sha256),profile_bytes,blob(pd),old.profile_revision])?;
        if changed != 1 {
            return Err(invalid("PROFILE_REVISION_CONFLICT"));
        }
        let mode = match r.replacement_schedule.mode {
            SleepScheduleModeV1::Auto => "auto",
            SleepScheduleModeV1::Fixed => "fixed",
        };
        let changed=tx.execute("UPDATE embodiment_sleep_schedule_v1 SET revision=?2,profile_revision=?3,mode=?4,schedule_bytes=?5,schedule_digest=?6 WHERE persona_scope=?1 AND revision=?7",params![blob(p),h.schedule_revision,h.profile_revision,mode,schedule_bytes,blob(sd),old.schedule_revision])?;
        if changed != 1 {
            return Err(invalid("PROFILE_REVISION_CONFLICT"));
        }
        let event = core_id(
            b"ae.embodiment.profile-cas.event-id.v1",
            &[&p, &op, &request],
        );
        let result = result_core(
            &r.scope,
            &h,
            EmbodimentClockMutationKindV1::ProfileCas,
            op,
            event,
            request,
            raw,
            elapsed,
            mask,
        )?;
        let receipt = append_clock(&tx, &old, &mut h, result)?;
        tx.commit()?;
        Ok(EmbodimentClockCommitOutcomeV1 {
            commit_status: CoreCommitStatusV1::Committed,
            receipt,
        })
    }
}

impl Store {
    pub fn get_embodiment_persona_v1(
        &mut self,
        scope: &PersonaScopeRef,
    ) -> Result<EmbodimentPersonaLookupV1, StoreError> {
        let tx = self
            .conn
            .as_mut()
            .ok_or(StoreError::Closed)?
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        check_core(&tx, scope)?;
        let (incarnation, _) = binding(&tx, scope)?;
        let Some((first_creation_receipt, _)) = anchor(&tx, scope)? else {
            return Ok(EmbodimentPersonaLookupV1::Missing {
                incarnation_digest: incarnation,
            });
        };
        let h = read_head(&tx, core_persona_digest(scope))?.ok_or(invalid("CLOCK_HEAD_MISSING"))?;
        let profile = profile_read(&tx, scope)?;
        if profile.profile_revision != h.profile_revision
            || profile.schedule_revision != h.schedule_revision
        {
            return Err(invalid("CLOCK_PROFILE_CLOSURE_INVALID"));
        }
        Ok(EmbodimentPersonaLookupV1::Present {
            first_creation_receipt,
            incarnation_digest: incarnation,
            profile_revision: h.profile_revision,
            schedule_revision: h.schedule_revision,
            clock_head: h,
        })
    }
    pub fn create_embodiment_persona_if_missing_v1(
        &mut self,
        r: &CreateEmbodimentPersonaIfMissingV1,
    ) -> Result<EmbodimentPersonaCreateOutcomeV1, StoreError> {
        if r.schema_version != 1 || !core_scope_valid(&r.scope) || r.incarnation_digest == [0; 32] {
            return Err(invalid("INVALID_PERSONA_CREATE"));
        }
        let p = core_persona_digest(&r.scope);
        if r.operation_id
            != core_id(
                b"ae.embodiment.create.operation-id.v1",
                &[&p, &r.incarnation_digest],
            )
        {
            return Err(invalid("INVALID_OPERATION_ID"));
        }
        let tx = self
            .conn
            .as_mut()
            .ok_or(StoreError::Closed)?
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_core(&tx, &r.scope)?;
        let (incarnation, revision) = binding(&tx, &r.scope)?;
        if incarnation != r.incarnation_digest {
            return Err(invalid("PERSONA_INCARNATION_CONFLICT"));
        }
        if let Some((receipt, _)) = anchor(&tx, &r.scope)? {
            return Ok(EmbodimentPersonaCreateOutcomeV1 {
                commit_status: CoreCommitStatusV1::Existing,
                first_creation_receipt: receipt,
            });
        }
        if !r.profile_template.validate_v1()
            || !r.sleep_schedule.validate_v1()
            || !r.frozen_creation.validate_v1()
            || r.profile_template.persona_tzid != r.frozen_creation.persona_tzid
            || r.profile_template.tzdb_release != r.frozen_creation.tzdb_release
            || r.profile_template.tzdb_content_sha256 != r.frozen_creation.tzdb_content_sha256
        {
            return Err(invalid("INVALID_EMBODIMENT_PROFILE"));
        }
        let profile_bytes = encode(&r.profile_template)?;
        let schedule_bytes = encode(&r.sleep_schedule)?;
        let profile_digest = digest(b"ae.embodiment.profile.v1", &profile_bytes);
        let schedule_digest = digest(b"ae.embodiment.sleep-schedule.v1", &schedule_bytes);
        let (semantic_revision, epoch, field, graph, semantic_formula) =
            current_matrix(&tx, &r.scope)?;
        let legacy:Option<String>=tx.query_row("SELECT CASE WHEN length(CAST(body_json AS BLOB))<=262144 THEN body_json ELSE '' END FROM autonomous_runtime_state WHERE persona_scope=?1",params![blob(p)],|row|row.get(0)).optional()?;
        let sleep = sleep_at(&r.sleep_schedule, r.frozen_creation.persona_local_minute);
        let mut state = EmbodimentClockStateV1 {
            schema_version: 1,
            process_s: Fixed::from_raw(250000),
            process_c: Fixed::from_raw(500000),
            arousal: Fixed::from_raw(500000),
            circadian_phase_minutes: Fixed::from_raw(
                i64::from(r.frozen_creation.persona_local_minute) * 1_000_000,
            ),
            sleep_started_at_utc_ms: if sleep == SleepStateV1::Asleep {
                r.frozen_creation.now_utc_ms
            } else {
                0
            },
            entrainment_shift_milliminutes: 0,
            homeostatic_remainder: 0,
            matrix_state_digest: state_digest(&field, &semantic_formula),
        };
        let legacy_sleep_anchor_digest = if let Some(body) = legacy {
            let old: AutonomousRuntimeStateV1 =
                serde_json::from_str(&body).map_err(|_| invalid("LEGACY_SLEEP_ANCHOR_INVALID"))?;
            if old.schema_version != 1
                || old.persona_scope != p
                || [old.process_s, old.process_c, old.arousal]
                    .iter()
                    .any(|v| !(0..=1000000).contains(&v.raw()))
            {
                return Err(invalid("LEGACY_SLEEP_ANCHOR_INVALID"));
            }
            state.process_s = old.process_s;
            state.process_c = old.process_c;
            state.arousal = old.arousal;
            digest(b"ae.embodiment.legacy-sleep-anchor.v1", body.as_bytes())
        } else {
            digest(b"ae.embodiment.legacy-sleep-missing.v1", &p)
        };
        let receipt = EmbodimentPersonaAnchorReceiptV1 {
            schema_version: 1,
            scope: r.scope.clone(),
            operation_id: r.operation_id,
            incarnation_digest: incarnation,
            binding_revision: revision,
            initial_semantic_revision: semantic_revision,
            anchored_at_utc_ms: r.frozen_creation.now_utc_ms,
            legacy_sleep_anchor_digest,
            profile_digest,
            schedule_digest,
            initial_state_digest: clock_state_digest(&state)?,
            request_digest: digest(b"ae.embodiment.create.request.v1", &encode(r)?),
        };
        let receipt_bytes = encode(&receipt)?;
        let receipt_digest = digest(b"ae.embodiment.persona-anchor.v1", &receipt_bytes);
        let chain = wire::domain_hash(b"ae.embodiment.clock-chain-root.v1", &[&p, &receipt_digest]);
        let mut head = EmbodimentClockHeadV1 {
            schema_version: 1,
            persona_scope: p,
            sequence: 0,
            compacted_count: 0,
            recent_count: 0,
            time_revision: 1,
            state_revision: 1,
            profile_revision: 1,
            schedule_revision: 1,
            tzdb_release: r.profile_template.tzdb_release.clone(),
            tzdb_content_sha256: r.profile_template.tzdb_content_sha256,
            last_now_utc_ms: r.frozen_creation.now_utc_ms,
            next_due_at_utc_ms: next_due(&r.frozen_creation, &r.sleep_schedule)?,
            compacted_through_now_utc_ms: 0,
            sleep_state: sleep,
            sleep_episode_ordinal: u64::from(sleep == SleepStateV1::Asleep),
            dream_consolidated_episode_ordinal: 0,
            endogenous_phase_code: 0,
            matrix_epoch: epoch,
            matrix_anchor_graph_digest: graph_digest(&graph),
            formula_digest: formula(&r.profile_template, &r.sleep_schedule, semantic_formula)?,
            state,
            compacted_chain_digest: chain,
            recent_chain_digest: chain,
            head_digest: [0; 32],
        };
        head.head_digest = head
            .digest_v1()
            .map_err(|_| invalid("CLOCK_HEAD_INVALID"))?;
        tx.execute("INSERT INTO embodiment_persona_anchor_v1(persona_scope,bot_token,persona_token,create_operation_id,binding_revision,initial_semantic_revision,incarnation_digest,legacy_sleep_anchor_digest,profile_digest,schedule_digest,initial_state_digest,anchored_at_utc_ms,request_digest,receipt_bytes,receipt_digest) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",params![blob(p),blob(r.scope.bot_token),blob(r.scope.persona_token),blob(r.operation_id),revision,semantic_revision,blob(incarnation),blob(legacy_sleep_anchor_digest),blob(profile_digest),blob(schedule_digest),blob(receipt.initial_state_digest),receipt.anchored_at_utc_ms,blob(receipt.request_digest),receipt_bytes,blob(receipt_digest)])?;
        tx.execute(
            "INSERT INTO embodiment_profile_v1 VALUES(?1,1,1,?2,?3,?4,?5,?6)",
            params![
                blob(p),
                r.profile_template.persona_tzid,
                r.profile_template.tzdb_release,
                blob(r.profile_template.tzdb_content_sha256),
                profile_bytes,
                blob(profile_digest)
            ],
        )?;
        let mode = match r.sleep_schedule.mode {
            SleepScheduleModeV1::Auto => "auto",
            SleepScheduleModeV1::Fixed => "fixed",
        };
        tx.execute(
            "INSERT INTO embodiment_sleep_schedule_v1 VALUES(?1,1,1,1,?2,?3,?4)",
            params![blob(p), mode, schedule_bytes, blob(schedule_digest)],
        )?;
        insert_head(&tx, &head)?;
        tx.commit()?;
        Ok(EmbodimentPersonaCreateOutcomeV1 {
            commit_status: CoreCommitStatusV1::Committed,
            first_creation_receipt: receipt,
        })
    }
}

fn inventory_head(conn: &Connection) -> Result<(u64, u64, Digest), StoreError> {
    let (epoch,count,schema,state):(u64,u64,Vec<u8>,String)=conn.query_row("SELECT i.epoch,i.entry_count,c.schema_digest,c.state FROM embodiment_inventory_head_v1 i JOIN core_boundary_control_v1 c ON c.singleton=i.singleton WHERE i.singleton=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    if state != "applied" || schema.len() != 32 || epoch == 0 {
        return Err(invalid("CORE_BOUNDARY_NOT_APPLIED"));
    }
    let token = wire::domain_hash(
        b"ae.embodiment-inventory.snapshot.v1",
        &[&schema, &epoch.to_le_bytes(), &count.to_le_bytes()],
    );
    Ok((epoch, count, token))
}
// Preserve the established storage/API shape in this compatibility boundary.
#[allow(clippy::type_complexity)]
pub(crate) fn profile_read(
    conn: &Connection,
    scope: &PersonaScopeRef,
) -> Result<EmbodimentProfileReadV1, StoreError> {
    let p = core_persona_digest(scope);
    let raw:(u64,Vec<u8>,Vec<u8>,String,String,Vec<u8>,u64,u64,Vec<u8>,Vec<u8>,String)=conn.query_row("SELECT p.revision,p.profile_bytes,p.profile_digest,p.persona_tzid,p.tzdb_release,p.tzdb_content_sha256,s.revision,s.profile_revision,s.schedule_bytes,s.schedule_digest,s.mode FROM embodiment_profile_v1 p JOIN embodiment_sleep_schedule_v1 s ON s.persona_scope=p.persona_scope WHERE p.persona_scope=?1",params![blob(p)],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?)))?;
    if raw.1.len() > 16384 || raw.8.len() > 4096 || raw.0 == 0 || raw.6 == 0 || raw.0 != raw.7 {
        return Err(invalid("EMBODIMENT_PROFILE_CLOSURE_INVALID"));
    }
    let profile: EmbodimentTemporalProfileV1 = decode(&raw.1)?;
    let schedule: EmbodimentSleepScheduleV1 = decode(&raw.8)?;
    let mode = match schedule.mode {
        SleepScheduleModeV1::Auto => "auto",
        SleepScheduleModeV1::Fixed => "fixed",
    };
    if !profile.validate_v1()
        || !schedule.validate_v1()
        || digest(b"ae.embodiment.profile.v1", &raw.1).as_slice() != raw.2
        || digest(b"ae.embodiment.sleep-schedule.v1", &raw.8).as_slice() != raw.9
        || profile.persona_tzid != raw.3
        || profile.tzdb_release != raw.4
        || profile.tzdb_content_sha256.as_slice() != raw.5
        || mode != raw.10
    {
        return Err(invalid("EMBODIMENT_PROFILE_CLOSURE_INVALID"));
    }
    Ok(EmbodimentProfileReadV1 {
        profile,
        schedule,
        profile_revision: raw.0,
        schedule_revision: raw.6,
    })
}
impl Store {
    pub fn list_embodiment_personas_v1(
        &mut self,
        r: &ListEmbodimentPersonasV1,
    ) -> Result<EmbodimentPersonaInventoryPageV1, StoreError> {
        if r.schema_version != 1
            || !(1..=64).contains(&r.limit)
            || r.after.is_some() && r.snapshot_token.is_none()
        {
            return Err(invalid("INVALID_INVENTORY_REQUEST"));
        }
        let tx = self
            .conn
            .as_mut()
            .ok_or(StoreError::Closed)?
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        if crate::core_boundary_v9::preflight(&tx)? != crate::core_boundary_v9::OpenRoute::V9 {
            return Err(invalid("V9_REQUIRED"));
        }
        let schema = crate::core_boundary_v9::render_schema()?;
        crate::core_boundary_v9::verify_schema_catalog(
            &crate::core_boundary_v9::bounded_catalog(&tx)?,
            &schema,
        )?;
        let (epoch, entry_count, token) = inventory_head(&tx)?;
        if r.snapshot_token.is_some_and(|t| t != token) {
            return Err(invalid("INVENTORY_CHANGED"));
        }
        let mut entries = Vec::new();
        {
            let mut statement=tx.prepare("SELECT bot_token,persona_token,revision FROM active_bindings WHERE ?1=0 OR (bot_token,persona_token)>(?2,?3) ORDER BY bot_token,persona_token LIMIT ?4")?;
            let after = r.after.as_ref();
            let mut rows = statement.query(params![
                after.is_some(),
                after.map(|s| s.bot_token.to_vec()).unwrap_or_default(),
                after.map(|s| s.persona_token.to_vec()).unwrap_or_default(),
                u64::from(r.limit) + 1
            ])?;
            while let Some(row) = rows.next()? {
                let b: Vec<u8> = row.get(0)?;
                let p: Vec<u8> = row.get(1)?;
                let revision: u64 = row.get(2)?;
                let scope = PersonaScopeRef {
                    bot_token: b
                        .try_into()
                        .map_err(|_| invalid("INVENTORY_BINDING_INVALID"))?,
                    persona_token: p
                        .try_into()
                        .map_err(|_| invalid("INVENTORY_BINDING_INVALID"))?,
                };
                if revision == 0 || !core_scope_valid(&scope) {
                    return Err(invalid("INVENTORY_BINDING_INVALID"));
                }
                entries.push(EmbodimentPersonaInventoryEntryV1 {
                    persona_scope: core_persona_digest(&scope),
                    scope,
                    binding_revision: revision,
                });
            }
        }
        let next_after = if entries.len() > usize::from(r.limit) {
            entries.pop();
            entries.last().map(|e| e.scope.clone())
        } else {
            None
        };
        if inventory_head(&tx)? != (epoch, entry_count, token) {
            return Err(invalid("INVENTORY_CHANGED"));
        }
        Ok(EmbodimentPersonaInventoryPageV1 {
            entries,
            snapshot_token: token,
            epoch,
            entry_count,
            next_after,
        })
    }
    pub fn read_embodiment_profile_v1(
        &mut self,
        scope: &PersonaScopeRef,
    ) -> Result<EmbodimentProfileReadV1, StoreError> {
        let tx = self
            .conn
            .as_mut()
            .ok_or(StoreError::Closed)?
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        check_core(&tx, scope)?;
        profile_read(&tx, scope)
    }
}
