//! Closed semantic snapshot and frozen AESEM2 writer verifier used by the
//! Store authority boundary.  This deliberately duplicates the narrow
//! predecessor algorithm instead of accepting a runtime-supplied summary.

use crate::{
    LegacySemanticFieldDomainUpgradeV1, StoreError, JOINT_MAX_LINEAR_FXP6_V1,
    LEGACY_FIELD_FXP6_SCALE,
};
use ae_attention::r7::assemble_full_vector_load;
use ae_contracts::{
    legacy_reserved_zero_digest_v1, wire, CommitStatus, Digest, EvidenceVector,
    NativeTelemetryReceiptV1, TransitionReceipt, TransitionReceiptV2,
};
use ae_fixed::Fixed;
use ae_neurofield::{
    graph_digest, state_digest, NeuralField, SparseGraph, Synapse, EDGE_CAPACITY, NEURON_SLOTS,
    REGION_LAYOUT,
};

const AESEM2_MAGIC: &[u8] = b"AESEM2\0";
const AESEM2_SCHEMA: u16 = 2;
const AESEM3_MAGIC: &[u8] = b"AESEM3\0";
const AESEM3_SCHEMA: u16 = 3;
const LEGACY_NEUTRAL_RELAXATION_MAX_RATE: Fixed = Fixed::from_raw(125_000);

pub(crate) struct DecodedSemanticSnapshotV2 {
    pub(crate) field: NeuralField,
    pub(crate) graph: SparseGraph,
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], StoreError> {
        let end = self
            .position
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(StoreError::ContinuityFence("semantic_snapshot_wire"))?;
        let value = &self.bytes[self.position..end];
        self.position = end;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16, StoreError> {
        let mut value = [0_u8; 2];
        value.copy_from_slice(self.take(2)?);
        Ok(u16::from_le_bytes(value))
    }

    fn u32(&mut self) -> Result<u32, StoreError> {
        let mut value = [0_u8; 4];
        value.copy_from_slice(self.take(4)?);
        Ok(u32::from_le_bytes(value))
    }

    fn fixed(&mut self) -> Result<Fixed, StoreError> {
        let mut value = [0_u8; 8];
        value.copy_from_slice(self.take(8)?);
        Ok(Fixed::decode(value))
    }

    fn eof(&self) -> bool {
        self.position == self.bytes.len()
    }
}

fn encode_field(field: &NeuralField) -> Result<Vec<u8>, StoreError> {
    if !field.validate() {
        return Err(StoreError::ContinuityFence("semantic_field_shape"));
    }
    let mut out = Vec::with_capacity(8 * (4 + NEURON_SLOTS * 8));
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
        out.extend_from_slice(
            &(u32::try_from(values.len())
                .map_err(|_| StoreError::ContinuityFence("semantic_field_shape"))?)
            .to_le_bytes(),
        );
        for value in values {
            out.extend_from_slice(&value.encode());
        }
    }
    Ok(out)
}

fn decode_field(bytes: &[u8]) -> Result<NeuralField, StoreError> {
    let mut cursor = Cursor::new(bytes);
    let mut vectors = Vec::with_capacity(8);
    for _ in 0..8 {
        if usize::try_from(cursor.u32()?)
            .map_err(|_| StoreError::ContinuityFence("semantic_field_shape"))?
            != NEURON_SLOTS
        {
            return Err(StoreError::ContinuityFence("semantic_field_shape"));
        }
        let mut values = Vec::with_capacity(NEURON_SLOTS);
        for _ in 0..NEURON_SLOTS {
            values.push(cursor.fixed()?);
        }
        vectors.push(values);
    }
    if !cursor.eof() {
        return Err(StoreError::ContinuityFence("semantic_field_wire"));
    }
    let mut vectors = vectors.into_iter();
    let field = NeuralField {
        potential: vectors
            .next()
            .ok_or(StoreError::ContinuityFence("semantic_field_shape"))?,
        excitation: vectors
            .next()
            .ok_or(StoreError::ContinuityFence("semantic_field_shape"))?,
        inhibition: vectors
            .next()
            .ok_or(StoreError::ContinuityFence("semantic_field_shape"))?,
        adaptation: vectors
            .next()
            .ok_or(StoreError::ContinuityFence("semantic_field_shape"))?,
        precision: vectors
            .next()
            .ok_or(StoreError::ContinuityFence("semantic_field_shape"))?,
        prediction_error: vectors
            .next()
            .ok_or(StoreError::ContinuityFence("semantic_field_shape"))?,
        eligibility: vectors
            .next()
            .ok_or(StoreError::ContinuityFence("semantic_field_shape"))?,
        metabolic_reserve: vectors
            .next()
            .ok_or(StoreError::ContinuityFence("semantic_field_shape"))?,
    };
    if !field.validate() || encode_field(&field)? != bytes {
        return Err(StoreError::ContinuityFence("semantic_field_canonical"));
    }
    Ok(field)
}

