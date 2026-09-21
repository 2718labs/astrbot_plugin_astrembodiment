use ae_contracts::{
    wire, CapacityTelemetryV1, Digest, EnergyTelemetryV1, InvariantResiduals, MatrixSleepPhaseV1,
    MatrixTimeEpochV1, NativeTelemetryFormulaV1, NativeTelemetryPhaseV1, NativeTelemetryReceiptV1,
    SemanticVectorFormulaV2, SemanticVectorReceiptV2, NATIVE_TELEMETRY_RECEIPT_SCHEMA_V1,
};
use ae_fixed::Fixed;
use ae_neurofield::{
    graph_digest, state_digest, NeuralField, SparseGraph, Synapse, EDGE_CAPACITY, NEURON_SLOTS,
    REGION_LAYOUT,
};

use crate::{SemanticCoreError, TransitionReceiptV2};

const SNAPSHOT_MAGIC_V3: &[u8] = b"AESEM3\0";
const SNAPSHOT_SCHEMA_V3: u16 = 3;
const TIME_SNAPSHOT_MAGIC_V1: &[u8] = b"AESET1\0";
const TIME_SNAPSHOT_SCHEMA_V1: u16 = 1;
/// AESET1 is deliberately a compact projection. The 16K field and sparse
/// graph live only at a Genesis/perception anchor and are re-derived with the
/// cumulative epoch when the current time view is hydrated.
pub const TIME_SNAPSHOT_WIRE_LEN_V1: usize = 431;
const FIELD_WIRE_LEN: usize = 8 * (4 + NEURON_SLOTS * 8);
const GRAPH_WIRE_MIN_LEN: usize = 4 + (NEURON_SLOTS + 1) * 4 + 4;
const GRAPH_EDGE_WIRE_LEN: usize = 16;
const GRAPH_WIRE_MAX_EDGE_BYTES: usize = match EDGE_CAPACITY.checked_mul(GRAPH_EDGE_WIRE_LEN) {
    Some(value) => value,
    None => panic!("semantic graph edge wire bound overflow"),
};
const GRAPH_WIRE_MAX_LEN: usize = match GRAPH_WIRE_MIN_LEN.checked_add(GRAPH_WIRE_MAX_EDGE_BYTES) {
    Some(value) => value,
    None => panic!("semantic graph wire bound overflow"),
};
pub const TRANSITION_RECEIPT_V2_WIRE_LEN: usize = 302;
pub const NATIVE_TELEMETRY_RECEIPT_V1_WIRE_LEN: usize = 588;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CanonicalAesem3Blocks<'a> {
    pub field: &'a [u8],
    pub graph: &'a [u8],
    pub telemetry: &'a [u8],
}

pub struct DecodedCanonicalSemanticSnapshotV3 {
    pub field: NeuralField,
    pub graph: SparseGraph,
    pub telemetry: NativeTelemetryReceiptV1,
}

/// Evidence-free canonical snapshot used only by committed semantic time
/// transitions.  Its distinct magic prevents an AESEM3 evidence snapshot from
/// being reinterpreted as a local time transition (or vice versa).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedTimeSnapshotV1 {
    pub semantic_formula_digest: Digest,
    pub time_formula_digest: Digest,
    pub epoch_before: MatrixTimeEpochV1,
    pub epoch_after: MatrixTimeEpochV1,
    pub requested_elapsed_ms: u64,
    pub applied_elapsed_ms: u64,
    pub capped_gap: bool,
    pub pre_sleep_phase: MatrixSleepPhaseV1,
    pub state_before: Digest,
    pub state_after: Digest,
    pub graph_digest: Digest,
    pub authority_digest: Digest,
    pub commitment_digest: Digest,
}

/// Read-only decoder result for prerelease V4 AESET1 rows. New writes can
/// never use this representation; it exists solely so the V4->V5 migration
/// can authenticate and compact already-created development databases.
pub struct DecodedLegacyTimeSnapshotV1 {
    pub semantic_formula_digest: Digest,
    pub time_formula_digest: Digest,
    pub field: NeuralField,
    pub graph: SparseGraph,
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], SemanticCoreError> {
        let end = self
            .position
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(SemanticCoreError::SnapshotWireInvalid)?;
        let value = &self.bytes[self.position..end];
        self.position = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, SemanticCoreError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, SemanticCoreError> {
        let mut value = [0; 2];
        value.copy_from_slice(self.take(2)?);
        Ok(u16::from_le_bytes(value))
    }

    fn u32(&mut self) -> Result<u32, SemanticCoreError> {
        let mut value = [0; 4];
        value.copy_from_slice(self.take(4)?);
        Ok(u32::from_le_bytes(value))
    }

    fn u64(&mut self) -> Result<u64, SemanticCoreError> {
        let mut value = [0; 8];
        value.copy_from_slice(self.take(8)?);
        Ok(u64::from_le_bytes(value))
    }

    fn digest(&mut self) -> Result<Digest, SemanticCoreError> {
        let mut value = [0; 32];
        value.copy_from_slice(self.take(32)?);
        Ok(value)
    }

    fn bool(&mut self) -> Result<bool, SemanticCoreError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(SemanticCoreError::SnapshotWireInvalid),
        }
    }

    fn opt_digest(&mut self) -> Result<Option<Digest>, SemanticCoreError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.digest()?)),
            _ => Err(SemanticCoreError::SnapshotWireInvalid),
        }
    }

    fn fixed(&mut self) -> Result<Fixed, SemanticCoreError> {
        let mut value = [0; 8];
        value.copy_from_slice(self.take(8)?);
        Ok(Fixed::decode(value))
    }

    fn eof(&self) -> bool {
        self.position == self.bytes.len()
    }
}

