//! Fail-closed replay for persisted sparse-graph history.
//!
//! A persisted history is anchored by one canonical snapshot and sealed with
//! the authoritative snapshot reached by applying every structural delta.
//! Reopening never falls back to Genesis: any formula, revision, digest, or
//! canonical-byte disagreement is rejected before a graph is returned.

use std::fmt;

use ae_contracts::Digest;
use serde::{
    de::{DeserializeSeed, Error as _, SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize,
};

use crate::{
    graph_digest, structural_delta::apply_delta_with_rule_digest, DeltaError, SparseGraph,
    StructuralDeltaV1, Synapse, EDGE_CAPACITY, NEURON_SLOTS,
};

/// The only graph replay formula supported by this implementation.
pub const GRAPH_REPLAY_FORMULA_V1: u16 = 1;

/// Maximum deltas admitted between authoritative v1 graph snapshots.
///
/// A writer must compact and seal a fresh snapshot before appending delta 65.
/// Combined with crate::MAX_OPERATIONS_PER_DELTA_V1, a replay interval can
/// describe at most 262,144 operations: one complete canonical v1 graph scale.
pub const MAX_REPLAY_DELTAS_V1: usize = 64;

/// Exact largest canonical encoding admitted for a v1 graph snapshot.
///
/// The layout is the row-count word, 16,385 row offsets, the edge-count
/// word, and 524,288 canonical 16-byte synapses.
pub const MAX_SNAPSHOT_CANONICAL_BYTES_V1: usize = 8_454_156;

/// Version of the closed replay/delta admission policy layered over formula v1.
pub const GRAPH_ADMISSION_PROFILE_VERSION_V1: u16 = 1;

/// Domain separator for the closed admission-policy digest.
pub const GRAPH_ADMISSION_DOMAIN_V1: &str = "ae.neurofield.graph-admission.v1";

const V1_OPERATOR_TYPE_COUNT: u8 = 4;
const V1_DELAY_CLASS_COUNT: u8 = 8;

/// Persisted admission identity for graph replay histories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphAdmissionProfileV1 {
    /// Immutable-source wire data without an admission-profile field. It may
    /// only be consumed through explicit migration.
    #[serde(rename = "LegacySourceV1")]
    LegacySourceV1,
    /// Closed profile with bounded snapshots, histories, and deltas.
    #[serde(rename = "ClosedV1")]
    ClosedV1,
}

/// A complete, canonical graph checkpoint.
///
/// The graph's formula version, authority revision, digest, and bytes are all
/// persisted together. `canonical_bytes` is exactly
/// [`SparseGraph::canonical_bytes`], rather than a serde representation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphSnapshotV1 {
    pub formula_version: u16,
    pub revision: u64,
    pub graph_digest: Digest,
    #[serde(deserialize_with = "deserialize_canonical_bytes_v1")]
    pub canonical_bytes: Vec<u8>,
}

/// A sealed graph history suitable for persistence and later reopening.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GraphReplayV1 {
    pub anchor: GraphSnapshotV1,
    pub deltas: Vec<StructuralDeltaV1>,
    pub authoritative: GraphSnapshotV1,
    pub admission_profile: GraphAdmissionProfileV1,
    pub admission_profile_digest: Digest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphReplayWireV1 {
    anchor: GraphSnapshotV1,
    #[serde(deserialize_with = "deserialize_deltas_v1")]
    deltas: Vec<StructuralDeltaV1>,
    authoritative: GraphSnapshotV1,
    #[serde(default)]
    admission_profile: Option<GraphAdmissionProfileV1>,
    #[serde(default)]
    admission_profile_digest: Option<Digest>,
}

impl<'de> Deserialize<'de> for GraphReplayV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = GraphReplayWireV1::deserialize(deserializer)?;
        let (admission_profile, admission_profile_digest) =
            match (wire.admission_profile, wire.admission_profile_digest) {
                (None, None) => (
                    GraphAdmissionProfileV1::LegacySourceV1,
                    legacy_graph_replay_rule_digest(GRAPH_REPLAY_FORMULA_V1).map_err(|_| {
                        D::Error::custom("legacy v1 graph formula digest is unavailable")
                    })?,
                ),
                (Some(profile), Some(digest)) => (profile, digest),
                _ => {
                    return Err(D::Error::custom(
                        "v1 admission profile and digest must be persisted together",
                    ));
                }
            };
        Ok(Self {
            anchor: wire.anchor,
            deltas: wire.deltas,
            authoritative: wire.authoritative,
            admission_profile,
            admission_profile_digest,
        })
    }
}