fn encode_graph(graph: &SparseGraph) -> Result<Vec<u8>, StoreError> {
    if !graph.validate() {
        return Err(StoreError::ContinuityFence("semantic_graph_shape"));
    }
    Ok(graph.canonical_bytes())
}

fn decode_graph(bytes: &[u8]) -> Result<SparseGraph, StoreError> {
    let mut cursor = Cursor::new(bytes);
    let offsets_len = usize::try_from(cursor.u32()?)
        .map_err(|_| StoreError::ContinuityFence("semantic_graph_shape"))?;
    if offsets_len != NEURON_SLOTS + 1 {
        return Err(StoreError::ContinuityFence("semantic_graph_shape"));
    }
    let mut row_offsets = Vec::with_capacity(offsets_len);
    for _ in 0..offsets_len {
        row_offsets.push(cursor.u32()?);
    }
    let edge_len = usize::try_from(cursor.u32()?)
        .map_err(|_| StoreError::ContinuityFence("semantic_graph_shape"))?;
    if edge_len > EDGE_CAPACITY {
        return Err(StoreError::ContinuityFence("semantic_graph_shape"));
    }
    let mut edges = Vec::with_capacity(edge_len);
    for _ in 0..edge_len {
        let target = cursor.u32()?;
        let mut weight = [0_u8; 2];
        weight.copy_from_slice(cursor.take(2)?);
        let mut eligibility = [0_u8; 2];
        eligibility.copy_from_slice(cursor.take(2)?);
        let mut stability = [0_u8; 2];
        stability.copy_from_slice(cursor.take(2)?);
        let mut last_used_epoch = [0_u8; 2];
        last_used_epoch.copy_from_slice(cursor.take(2)?);
        let operator_id = cursor.take(1)?[0];
        let delay_class = cursor.take(1)?[0];
        let mut flags = [0_u8; 2];
        flags.copy_from_slice(cursor.take(2)?);
        edges.push(Synapse {
            target,
            weight: i16::from_le_bytes(weight),
            eligibility: i16::from_le_bytes(eligibility),
            stability: u16::from_le_bytes(stability),
            last_used_epoch: u16::from_le_bytes(last_used_epoch),
            operator_id,
            delay_class,
            flags: u16::from_le_bytes(flags),
        });
    }
    if !cursor.eof() {
        return Err(StoreError::ContinuityFence("semantic_graph_wire"));
    }
    let graph = SparseGraph { row_offsets, edges };
    if !graph.validate() || encode_graph(&graph)? != bytes {
        return Err(StoreError::ContinuityFence("semantic_graph_canonical"));
    }
    Ok(graph)
}

fn semantic_v2_matches_legacy_receipt(
    vector: &TransitionReceiptV2,
    legacy: &TransitionReceipt,
) -> bool {
    vector.schema_version == TransitionReceiptV2::SCHEMA_VERSION
        && vector.formula_digest == legacy.formula_digest
        && vector.scope_digest == legacy.scope_digest
        && vector.event_digest == legacy.event_digest
        && vector.authority_digest == legacy.authority_digest
        && vector.base_revision == legacy.base_revision
        && vector.next_revision == legacy.next_revision
        && vector.state_before == legacy.state_before
        && vector.state_after == legacy.state_after
        && vector.graph_after == legacy.graph_after
        && vector.action_contract == legacy.action_contract
        && vector.active_nodes == legacy.active_nodes
        && vector.active_edges == legacy.active_edges
        && vector.residuals == legacy.residuals
        && vector.status == legacy.status
}

