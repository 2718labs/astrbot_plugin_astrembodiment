//! Fail-closed replay for persisted sparse-graph history.
//!
//! A persisted history is anchored by one canonical snapshot and sealed with
//! the authoritative snapshot reached by applying every structural delta.
//! Reopening never falls back to Genesis: any formula, revision, digest, or
//! canonical-byte disagreement is rejected before a graph is returned.

use ae_contracts::Digest;
use serde::{Deserialize, Serialize};

use crate::{
    apply_delta, graph_digest, DeltaError, SparseGraph, StructuralDeltaV1, Synapse, EDGE_CAPACITY,
    NEURON_SLOTS,
};

/// The only graph replay formula supported by this implementation.
pub const GRAPH_REPLAY_FORMULA_V1: u16 = 1;

const V1_OPERATOR_TYPE_COUNT: u8 = 4;
const V1_DELAY_CLASS_COUNT: u8 = 8;
const GRAPH_REPLAY_RULE_DIGEST_V1: Digest = [7; 32];

/// A complete, canonical graph checkpoint.
///
/// The graph's formula version, authority revision, digest, and bytes are all
/// persisted together. `canonical_bytes` is exactly
/// [`SparseGraph::canonical_bytes`], rather than a serde representation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphSnapshotV1 {
    pub formula_version: u16,
    pub revision: u64,
    pub graph_digest: Digest,
    pub canonical_bytes: Vec<u8>,
}

/// A sealed graph history suitable for persistence and later reopening.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphReplayV1 {
    pub anchor: GraphSnapshotV1,
    pub deltas: Vec<StructuralDeltaV1>,
    pub authoritative: GraphSnapshotV1,
}

/// Public, non-sensitive rejection classifications for graph replay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphReplayError {
    UnsupportedFormulaVersion,
    CanonicalEncodingMismatch,
    SnapshotDigestMismatch,
    RevisionDiscontinuity,
    DeltaSequenceDiscontinuity,
    BeforeDigestMismatch,
    AfterDigestMismatch,
    DeltaRejected,
    AuthoritativeSnapshotMismatch,
}

impl GraphSnapshotV1 {
    /// Captures a valid graph with its exact canonical encoding and digest.
    pub fn from_graph(
        formula_version: u16,
        revision: u64,
        graph: &SparseGraph,
    ) -> Result<Self, GraphReplayError> {
        if formula_version != GRAPH_REPLAY_FORMULA_V1 {
            return Err(GraphReplayError::UnsupportedFormulaVersion);
        }
        if !graph_is_replay_valid(graph) {
            return Err(GraphReplayError::CanonicalEncodingMismatch);
        }

        let snapshot = Self {
            formula_version,
            revision,
            graph_digest: graph_digest(graph),
            canonical_bytes: graph.canonical_bytes(),
        };
        let restored = snapshot.restore()?;
        if restored.canonical_bytes() != snapshot.canonical_bytes {
            return Err(GraphReplayError::CanonicalEncodingMismatch);
        }
        Ok(snapshot)
    }

    /// Decodes and validates the persisted checkpoint without exposing its
    /// contents on failure.
    pub fn restore(&self) -> Result<SparseGraph, GraphReplayError> {
        if self.formula_version != GRAPH_REPLAY_FORMULA_V1 {
            return Err(GraphReplayError::UnsupportedFormulaVersion);
        }

        let graph = decode_canonical_graph(&self.canonical_bytes)
            .ok_or(GraphReplayError::CanonicalEncodingMismatch)?;
        if graph.canonical_bytes() != self.canonical_bytes {
            return Err(GraphReplayError::CanonicalEncodingMismatch);
        }
        if graph_digest(&graph) != self.graph_digest {
            return Err(GraphReplayError::SnapshotDigestMismatch);
        }
        Ok(graph)
    }
}

impl GraphReplayV1 {
    /// Seals a history by calculating the graph that later reopening must
    /// reproduce exactly.
    pub fn seal(
        anchor: GraphSnapshotV1,
        deltas: Vec<StructuralDeltaV1>,
    ) -> Result<Self, GraphReplayError> {
        let authoritative = replay_snapshot(&anchor, &deltas)?;
        Ok(Self {
            anchor,
            deltas,
            authoritative,
        })
    }

