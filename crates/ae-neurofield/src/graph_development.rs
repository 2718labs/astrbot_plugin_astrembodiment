//! Deterministic Genesis sparse-graph projector.
//!
//! Graph formula v1 has a fixed 16,384-node topology with exactly sixteen
//! outgoing edges per source. Its only variable inputs are the committed
//! manifest digest and the development-seed digest.

use std::collections::BTreeSet;

use ae_contracts::{wire, Digest};

use crate::{SparseGraph, Synapse, EDGE_CAPACITY, NEURON_SLOTS, REGION_LAYOUT};

const GRAPH_DEVELOPMENT_V1_DOMAIN: &[u8] = b"ae.neurofield.graph-development.v1";
const GRAPH_FORMULA_V1_BYTES: &[u8] = b"graph-formula-v1";
const EDGES_PER_SOURCE: usize = 16;
const V1_EDGE_TARGET: usize = NEURON_SLOTS * EDGES_PER_SOURCE;
const TARGET_REGION_QUOTAS: [u8; 9] = [2, 2, 2, 2, 2, 2, 2, 1, 1];
const OPERATOR_TYPE_COUNT: u8 = 4;
const DELAY_CLASS_COUNT: u8 = 8;

/// Versioned set of all fixed Genesis graph parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphFormula {
    V1,
}

/// Errors emitted by the bounded graph projector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphDevelopmentError {
    EdgeCapacityExceeded,
}

/// Fixed, platform-independent SplitMix64 stream seeded from a
/// domain-separated BLAKE3 digest. All integer conversion is little-endian.
struct V1Prng {
    state: u64,
}

impl V1Prng {
    fn for_source(manifest_digest: &Digest, development_seed_digest: &Digest, source: u32) -> Self {
        let source_bytes = source.to_le_bytes();
        let seed = wire::domain_hash(
            GRAPH_DEVELOPMENT_V1_DOMAIN,
            &[
                GRAPH_FORMULA_V1_BYTES,
                manifest_digest,
                development_seed_digest,
                &source_bytes,
            ],
        );
        let low = u64::from_le_bytes(seed[..8].try_into().expect("fixed digest width"));
        let high = u64::from_le_bytes(seed[8..16].try_into().expect("fixed digest width"));
        Self { state: low ^ high }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }
}

fn synapse_from_stream(target: u32, stream: &mut V1Prng) -> Synapse {
    let weight = (stream.next_u64() % 2_001) as i16 - 1_000;
    let eligibility = (stream.next_u64() % 2_001) as i16 - 1_000;
    let stability = (stream.next_u64() % u16::MAX as u64) as u16 + 1;
    let last_used_epoch = stream.next_u64() as u16;
    let operator_id = (stream.next_u64() % OPERATOR_TYPE_COUNT as u64) as u8;
    let delay_class = (stream.next_u64() % DELAY_CLASS_COUNT as u64) as u8;

    Synapse {
        target,
        weight,
        eligibility,
        stability,
        last_used_epoch,
        operator_id,
        delay_class,
        flags: 0,
    }
}

/// Develops the deterministic non-empty Genesis graph for formula v1.
///
/// Target-region quotas, candidate sampling, collision resolution, synapse
/// attributes, and edge order are all fixed by [`GraphFormula::V1`]. No host
/// resource characteristic participates in this function.
pub fn develop_graph(
    manifest_digest: &Digest,
    development_seed_digest: &Digest,
    formula: GraphFormula,
) -> Result<SparseGraph, GraphDevelopmentError> {
    match formula {
        GraphFormula::V1 => develop_graph_v1(manifest_digest, development_seed_digest),
    }
}

fn develop_graph_v1(
    manifest_digest: &Digest,
    development_seed_digest: &Digest,
) -> Result<SparseGraph, GraphDevelopmentError> {
    debug_assert_eq!(
        TARGET_REGION_QUOTAS
            .iter()
            .map(|quota| *quota as usize)
            .sum::<usize>(),
        EDGES_PER_SOURCE
    );
    if V1_EDGE_TARGET > EDGE_CAPACITY {
        return Err(GraphDevelopmentError::EdgeCapacityExceeded);
    }

    let mut row_offsets = Vec::with_capacity(NEURON_SLOTS + 1);
    let mut edges = Vec::with_capacity(V1_EDGE_TARGET);
    row_offsets.push(0);

    for source in 0..NEURON_SLOTS {
        let mut stream =
            V1Prng::for_source(manifest_digest, development_seed_digest, source as u32);
        let mut selected_targets = BTreeSet::new();
        let mut source_edges = Vec::with_capacity(EDGES_PER_SOURCE);

        for ((target_start, target_count), quota) in
            REGION_LAYOUT.iter().copied().zip(TARGET_REGION_QUOTAS)
        {
            for _ in 0..quota {
                let mut target = target_start + (stream.next_u64() % target_count as u64) as usize;
                while !selected_targets.insert(target as u32) {
                    target = target_start + (target + 1 - target_start) % target_count;
                }
                source_edges.push(synapse_from_stream(target as u32, &mut stream));
            }
        }

        source_edges.sort_unstable_by_key(|edge| edge.target);
        debug_assert_eq!(source_edges.len(), EDGES_PER_SOURCE);
        edges.extend(source_edges);
        row_offsets.push(edges.len() as u32);
    }

    debug_assert_eq!(edges.len(), V1_EDGE_TARGET);
    Ok(SparseGraph { row_offsets, edges })
}

impl SparseGraph {
    /// Canonical row-offset and synapse encoding used for graph equality,
    /// golden vectors, and graph digests. Every integer uses little-endian
    /// representation and source order is represented by row order.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut body = Vec::with_capacity(self.row_offsets.len() * 4 + self.edges.len() * 16 + 8);
        body.extend_from_slice(&(self.row_offsets.len() as u32).to_le_bytes());
        for offset in &self.row_offsets {
            body.extend_from_slice(&offset.to_le_bytes());
        }
        body.extend_from_slice(&(self.edges.len() as u32).to_le_bytes());
        for edge in &self.edges {
            body.extend_from_slice(&edge.target.to_le_bytes());
            body.extend_from_slice(&edge.weight.to_le_bytes());
            body.extend_from_slice(&edge.eligibility.to_le_bytes());
            body.extend_from_slice(&edge.stability.to_le_bytes());
            body.extend_from_slice(&edge.last_used_epoch.to_le_bytes());
            body.push(edge.operator_id);
            body.push(edge.delay_class);
            body.extend_from_slice(&edge.flags.to_le_bytes());
        }
        body
    }
}