fn deserialize_canonical_bytes_v1<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    struct CanonicalBytesVisitor;
    struct CanonicalByteSeed {
        admitted: bool,
    }

    impl<'de> DeserializeSeed<'de> for CanonicalByteSeed {
        type Value = u8;

        fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            if !self.admitted {
                return Err(D::Error::custom(
                    "v1 snapshot canonical bytes exceed admission limit",
                ));
            }
            u8::deserialize(deserializer)
        }
    }

    impl<'de> Visitor<'de> for CanonicalBytesVisitor {
        type Value = Vec<u8>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                formatter,
                "at most {MAX_SNAPSHOT_CANONICAL_BYTES_V1} canonical v1 snapshot bytes"
            )
        }

        fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let size_hint = sequence.size_hint().unwrap_or(0);
            if size_hint > MAX_SNAPSHOT_CANONICAL_BYTES_V1 {
                return Err(A::Error::custom(
                    "v1 snapshot canonical bytes exceed admission limit",
                ));
            }

            let mut bytes = Vec::with_capacity(size_hint.min(MAX_SNAPSHOT_CANONICAL_BYTES_V1));
            loop {
                let next_count = bytes.len().checked_add(1).ok_or_else(|| {
                    A::Error::custom("v1 snapshot canonical bytes exceed admission limit")
                })?;
                match sequence.next_element_seed(CanonicalByteSeed {
                    admitted: next_count <= MAX_SNAPSHOT_CANONICAL_BYTES_V1,
                })? {
                    Some(byte) if next_count <= MAX_SNAPSHOT_CANONICAL_BYTES_V1 => bytes.push(byte),
                    Some(_) => {
                        return Err(A::Error::custom(
                            "v1 snapshot canonical bytes exceed admission limit",
                        ));
                    }
                    None => return Ok(bytes),
                }
            }
        }
    }

    deserializer.deserialize_seq(CanonicalBytesVisitor)
}

fn deserialize_deltas_v1<'de, D>(deserializer: D) -> Result<Vec<StructuralDeltaV1>, D::Error>
where
    D: Deserializer<'de>,
{
    struct DeltasVisitor;

    impl<'de> Visitor<'de> for DeltasVisitor {
        type Value = Vec<StructuralDeltaV1>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                formatter,
                "at most {MAX_REPLAY_DELTAS_V1} deltas before v1 snapshot compaction"
            )
        }

        fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let size_hint = sequence.size_hint().unwrap_or(0);
            if size_hint > MAX_REPLAY_DELTAS_V1 {
                return Err(A::Error::custom(
                    "v1 graph replay requires snapshot compaction",
                ));
            }

            let mut deltas = Vec::with_capacity(size_hint.min(MAX_REPLAY_DELTAS_V1));
            while let Some(delta) = sequence.next_element()? {
                if deltas.len() == MAX_REPLAY_DELTAS_V1 {
                    return Err(A::Error::custom(
                        "v1 graph replay requires snapshot compaction",
                    ));
                }
                deltas.push(delta);
            }
            Ok(deltas)
        }
    }

    deserializer.deserialize_seq(DeltasVisitor)
}

/// Public, non-sensitive rejection classifications for graph replay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphReplayError {
    TooManyDeltas,
    SnapshotCanonicalBytesTooLarge,
    UnsupportedFormulaVersion,
    RuleDescriptorMismatch,
    AdmissionProfileMismatch,
    LegacyAdmissionProfileRequiresMigration,
    CanonicalEncodingMismatch,
    SnapshotDigestMismatch,
    RevisionDiscontinuity,
    DeltaSequenceDiscontinuity,
    BeforeDigestMismatch,
    AfterDigestMismatch,
    DeltaRejected,
    AuthoritativeSnapshotMismatch,
}

/// Returns the frozen source formula descriptor for v1 graph transitions.
///
/// This descriptor's digest no longer serves as the admission identity for
/// newly sealed histories. [`graph_admission_profile_descriptor`] layers the
/// closed resource and schema policy over these unchanged formula semantics.
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

/// Returns the immutable-source formula digest used by historical v1 deltas.
pub fn legacy_graph_replay_rule_digest(formula_version: u16) -> Result<Digest, GraphReplayError> {
    graph_replay_rule_digest(formula_version)
}