    /// Reopens the authority graph only after every persisted transition and
    /// final checkpoint agree. There is intentionally no Genesis fallback.
    pub fn reopen(&self) -> Result<(u64, SparseGraph), GraphReplayError> {
        let replayed = replay_snapshot(&self.anchor, &self.deltas)?;
        let authoritative = self.authoritative.restore()?;
        if replayed.formula_version != self.authoritative.formula_version
            || replayed.revision != self.authoritative.revision
            || replayed.graph_digest != self.authoritative.graph_digest
            || replayed.canonical_bytes != self.authoritative.canonical_bytes
            || replayed.canonical_bytes != authoritative.canonical_bytes()
        {
            return Err(GraphReplayError::AuthoritativeSnapshotMismatch);
        }
        Ok((replayed.revision, authoritative))
    }
}

fn replay_snapshot(
    anchor: &GraphSnapshotV1,
    deltas: &[StructuralDeltaV1],
) -> Result<GraphSnapshotV1, GraphReplayError> {
    let mut graph = anchor.restore()?;
    let mut revision = anchor.revision;
    let mut digest = anchor.graph_digest;
    let mut expected_delta_sequence = 1u64;

    for delta in deltas {
        if delta.rule_digest != GRAPH_REPLAY_RULE_DIGEST_V1 {
            return Err(GraphReplayError::DeltaRejected);
        }
        if delta.base_revision != revision {
            return Err(GraphReplayError::RevisionDiscontinuity);
        }
        if delta.delta_sequence != expected_delta_sequence {
            return Err(GraphReplayError::DeltaSequenceDiscontinuity);
        }
        if delta.base_graph_digest != digest {
            return Err(GraphReplayError::BeforeDigestMismatch);
        }

        let next = apply_delta(revision, &digest, revision, &graph, delta).map_err(|error| {
            if error == DeltaError::AfterGraphDigestMismatch {
                GraphReplayError::AfterDigestMismatch
            } else {
                GraphReplayError::DeltaRejected
            }
        })?;
        let next_revision = revision
            .checked_add(1)
            .ok_or(GraphReplayError::RevisionDiscontinuity)?;
        let snapshot = GraphSnapshotV1::from_graph(GRAPH_REPLAY_FORMULA_V1, next_revision, &next)?;
        if snapshot.graph_digest != delta.after_graph_digest {
            return Err(GraphReplayError::AfterDigestMismatch);
        }

        // Restoring each freshly encoded snapshot proves that the transition's
        // graph bytes can be decoded and canonically re-encoded before it is
        // used as the next delta's authority.
        graph = snapshot.restore()?;
        revision = snapshot.revision;
        digest = snapshot.graph_digest;
        expected_delta_sequence = expected_delta_sequence
            .checked_add(1)
            .ok_or(GraphReplayError::DeltaSequenceDiscontinuity)?;
    }

    GraphSnapshotV1::from_graph(GRAPH_REPLAY_FORMULA_V1, revision, &graph)
}

fn graph_is_replay_valid(graph: &SparseGraph) -> bool {
    graph.validate()
        && graph.edges.iter().all(|edge| {
            edge.operator_id < V1_OPERATOR_TYPE_COUNT && edge.delay_class < V1_DELAY_CLASS_COUNT
        })
}

fn decode_canonical_graph(bytes: &[u8]) -> Option<SparseGraph> {
    let mut reader = ByteReader::new(bytes);
    let row_count = reader.read_u32()? as usize;
    if row_count != NEURON_SLOTS + 1 {
        return None;
    }

    let mut row_offsets = Vec::with_capacity(row_count);
    for _ in 0..row_count {
        row_offsets.push(reader.read_u32()?);
    }

    let edge_count = reader.read_u32()? as usize;
    if edge_count > EDGE_CAPACITY {
        return None;
    }
    let mut edges = Vec::with_capacity(edge_count);
    for _ in 0..edge_count {
        edges.push(Synapse {
            target: reader.read_u32()?,
            weight: reader.read_i16()?,
            eligibility: reader.read_i16()?,
            stability: reader.read_u16()?,
            last_used_epoch: reader.read_u16()?,
            operator_id: reader.read_u8()?,
            delay_class: reader.read_u8()?,
            flags: reader.read_u16()?,
        });
    }
    if !reader.finished() {
        return None;
    }

    let graph = SparseGraph { row_offsets, edges };
    graph_is_replay_valid(&graph).then_some(graph)
}

struct ByteReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> ByteReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn read_u8(&mut self) -> Option<u8> {
        Some(*self.take(1)?.first()?)
    }

    fn read_u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn read_i16(&mut self) -> Option<i16> {
        Some(i16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn read_u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn take(&mut self, width: usize) -> Option<&'a [u8]> {
        let end = self.position.checked_add(width)?;
        let value = self.bytes.get(self.position..end)?;
        self.position = end;
        Some(value)
    }

    fn finished(&self) -> bool {
        self.position == self.bytes.len()
    }
}