fn encode_transition_receipt_v2_unchecked(receipt: &TransitionReceiptV2) -> Vec<u8> {
    let mut out = Vec::with_capacity(TRANSITION_RECEIPT_V2_WIRE_LEN);
    out.extend_from_slice(&receipt.schema_version.to_le_bytes());
    for digest in [
        &receipt.formula_digest,
        &receipt.scope_digest,
        &receipt.event_digest,
        &receipt.authority_digest,
    ] {
        out.extend_from_slice(digest);
    }
    out.extend_from_slice(&receipt.base_revision.to_le_bytes());
    out.extend_from_slice(&receipt.next_revision.to_le_bytes());
    for digest in [
        &receipt.state_before,
        &receipt.state_after,
        &receipt.graph_after,
    ] {
        out.extend_from_slice(digest);
    }
    match receipt.action_contract {
        None => out.push(0),
        Some(digest) => {
            out.push(1);
            out.extend_from_slice(&digest);
        }
    }
    out.extend_from_slice(&receipt.active_nodes.to_le_bytes());
    out.extend_from_slice(&receipt.active_edges.to_le_bytes());
    for value in [
        receipt.residuals.authority,
        receipt.residuals.continuity,
        receipt.residuals.energy,
        receipt.residuals.renormalization,
        receipt.residuals.capacity,
    ] {
        out.extend_from_slice(&value.encode());
    }
    out.push(wire::commit_status_code(receipt.status));
    let vector = &receipt.semantic_vector;
    out.extend_from_slice(&vector.schema_version.to_le_bytes());
    out.push(match vector.formula {
        SemanticVectorFormulaV2::FullVectorRouteNeutralRelaxationV1 => 1,
    });
    out.push(vector.dimension_slot_count);
    out.push(vector.evaluated_dimension_count);
    out.push(vector.injected_dimension_count);
    out.push(vector.nonzero_evidence_dimension_count);
    out.push(vector.neutral_baseline_dimension_count);
    out.push(vector.unavailable_dimension_count);
    out.push(u8::from(vector.state_changed));
    out
}

pub fn encode_transition_receipt_v2(
    receipt: &TransitionReceiptV2,
) -> Result<Vec<u8>, SemanticCoreError> {
    if !receipt.validate() {
        return Err(SemanticCoreError::SemanticClosureInvalid);
    }
    let out = encode_transition_receipt_v2_unchecked(receipt);
    if out.len() != TRANSITION_RECEIPT_V2_WIRE_LEN {
        return Err(SemanticCoreError::SemanticClosureInvalid);
    }
    Ok(out)
}

pub fn decode_transition_receipt_v2(
    bytes: &[u8],
) -> Result<TransitionReceiptV2, SemanticCoreError> {
    if bytes.len() != TRANSITION_RECEIPT_V2_WIRE_LEN {
        return Err(SemanticCoreError::SnapshotWireInvalid);
    }
    let mut cursor = Cursor::new(bytes);
    let schema_version = cursor.u16()?;
    if schema_version != TransitionReceiptV2::SCHEMA_VERSION {
        return Err(SemanticCoreError::SnapshotWireInvalid);
    }
    let receipt = TransitionReceiptV2 {
        schema_version,
        formula_digest: cursor.digest()?,
        scope_digest: cursor.digest()?,
        event_digest: cursor.digest()?,
        authority_digest: cursor.digest()?,
        base_revision: cursor.u64()?,
        next_revision: cursor.u64()?,
        state_before: cursor.digest()?,
        state_after: cursor.digest()?,
        graph_after: cursor.digest()?,
        action_contract: cursor.opt_digest()?,
        active_nodes: cursor.u32()?,
        active_edges: cursor.u32()?,
        residuals: InvariantResiduals {
            authority: cursor.fixed()?,
            continuity: cursor.fixed()?,
            energy: cursor.fixed()?,
            renormalization: cursor.fixed()?,
            capacity: cursor.fixed()?,
        },
        status: wire::commit_status_from_code(cursor.u8()?)
            .ok_or(SemanticCoreError::SnapshotWireInvalid)?,
        semantic_vector: SemanticVectorReceiptV2 {
            schema_version: cursor.u16()?,
            formula: match cursor.u8()? {
                1 => SemanticVectorFormulaV2::FullVectorRouteNeutralRelaxationV1,
                _ => return Err(SemanticCoreError::SnapshotWireInvalid),
            },
            dimension_slot_count: cursor.u8()?,
            evaluated_dimension_count: cursor.u8()?,
            injected_dimension_count: cursor.u8()?,
            nonzero_evidence_dimension_count: cursor.u8()?,
            neutral_baseline_dimension_count: cursor.u8()?,
            unavailable_dimension_count: cursor.u8()?,
            state_changed: cursor.bool()?,
        },
    };
    if !cursor.eof()
        || !receipt.validate()
        || encode_transition_receipt_v2_unchecked(&receipt) != bytes
    {
        return Err(SemanticCoreError::SnapshotWireInvalid);
    }
    Ok(receipt)
}