fn semantic_v3_matches_legacy_receipt(
    telemetry: &NativeTelemetryReceiptV1,
    legacy: &TransitionReceipt,
) -> bool {
    legacy.schema_version == 1
        && legacy.status == CommitStatus::Committed
        && legacy.action_contract.is_none()
        && telemetry.validate()
        && telemetry.formula_digest == legacy.formula_digest
        && telemetry.scope_digest == legacy.scope_digest
        && telemetry.event_digest == legacy.event_digest
        && telemetry.base_revision == legacy.base_revision
        && telemetry.next_revision == legacy.next_revision
        && telemetry.state_before == legacy.state_before
        && telemetry.state_after == legacy.state_after
        && telemetry.graph_after == legacy.graph_after
        && telemetry.residuals == legacy.residuals
}

fn encode_snapshot_v2(
    field: &NeuralField,
    graph: &SparseGraph,
    receipt: &TransitionReceiptV2,
) -> Result<Vec<u8>, StoreError> {
    let field = encode_field(field)?;
    let graph = encode_graph(graph)?;
    let receipt = wire::encode_transition_receipt_v2(receipt);
    let mut out =
        Vec::with_capacity(AESEM2_MAGIC.len() + 2 + 12 + field.len() + graph.len() + receipt.len());
    out.extend_from_slice(AESEM2_MAGIC);
    out.extend_from_slice(&AESEM2_SCHEMA.to_le_bytes());
    for block in [&field, &graph, &receipt] {
        out.extend_from_slice(
            &(u32::try_from(block.len())
                .map_err(|_| StoreError::ContinuityFence("semantic_snapshot_wire"))?)
            .to_le_bytes(),
        );
        out.extend_from_slice(block);
    }
    Ok(out)
}

pub(crate) fn decode_semantic_snapshot_v2(
    bytes: &[u8],
    expected_formula_digest: &Digest,
    expected_state_digest: &Digest,
    expected_graph_digest: &Digest,
    legacy_receipt: &TransitionReceipt,
) -> Result<DecodedSemanticSnapshotV2, StoreError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take(AESEM2_MAGIC.len())? != AESEM2_MAGIC || cursor.u16()? != AESEM2_SCHEMA {
        return Err(StoreError::ContinuityFence("semantic_aesem2_magic"));
    }
    let field_len = usize::try_from(cursor.u32()?)
        .map_err(|_| StoreError::ContinuityFence("semantic_snapshot_wire"))?;
    let field = decode_field(cursor.take(field_len)?)?;
    let graph_len = usize::try_from(cursor.u32()?)
        .map_err(|_| StoreError::ContinuityFence("semantic_snapshot_wire"))?;
    let graph = decode_graph(cursor.take(graph_len)?)?;
    let receipt_len = usize::try_from(cursor.u32()?)
        .map_err(|_| StoreError::ContinuityFence("semantic_snapshot_wire"))?;
    let receipt_bytes = cursor.take(receipt_len)?;
    if !cursor.eof() {
        return Err(StoreError::ContinuityFence("semantic_snapshot_wire"));
    }
    let vector_receipt = wire::decode_transition_receipt_v2(receipt_bytes)
        .map_err(|_| StoreError::ContinuityFence("semantic_aesem2_receipt"))?;
    if wire::encode_transition_receipt_v2(&vector_receipt) != receipt_bytes
        || !vector_receipt.validate()
        || !semantic_v2_matches_legacy_receipt(&vector_receipt, legacy_receipt)
        || vector_receipt.formula_digest != *expected_formula_digest
        || vector_receipt.state_after != *expected_state_digest
        || vector_receipt.graph_after != *expected_graph_digest
        || state_digest(&field, expected_formula_digest) != *expected_state_digest
        || graph_digest(&graph) != *expected_graph_digest
        || encode_snapshot_v2(&field, &graph, &vector_receipt)? != bytes
    {
        return Err(StoreError::ContinuityFence("semantic_aesem2_closure"));
    }
    Ok(DecodedSemanticSnapshotV2 { field, graph })
}

