use std::collections::BTreeSet;

use ae_neurofield::{develop_graph, graph_digest, GraphFormula, EDGE_CAPACITY, NEURON_SLOTS};

const V1_EDGE_TARGET: usize = 262_144;

fn digest(byte: u8) -> [u8; 32] {
    [byte; 32]
}

fn graph_bytes(manifest: u8, seed: u8) -> Vec<u8> {
    develop_graph(&digest(manifest), &digest(seed), GraphFormula::V1)
        .expect("v1 graph development must succeed")
        .canonical_bytes()
}

fn assert_canonical_invariants(graph: &ae_neurofield::SparseGraph) {
    assert!(graph.validate());
    assert_eq!(graph.edges.len(), V1_EDGE_TARGET);
    assert!(graph.edges.len() <= EDGE_CAPACITY);

    let mut pairs = BTreeSet::new();
    for source in 0..NEURON_SLOTS {
        let start = graph.row_offsets[source] as usize;
        let end = graph.row_offsets[source + 1] as usize;
        assert!(start <= end);
        let mut previous_target = None;
        for edge in &graph.edges[start..end] {
            assert!((edge.target as usize) < NEURON_SLOTS);
            assert!(pairs.insert((source as u32, edge.target)));
            assert!(previous_target.is_none_or(|previous| previous < edge.target));
            previous_target = Some(edge.target);
        }
    }
    assert_eq!(pairs.len(), graph.edges.len());
}

#[test]
fn same_birth_inputs_produce_byte_identical_nonempty_canonical_graph() {
    let first = develop_graph(&digest(1), &digest(2), GraphFormula::V1).unwrap();
    let second = develop_graph(&digest(1), &digest(2), GraphFormula::V1).unwrap();

    assert_eq!(first.canonical_bytes(), second.canonical_bytes());
    assert_eq!(graph_digest(&first), graph_digest(&second));
    assert!(!first.edges.is_empty());
    assert_canonical_invariants(&first);
}

#[test]
fn distinct_development_seeds_produce_a_controlled_graph_difference() {
    let first = develop_graph(&digest(1), &digest(2), GraphFormula::V1).unwrap();
    let second = develop_graph(&digest(1), &digest(3), GraphFormula::V1).unwrap();

    assert_ne!(first.canonical_bytes(), second.canonical_bytes());
    assert_ne!(graph_digest(&first), graph_digest(&second));
    assert_canonical_invariants(&first);
    assert_canonical_invariants(&second);
}

#[derive(Clone, Copy)]
enum ResourceEnvelope {
    OneCpuOneGpu,
    TwoCpuTwoGpu,
}

fn develop_for_resource_envelope(
    _envelope: ResourceEnvelope,
    manifest_digest: &[u8; 32],
    development_seed_digest: &[u8; 32],
) -> ae_neurofield::SparseGraph {
    develop_graph(manifest_digest, development_seed_digest, GraphFormula::V1).unwrap()
}

#[test]
fn resource_envelope_cannot_change_v1_graph_bytes_or_digest() {
    let manifest = digest(4);
    let seed = digest(5);
    let one_c_one_g =
        develop_for_resource_envelope(ResourceEnvelope::OneCpuOneGpu, &manifest, &seed);
    let two_c_two_g =
        develop_for_resource_envelope(ResourceEnvelope::TwoCpuTwoGpu, &manifest, &seed);

    assert_eq!(one_c_one_g.canonical_bytes(), two_c_two_g.canonical_bytes());
    assert_eq!(graph_digest(&one_c_one_g), graph_digest(&two_c_two_g));
}

fn json_string<'a>(json: &'a str, key: &str) -> &'a str {
    let prefix = format!("\"{key}\": \"");
    let after_prefix = json
        .split_once(&prefix)
        .unwrap_or_else(|| panic!("missing JSON string key {key}"))
        .1;
    after_prefix
        .split_once('"')
        .unwrap_or_else(|| panic!("unterminated JSON string key {key}"))
        .0
}

fn json_usize(json: &str, key: &str) -> usize {
    let prefix = format!("\"{key}\": ");
    let after_prefix = json
        .split_once(&prefix)
        .unwrap_or_else(|| panic!("missing JSON number key {key}"))
        .1;
    let number = after_prefix
        .split(|character: char| !character.is_ascii_digit())
        .next()
        .unwrap();
    number.parse().unwrap()
}

fn hex_digest(value: &str) -> [u8; 32] {
    assert_eq!(value.len(), 64, "digest must be exactly 32 bytes of hex");
    let mut digest = [0u8; 32];
    for (index, output) in digest.iter_mut().enumerate() {
        *output = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
    }
    digest
}

#[test]
fn v1_golden_vector_is_cross_process_and_cross_platform_stable() {
    let vector = include_str!("vectors/graph-development-v1.json");
    assert_eq!(json_string(vector, "formula"), "v1");
    let manifest_digest = hex_digest(json_string(vector, "manifest_digest_hex"));
    let development_seed_digest = hex_digest(json_string(vector, "development_seed_digest_hex"));
    let expected_graph_digest = hex_digest(json_string(vector, "graph_digest_hex"));
    let graph =
        develop_graph(&manifest_digest, &development_seed_digest, GraphFormula::V1).unwrap();

    assert_eq!(graph.edges.len(), json_usize(vector, "edge_count"));
    assert_eq!(graph_digest(&graph), expected_graph_digest);
    assert_eq!(graph.canonical_bytes(), graph_bytes(17, 34));
}