fn encode_native_telemetry_receipt_v1_unchecked(receipt: &NativeTelemetryReceiptV1) -> Vec<u8> {
    let mut out = Vec::with_capacity(NATIVE_TELEMETRY_RECEIPT_V1_WIRE_LEN);
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.push(match receipt.formula {
        NativeTelemetryFormulaV1::Phase0NativePropagationFxp6V1 => 1,
    });
    out.push(match receipt.phase {
        NativeTelemetryPhaseV1::Prepare => 1,
    });
    for digest in [
        &receipt.formula_digest,
        &receipt.scope_digest,
        &receipt.event_digest,
        &receipt.source_digest,
    ] {
        out.extend_from_slice(digest);
    }
    out.extend_from_slice(&receipt.base_revision.to_le_bytes());
    out.extend_from_slice(&receipt.next_revision.to_le_bytes());
    for digest in [
        &receipt.state_before,
        &receipt.state_after,
        &receipt.graph_before,
        &receipt.graph_after,
        &receipt.local_digest,
        &receipt.compensation_digest,
        &receipt.effective_digest,
    ] {
        out.extend_from_slice(digest);
    }
    for value in [
        receipt.energy.reserve_before,
        receipt.energy.reserve_after,
        receipt.energy.recovered,
        receipt.energy.spent,
        receipt.energy.headroom,
        receipt.energy.residual,
    ] {
        out.extend_from_slice(&value.encode());
    }
    out.extend_from_slice(&receipt.capacity.upper_saturated_nodes.to_le_bytes());
    out.extend_from_slice(&receipt.capacity.node_limit.to_le_bytes());
    out.extend_from_slice(&receipt.capacity.node_headroom.encode());
    out.extend_from_slice(&receipt.capacity.edge_used.to_le_bytes());
    out.extend_from_slice(&receipt.capacity.edge_limit.to_le_bytes());
    for value in [
        receipt.capacity.edge_headroom,
        receipt.capacity.headroom,
        receipt.capacity.residual,
        receipt.residuals.authority,
        receipt.residuals.continuity,
        receipt.residuals.energy,
        receipt.residuals.renormalization,
        receipt.residuals.capacity,
        receipt.residual_health,
        receipt.native_gate,
    ] {
        out.extend_from_slice(&value.encode());
    }
    out.extend_from_slice(&receipt.checkpoint_digest);
    out.extend_from_slice(&receipt.telemetry_digest);
    out
}

pub fn encode_native_telemetry_receipt_v1(
    receipt: &NativeTelemetryReceiptV1,
) -> Result<Vec<u8>, SemanticCoreError> {
    if !receipt.validate() {
        return Err(SemanticCoreError::SemanticClosureInvalid);
    }
    let out = encode_native_telemetry_receipt_v1_unchecked(receipt);
    if out.len() != NATIVE_TELEMETRY_RECEIPT_V1_WIRE_LEN {
        return Err(SemanticCoreError::SemanticClosureInvalid);
    }
    Ok(out)
}

pub fn decode_native_telemetry_receipt_v1(
    bytes: &[u8],
) -> Result<NativeTelemetryReceiptV1, SemanticCoreError> {
    if bytes.len() != NATIVE_TELEMETRY_RECEIPT_V1_WIRE_LEN {
        return Err(SemanticCoreError::SnapshotWireInvalid);
    }
    let mut cursor = Cursor::new(bytes);
    if cursor.u16()? != 1 {
        return Err(SemanticCoreError::SnapshotWireInvalid);
    }
    let formula = match cursor.u8()? {
        1 => NativeTelemetryFormulaV1::Phase0NativePropagationFxp6V1,
        _ => return Err(SemanticCoreError::SnapshotWireInvalid),
    };
    let phase = match cursor.u8()? {
        1 => NativeTelemetryPhaseV1::Prepare,
        _ => return Err(SemanticCoreError::SnapshotWireInvalid),
    };
    let receipt = NativeTelemetryReceiptV1 {
        schema: NATIVE_TELEMETRY_RECEIPT_SCHEMA_V1.to_owned(),
        formula,
        phase,
        formula_digest: cursor.digest()?,
        scope_digest: cursor.digest()?,
        event_digest: cursor.digest()?,
        source_digest: cursor.digest()?,
        base_revision: cursor.u64()?,
        next_revision: cursor.u64()?,
        state_before: cursor.digest()?,
        state_after: cursor.digest()?,
        graph_before: cursor.digest()?,
        graph_after: cursor.digest()?,
        local_digest: cursor.digest()?,
        compensation_digest: cursor.digest()?,
        effective_digest: cursor.digest()?,
        energy: EnergyTelemetryV1 {
            reserve_before: cursor.fixed()?,
            reserve_after: cursor.fixed()?,
            recovered: cursor.fixed()?,
            spent: cursor.fixed()?,
            headroom: cursor.fixed()?,
            residual: cursor.fixed()?,
        },
        capacity: CapacityTelemetryV1 {
            upper_saturated_nodes: cursor.u32()?,
            node_limit: cursor.u32()?,
            node_headroom: cursor.fixed()?,
            edge_used: cursor.u32()?,
            edge_limit: cursor.u32()?,
            edge_headroom: cursor.fixed()?,
            headroom: cursor.fixed()?,
            residual: cursor.fixed()?,
        },
        residuals: InvariantResiduals {
            authority: cursor.fixed()?,
            continuity: cursor.fixed()?,
            energy: cursor.fixed()?,
            renormalization: cursor.fixed()?,
            capacity: cursor.fixed()?,
        },
        residual_health: cursor.fixed()?,
        native_gate: cursor.fixed()?,
        checkpoint_digest: cursor.digest()?,
        telemetry_digest: cursor.digest()?,
    };
    if !cursor.eof()
        || !receipt.validate()
        || encode_native_telemetry_receipt_v1_unchecked(&receipt) != bytes
    {
        return Err(SemanticCoreError::SnapshotWireInvalid);
    }
    Ok(receipt)
}