fn encode_snapshot_v3(
    field: &NeuralField,
    graph: &SparseGraph,
    telemetry: &NativeTelemetryReceiptV1,
) -> Result<Vec<u8>, StoreError> {
    let field = encode_field(field)?;
    let graph = encode_graph(graph)?;
    let telemetry = wire::encode_native_telemetry_receipt_v1(telemetry);
    let mut reserved = Vec::with_capacity(REGION_LAYOUT.len() * 8);
    for _ in REGION_LAYOUT {
        reserved.extend_from_slice(&Fixed::ZERO.encode());
    }
    let mut out = Vec::with_capacity(
        AESEM3_MAGIC.len() + 2 + 16 + field.len() + graph.len() + telemetry.len() + reserved.len(),
    );
    out.extend_from_slice(AESEM3_MAGIC);
    out.extend_from_slice(&AESEM3_SCHEMA.to_le_bytes());
    for block in [&field, &graph, &telemetry, &reserved] {
        out.extend_from_slice(
            &(u32::try_from(block.len())
                .map_err(|_| StoreError::ContinuityFence("semantic_snapshot_wire"))?)
            .to_le_bytes(),
        );
        out.extend_from_slice(block);
    }
    Ok(out)
}

/// Store-local canonical AESEM3 closure verification for an incoming migration
/// transition. The Store does not accept an opaque caller-provided snapshot.
pub(crate) fn verify_semantic_snapshot_v3(
    bytes: &[u8],
    expected_formula_digest: &Digest,
    expected_state_digest: &Digest,
    expected_graph_digest: &Digest,
    legacy_receipt: &TransitionReceipt,
) -> Result<Vec<u8>, StoreError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take(AESEM3_MAGIC.len())? != AESEM3_MAGIC || cursor.u16()? != AESEM3_SCHEMA {
        return Err(StoreError::ContinuityFence("semantic_aesem3_magic"));
    }
    let field_len = usize::try_from(cursor.u32()?)
        .map_err(|_| StoreError::ContinuityFence("semantic_snapshot_wire"))?;
    let field = decode_field(cursor.take(field_len)?)?;
    let graph_len = usize::try_from(cursor.u32()?)
        .map_err(|_| StoreError::ContinuityFence("semantic_snapshot_wire"))?;
    let graph = decode_graph(cursor.take(graph_len)?)?;
    let telemetry_len = usize::try_from(cursor.u32()?)
        .map_err(|_| StoreError::ContinuityFence("semantic_snapshot_wire"))?;
    let telemetry_bytes = cursor.take(telemetry_len)?;
    let reserved_len = usize::try_from(cursor.u32()?)
        .map_err(|_| StoreError::ContinuityFence("semantic_snapshot_wire"))?;
    if reserved_len != REGION_LAYOUT.len() * 8 {
        return Err(StoreError::ContinuityFence("semantic_aesem3_reserved"));
    }
    let reserved = cursor.take(reserved_len)?;
    let (reserved_chunks, reserved_remainder) = reserved.as_chunks::<8>();
    if !cursor.eof()
        || !reserved_remainder.is_empty()
        || reserved_chunks.len() != REGION_LAYOUT.len()
    {
        return Err(StoreError::ContinuityFence("semantic_aesem3_reserved"));
    }
    for raw in reserved_chunks {
        if Fixed::decode(*raw) != Fixed::ZERO {
            return Err(StoreError::ContinuityFence("semantic_aesem3_reserved"));
        }
    }
    let telemetry = wire::decode_native_telemetry_receipt_v1(telemetry_bytes)
        .map_err(|_| StoreError::ContinuityFence("semantic_aesem3_receipt"))?;
    if wire::encode_native_telemetry_receipt_v1(&telemetry) != telemetry_bytes
        || !telemetry.validate()
        || !semantic_v3_matches_legacy_receipt(&telemetry, legacy_receipt)
        || telemetry.formula_digest != *expected_formula_digest
        || telemetry.state_after != *expected_state_digest
        || telemetry.graph_after != *expected_graph_digest
        || telemetry.compensation_digest != legacy_reserved_zero_digest_v1()
        || state_digest(&field, expected_formula_digest) != *expected_state_digest
        || graph_digest(&graph) != *expected_graph_digest
        || encode_snapshot_v3(&field, &graph, &telemetry)? != bytes
    {
        return Err(StoreError::ContinuityFence("semantic_aesem3_closure"));
    }
    Ok(graph.canonical_bytes())
}