/// Returns the canonical descriptor for a persisted replay admission profile.
pub fn graph_admission_profile_descriptor(
    profile: GraphAdmissionProfileV1,
) -> Result<String, GraphReplayError> {
    match profile {
        GraphAdmissionProfileV1::LegacySourceV1 => {
            graph_replay_rule_descriptor(GRAPH_REPLAY_FORMULA_V1)
        }
        GraphAdmissionProfileV1::ClosedV1 => {
            let formula_digest = legacy_graph_replay_rule_digest(GRAPH_REPLAY_FORMULA_V1)?;
            Ok(format!(
                concat!(
                    "ae-neurofield-graph-admission-profile-v1;",
                    "domain={};",
                    "profile_version={};",
                    "formula_version={};",
                    "formula_rule_sha256={};",
                    "max_snapshot_canonical_bytes={};",
                    "max_replay_deltas={};",
                    "max_operations_per_delta={};",
                    "unknown_fields=deny-GraphSnapshotV1-GraphReplayV1-StructuralDeltaV1-EdgeOperationV1-Synapse;",
                    "legacy_source_v1=explicit-migration-required"
                ),
                GRAPH_ADMISSION_DOMAIN_V1,
                GRAPH_ADMISSION_PROFILE_VERSION_V1,
                GRAPH_REPLAY_FORMULA_V1,
                digest_hex(&formula_digest),
                MAX_SNAPSHOT_CANONICAL_BYTES_V1,
                MAX_REPLAY_DELTAS_V1,
                crate::MAX_OPERATIONS_PER_DELTA_V1,
            ))
        }
    }
}

/// Derives the domain-separated digest persisted by replay envelopes and
/// every newly bound structural delta.
pub fn graph_admission_profile_digest(
    profile: GraphAdmissionProfileV1,
) -> Result<Digest, GraphReplayError> {
    Ok(sha256_digest(
        graph_admission_profile_descriptor(profile)?.as_bytes(),
    ))
}