fn encode_field(field: &NeuralField) -> Result<Vec<u8>, SemanticCoreError> {
    if !field.validate() {
        return Err(SemanticCoreError::FieldStateInvalid);
    }
    let mut out = Vec::with_capacity(FIELD_WIRE_LEN);
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
            &u32::try_from(values.len())
                .map_err(|_| SemanticCoreError::FieldStateInvalid)?
                .to_le_bytes(),
        );
        for value in values {
            out.extend_from_slice(&value.encode());
        }
    }
    if out.len() != FIELD_WIRE_LEN {
        return Err(SemanticCoreError::FieldStateInvalid);
    }
    Ok(out)
}

fn decode_field(bytes: &[u8]) -> Result<NeuralField, SemanticCoreError> {
    if bytes.len() != FIELD_WIRE_LEN {
        return Err(SemanticCoreError::FieldStateInvalid);
    }
    let mut cursor = Cursor::new(bytes);
    let mut vectors = Vec::with_capacity(8);
    for _ in 0..8 {
        if usize::try_from(cursor.u32()?).map_err(|_| SemanticCoreError::FieldStateInvalid)?
            != NEURON_SLOTS
        {
            return Err(SemanticCoreError::FieldStateInvalid);
        }
        let mut values = Vec::with_capacity(NEURON_SLOTS);
        for _ in 0..NEURON_SLOTS {
            values.push(cursor.fixed()?);
        }
        vectors.push(values);
    }
    if !cursor.eof() {
        return Err(SemanticCoreError::FieldStateInvalid);
    }
    let mut vectors = vectors.into_iter();
    let field = NeuralField {
        potential: vectors.next().ok_or(SemanticCoreError::FieldStateInvalid)?,
        excitation: vectors.next().ok_or(SemanticCoreError::FieldStateInvalid)?,
        inhibition: vectors.next().ok_or(SemanticCoreError::FieldStateInvalid)?,
        adaptation: vectors.next().ok_or(SemanticCoreError::FieldStateInvalid)?,
        precision: vectors.next().ok_or(SemanticCoreError::FieldStateInvalid)?,
        prediction_error: vectors.next().ok_or(SemanticCoreError::FieldStateInvalid)?,
        eligibility: vectors.next().ok_or(SemanticCoreError::FieldStateInvalid)?,
        metabolic_reserve: vectors.next().ok_or(SemanticCoreError::FieldStateInvalid)?,
    };
    if field.validate() {
        Ok(field)
    } else {
        Err(SemanticCoreError::FieldStateInvalid)
    }
}

fn encode_graph(graph: &SparseGraph) -> Result<Vec<u8>, SemanticCoreError> {
    if !graph.validate() {
        return Err(SemanticCoreError::GraphStateInvalid);
    }
    let mut out = Vec::with_capacity(4 + graph.row_offsets.len() * 4 + 4 + graph.edges.len() * 16);
    out.extend_from_slice(
        &u32::try_from(graph.row_offsets.len())
            .map_err(|_| SemanticCoreError::GraphStateInvalid)?
            .to_le_bytes(),
    );
    for offset in &graph.row_offsets {
        out.extend_from_slice(&offset.to_le_bytes());
    }
    out.extend_from_slice(
        &u32::try_from(graph.edges.len())
            .map_err(|_| SemanticCoreError::GraphStateInvalid)?
            .to_le_bytes(),
    );
    for edge in &graph.edges {
        out.extend_from_slice(&edge.target.to_le_bytes());
        out.extend_from_slice(&edge.weight.to_le_bytes());
        out.extend_from_slice(&edge.eligibility.to_le_bytes());
        out.extend_from_slice(&edge.stability.to_le_bytes());
        out.extend_from_slice(&edge.last_used_epoch.to_le_bytes());
        out.push(edge.operator_id);
        out.push(edge.delay_class);
        out.extend_from_slice(&edge.flags.to_le_bytes());
    }
    Ok(out)
}