#[derive(Clone, Debug)]
pub(crate) struct LegacyReplayTransitionV1 {
    pub(crate) next_field: NeuralField,
    pub(crate) active_nodes: u32,
}

fn legacy_component_update(
    current: Fixed,
    baseline: Fixed,
    drive: Fixed,
    neutral_rate: Fixed,
) -> Result<(Fixed, Fixed), StoreError> {
    let displacement = current.saturating_sub(baseline);
    let recovery = displacement
        .checked_mul(neutral_rate)
        .ok_or(StoreError::ContinuityFence("legacy_replay_arithmetic"))?;
    Ok((
        current.saturating_add(drive).saturating_sub(recovery),
        recovery,
    ))
}

/// Frozen predecessor writer used exclusively to authenticate legacy history.
pub(crate) fn replay_legacy_aesem2_transition_v1(
    field: &NeuralField,
    baseline: &NeuralField,
    dimensions: &EvidenceVector,
    estimator_confidence: Fixed,
) -> Result<LegacyReplayTransitionV1, StoreError> {
    if !field.validate()
        || !baseline.validate()
        || !(Fixed::ZERO < estimator_confidence && estimator_confidence <= Fixed::ONE)
    {
        return Err(StoreError::ContinuityFence("legacy_replay_input"));
    }
    let full_vector_load = assemble_full_vector_load(dimensions)
        .map_err(|_| StoreError::ContinuityFence("legacy_replay_input"))?;
    if full_vector_load.evaluated_dimension_count != 15
        || full_vector_load.injected_dimension_count != 15
    {
        return Err(StoreError::ContinuityFence("legacy_replay_input"));
    }
    let mut next_field = field.clone();
    let mut active_nodes = 0_u32;
    for (region, &(start, count)) in REGION_LAYOUT.iter().enumerate() {
        let drive = full_vector_load.evidence_means[region]
            .checked_mul(estimator_confidence)
            .ok_or(StoreError::ContinuityFence("legacy_replay_arithmetic"))?;
        let neutral_rate = full_vector_load.neutral_means[region]
            .checked_mul(LEGACY_NEUTRAL_RELAXATION_MAX_RATE)
            .ok_or(StoreError::ContinuityFence("legacy_replay_arithmetic"))?;
        let end = start
            .checked_add(count)
            .filter(|end| *end <= NEURON_SLOTS)
            .ok_or(StoreError::ContinuityFence("legacy_replay_shape"))?;
        for node in start..end {
            let (potential, potential_recovery) = legacy_component_update(
                field.potential[node],
                baseline.potential[node],
                drive,
                neutral_rate,
            )?;
            let (excitation, excitation_recovery) = legacy_component_update(
                field.excitation[node],
                baseline.excitation[node],
                drive,
                neutral_rate,
            )?;
            if drive == Fixed::ZERO
                && potential_recovery == Fixed::ZERO
                && excitation_recovery == Fixed::ZERO
            {
                continue;
            }
            active_nodes = active_nodes
                .checked_add(1)
                .ok_or(StoreError::ContinuityFence("legacy_replay_arithmetic"))?;
            next_field.potential[node] = potential;
            next_field.excitation[node] = excitation;
        }
    }
    if !next_field.validate() {
        return Err(StoreError::ContinuityFence("legacy_replay_shape"));
    }
    Ok(LegacyReplayTransitionV1 {
        next_field,
        active_nodes,
    })
}

fn checked_joint_scaled_fxp6(value: i64, common_max: i64) -> Result<i64, StoreError> {
    let numerator = i128::from(value)
        .checked_mul(i128::from(LEGACY_FIELD_FXP6_SCALE))
        .ok_or(StoreError::ContinuityFence("field_transform"))?;
    let denominator = i128::from(common_max);
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    let doubled_remainder = remainder
        .checked_mul(2)
        .ok_or(StoreError::ContinuityFence("field_transform"))?;
    let rounded = if doubled_remainder > denominator
        || (doubled_remainder == denominator && quotient % 2 != 0)
    {
        quotient
            .checked_add(1)
            .ok_or(StoreError::ContinuityFence("field_transform"))?
    } else {
        quotient
    };
    let scaled =
        i64::try_from(rounded).map_err(|_| StoreError::ContinuityFence("field_transform"))?;
    if !(0..=i64::from(LEGACY_FIELD_FXP6_SCALE)).contains(&scaled) {
        return Err(StoreError::ContinuityFence("field_transform"));
    }
    Ok(scaled)
}

