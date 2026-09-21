use ae_contracts::hex;
use ae_store::{
    discover_legacy, validate_legacy_candidate, verify_candidate, CandidateFences, Discovery,
    DiscoveryRejectCode, DiscoverySources,
};
use rusqlite::{params, Connection};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

fn fixture_root(name: &str) -> PathBuf {
    let number = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "ae-store-legacy-discovery-{name}-{}-{number}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn fences(marker: u8) -> CandidateFences {
    CandidateFences {
        store_uuid: [marker; 16],
        scope_digest: [marker.wrapping_add(1); 32],
        incarnation_id: [marker.wrapping_add(2); 32],
        seed_code_digest: [marker.wrapping_add(3); 32],
        formula_digest: [marker.wrapping_add(4); 32],
        graph_digest: [marker.wrapping_add(5); 32],
        revision: u64::from(marker) + 7,
    }
}

fn write_fence_file(root: &Path, fences: &CandidateFences) {
    let body = format!(
        concat!(
            "version=1\n",
            "store_uuid={}\n",
            "scope_digest={}\n",
            "incarnation_id={}\n",
            "seed_code_digest={}\n",
            "formula_digest={}\n",
            "graph_digest={}\n",
            "revision={}\n"
        ),
        hex::encode16(&fences.store_uuid),
        hex::encode32(&fences.scope_digest),
        hex::encode32(&fences.incarnation_id),
        hex::encode32(&fences.seed_code_digest),
        hex::encode32(&fences.formula_digest),
        hex::encode32(&fences.graph_digest),
        fences.revision,
    );
    fs::write(root.join("candidate-fences.v1"), body).unwrap();
}

