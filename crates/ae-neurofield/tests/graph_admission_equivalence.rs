use ae_neurofield::{
    apply_delta, bind_delta_to_graph_replay_rule, graph_admission_profile_descriptor,
    graph_admission_profile_digest, graph_digest, legacy_graph_replay_rule_digest, DeltaError,
    EdgeOperationV1, GraphAdmissionProfileV1, GraphReplayError, GraphReplayV1, GraphSnapshotV1,
    SparseGraph, StructuralDeltaV1, Synapse, EDGE_CAPACITY, GRAPH_ADMISSION_DOMAIN_V1,
    GRAPH_ADMISSION_PROFILE_VERSION_V1, GRAPH_REPLAY_FORMULA_V1, MAX_OPERATIONS_PER_DELTA_V1,
    MAX_REPLAY_DELTAS_V1, MAX_SNAPSHOT_CANONICAL_BYTES_V1, NEURON_SLOTS,
};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

const SOURCE_COMMIT: &str = "710829ae5d3bef82ce818754354272517cb28056";
const HISTORICAL_REPLAY_SHA256: &str =
    "8dbce3645097d566c7b5db5b87b6f774c226e4c077293342ea423862d0deace0";
const SOURCE_COMPOSITE_SHA256: &str =
    "2d1cd77abf21cf0224ab409b2ec7121eee17ef034a0d0941874c88163769183c";
const SOURCE_COMPOSITE_GRAPH_DIGEST: &str =
    "5ae364b34510e678cb2a1cd8e2d5aae4e888c37369ec3fe424c470ec6754b7d0";
const LEGACY_RULE_DIGEST: &str = "76927d2e56c9abaf718d87bcf13e59a72cdfdfa12d8efd7f74e72272227fb189";
const CLOSED_ADMISSION_DIGEST: &str =
    "9f470a1210c9e35a4ebbb3fc4176ddbee25f1e2e45c8daa7cf595d94ac602281";
const CLOSED_ADMISSION_DESCRIPTOR: &str = concat!(
    "ae-neurofield-graph-admission-profile-v1;",
    "domain=ae.neurofield.graph-admission.v1;",
    "profile_version=1;",
    "formula_version=1;",
    "formula_rule_sha256=76927d2e56c9abaf718d87bcf13e59a72cdfdfa12d8efd7f74e72272227fb189;",
    "max_snapshot_canonical_bytes=8454156;",
    "max_replay_deltas=64;",
    "max_operations_per_delta=4096;",
    "unknown_fields=deny-GraphSnapshotV1-GraphReplayV1-StructuralDeltaV1-EdgeOperationV1-Synapse;",
    "legacy_source_v1=explicit-migration-required",
);

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn digest_from_hex(value: &str) -> [u8; 32] {
    assert_eq!(value.len(), 64);
    let mut output = [0_u8; 32];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
    }
    output
}

fn edge(target: u32, weight: i16) -> Synapse {
    Synapse {
        target,
        weight,
        eligibility: 20,
        stability: 30,
        last_used_epoch: 40,
        operator_id: 0,
        delay_class: 0,
        flags: 0,
    }
}

fn graph_with_entries(entries: &[(u32, Synapse)]) -> SparseGraph {
    let mut rows = vec![Vec::new(); NEURON_SLOTS];
    for (source, value) in entries {
        rows[*source as usize].push(*value);
    }
    for row in &mut rows {
        row.sort_unstable_by_key(|value| value.target);
    }

    let mut graph = SparseGraph::empty();
    graph.row_offsets.clear();
    graph.row_offsets.push(0);
    for row in rows {
        graph.edges.extend(row);
        graph.row_offsets.push(graph.edges.len() as u32);
    }
    assert!(graph.validate());
    graph
}

