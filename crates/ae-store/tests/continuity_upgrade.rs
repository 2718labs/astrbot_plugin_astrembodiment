use ae_contracts::{wire, Digest};
use ae_store::{
    migrate_continuity, open_current_generation, ContinuityMigrationDecision,
    ContinuityMigrationFault,
};
use rusqlite::{params, Connection};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

type PersistedReceiptRow = (Vec<u8>, i64, Vec<u8>, Vec<u8>, Vec<u8>, i64, String);

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExpectedAuthority {
    incarnation_id: Digest,
    revision: u64,
    state_digest: Digest,
    graph_digest: Digest,
    history_digest: Digest,
}

fn fixture_root(name: &str) -> PathBuf {
    loop {
        let number = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "ae-store-continuity-upgrade-{name}-{}-{number}",
            std::process::id()
        ));
        match fs::create_dir(&root) {
            Ok(()) => {
                fs::create_dir_all(root.join("generations").join("generation-alpha")).unwrap();
                return root;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("creating isolated fixture root failed: {error}"),
        }
    }
}

fn owner_cbor(generation_id: &str, store_uuid: [u8; 16]) -> Vec<u8> {
    assert!(generation_id.len() <= 23);
    let mut value = vec![0xa2, 0x6d];
    value.extend_from_slice(b"generation_id");
    value.push(0x60 + generation_id.len() as u8);
    value.extend_from_slice(generation_id.as_bytes());
    value.push(0x6a);
    value.extend_from_slice(b"store_uuid");
    value.push(0x50);
    value.extend_from_slice(&store_uuid);
    value
}