fn decode_graph(bytes: &[u8]) -> Result<SparseGraph, SemanticCoreError> {
    if !(GRAPH_WIRE_MIN_LEN..=GRAPH_WIRE_MAX_LEN).contains(&bytes.len()) {
        return Err(SemanticCoreError::GraphStateInvalid);
    }
    let mut cursor = Cursor::new(bytes);
    let offsets_len =
        usize::try_from(cursor.u32()?).map_err(|_| SemanticCoreError::GraphStateInvalid)?;
    if offsets_len != NEURON_SLOTS + 1 {
        return Err(SemanticCoreError::GraphStateInvalid);
    }
    let mut row_offsets = Vec::with_capacity(offsets_len);
    for _ in 0..offsets_len {
        row_offsets.push(cursor.u32()?);
    }
    let edge_len =
        usize::try_from(cursor.u32()?).map_err(|_| SemanticCoreError::GraphStateInvalid)?;
    if edge_len > EDGE_CAPACITY {
        return Err(SemanticCoreError::GraphStateInvalid);
    }
    let mut edges = Vec::with_capacity(edge_len);
    for _ in 0..edge_len {
        let target = cursor.u32()?;
        let mut weight = [0; 2];
        weight.copy_from_slice(cursor.take(2)?);
        let mut eligibility = [0; 2];
        eligibility.copy_from_slice(cursor.take(2)?);
        let mut stability = [0; 2];
        stability.copy_from_slice(cursor.take(2)?);
        let mut last_used_epoch = [0; 2];
        last_used_epoch.copy_from_slice(cursor.take(2)?);
        let operator_id = cursor.u8()?;
        let delay_class = cursor.u8()?;
        let mut flags = [0; 2];
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
        return Err(SemanticCoreError::GraphStateInvalid);
    }
    let graph = SparseGraph { row_offsets, edges };
    if graph.validate() {
        Ok(graph)
    } else {
        Err(SemanticCoreError::GraphStateInvalid)
    }
}

/// Canonical standalone graph wire used by Store-owned semantic sidecars.
///
/// Keeping this codec in the pure core prevents persistence code from
/// maintaining a second graph encoding implementation.
pub fn encode_canonical_graph_v1(graph: &SparseGraph) -> Result<Vec<u8>, SemanticCoreError> {
    encode_graph(graph)
}

/// Strict inverse of [`encode_canonical_graph_v1`].
pub fn decode_canonical_graph_v1(bytes: &[u8]) -> Result<SparseGraph, SemanticCoreError> {
    let graph = decode_graph(bytes)?;
    if encode_graph(&graph)? != bytes {
        return Err(SemanticCoreError::SnapshotWireInvalid);
    }
    Ok(graph)
}

/// Parse exactly the four bounded blocks of the current AESEM3 envelope.
pub fn decode_canonical_aesem3_blocks(
    bytes: &[u8],
) -> Result<CanonicalAesem3Blocks<'_>, SemanticCoreError> {
    let mut cursor = Cursor::new(bytes);
    let magic = cursor.take(SNAPSHOT_MAGIC_V3.len())?;
    let schema = cursor.u16()?;
    if magic != SNAPSHOT_MAGIC_V3 || schema != SNAPSHOT_SCHEMA_V3 {
        return Err(SemanticCoreError::Aesem3MagicOrSchema);
    }
    let field_len =
        usize::try_from(cursor.u32()?).map_err(|_| SemanticCoreError::SnapshotWireInvalid)?;
    if field_len != FIELD_WIRE_LEN {
        return Err(SemanticCoreError::SnapshotWireInvalid);
    }
    let field = cursor.take(field_len)?;
    let graph_len =
        usize::try_from(cursor.u32()?).map_err(|_| SemanticCoreError::SnapshotWireInvalid)?;
    if !(GRAPH_WIRE_MIN_LEN..=GRAPH_WIRE_MAX_LEN).contains(&graph_len) {
        return Err(SemanticCoreError::SnapshotWireInvalid);
    }
    let graph = cursor.take(graph_len)?;
    let telemetry_len =
        usize::try_from(cursor.u32()?).map_err(|_| SemanticCoreError::SnapshotWireInvalid)?;
    if telemetry_len != NATIVE_TELEMETRY_RECEIPT_V1_WIRE_LEN {
        return Err(SemanticCoreError::SnapshotWireInvalid);
    }
    let telemetry = cursor.take(telemetry_len)?;
    let reserved_len =
        usize::try_from(cursor.u32()?).map_err(|_| SemanticCoreError::SnapshotWireInvalid)?;
    if reserved_len != REGION_LAYOUT.len() * 8 {
        return Err(SemanticCoreError::SnapshotWireInvalid);
    }
    let reserved = cursor.take(reserved_len)?;
    if !cursor.eof() {
        return Err(SemanticCoreError::SnapshotWireInvalid);
    }
    for raw in reserved.chunks_exact(8) {
        let mut value = [0; 8];
        value.copy_from_slice(raw);
        if Fixed::decode(value) != Fixed::ZERO {
            return Err(SemanticCoreError::Aesem3RetiredCompensationNonzero);
        }
    }
    Ok(CanonicalAesem3Blocks {
        field,
        graph,
        telemetry,
    })
}