fn assert_fixture_provenance(path: &str, expected_sha256: &str) {
    let provenance: Value = serde_json::from_str(include_str!(
        "vectors/task3-adapted-source-v1-fixtures.provenance.json"
    ))
    .unwrap();
    assert_eq!(provenance["source_commit"], SOURCE_COMMIT);
    let record = provenance["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["path"] == path)
        .unwrap();
    assert_eq!(record["sha256"], expected_sha256);
}

#[test]
fn historical_nonempty_replay_migrates_to_closed_admission_profile_equivalently() {
    let fixture = include_bytes!("vectors/graph-replay-source-v1-nonempty.json");
    assert_eq!(sha256_hex(fixture), HISTORICAL_REPLAY_SHA256);
    assert_fixture_provenance(
        "crates/ae-neurofield/tests/vectors/graph-replay-source-v1-nonempty.json",
        HISTORICAL_REPLAY_SHA256,
    );

    assert_eq!(
        GRAPH_ADMISSION_DOMAIN_V1,
        "ae.neurofield.graph-admission.v1"
    );
    assert_eq!(GRAPH_ADMISSION_PROFILE_VERSION_V1, 1);
    assert_eq!(
        graph_admission_profile_descriptor(GraphAdmissionProfileV1::ClosedV1).unwrap(),
        CLOSED_ADMISSION_DESCRIPTOR
    );
    assert_eq!(
        sha256_hex(CLOSED_ADMISSION_DESCRIPTOR.as_bytes()),
        CLOSED_ADMISSION_DIGEST
    );
    assert_eq!(
        graph_admission_profile_digest(GraphAdmissionProfileV1::ClosedV1).unwrap(),
        digest_from_hex(CLOSED_ADMISSION_DIGEST)
    );
    assert_eq!(
        legacy_graph_replay_rule_digest(GRAPH_REPLAY_FORMULA_V1).unwrap(),
        digest_from_hex(LEGACY_RULE_DIGEST)
    );

    let legacy: GraphReplayV1 = serde_json::from_slice(fixture).unwrap();
    assert_eq!(
        legacy.admission_profile,
        GraphAdmissionProfileV1::LegacySourceV1
    );
    assert_eq!(
        legacy.admission_profile_digest,
        digest_from_hex(LEGACY_RULE_DIGEST)
    );
    assert_eq!(
        legacy.reopen().unwrap_err(),
        GraphReplayError::LegacyAdmissionProfileRequiresMigration
    );

    let migrated = legacy.migrate_legacy_source_v1().unwrap();
    assert_eq!(
        migrated.admission_profile,
        GraphAdmissionProfileV1::ClosedV1
    );
    assert_eq!(
        migrated.admission_profile_digest,
        digest_from_hex(CLOSED_ADMISSION_DIGEST)
    );
    assert!(migrated
        .deltas
        .iter()
        .all(|delta| delta.rule_digest == migrated.admission_profile_digest));
    let persisted = serde_json::to_value(&migrated).unwrap();
    assert_eq!(persisted["admission_profile"].as_str(), Some("ClosedV1"));
    assert!(persisted.get("admission_profile_digest").is_some());
    let round_trip: GraphReplayV1 = serde_json::from_value(persisted).unwrap();

    let mut tampered_profile = round_trip.clone();
    tampered_profile.admission_profile_digest[0] ^= 0x80;
    assert_eq!(
        tampered_profile.reopen().unwrap_err(),
        GraphReplayError::AdmissionProfileMismatch
    );

    let (revision, graph) = round_trip.reopen().unwrap();
    assert_eq!(revision, 12);
    let expected = graph_with_entries(&[(2, edge(3, -99))]);
    assert_eq!(graph.canonical_bytes(), expected.canonical_bytes());
    assert_eq!(graph_digest(&graph), graph_digest(&expected));
}

#[test]
fn composite_add_update_remove_matches_immutable_source_bytes_and_digest() {
    let source_bytes = include_bytes!("vectors/structural-delta-source-v1-composite.bin");
    assert_eq!(sha256_hex(source_bytes), SOURCE_COMPOSITE_SHA256);
    assert_fixture_provenance(
        "crates/ae-neurofield/tests/vectors/structural-delta-source-v1-composite.bin",
        SOURCE_COMPOSITE_SHA256,
    );

    let base = graph_with_entries(&[(1, edge(2, 10)), (3, edge(4, 20))]);
    let base_before = base.canonical_bytes();
    let expected = graph_with_entries(&[(0, edge(1, 30)), (1, edge(2, -10))]);
    assert_eq!(expected.canonical_bytes(), source_bytes);
    assert_eq!(
        graph_digest(&expected),
        digest_from_hex(SOURCE_COMPOSITE_GRAPH_DIGEST)
    );

    let mut value = StructuralDeltaV1 {
        base_revision: 20,
        base_graph_digest: graph_digest(&base),
        delta_sequence: 1,
        rule_digest: [0; 32],
        operations: vec![
            EdgeOperationV1::Add {
                source: 0,
                edge: edge(1, 30),
            },
            EdgeOperationV1::Update {
                source: 1,
                edge: edge(2, -10),
            },
            EdgeOperationV1::Remove {
                source: 3,
                target: 4,
            },
        ],
        after_graph_digest: graph_digest(&expected),
    };
    bind_delta_to_graph_replay_rule(GRAPH_REPLAY_FORMULA_V1, &mut value).unwrap();
    assert_eq!(value.rule_digest, digest_from_hex(CLOSED_ADMISSION_DIGEST));

    let mut legacy_bound = value.clone();
    legacy_bound.rule_digest = digest_from_hex(LEGACY_RULE_DIGEST);
    assert_eq!(
        apply_delta(20, &graph_digest(&base), 20, &base, &legacy_bound,).unwrap_err(),
        DeltaError::AdmissionProfileMismatch
    );
    assert_eq!(base.canonical_bytes(), base_before);

    let actual = apply_delta(20, &graph_digest(&base), 20, &base, &value).unwrap();
    assert_eq!(actual.canonical_bytes(), source_bytes);
    assert_eq!(
        graph_digest(&actual),
        digest_from_hex(SOURCE_COMPOSITE_GRAPH_DIGEST)
    );
    assert_eq!(base.canonical_bytes(), base_before);
}

fn oversized_snapshot_json() -> String {
    let mut json = String::with_capacity(MAX_SNAPSHOT_CANONICAL_BYTES_V1 * 2 + 256);
    json.push_str(
        r#"{"formula_version":1,"revision":0,"graph_digest":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"canonical_bytes":["#,
    );
    for index in 0..=MAX_SNAPSHOT_CANONICAL_BYTES_V1 {
        if index != 0 {
            json.push(',');
        }
        json.push('0');
    }
    json.push_str("]}");
    json
}

#[test]
fn oversized_snapshot_and_anchor_are_rejected_without_mutating_caller_graph() {
    assert_eq!(
        MAX_SNAPSHOT_CANONICAL_BYTES_V1,
        4 + (NEURON_SLOTS + 1) * 4 + 4 + EDGE_CAPACITY * 16
    );
    assert_eq!(MAX_REPLAY_DELTAS_V1, 64);
    assert_eq!(MAX_OPERATIONS_PER_DELTA_V1, 4_096);

    let caller = graph_with_entries(&[(0, edge(1, 9))]);
    let caller_before = caller.canonical_bytes();
    let oversized = oversized_snapshot_json();
    let snapshot_error = serde_json::from_str::<GraphSnapshotV1>(&oversized).unwrap_err();
    assert!(snapshot_error
        .to_string()
        .contains("v1 snapshot canonical bytes exceed admission limit"));

    let valid_snapshot = serde_json::to_string(
        &GraphSnapshotV1::from_graph(GRAPH_REPLAY_FORMULA_V1, 0, &SparseGraph::empty()).unwrap(),
    )
    .unwrap();
    let malicious_anchor =
        format!(r#"{{"anchor":{oversized},"deltas":[],"authoritative":{valid_snapshot}}}"#);
    let replay_error = serde_json::from_str::<GraphReplayV1>(&malicious_anchor).unwrap_err();
    assert!(replay_error
        .to_string()
        .contains("v1 snapshot canonical bytes exceed admission limit"));

    let direct = GraphSnapshotV1 {
        formula_version: GRAPH_REPLAY_FORMULA_V1,
        revision: 0,
        graph_digest: [0; 32],
        canonical_bytes: vec![0; MAX_SNAPSHOT_CANONICAL_BYTES_V1 + 1],
    };
    assert_eq!(
        direct.restore().unwrap_err(),
        GraphReplayError::SnapshotCanonicalBytesTooLarge
    );
    assert_eq!(caller.canonical_bytes(), caller_before);
}