fn hex(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn write_vault_locator(root: &Path, incarnation_id: Digest, revision: u64) {
    fs::write(
        root.join("owner.cbor"),
        owner_cbor("generation-alpha", [3; 16]),
    )
    .unwrap();
    fs::write(
        root.join("current"),
        format!(
            "generation_id=generation-alpha\nincarnation_id={}\nrevision={revision}\nmode=ready\n",
            hex(&incarnation_id)
        ),
    )
    .unwrap();
}

fn create_source_store(root: &Path) -> ExpectedAuthority {
    let expected = ExpectedAuthority {
        incarnation_id: [11; 32],
        revision: 1,
        state_digest: [12; 32],
        graph_digest: [13; 32],
        history_digest: [14; 32],
    };
    write_vault_locator(root, expected.incarnation_id, expected.revision);

    let source = root
        .join("generations")
        .join("generation-alpha")
        .join("authority.sqlite");
    let conn = Connection::open(source).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE active_bindings (
            bot_token BLOB NOT NULL,
            persona_token BLOB NOT NULL,
            incarnation_id BLOB NOT NULL,
            revision INTEGER NOT NULL
        );
        CREATE TABLE incarnations (
            incarnation_id BLOB PRIMARY KEY,
            graph_digest BLOB NOT NULL
        );
        CREATE TABLE snapshots (
            revision INTEGER NOT NULL,
            scope_digest BLOB NOT NULL,
            state_digest BLOB NOT NULL
        );
        CREATE TABLE journal (
            scope_digest BLOB NOT NULL,
            logical_revision INTEGER NOT NULL,
            chain_digest BLOB NOT NULL
        );
        "#,
    )
    .unwrap();
    let bot = [21; 16];
    let persona = [22; 16];
    let scope = wire::persona_scope_digest(&bot, &persona, None);
    conn.execute(
        "INSERT INTO active_bindings (bot_token, persona_token, incarnation_id, revision) VALUES (?1, ?2, ?3, ?4)",
        params![bot.to_vec(), persona.to_vec(), expected.incarnation_id.to_vec(), 1_i64],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO incarnations (incarnation_id, graph_digest) VALUES (?1, ?2)",
        params![
            expected.incarnation_id.to_vec(),
            expected.graph_digest.to_vec()
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO snapshots (revision, scope_digest, state_digest) VALUES (?1, ?2, ?3)",
        params![1_i64, scope.to_vec(), expected.state_digest.to_vec()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO journal (scope_digest, logical_revision, chain_digest) VALUES (?1, ?2, ?3)",
        params![scope.to_vec(), 1_i64, expected.history_digest.to_vec()],
    )
    .unwrap();
    expected
}

#[test]
fn atomic_shadow_upgrade_preserves_authority_and_reopens_current_generation() {
    let root = fixture_root("preserves-authority");
    let expected = create_source_store(&root);

    let receipt = migrate_continuity(&root, "generation-beta", None).unwrap();

    assert_eq!(receipt.before.incarnation_id, expected.incarnation_id);
    assert_eq!(receipt.before.revision, expected.revision);
    assert_eq!(receipt.before.state_digest, expected.state_digest);
    assert_eq!(receipt.before.graph_digest, expected.graph_digest);
    assert_eq!(receipt.before.history_digest, expected.history_digest);
    assert_eq!(receipt.before, receipt.after);
    assert!(!receipt.replay);
    assert_eq!(receipt.decision, ContinuityMigrationDecision::Switched);
    assert!(root
        .join("generations")
        .join("generation-alpha")
        .join("authority.sqlite")
        .is_file());
    assert!(root
        .join("generations")
        .join("generation-beta")
        .join("authority.sqlite")
        .is_file());

    let current = open_current_generation(&root).unwrap();
    assert_eq!(current.generation_id, "generation-beta");
    assert_eq!(current.authority, receipt.after);

    let audit = Connection::open(root.join("continuity_locator.sqlite")).unwrap();
    let (incarnation_id, revision, state_digest, graph_digest, history_digest, replay, decision):
        PersistedReceiptRow = audit
        .query_row(
            "SELECT before_incarnation_id, before_revision, before_state_digest, before_graph_digest, before_history_digest, replay, decision FROM continuity_migration_receipts WHERE source_generation = ?1 AND target_generation = ?2",
            params!["generation-alpha", "generation-beta"],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
        )
        .unwrap();
    assert_eq!(incarnation_id, receipt.before.incarnation_id.to_vec());
    assert_eq!(revision, receipt.before.revision as i64);
    assert_eq!(state_digest, receipt.before.state_digest.to_vec());
    assert_eq!(graph_digest, receipt.before.graph_digest.to_vec());
    assert_eq!(history_digest, receipt.before.history_digest.to_vec());
    assert_eq!(replay, 0);
    assert_eq!(decision, "switched");

    let replay = migrate_continuity(&root, "generation-beta", None).unwrap();
    assert!(replay.replay);
    assert_eq!(replay.decision, ContinuityMigrationDecision::Replayed);
    assert_eq!(replay.before, receipt.after);
    assert_eq!(replay.after, receipt.after);
    let (replay_flag, replay_decision): (i64, String) = audit
        .query_row(
            "SELECT replay, decision FROM continuity_migration_receipts WHERE source_generation = ?1 AND target_generation = ?2",
            params!["generation-beta", "generation-beta"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(replay_flag, 1);
    assert_eq!(replay_decision, "replayed");

    let no_fault = ContinuityMigrationFault::BeforeBackup;
    assert_ne!(no_fault, ContinuityMigrationFault::AfterLocatorCas);
}

#[test]
fn empty_uninitialized_locator_falls_back_to_the_original_current_authority() {
    let root = fixture_root("empty-uninitialized-locator");
    let expected = create_source_store(&root);
    let locator = root.join("continuity_locator.sqlite");
    let connection = Connection::open(&locator).unwrap();
    connection.execute_batch("PRAGMA user_version = 0").unwrap();
    drop(connection);

    let current = open_current_generation(&root).unwrap();

    assert_eq!(current.generation_id, "generation-alpha");
    assert_eq!(current.authority.incarnation_id, expected.incarnation_id);
    assert_eq!(current.authority.revision, expected.revision);
}

#[test]
fn partial_locator_is_rejected_without_exposing_its_dynamic_schema_name() {
    let root = fixture_root("partial-locator-RAW_CONTENT_SENTINEL");
    create_source_store(&root);
    let locator = root.join("continuity_locator.sqlite");
    let connection = Connection::open(&locator).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE continuity_generation_locator (slot INTEGER PRIMARY KEY, generation_id TEXT NOT NULL);\
             CREATE TABLE continuity_migration_receipts (source_generation TEXT NOT NULL);\
             CREATE TABLE user_object_RAW_CONTENT_SENTINEL (value INTEGER)",
        )
        .unwrap();
    drop(connection);

    let error = open_current_generation(&root).unwrap_err();
    let public = error.to_string();

    assert_eq!(public, "CONTINUITY_MIGRATION_LOCATOR_INVALID");
    assert!(!public.contains("user_object_RAW_CONTENT_SENTINEL"));
    assert!(!public.contains("RAW_CONTENT_SENTINEL"));
}

#[test]
fn public_migration_errors_are_fixed_codes_without_dynamic_path_or_content() {
    let root = fixture_root("absolute-path-RAW_CONTENT_SENTINEL");
    fs::write(
        root.join("owner.cbor"),
        owner_cbor("generation-alpha", [3; 16]),
    )
    .unwrap();
    fs::write(
        root.join("current"),
        "generation_id=generation-alpha\nincarnation_id=0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0\nrevision=1\nmode=ready\nRAW_CONTENT_SENTINEL=user_object_RAW_CONTENT_SENTINEL\n",
    )
    .unwrap();

    let error = migrate_continuity(&root, "generation-beta", None).unwrap_err();
    let public = error.to_string();
    let root_text = root.to_string_lossy();

    assert_eq!(public, "CONTINUITY_MIGRATION_VAULT_FAILURE");
    assert!(!public.contains(root_text.as_ref()));
    assert!(!public.contains("RAW_CONTENT_SENTINEL"));
    assert!(!public.contains("user_object_RAW_CONTENT_SENTINEL"));
}

#[test]
fn existing_shadow_name_collision_allocates_another_checked_name() {
    let root = fixture_root("shadow-name-collision");
    create_source_store(&root);
    let generations = root.join("generations");
    for sequence in 0..128 {
        fs::create_dir(generations.join(format!(
            ".shadow-generation-beta-{}-{sequence}",
            std::process::id()
        )))
        .unwrap();
    }

    let receipt = migrate_continuity(&root, "generation-beta", None).unwrap();

    assert_eq!(receipt.decision, ContinuityMigrationDecision::Switched);
    let current = open_current_generation(&root).unwrap();
    assert_eq!(current.generation_id, "generation-beta");
}