pub fn encode_semantic_snapshot_v3(
    formula_digest: &Digest,
    field: &NeuralField,
    graph: &SparseGraph,
    telemetry: &NativeTelemetryReceiptV1,
) -> Result<Vec<u8>, SemanticCoreError> {
    if !telemetry.validate() {
        return Err(SemanticCoreError::SemanticClosureInvalid);
    }
    if telemetry.formula_digest != *formula_digest
        || telemetry.state_after != state_digest(field, formula_digest)
        || telemetry.graph_after != graph_digest(graph)
        || telemetry.compensation_digest != ae_contracts::legacy_reserved_zero_digest_v1()
    {
        return Err(SemanticCoreError::SnapshotAttestationMismatch);
    }
    let field_bytes = encode_field(field)?;
    let graph_bytes = encode_graph(graph)?;
    let telemetry_bytes = encode_native_telemetry_receipt_v1(telemetry)?;
    let mut reserved_zero_bytes = Vec::with_capacity(REGION_LAYOUT.len() * 8);
    for _ in 0..REGION_LAYOUT.len() {
        reserved_zero_bytes.extend_from_slice(&Fixed::ZERO.encode());
    }
    let mut out = Vec::with_capacity(
        SNAPSHOT_MAGIC_V3.len()
            + 2
            + 16
            + field_bytes.len()
            + graph_bytes.len()
            + telemetry_bytes.len()
            + reserved_zero_bytes.len(),
    );
    out.extend_from_slice(SNAPSHOT_MAGIC_V3);
    out.extend_from_slice(&SNAPSHOT_SCHEMA_V3.to_le_bytes());
    for bytes in [
        &field_bytes,
        &graph_bytes,
        &telemetry_bytes,
        &reserved_zero_bytes,
    ] {
        out.extend_from_slice(
            &u32::try_from(bytes.len())
                .map_err(|_| SemanticCoreError::SnapshotWireInvalid)?
                .to_le_bytes(),
        );
        out.extend_from_slice(bytes);
    }
    Ok(out)
}

/// Decode current semantic state without depending on a journal receipt. The
/// envelope, telemetry, state digest and graph digest all re-close exactly.
pub fn decode_canonical_semantic_snapshot_v3(
    bytes: &[u8],
) -> Result<DecodedCanonicalSemanticSnapshotV3, SemanticCoreError> {
    let blocks = decode_canonical_aesem3_blocks(bytes)?;
    let field = decode_field(blocks.field)?;
    let graph = decode_graph(blocks.graph)?;
    let telemetry = decode_native_telemetry_receipt_v1(blocks.telemetry)?;
    if telemetry.compensation_digest != ae_contracts::legacy_reserved_zero_digest_v1()
        || state_digest(&field, &telemetry.formula_digest) != telemetry.state_after
        || graph_digest(&graph) != telemetry.graph_after
        || encode_semantic_snapshot_v3(&telemetry.formula_digest, &field, &graph, &telemetry)?
            != bytes
    {
        return Err(SemanticCoreError::SnapshotAttestationMismatch);
    }
    Ok(DecodedCanonicalSemanticSnapshotV3 {
        field,
        graph,
        telemetry,
    })
}