fn create_candidate(parent: &Path, name: &str, fences: &CandidateFences) -> PathBuf {
    let root = parent.join(name);
    fs::create_dir_all(&root).unwrap();
    write_fence_file(&root, fences);

    let database_path = root.join("continuity.sqlite3");
    let connection = Connection::open(&database_path).unwrap();
    connection
        .execute_batch(
            r#"
            PRAGMA application_id = 0x41454332;
            PRAGMA user_version = 1;
            CREATE TABLE legacy_identity_v1 (
                store_uuid BLOB NOT NULL,
                scope_digest BLOB NOT NULL,
                incarnation_id BLOB NOT NULL,
                seed_code_digest BLOB NOT NULL,
                formula_digest BLOB NOT NULL,
                graph_digest BLOB NOT NULL,
                revision INTEGER NOT NULL
            );
            CREATE TABLE legacy_journal_v1 (
                revision INTEGER PRIMARY KEY,
                scope_digest BLOB NOT NULL,
                base_revision INTEGER NOT NULL,
                event_digest BLOB NOT NULL,
                chain_digest BLOB NOT NULL
            );
            CREATE TABLE legacy_snapshots_v1 (
                revision INTEGER PRIMARY KEY,
                scope_digest BLOB NOT NULL,
                state_digest BLOB NOT NULL
            );
            CREATE TABLE legacy_graph_v1 (
                revision INTEGER PRIMARY KEY,
                graph_digest BLOB NOT NULL
            );
            CREATE TABLE legacy_replay_v1 (
                revision INTEGER PRIMARY KEY,
                replay_digest BLOB NOT NULL
            );
            "#,
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO legacy_identity_v1 (store_uuid, scope_digest, incarnation_id, seed_code_digest, formula_digest, graph_digest, revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                fences.store_uuid.to_vec(),
                fences.scope_digest.to_vec(),
                fences.incarnation_id.to_vec(),
                fences.seed_code_digest.to_vec(),
                fences.formula_digest.to_vec(),
                fences.graph_digest.to_vec(),
                i64::try_from(fences.revision).unwrap(),
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO legacy_journal_v1 (revision, scope_digest, base_revision, event_digest, chain_digest) VALUES (?1, ?2, 0, ?3, ?4)",
            params![
                i64::try_from(fences.revision).unwrap(),
                fences.scope_digest.to_vec(),
                vec![0x31_u8; 32],
                vec![0x32_u8; 32],
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO legacy_snapshots_v1 (revision, scope_digest, state_digest) VALUES (?1, ?2, ?3)",
            params![
                i64::try_from(fences.revision).unwrap(),
                fences.scope_digest.to_vec(),
                vec![0x33_u8; 32],
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO legacy_graph_v1 (revision, graph_digest) VALUES (?1, ?2)",
            params![
                i64::try_from(fences.revision).unwrap(),
                fences.graph_digest.to_vec(),
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO legacy_replay_v1 (revision, replay_digest) VALUES (?1, ?2)",
            params![i64::try_from(fences.revision).unwrap(), vec![0x34_u8; 32]],
        )
        .unwrap();
    drop(connection);
    root
}

fn tree_receipt(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn collect(root: &Path, current: &Path, output: &mut Vec<(PathBuf, Vec<u8>)>) {
        let mut entries = fs::read_dir(current)
            .unwrap()
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                collect(root, &path, output);
            } else {
                output.push((
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                ));
            }
        }
    }

    let mut output = Vec::new();
    collect(root, root, &mut output);
    output
}

fn source_with_record(record: PathBuf) -> DiscoverySources {
    DiscoverySources {
        installation_records: vec![record],
        ..DiscoverySources::default()
    }
}

fn selected(discovery: Discovery) -> ae_store::LegacyCandidate {
    match discovery {
        Discovery::Selected(candidate) => *candidate,
        other => panic!("expected exactly one candidate, got {other:?}"),
    }
}

#[test]
fn discovery_returns_none_without_an_explicitly_authorized_source() {
    let root = fixture_root("no-source");
    let candidate = create_candidate(&root, "unlisted", &fences(1));
    let before = tree_receipt(&root);

    assert_eq!(
        discover_legacy(&DiscoverySources::default()),
        Discovery::None
    );

    assert_eq!(tree_receipt(&root), before);
    assert!(candidate.is_dir());
}

#[test]
fn one_valid_candidate_from_an_install_record_is_selected_without_writes() {
    let root = fixture_root("one-valid");
    let candidate_root = create_candidate(&root, "candidate", &fences(2));
    let record = root.join("installed-continuity.txt");
    fs::write(
        &record,
        format!("legacy_candidate={}\n", candidate_root.display()),
    )
    .unwrap();
    let before = tree_receipt(&root);

    let candidate = selected(discover_legacy(&source_with_record(record)));

    assert_eq!(candidate.root, fs::canonicalize(candidate_root).unwrap());
    assert_eq!(candidate.fences, fences(2));
    assert_eq!(tree_receipt(&root), before);
}

#[test]
fn two_distinct_valid_candidates_are_ambiguous_and_never_merged() {
    let root = fixture_root("ambiguous");
    let first = create_candidate(&root, "first", &fences(3));
    let second = create_candidate(&root, "second", &fences(4));
    let before = tree_receipt(&root);
    let sources = DiscoverySources {
        explicit_configuration: vec![first, second],
        ..DiscoverySources::default()
    };

    assert_eq!(discover_legacy(&sources), Discovery::Ambiguous);

    assert_eq!(tree_receipt(&root), before);
}

#[test]
fn identical_candidate_content_is_deduplicated_by_identity_across_allowed_sources() {
    let root = fixture_root("deduplicate");
    let original = create_candidate(&root, "original", &fences(5));
    let duplicate = root.join("duplicate");
    fs::create_dir_all(&duplicate).unwrap();
    for name in ["candidate-fences.v1", "continuity.sqlite3"] {
        fs::copy(original.join(name), duplicate.join(name)).unwrap();
    }
    let before = tree_receipt(&root);
    let sources = DiscoverySources {
        explicit_configuration: vec![original],
        historical_allowlist: vec![duplicate],
        ..DiscoverySources::default()
    };

    let candidate = selected(discover_legacy(&sources));

    assert_eq!(candidate.fences, fences(5));
    assert_eq!(tree_receipt(&root), before);
}

#[test]
fn each_fence_mismatch_has_a_stable_refusal_code_and_zero_writes() {
    let root = fixture_root("fences");
    let actual = fences(6);
    let candidate_root = create_candidate(&root, "candidate", &actual);
    let record = root.join("installed-continuity.txt");
    fs::write(
        &record,
        format!("legacy_candidate={}\n", candidate_root.display()),
    )
    .unwrap();
    let candidate = selected(discover_legacy(&source_with_record(record)));

    let mut store_uuid = actual.clone();
    store_uuid.store_uuid[0] ^= 1;
    let mut incarnation = actual.clone();
    incarnation.incarnation_id[0] ^= 1;
    let mut scope = actual.clone();
    scope.scope_digest[0] ^= 1;
    let mut seed = actual.clone();
    seed.seed_code_digest[0] ^= 1;
    let mut formula = actual.clone();
    formula.formula_digest[0] ^= 1;
    let mut graph = actual.clone();
    graph.graph_digest[0] ^= 1;
    let mut revision = actual.clone();
    revision.revision += 1;

    for (expected, code) in [
        (store_uuid, DiscoveryRejectCode::WriteRefusedIdentity),
        (incarnation, DiscoveryRejectCode::WriteRefusedIdentity),
        (scope, DiscoveryRejectCode::WriteRefusedScope),
        (seed, DiscoveryRejectCode::WriteRefusedSeedCode),
        (formula, DiscoveryRejectCode::WriteRefusedFormula),
        (graph, DiscoveryRejectCode::WriteRefusedGraph),
        (revision, DiscoveryRejectCode::WriteRefusedRevision),
    ] {
        let before = tree_receipt(&root);

        let rejection = verify_candidate(&candidate, &expected).unwrap_err();

        assert_eq!(rejection, code);
        assert!(rejection.code().starts_with("WRITE_REFUSED_"));
        assert_eq!(tree_receipt(&root), before);
    }
}

#[test]
fn candidate_file_database_schema_journal_snapshot_graph_and_replay_are_read_only_validated() {
    let root = fixture_root("validation");
    let cases: [(&str, &str); 6] = [
        ("candidate-file", "candidate-fences.v1"),
        ("schema", "legacy_identity_v1"),
        ("journal", "legacy_journal_v1"),
        ("snapshot", "legacy_snapshots_v1"),
        ("graph", "legacy_graph_v1"),
        ("replay", "legacy_replay_v1"),
    ];

    for (index, (name, target)) in cases.into_iter().enumerate() {
        let case_root = root.join(name);
        fs::create_dir_all(&case_root).unwrap();
        let candidate_root = create_candidate(&case_root, "candidate", &fences(20 + index as u8));
        if target == "candidate-fences.v1" {
            fs::write(candidate_root.join(target), "version=wrong\n").unwrap();
        } else {
            let connection = Connection::open(candidate_root.join("continuity.sqlite3")).unwrap();
            connection
                .execute(&format!("DROP TABLE {target}"), [])
                .unwrap();
        }
        let before = tree_receipt(&case_root);

        let rejection = validate_legacy_candidate(&candidate_root).unwrap_err();

        assert_eq!(rejection, DiscoveryRejectCode::WriteRefusedCandidate);
        assert_eq!(tree_receipt(&case_root), before);
    }
}