/// Binds a structural delta to the current closed replay admission profile.
pub fn bind_delta_to_graph_replay_rule(
    formula_version: u16,
    delta: &mut StructuralDeltaV1,
) -> Result<(), GraphReplayError> {
    if formula_version != GRAPH_REPLAY_FORMULA_V1 {
        return Err(GraphReplayError::UnsupportedFormulaVersion);
    }
    delta.rule_digest = graph_admission_profile_digest(GraphAdmissionProfileV1::ClosedV1)?;
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

        let canonical_bytes = graph.canonical_bytes();
        if canonical_bytes.len() > MAX_SNAPSHOT_CANONICAL_BYTES_V1 {
            return Err(GraphReplayError::SnapshotCanonicalBytesTooLarge);
        }
        let snapshot = Self {
            formula_version,
            revision,
            graph_digest: graph_digest(graph),
            canonical_bytes,
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
        if self.canonical_bytes.len() > MAX_SNAPSHOT_CANONICAL_BYTES_V1 {
            return Err(GraphReplayError::SnapshotCanonicalBytesTooLarge);
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
        if deltas.len() > MAX_REPLAY_DELTAS_V1 {
            return Err(GraphReplayError::TooManyDeltas);
        }
        let admission_profile = GraphAdmissionProfileV1::ClosedV1;
        let admission_profile_digest = graph_admission_profile_digest(admission_profile)?;
        let authoritative = replay_snapshot(&anchor, &deltas, &admission_profile_digest)?;
        Ok(Self {
            anchor,
            deltas,
            authoritative,
            admission_profile,
            admission_profile_digest,
        })
    }

    /// Reopens the authority graph only after every persisted transition and
    /// final checkpoint agree. There is intentionally no Genesis fallback.
    pub fn reopen(&self) -> Result<(u64, SparseGraph), GraphReplayError> {
        if self.admission_profile == GraphAdmissionProfileV1::LegacySourceV1 {
            return Err(GraphReplayError::LegacyAdmissionProfileRequiresMigration);
        }
        let expected = graph_admission_profile_digest(GraphAdmissionProfileV1::ClosedV1)?;
        if self.admission_profile != GraphAdmissionProfileV1::ClosedV1
            || self.admission_profile_digest != expected
        {
            return Err(GraphReplayError::AdmissionProfileMismatch);
        }
        self.reopen_with_rule_digest(&expected)
    }

    /// Validates an immutable-source history under its historical rule digest,
    /// then explicitly rebinds it to the closed admission profile.
    pub fn migrate_legacy_source_v1(mut self) -> Result<Self, GraphReplayError> {
        if self.admission_profile != GraphAdmissionProfileV1::LegacySourceV1 {
            return Err(GraphReplayError::AdmissionProfileMismatch);
        }
        let legacy = legacy_graph_replay_rule_digest(GRAPH_REPLAY_FORMULA_V1)?;
        if self.admission_profile_digest != legacy {
            return Err(GraphReplayError::AdmissionProfileMismatch);
        }
        self.reopen_with_rule_digest(&legacy)?;

        let closed = graph_admission_profile_digest(GraphAdmissionProfileV1::ClosedV1)?;
        for delta in &mut self.deltas {
            delta.rule_digest = closed;
        }
        self.admission_profile = GraphAdmissionProfileV1::ClosedV1;
        self.admission_profile_digest = closed;
        self.reopen()?;
        Ok(self)
    }

    fn reopen_with_rule_digest(
        &self,
        rule_digest: &Digest,
    ) -> Result<(u64, SparseGraph), GraphReplayError> {
        let replayed = replay_snapshot(&self.anchor, &self.deltas, rule_digest)?;
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
    rule_digest: &Digest,
) -> Result<GraphSnapshotV1, GraphReplayError> {
    if deltas.len() > MAX_REPLAY_DELTAS_V1 {
        return Err(GraphReplayError::TooManyDeltas);
    }
    let mut graph = anchor.restore()?;
    let mut revision = anchor.revision;
    let mut digest = anchor.graph_digest;
    let mut expected_delta_sequence = 1u64;
    for delta in deltas {
        if delta.rule_digest != *rule_digest {
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

        let next =
            apply_delta_with_rule_digest(revision, &digest, revision, &graph, delta, rule_digest)
                .map_err(|error| {
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

fn digest_hex(value: &Digest) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use serde::de::{self, DeserializeSeed, SeqAccess, Visitor};

    use super::{deserialize_canonical_bytes_v1, MAX_SNAPSHOT_CANONICAL_BYTES_V1};

    struct HugeDeclaredBinarySequence<'a> {
        element_requested: &'a Cell<bool>,
    }

    struct HugeSequenceAccess<'a> {
        element_requested: &'a Cell<bool>,
    }

    impl<'de> de::Deserializer<'de> for HugeDeclaredBinarySequence<'_> {
        type Error = de::value::Error;

        fn deserialize_any<V>(self, _visitor: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            Err(<Self::Error as de::Error>::custom(
                "expected bounded sequence decoding",
            ))
        }

        fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            visitor.visit_seq(HugeSequenceAccess {
                element_requested: self.element_requested,
            })
        }

        fn deserialize_byte_buf<V>(self, _visitor: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            panic!("byte-buffer decoding can allocate before the v1 admission bound is checked")
        }

        serde::forward_to_deserialize_any! {
            bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
            bytes option unit unit_struct newtype_struct tuple tuple_struct map struct enum
            identifier ignored_any
        }
    }

    impl<'de> SeqAccess<'de> for HugeSequenceAccess<'_> {
        type Error = de::value::Error;

        fn next_element_seed<T>(&mut self, _seed: T) -> Result<Option<T::Value>, Self::Error>
        where
            T: DeserializeSeed<'de>,
        {
            self.element_requested.set(true);
            Err(<Self::Error as de::Error>::custom(
                "an element was requested before rejecting the declared length",
            ))
        }

        fn size_hint(&self) -> Option<usize> {
            Some(MAX_SNAPSHOT_CANONICAL_BYTES_V1 + 1)
        }
    }

    #[test]
    fn oversized_binary_length_hint_rejects_before_requesting_elements() {
        let element_requested = Cell::new(false);
        let error = deserialize_canonical_bytes_v1(HugeDeclaredBinarySequence {
            element_requested: &element_requested,
        })
        .expect_err("an oversized declared sequence must fail closed");

        assert_eq!(
            error.to_string(),
            "v1 snapshot canonical bytes exceed admission limit"
        );
        assert!(
            !element_requested.get(),
            "the bounded visitor must reject the size hint before requesting bytes"
        );
    }
}