fn encode_legacy_time_snapshot_v1(
    semantic_formula_digest: &Digest,
    time_formula_digest: &Digest,
    field: &NeuralField,
    graph: &SparseGraph,
) -> Result<Vec<u8>, SemanticCoreError> {
    let field_bytes = encode_field(field)?;
    let graph_bytes = encode_graph(graph)?;
    let mut out = Vec::with_capacity(
        TIME_SNAPSHOT_MAGIC_V1.len() + 2 + 64 + 4 + field_bytes.len() + 4 + graph_bytes.len() + 64,
    );
    out.extend_from_slice(TIME_SNAPSHOT_MAGIC_V1);
    out.extend_from_slice(&TIME_SNAPSHOT_SCHEMA_V1.to_le_bytes());
    out.extend_from_slice(semantic_formula_digest);
    out.extend_from_slice(time_formula_digest);
    out.extend_from_slice(&(field_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&field_bytes);
    out.extend_from_slice(&(graph_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&graph_bytes);
    out.extend_from_slice(&state_digest(field, semantic_formula_digest));
    out.extend_from_slice(&graph_digest(graph));
    Ok(out)
}

pub fn decode_legacy_time_snapshot_v1(
    bytes: &[u8],
) -> Result<DecodedLegacyTimeSnapshotV1, SemanticCoreError> {
    let minimum = TIME_SNAPSHOT_MAGIC_V1
        .len()
        .checked_add(2 + 64 + 4 + FIELD_WIRE_LEN + 4 + GRAPH_WIRE_MIN_LEN + 64)
        .ok_or(SemanticCoreError::SnapshotWireInvalid)?;
    let maximum = TIME_SNAPSHOT_MAGIC_V1
        .len()
        .checked_add(2 + 64 + 4 + FIELD_WIRE_LEN + 4 + GRAPH_WIRE_MAX_LEN + 64)
        .ok_or(SemanticCoreError::SnapshotWireInvalid)?;
    if !(minimum..=maximum).contains(&bytes.len()) {
        return Err(SemanticCoreError::SnapshotWireInvalid);
    }
    let mut cursor = Cursor::new(bytes);
    if cursor.take(TIME_SNAPSHOT_MAGIC_V1.len())? != TIME_SNAPSHOT_MAGIC_V1
        || cursor.u16()? != TIME_SNAPSHOT_SCHEMA_V1
    {
        return Err(SemanticCoreError::Aesem3MagicOrSchema);
    }
    let semantic_formula_digest = cursor.digest()?;
    let time_formula_digest = cursor.digest()?;
    if semantic_formula_digest == [0; 32]
        || time_formula_digest != crate::matrix_time_formula_digest_v1(&semantic_formula_digest)
    {
        return Err(SemanticCoreError::SnapshotAttestationMismatch);
    }
    let field_len = cursor.u32()? as usize;
    if field_len != FIELD_WIRE_LEN {
        return Err(SemanticCoreError::SnapshotWireInvalid);
    }
    let field = decode_field(cursor.take(field_len)?)?;
    let graph_len = cursor.u32()? as usize;
    if !(GRAPH_WIRE_MIN_LEN..=GRAPH_WIRE_MAX_LEN).contains(&graph_len) {
        return Err(SemanticCoreError::SnapshotWireInvalid);
    }
    let graph = decode_graph(cursor.take(graph_len)?)?;
    let expected_state = cursor.digest()?;
    let expected_graph = cursor.digest()?;
    if !cursor.eof()
        || state_digest(&field, &semantic_formula_digest) != expected_state
        || graph_digest(&graph) != expected_graph
        || encode_legacy_time_snapshot_v1(
            &semantic_formula_digest,
            &time_formula_digest,
            &field,
            &graph,
        )? != bytes
    {
        return Err(SemanticCoreError::SnapshotAttestationMismatch);
    }
    Ok(DecodedLegacyTimeSnapshotV1 {
        semantic_formula_digest,
        time_formula_digest,
        field,
        graph,
    })
}

fn encode_time_epoch_v1(out: &mut Vec<u8>, epoch: &MatrixTimeEpochV1) {
    out.extend_from_slice(&epoch.schema_version.to_le_bytes());
    out.extend_from_slice(&epoch.anchor_semantic_revision.to_le_bytes());
    out.extend_from_slice(&epoch.anchor_state_digest);
    for value in [
        epoch.awake_ticks,
        epoch.drowsy_ticks,
        epoch.asleep_ticks,
        epoch.awake_remainder_ms,
        epoch.drowsy_remainder_ms,
        epoch.asleep_remainder_ms,
    ] {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

fn decode_time_epoch_v1(cursor: &mut Cursor<'_>) -> Result<MatrixTimeEpochV1, SemanticCoreError> {
    Ok(MatrixTimeEpochV1 {
        schema_version: cursor.u16()?,
        anchor_semantic_revision: cursor.u64()?,
        anchor_state_digest: cursor.digest()?,
        awake_ticks: cursor.u64()?,
        drowsy_ticks: cursor.u64()?,
        asleep_ticks: cursor.u64()?,
        awake_remainder_ms: cursor.u64()?,
        drowsy_remainder_ms: cursor.u64()?,
        asleep_remainder_ms: cursor.u64()?,
    })
}

fn time_phase_code_v1(phase: MatrixSleepPhaseV1) -> u8 {
    match phase {
        MatrixSleepPhaseV1::Awake => 1,
        MatrixSleepPhaseV1::Drowsy => 2,
        MatrixSleepPhaseV1::Asleep => 3,
    }
}

fn decode_time_phase_v1(code: u8) -> Result<MatrixSleepPhaseV1, SemanticCoreError> {
    match code {
        1 => Ok(MatrixSleepPhaseV1::Awake),
        2 => Ok(MatrixSleepPhaseV1::Drowsy),
        3 => Ok(MatrixSleepPhaseV1::Asleep),
        _ => Err(SemanticCoreError::SnapshotWireInvalid),
    }
}

fn validate_time_projection_v1(value: &DecodedTimeSnapshotV1) -> bool {
    let nonzero = [
        value.semantic_formula_digest,
        value.time_formula_digest,
        value.epoch_before.anchor_state_digest,
        value.epoch_after.anchor_state_digest,
        value.state_before,
        value.state_after,
        value.graph_digest,
        value.authority_digest,
        value.commitment_digest,
    ]
    .into_iter()
    .all(|digest| digest != [0; 32]);
    let accounted_delta = value
        .epoch_after
        .accounted_elapsed_ms(crate::MATRIX_TIME_QUANTUM_MS)
        .zip(
            value
                .epoch_before
                .accounted_elapsed_ms(crate::MATRIX_TIME_QUANTUM_MS),
        )
        .and_then(|(after, before)| after.checked_sub(before));
    nonzero
        && value.time_formula_digest
            == crate::matrix_time_formula_digest_v1(&value.semantic_formula_digest)
        && value.epoch_before.schema_version == MatrixTimeEpochV1::SCHEMA_VERSION
        && value.epoch_after.schema_version == MatrixTimeEpochV1::SCHEMA_VERSION
        && value.epoch_before.anchor_semantic_revision == value.epoch_after.anchor_semantic_revision
        && value.epoch_before.anchor_state_digest == value.epoch_after.anchor_state_digest
        && value.requested_elapsed_ms > 0
        && value.applied_elapsed_ms
            == value
                .requested_elapsed_ms
                .min(crate::MATRIX_TIME_MAX_ELAPSED_MS)
        && value.capped_gap == (value.requested_elapsed_ms > crate::MATRIX_TIME_MAX_ELAPSED_MS)
        && accounted_delta == Some(value.applied_elapsed_ms)
        && [
            value.epoch_before.awake_remainder_ms,
            value.epoch_before.drowsy_remainder_ms,
            value.epoch_before.asleep_remainder_ms,
            value.epoch_after.awake_remainder_ms,
            value.epoch_after.drowsy_remainder_ms,
            value.epoch_after.asleep_remainder_ms,
        ]
        .into_iter()
        .all(|remainder| remainder < crate::MATRIX_TIME_QUANTUM_MS)
}

/// Encode the fixed-size evidence-free time projection. No field, graph,
/// semantic receipt or native telemetry bytes can enter this wire.
pub fn encode_time_snapshot_v1(
    value: &DecodedTimeSnapshotV1,
) -> Result<Vec<u8>, SemanticCoreError> {
    if !validate_time_projection_v1(value) {
        return Err(SemanticCoreError::SnapshotAttestationMismatch);
    }
    let mut out = Vec::with_capacity(TIME_SNAPSHOT_WIRE_LEN_V1);
    out.extend_from_slice(TIME_SNAPSHOT_MAGIC_V1);
    out.extend_from_slice(&TIME_SNAPSHOT_SCHEMA_V1.to_le_bytes());
    out.extend_from_slice(&value.semantic_formula_digest);
    out.extend_from_slice(&value.time_formula_digest);
    encode_time_epoch_v1(&mut out, &value.epoch_before);
    encode_time_epoch_v1(&mut out, &value.epoch_after);
    out.extend_from_slice(&value.requested_elapsed_ms.to_le_bytes());
    out.extend_from_slice(&value.applied_elapsed_ms.to_le_bytes());
    out.push(u8::from(value.capped_gap));
    out.push(time_phase_code_v1(value.pre_sleep_phase));
    out.extend_from_slice(&value.state_before);
    out.extend_from_slice(&value.state_after);
    out.extend_from_slice(&value.graph_digest);
    out.extend_from_slice(&value.authority_digest);
    out.extend_from_slice(&value.commitment_digest);
    if out.len() != TIME_SNAPSHOT_WIRE_LEN_V1 {
        return Err(SemanticCoreError::SnapshotWireInvalid);
    }
    Ok(out)
}

pub fn decode_time_snapshot_v1(bytes: &[u8]) -> Result<DecodedTimeSnapshotV1, SemanticCoreError> {
    if bytes.len() != TIME_SNAPSHOT_WIRE_LEN_V1 {
        return Err(SemanticCoreError::SnapshotWireInvalid);
    }
    let mut cursor = Cursor::new(bytes);
    if cursor.take(TIME_SNAPSHOT_MAGIC_V1.len())? != TIME_SNAPSHOT_MAGIC_V1
        || cursor.u16()? != TIME_SNAPSHOT_SCHEMA_V1
    {
        return Err(SemanticCoreError::Aesem3MagicOrSchema);
    }
    let value = DecodedTimeSnapshotV1 {
        semantic_formula_digest: cursor.digest()?,
        time_formula_digest: cursor.digest()?,
        epoch_before: decode_time_epoch_v1(&mut cursor)?,
        epoch_after: decode_time_epoch_v1(&mut cursor)?,
        requested_elapsed_ms: cursor.u64()?,
        applied_elapsed_ms: cursor.u64()?,
        capped_gap: match cursor.u8()? {
            0 => false,
            1 => true,
            _ => return Err(SemanticCoreError::SnapshotWireInvalid),
        },
        pre_sleep_phase: decode_time_phase_v1(cursor.u8()?)?,
        state_before: cursor.digest()?,
        state_after: cursor.digest()?,
        graph_digest: cursor.digest()?,
        authority_digest: cursor.digest()?,
        commitment_digest: cursor.digest()?,
    };
    if !cursor.eof()
        || !validate_time_projection_v1(&value)
        || encode_time_snapshot_v1(&value)? != bytes
    {
        return Err(SemanticCoreError::SnapshotAttestationMismatch);
    }
    Ok(value)
}
