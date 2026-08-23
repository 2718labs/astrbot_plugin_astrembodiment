use ae_contracts::{wire, Digest};
use ae_store::{
    migrate_continuity, open_current_generation, ContinuityMigrationDecision,
    ContinuityMigrationError, ContinuityMigrationFault,
};
use rusqlite::{params, Connection};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

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
            "ae-store-continuity-fault-{name}-{}-{number}",
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

fn create_source_store(root: &Path) -> ExpectedAuthority {
    let expected = ExpectedAuthority {
        incarnation_id: [31; 32],
        revision: 1,
        state_digest: [32; 32],
        graph_digest: [33; 32],
        history_digest: [34; 32],
    };
    fs::write(
        root.join("owner.cbor"),
        owner_cbor("generation-alpha", [8; 16]),
    )
    .unwrap();
    fs::write(
        root.join("current"),
        format!(
            "generation_id=generation-alpha\nincarnation_id={}\nrevision=1\nmode=ready\n",
            hex(&expected.incarnation_id)
        ),
    )
    .unwrap();

    let source = root
        .join("generations")
        .join("generation-alpha")
        .join("authority.sqlite");
    let conn = Connection::open(source).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE active_bindings (bot_token BLOB NOT NULL, persona_token BLOB NOT NULL, incarnation_id BLOB NOT NULL, revision INTEGER NOT NULL);
        CREATE TABLE incarnations (incarnation_id BLOB PRIMARY KEY, graph_digest BLOB NOT NULL);
        CREATE TABLE snapshots (revision INTEGER NOT NULL, scope_digest BLOB NOT NULL, state_digest BLOB NOT NULL);
        CREATE TABLE journal (scope_digest BLOB NOT NULL, logical_revision INTEGER NOT NULL, chain_digest BLOB NOT NULL);
        "#,
    )
    .unwrap();
    let bot = [41; 16];
    let persona = [42; 16];
    let scope = wire::persona_scope_digest(&bot, &persona, None);
    conn.execute(
        "INSERT INTO active_bindings VALUES (?1, ?2, ?3, ?4)",
        params![
            bot.to_vec(),
            persona.to_vec(),
            expected.incarnation_id.to_vec(),
            1_i64
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO incarnations VALUES (?1, ?2)",
        params![
            expected.incarnation_id.to_vec(),
            expected.graph_digest.to_vec()
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO snapshots VALUES (?1, ?2, ?3)",
        params![1_i64, scope.to_vec(), expected.state_digest.to_vec()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO journal VALUES (?1, ?2, ?3)",
        params![scope.to_vec(), 1_i64, expected.history_digest.to_vec()],
    )
    .unwrap();
    expected
}

#[test]
fn injected_failures_never_publish_partial_target_or_fallback_to_genesis() {
    for point in [
        ContinuityMigrationFault::BeforeBackup,
        ContinuityMigrationFault::AfterBackup,
    ] {
        let root = fixture_root("before-cas");
        let expected = create_source_store(&root);
        let source = open_current_generation(&root).unwrap();
        assert_eq!(source.generation_id, "generation-alpha");
        assert_eq!(source.authority.incarnation_id, expected.incarnation_id);

        let error = migrate_continuity(&root, "generation-beta", Some(point)).unwrap_err();

        assert!(
            matches!(error, ContinuityMigrationError::InjectedFault(found) if found == point),
            "point={point:?}; unexpected migration result: {error:?}"
        );
        let current = open_current_generation(&root).unwrap();
        assert_eq!(current.generation_id, "generation-alpha");
        assert_eq!(current.authority.incarnation_id, expected.incarnation_id);
        assert_eq!(current.authority.revision, expected.revision);
        assert_eq!(current.authority.state_digest, expected.state_digest);
        assert_eq!(current.authority.graph_digest, expected.graph_digest);
        assert_eq!(current.authority.history_digest, expected.history_digest);
        assert!(!root.join("generations").join("generation-beta").exists());
    }
}

#[test]
fn publication_fault_after_rename_keeps_old_locator_and_only_a_complete_target() {
    let root = fixture_root("publication-fault-after-rename");
    let expected = create_source_store(&root);

    let error = migrate_continuity(
        &root,
        "generation-beta",
        Some(ContinuityMigrationFault::BeforeLocatorCas),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ContinuityMigrationError::InjectedFault(ContinuityMigrationFault::BeforeLocatorCas)
    ));
    let old = open_current_generation(&root).unwrap();
    assert_eq!(old.generation_id, "generation-alpha");
    assert_eq!(old.authority.incarnation_id, expected.incarnation_id);
    assert_eq!(old.authority.revision, expected.revision);

    let target = root
        .join("generations")
        .join("generation-beta")
        .join("authority.sqlite");
    assert!(
        target.is_file(),
        "post-publication fault must retain the target"
    );
    let target_connection = Connection::open(&target).unwrap();
    let active_bindings: i64 = target_connection
        .query_row("SELECT COUNT(*) FROM active_bindings", [], |row| row.get(0))
        .unwrap();
    assert_eq!(active_bindings, 1);
    drop(target_connection);
    assert!(root
        .join("generations")
        .join("generation-beta")
        .join("migration.intent")
        .is_file());

    let retry = migrate_continuity(&root, "generation-beta", None).unwrap();
    assert_eq!(retry.decision, ContinuityMigrationDecision::Switched);
    let reopened = open_current_generation(&root).unwrap();
    assert_eq!(reopened.generation_id, "generation-beta");
    assert_eq!(reopened.authority, retry.after);
}

#[test]
fn replay_receipt_waits_for_the_source_writer_and_records_fenced_authority() {
    let root = fixture_root("replay-source-writer-race");
    create_source_store(&root);
    migrate_continuity(&root, "generation-beta", None).unwrap();

    let source = root
        .join("generations")
        .join("generation-beta")
        .join("authority.sqlite");
    let updated_state_digest = [99; 32];
    let writer = Connection::open(&source).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    writer
        .execute(
            "UPDATE snapshots SET state_digest = ?1",
            params![updated_state_digest.to_vec()],
        )
        .unwrap();

    let (result_sender, result_receiver) = mpsc::channel();
    let worker_root = root.clone();
    let worker = thread::spawn(move || {
        result_sender
            .send(migrate_continuity(&worker_root, "generation-beta", None))
            .unwrap();
    });

    assert!(matches!(
        result_receiver.recv_timeout(Duration::from_millis(250)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    writer.execute_batch("COMMIT").unwrap();

    let receipt = result_receiver
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    worker.join().unwrap();
    assert!(receipt.replay);
    assert_eq!(receipt.before.state_digest, updated_state_digest);
    assert_eq!(receipt.after, receipt.before);

    let audit = Connection::open(root.join("continuity_locator.sqlite")).unwrap();
    let persisted_state_digest: Vec<u8> = audit
        .query_row(
            "SELECT before_state_digest FROM continuity_migration_receipts WHERE source_generation = ?1 AND target_generation = ?2",
            params!["generation-beta", "generation-beta"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(persisted_state_digest, updated_state_digest.to_vec());
}

#[test]
fn fault_after_locator_cas_leaves_new_current_authoritative_and_retry_is_replay() {
    let root = fixture_root("after-cas");
    let expected = create_source_store(&root);
    let source = open_current_generation(&root).unwrap();
    assert_eq!(source.generation_id, "generation-alpha");
    assert_eq!(source.authority.incarnation_id, expected.incarnation_id);

    let error = migrate_continuity(
        &root,
        "generation-beta",
        Some(ContinuityMigrationFault::AfterLocatorCas),
    )
    .unwrap_err();

    assert!(
        matches!(
            error,
            ContinuityMigrationError::InjectedFault(ContinuityMigrationFault::AfterLocatorCas)
        ),
        "unexpected migration result: {error:?}"
    );
    let current = open_current_generation(&root).unwrap();
    assert_eq!(current.generation_id, "generation-beta");
    assert_eq!(current.authority.incarnation_id, expected.incarnation_id);
    assert_eq!(current.authority.revision, expected.revision);
    assert_eq!(current.authority.state_digest, expected.state_digest);
    assert_eq!(current.authority.graph_digest, expected.graph_digest);
    assert_eq!(current.authority.history_digest, expected.history_digest);

    let replay = migrate_continuity(&root, "generation-beta", None).unwrap();
    assert!(replay.replay);
    assert_eq!(replay.decision, ContinuityMigrationDecision::Replayed);
    assert_eq!(replay.before, replay.after);
}

#[test]
fn first_locator_initialization_faults_reopen_the_old_authority_without_genesis() {
    for point in [
        ContinuityMigrationFault::AfterLocatorFileCreate,
        ContinuityMigrationFault::AfterFirstLocatorSchemaDdl,
        ContinuityMigrationFault::AfterSecondLocatorSchemaDdl,
    ] {
        let root = fixture_root("first-locator-initialization-fault");
        let expected = create_source_store(&root);

        let error = migrate_continuity(&root, "generation-beta", Some(point)).unwrap_err();

        assert!(matches!(
            error,
            ContinuityMigrationError::InjectedFault(found) if found == point
        ));
        let current = open_current_generation(&root).unwrap();
        assert_eq!(current.generation_id, "generation-alpha");
        assert_eq!(current.authority.incarnation_id, expected.incarnation_id);
        assert_eq!(current.authority.revision, expected.revision);
        assert!(root
            .join("generations")
            .join("generation-beta")
            .join("authority.sqlite")
            .is_file());

        let retry = migrate_continuity(&root, "generation-beta", None).unwrap();
        assert_eq!(retry.decision, ContinuityMigrationDecision::Switched);
        let reopened = open_current_generation(&root).unwrap();
        assert_eq!(reopened.generation_id, "generation-beta");
        assert_eq!(reopened.authority, retry.after);
    }
}

#[test]
fn locator_commit_fault_reopens_only_the_complete_new_authority() {
    let root = fixture_root("first-locator-commit-fault");
    let expected = create_source_store(&root);

    let error = migrate_continuity(
        &root,
        "generation-beta",
        Some(ContinuityMigrationFault::AfterLocatorCommit),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ContinuityMigrationError::InjectedFault(ContinuityMigrationFault::AfterLocatorCommit)
    ));
    let current = open_current_generation(&root).unwrap();
    assert_eq!(current.generation_id, "generation-beta");
    assert_eq!(current.authority.incarnation_id, expected.incarnation_id);
    assert_eq!(current.authority.revision, expected.revision);
    assert!(root
        .join("generations")
        .join("generation-beta")
        .join("authority.sqlite")
        .is_file());
}

#[test]
fn retry_promotes_a_complete_target_published_before_locator_cas() {
    let root = fixture_root("published-before-cas");
    let expected = create_source_store(&root);

    let initial = migrate_continuity(&root, "generation-beta", None).unwrap();
    assert_eq!(initial.decision, ContinuityMigrationDecision::Switched);
    let locator = root.join("continuity_locator.sqlite");
    let connection = Connection::open(&locator).unwrap();
    connection
        .execute(
            "UPDATE continuity_generation_locator SET generation_id = ?1 WHERE slot = 1",
            params!["generation-alpha"],
        )
        .unwrap();
    drop(connection);

    let resumed = migrate_continuity(&root, "generation-beta", None).unwrap();

    assert!(!resumed.replay);
    assert_eq!(resumed.decision, ContinuityMigrationDecision::Switched);
    assert_eq!(resumed.before.incarnation_id, expected.incarnation_id);
    assert_eq!(resumed.after, resumed.before);
    let current = open_current_generation(&root).unwrap();
    assert_eq!(current.generation_id, "generation-beta");
    assert_eq!(current.authority, resumed.after);
}

#[test]
fn retry_promotes_a_complete_orphan_from_an_equivalent_prior_source_generation() {
    let root = fixture_root("orphan-from-prior-source");
    let expected = create_source_store(&root);
    migrate_continuity(&root, "generation-beta", None).unwrap();

    let beta = root.join("generations").join("generation-beta");
    let orphan = root.join("generations").join("generation-gamma");
    fs::create_dir_all(&orphan).unwrap();
    fs::copy(
        beta.join("authority.sqlite"),
        orphan.join("authority.sqlite"),
    )
    .unwrap();
    fs::write(
        orphan.join("migration.intent"),
        format!(
            "source_generation=generation-alpha\ntarget_generation=generation-gamma\nincarnation_id={}\nrevision={}\nstate_digest={}\ngraph_digest={}\nhistory_digest={}\n",
            hex(&expected.incarnation_id),
            expected.revision,
            hex(&expected.state_digest),
            hex(&expected.graph_digest),
            hex(&expected.history_digest),
        ),
    )
    .unwrap();

    let resumed = migrate_continuity(&root, "generation-gamma", None).unwrap();

    assert!(!resumed.replay);
    assert_eq!(resumed.decision, ContinuityMigrationDecision::Switched);
    assert_eq!(resumed.before, resumed.after);
    let current = open_current_generation(&root).unwrap();
    assert_eq!(current.generation_id, "generation-gamma");
    assert_eq!(current.authority, resumed.after);
}

#[test]
fn locator_switch_during_source_fence_fails_before_creating_a_shadow() {
    for _attempt in 0..4 {
        let root = fixture_root("locator-switch-during-fence");
        let expected = create_source_store(&root);
        migrate_continuity(&root, "generation-alpha", None).unwrap();

        let generations = root.join("generations");
        let alpha = generations
            .join("generation-alpha")
            .join("authority.sqlite");
        let gamma = generations.join("generation-gamma");
        fs::create_dir_all(&gamma).unwrap();
        fs::copy(&alpha, gamma.join("authority.sqlite")).unwrap();

        // A worker which has observed alpha blocks on this lease.  Gamma is
        // also fenced so a worker that races and observes gamma is detected
        // by the timeout below and retried with a fresh fixture.
        let alpha_lock = Connection::open(&alpha).unwrap();
        alpha_lock.execute_batch("BEGIN IMMEDIATE").unwrap();
        let gamma_lock = Connection::open(gamma.join("authority.sqlite")).unwrap();
        gamma_lock.execute_batch("BEGIN IMMEDIATE").unwrap();

        let (started_sender, started_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        let worker_root = root.clone();
        let worker = thread::spawn(move || {
            started_sender.send(()).unwrap();
            result_sender
                .send(migrate_continuity(&worker_root, "generation-beta", None))
                .unwrap();
        });
        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        thread::sleep(Duration::from_millis(50));

        let locator = Connection::open(root.join("continuity_locator.sqlite")).unwrap();
        locator
            .execute(
                "UPDATE continuity_generation_locator SET generation_id = ?1 WHERE slot = 1",
                params!["generation-gamma"],
            )
            .unwrap();
        drop(locator);
        alpha_lock.execute_batch("COMMIT").unwrap();

        match result_receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(result) => {
                gamma_lock.execute_batch("COMMIT").unwrap();
                worker.join().unwrap();

                let error = result.unwrap_err();
                assert!(matches!(
                    error,
                    ContinuityMigrationError::ConcurrentLocatorChange
                ));
                assert!(!generations.join("generation-beta").exists());
                let current = open_current_generation(&root).unwrap();
                assert_eq!(current.generation_id, "generation-gamma");
                assert_eq!(current.authority.incarnation_id, expected.incarnation_id);
                assert_eq!(current.authority.revision, expected.revision);
                return;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                gamma_lock.execute_batch("COMMIT").unwrap();
                worker.join().unwrap();
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                gamma_lock.execute_batch("COMMIT").unwrap();
                worker.join().unwrap();
                panic!("migration worker disconnected before returning its result");
            }
        }
    }

    panic!("migration worker did not observe alpha before the locator switch");
}
