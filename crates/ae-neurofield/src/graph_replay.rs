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
    RuleDescriptorMismatch,
    CanonicalEncodingMismatch,
    SnapshotDigestMismatch,
    RevisionDiscontinuity,
    DeltaSequenceDiscontinuity,
    BeforeDigestMismatch,
    AfterDigestMismatch,
    DeltaRejected,
    AuthoritativeSnapshotMismatch,
}

/// Returns the canonical rule descriptor that every v1 replay delta binds to.
///
/// This descriptor deliberately names the versioned validation and transition
/// rules enforced by [`GraphSnapshotV1`] and [`apply_delta`].  It is the
/// single preimage for the persisted `StructuralDeltaV1::rule_digest`.
pub fn graph_replay_rule_descriptor(formula_version: u16) -> Result<String, GraphReplayError> {
    if formula_version != GRAPH_REPLAY_FORMULA_V1 {
        return Err(GraphReplayError::UnsupportedFormulaVersion);
    }

    Ok(format!(
        concat!(
            "graph-replay-rule-v1;",
            "formula_version={};",
            "snapshot=GraphSnapshotV1-canonical-bytes-graph-digest;",
            "delta_schema=StructuralDeltaV1;",
            "delta_sequence=contiguous-u64-from-1;",
            "apply_delta=v1-cas-canonical-operations;",
            "edge_constraints=source-target<{}-operator_id<{}-delay_class<{};",
            "transition=add-update-remove-canonical-order-no-duplicates;",
            "after_digest=canonical-graph-digest"
        ),
        formula_version, NEURON_SLOTS, V1_OPERATOR_TYPE_COUNT, V1_DELAY_CLASS_COUNT,
    ))
}

/// Derives the persisted rule digest from the versioned canonical descriptor
/// with SHA-256.  This is the authoritative producer and verifier value.
pub fn graph_replay_rule_digest(formula_version: u16) -> Result<Digest, GraphReplayError> {
    Ok(sha256_digest(
        graph_replay_rule_descriptor(formula_version)?.as_bytes(),
    ))
}

/// Verifies an externally supplied descriptor before deriving its rule digest.
///
/// Persistence never admits alternate spellings or incomplete descriptions:
/// only the exact canonical descriptor for its formula version is accepted.
pub fn graph_replay_rule_digest_for_descriptor(
    formula_version: u16,
    descriptor: &str,
) -> Result<Digest, GraphReplayError> {
    let canonical = graph_replay_rule_descriptor(formula_version)?;
    if descriptor != canonical {
        return Err(GraphReplayError::RuleDescriptorMismatch);
    }
    Ok(sha256_digest(canonical.as_bytes()))
}

/// Binds a structural delta to the authoritative graph replay rule contract.
pub fn bind_delta_to_graph_replay_rule(
    formula_version: u16,
    delta: &mut StructuralDeltaV1,
) -> Result<(), GraphReplayError> {
    delta.rule_digest = graph_replay_rule_digest(formula_version)?;
    Ok(())
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
    let rule_digest = graph_replay_rule_digest(GRAPH_REPLAY_FORMULA_V1)?;

    for delta in deltas {
        if delta.rule_digest != rule_digest {
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

fn sha256_digest(input: &[u8]) -> Digest {
    const INITIAL: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    const ROUND: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let bit_length = (input.len() as u64) * 8;
    let mut padded = Vec::with_capacity(input.len() + 72);
    padded.extend_from_slice(input);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_length.to_be_bytes());

    let mut hash = INITIAL;
    let (chunks, remainder) = padded.as_chunks::<64>();
    debug_assert!(remainder.is_empty());
    for chunk in chunks {
        let mut words = [0_u32; 64];
        for (index, word) in words[..16].iter_mut().enumerate() {
            let offset = index * 4;
            *word = u32::from_be_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }
        for index in 16..64 {
            let sigma0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let sigma1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(sigma0)
                .wrapping_add(words[index - 7])
                .wrapping_add(sigma1);
        }

        let mut a = hash[0];
        let mut b = hash[1];
        let mut c = hash[2];
        let mut d = hash[3];
        let mut e = hash[4];
        let mut f = hash[5];
        let mut g = hash[6];
        let mut h = hash[7];

        for index in 0..64 {
            let upper_sigma1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ ((!e) & g);
            let temporary1 = h
                .wrapping_add(upper_sigma1)
                .wrapping_add(choice)
                .wrapping_add(ROUND[index])
                .wrapping_add(words[index]);
            let upper_sigma0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temporary2 = upper_sigma0.wrapping_add(majority);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temporary1);
            d = c;
            c = b;
            b = a;
            a = temporary1.wrapping_add(temporary2);
        }

        hash[0] = hash[0].wrapping_add(a);
        hash[1] = hash[1].wrapping_add(b);
        hash[2] = hash[2].wrapping_add(c);
        hash[3] = hash[3].wrapping_add(d);
        hash[4] = hash[4].wrapping_add(e);
        hash[5] = hash[5].wrapping_add(f);
        hash[6] = hash[6].wrapping_add(g);
        hash[7] = hash[7].wrapping_add(h);
    }

    let mut output = [0_u8; 32];
    for (index, word) in hash.iter().enumerate() {
        output[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    output
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