/// Independently recompute the frozen joint P/E transform and its aggregate
/// receipt fields.  Caller-provided metadata is never used as an input.
pub(crate) fn normalize_legacy_aesem2_field_domain_v1(
    field: &NeuralField,
) -> Result<Option<(NeuralField, LegacySemanticFieldDomainUpgradeV1)>, StoreError> {
    if !field.validate() {
        return Err(StoreError::ContinuityFence("field_shape"));
    }
    for values in [
        &field.inhibition,
        &field.adaptation,
        &field.precision,
        &field.prediction_error,
        &field.eligibility,
        &field.metabolic_reserve,
    ] {
        if values
            .iter()
            .any(|value| !(0..=i64::from(LEGACY_FIELD_FXP6_SCALE)).contains(&value.raw()))
        {
            return Err(StoreError::ContinuityFence("field_nonpe_range"));
        }
    }
    let mut common_max = 0_i64;
    let mut out_of_range_count = 0_u32;
    let mut potential_out_of_range_count = 0_u32;
    let mut excitation_out_of_range_count = 0_u32;
    let mut signal_mass_before = 0_i128;
    for (values, component_count) in [
        (&field.potential, &mut potential_out_of_range_count),
        (&field.excitation, &mut excitation_out_of_range_count),
    ] {
        for value in values {
            let raw = value.raw();
            if raw < 0 {
                return Err(StoreError::ContinuityFence("field_pe_range"));
            }
            common_max = common_max.max(raw);
            signal_mass_before = signal_mass_before
                .checked_add(i128::from(raw))
                .ok_or(StoreError::ContinuityFence("field_transform"))?;
            if raw > i64::from(LEGACY_FIELD_FXP6_SCALE) {
                *component_count = component_count
                    .checked_add(1)
                    .ok_or(StoreError::ContinuityFence("field_transform"))?;
                out_of_range_count = out_of_range_count
                    .checked_add(1)
                    .ok_or(StoreError::ContinuityFence("field_transform"))?;
            }
        }
    }
    if common_max <= i64::from(LEGACY_FIELD_FXP6_SCALE) {
        return Ok(None);
    }
    let mut normalized = field.clone();
    let mut signal_mass_after = 0_i128;
    for (source, destination) in [
        (&field.potential, &mut normalized.potential),
        (&field.excitation, &mut normalized.excitation),
    ] {
        for (before, after) in source.iter().zip(destination.iter_mut()) {
            let scaled = checked_joint_scaled_fxp6(before.raw(), common_max)?;
            *after = Fixed::from_raw(scaled);
            signal_mass_after = signal_mass_after
                .checked_add(i128::from(scaled))
                .ok_or(StoreError::ContinuityFence("field_transform"))?;
        }
    }
    if !normalized.validate()
        || normalized
            .potential
            .iter()
            .chain(normalized.excitation.iter())
            .any(|value| !(0..=i64::from(LEGACY_FIELD_FXP6_SCALE)).contains(&value.raw()))
    {
        return Err(StoreError::ContinuityFence("field_transform"));
    }
    Ok(Some((
        normalized,
        LegacySemanticFieldDomainUpgradeV1 {
            algorithm: JOINT_MAX_LINEAR_FXP6_V1,
            fxp6_scale: LEGACY_FIELD_FXP6_SCALE,
            source_common_max: common_max,
            out_of_range_count,
            potential_out_of_range_count,
            excitation_out_of_range_count,
            signal_mass_before,
            signal_mass_after,
        },
    )))
}

pub(crate) fn p_and_e_within_legacy_revision_bound(field: &NeuralField, revision: u64) -> bool {
    let Some(limit) = i128::from(revision)
        .checked_add(1)
        .and_then(|value| value.checked_mul(i128::from(LEGACY_FIELD_FXP6_SCALE)))
    else {
        return false;
    };
    field
        .potential
        .iter()
        .chain(field.excitation.iter())
        .all(|value| value.raw() >= 0 && i128::from(value.raw()) <= limit)
}
