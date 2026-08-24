use ae_contracts::{wire, Digest};
use ae_store::{
    ContinuityAuthority, RebirthActionV1, RebirthFaultV1, RebirthLifecycleError, RebirthOutcomeV1,
    RebirthPreflightV1, RebirthPrepareRequestV1, RebirthResponseStateV1, RebirthStagedChildV1,
    UserAuthorizedRebirthV1, VaultLifecycle,
};
use rusqlite::{params, Connection};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

fn fixture_root(name: &str) -> PathBuf {
    let base = std::env::var_os("AE_STORE_TEST_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    loop {
        let number = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = base.join(format!(
            "ae-store-rebirth-{name}-{}-{number}",
            std::process::id()
        ));
        match fs::create_dir(&root) {
            Ok(()) => return root,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("creating fixture root failed: {error}"),
        }
    }
}

fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

fn create_legacy_authority(path: &Path, incarnation_id: Digest, revision: u64) -> Digest {
    let bot = [0x41; 16];
    let persona = [0x42; 16];
    let scope = wire::persona_scope_digest(&bot, &persona, None);
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE active_bindings (bot_token BLOB NOT NULL, persona_token BLOB NOT NULL, incarnation_id BLOB NOT NULL, revision INTEGER NOT NULL);
        CREATE TABLE incarnations (incarnation_id BLOB PRIMARY KEY, graph_digest BLOB NOT NULL);
        CREATE TABLE snapshots (revision INTEGER NOT NULL, scope_digest BLOB NOT NULL, state_digest BLOB NOT NULL);
        CREATE TABLE journal (scope_digest BLOB NOT NULL, logical_revision INTEGER NOT NULL, chain_digest BLOB NOT NULL);
        "#,
    )
    .unwrap();
    conn.execute(
        "INSERT INTO active_bindings VALUES (?1, ?2, ?3, ?4)",
        params![
            bot.to_vec(),
            persona.to_vec(),
            incarnation_id.to_vec(),
            revision as i64
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO incarnations VALUES (?1, ?2)",
        params![incarnation_id.to_vec(), vec![0x23_u8; 32]],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO snapshots VALUES (?1, ?2, ?3)",
        params![revision as i64, scope.to_vec(), vec![0x22_u8; 32]],
    )
    .unwrap();
    if revision != 0 {
        conn.execute(
            "INSERT INTO journal VALUES (?1, ?2, ?3)",
            params![scope.to_vec(), revision as i64, vec![0x24_u8; 32]],
        )
        .unwrap();
    }
    scope
}

fn child_authority(incarnation_id: Digest) -> ContinuityAuthority {
    ContinuityAuthority {
        incarnation_id,
        revision: 0,
        state_digest: [0x22; 32],
        graph_digest: [0x23; 32],
        history_digest: wire::domain_hash(
            b"astr-embodiment/continuity-empty-history-v1",
            &[&incarnation_id, &0_u64.to_le_bytes()],
        ),
    }
}

fn confirmation(
    scope_token: Digest,
    incarnation_id: Digest,
    revision: u64,
    action: RebirthActionV1,
    request_nonce: [u8; 32],
) -> UserAuthorizedRebirthV1 {
    UserAuthorizedRebirthV1 {
        scope_token,
        expected_incarnation_id: incarnation_id,
        expected_revision: revision,
        request_nonce,
        action,
        confirmed: true,
    }
}

fn stage_manual_child(
    lifecycle: &VaultLifecycle,
    parent: &ae_store::RebirthCurrentV1,
    scope_token: Digest,
    action: RebirthActionV1,
    child_incarnation: Digest,
) -> RebirthStagedChildV1 {
    let child_generation = VaultLifecycle::child_generation_id_for(&child_incarnation);
    let child_database = lifecycle
        .child_authority_database_path(&child_generation)
        .unwrap();
    fs::create_dir_all(child_database.parent().unwrap()).unwrap();
    create_legacy_authority(&child_database, child_incarnation, 0);
    RebirthStagedChildV1 {
        scope_token,
        action,
        parent_generation_id: parent.generation_id.clone(),
        parent_authority: parent.authority.clone(),
        child_generation_id: child_generation,
        child_authority: child_authority(child_incarnation),
    }
}

#[test]
fn pending_challenge_survives_close_reopen_without_persisting_raw_nonce() {
    let root = fixture_root("pending");
    let legacy = root.join("legacy.sqlite");
    let vault = root.join("continuity-vault");
    let incarnation_id = [0x21; 32];
    let scope_token = create_legacy_authority(&legacy, incarnation_id, 14);

    let lifecycle = VaultLifecycle::open(&vault).unwrap();
    let current = lifecycle.bootstrap_legacy_store_v1(&legacy).unwrap();
    assert_eq!(current.authority.incarnation_id, incarnation_id);
    assert_eq!(current.authority.revision, 14);

    let prepared = lifecycle
        .prepare_rebirth(RebirthPrepareRequestV1 {
            scope_token,
            expected_incarnation_id: incarnation_id,
            expected_revision: 14,
            action: RebirthActionV1::Rebirth,
        })
        .unwrap();
    assert_eq!(prepared.state, RebirthResponseStateV1::ConfirmationPending);
    assert_ne!(prepared.request_nonce, [0; 32]);
    let raw_nonce = prepared.request_nonce;
    let nonce_digest = prepared.request_nonce_digest;
    let binding_digest = prepared.binding_digest;
    drop(lifecycle);

    let reopened = VaultLifecycle::open(&vault).unwrap();
    let stored = reopened
        .challenge_by_nonce_digest(nonce_digest)
        .unwrap()
        .expect("durable pending challenge");
    assert_eq!(stored.request_nonce_digest, nonce_digest);
    assert_eq!(stored.binding_digest, binding_digest);
    assert_eq!(stored.scope_token, scope_token);
    assert_eq!(stored.expected_incarnation_id, incarnation_id);
    assert_eq!(stored.expected_revision, 14);
    assert_eq!(stored.action, RebirthActionV1::Rebirth);
    assert!(!stored.contains_raw_nonce());
    assert_eq!(
        reopened.current_fence(scope_token).unwrap(),
        (incarnation_id, 14)
    );

    let ledger = fs::read(vault.join("rebirth_lifecycle.sqlite")).unwrap();
    assert!(!contains(&ledger, &raw_nonce));
}

#[test]
fn confirm_installs_one_complete_child_and_replays_after_restart() {
    let root = fixture_root("commit-replay");
    let legacy = root.join("legacy.sqlite");
    let vault = root.join("continuity-vault");
    let parent_incarnation = [0x31; 32];
    let scope_token = create_legacy_authority(&legacy, parent_incarnation, 14);
    let lifecycle = VaultLifecycle::open(&vault).unwrap();
    let parent = lifecycle.bootstrap_legacy_store_v1(&legacy).unwrap();
    let prepared = lifecycle
        .prepare_rebirth(RebirthPrepareRequestV1 {
            scope_token,
            expected_incarnation_id: parent_incarnation,
            expected_revision: 14,
            action: RebirthActionV1::Rebirth,
        })
        .unwrap();
    let request = confirmation(
        scope_token,
        parent_incarnation,
        14,
        RebirthActionV1::Rebirth,
        prepared.request_nonce,
    );
    let permit = match lifecycle.preflight_rebirth_confirmation(&request).unwrap() {
        RebirthPreflightV1::Stage(permit) => permit,
        other => panic!("unexpected preflight result: {other:?}"),
    };
    let child_incarnation = [0x32; 32];
    let staged = stage_manual_child(
        &lifecycle,
        &parent,
        scope_token,
        RebirthActionV1::Rebirth,
        child_incarnation,
    );
    let child_generation = staged.child_generation_id.clone();
    let committed = lifecycle.commit_rebirth(&permit, &staged).unwrap();
    assert_eq!(committed.state, RebirthResponseStateV1::Committed);
    let receipt = committed.receipt.clone().expect("committed receipt");
    assert_eq!(receipt.outcome, RebirthOutcomeV1::Committed);
    assert_eq!(receipt.before_revision, 14);
    assert_eq!(receipt.after_revision, 0);
    assert!(receipt.audit_time_ms > 0);
    drop(lifecycle);

    let reopened = VaultLifecycle::open(&vault).unwrap();
    let current = reopened.current_authority_v1().unwrap();
    assert_eq!(current.generation_id, child_generation);
    assert_eq!(current.authority, child_authority(child_incarnation));
    let replayed = reopened.preflight_rebirth_confirmation(&request).unwrap();
    assert_eq!(
        replayed,
        RebirthPreflightV1::Replayed(ae_store::RebirthResponseEnvelopeV1 {
            state: RebirthResponseStateV1::Replayed,
            receipt: Some(receipt),
        })
    );
}

#[test]
fn false_confirmation_and_nonce_conflict_leave_old_authority_unchanged() {
    let root = fixture_root("reject");
    let legacy = root.join("legacy.sqlite");
    let vault = root.join("continuity-vault");
    let parent_incarnation = [0x41; 32];
    let scope_token = create_legacy_authority(&legacy, parent_incarnation, 14);
    let lifecycle = VaultLifecycle::open(&vault).unwrap();
    let parent = lifecycle.bootstrap_legacy_store_v1(&legacy).unwrap();
    let prepared = lifecycle
        .prepare_rebirth(RebirthPrepareRequestV1 {
            scope_token,
            expected_incarnation_id: parent_incarnation,
            expected_revision: 14,
            action: RebirthActionV1::ClearActiveState,
        })
        .unwrap();
    let false_confirmation = UserAuthorizedRebirthV1 {
        scope_token,
        expected_incarnation_id: parent_incarnation,
        expected_revision: 14,
        request_nonce: prepared.request_nonce,
        action: RebirthActionV1::ClearActiveState,
        confirmed: false,
    };
    assert_eq!(
        lifecycle
            .preflight_rebirth_confirmation(&false_confirmation)
            .unwrap_err()
            .code(),
        "REBIRTH_CONFIRMATION_REQUIRED"
    );
    let conflict = confirmation(
        scope_token,
        parent_incarnation,
        14,
        RebirthActionV1::Rebirth,
        prepared.request_nonce,
    );
    assert_eq!(
        lifecycle
            .preflight_rebirth_confirmation(&conflict)
            .unwrap_err()
            .code(),
        "REBIRTH_NONCE_CONFLICT"
    );
    assert_eq!(lifecycle.current_authority_v1().unwrap(), parent);
}

#[test]
fn crash_boundaries_leave_only_old_or_complete_new_authority() {
    for (name, fault, expect_new) in [
        ("before", RebirthFaultV1::BeforeLocatorCommit, false),
        ("after", RebirthFaultV1::AfterLocatorCommit, true),
    ] {
        let root = fixture_root(name);
        let legacy = root.join("legacy.sqlite");
        let vault = root.join("continuity-vault");
        let parent_incarnation = [0x51; 32];
        let scope_token = create_legacy_authority(&legacy, parent_incarnation, 14);
        let lifecycle = VaultLifecycle::open(&vault).unwrap();
        let parent = lifecycle.bootstrap_legacy_store_v1(&legacy).unwrap();
        let prepared = lifecycle
            .prepare_rebirth(RebirthPrepareRequestV1 {
                scope_token,
                expected_incarnation_id: parent_incarnation,
                expected_revision: 14,
                action: RebirthActionV1::Rebirth,
            })
            .unwrap();
        let request = confirmation(
            scope_token,
            parent_incarnation,
            14,
            RebirthActionV1::Rebirth,
            prepared.request_nonce,
        );
        let permit = match lifecycle.preflight_rebirth_confirmation(&request).unwrap() {
            RebirthPreflightV1::Stage(permit) => permit,
            other => panic!("unexpected preflight result: {other:?}"),
        };
        let child_incarnation = if expect_new { [0x53; 32] } else { [0x52; 32] };
        let staged = stage_manual_child(
            &lifecycle,
            &parent,
            scope_token,
            RebirthActionV1::Rebirth,
            child_incarnation,
        );
        assert_eq!(
            lifecycle
                .commit_rebirth_with_fault(&permit, &staged, Some(fault))
                .unwrap_err(),
            RebirthLifecycleError::InjectedFault(fault)
        );
        drop(lifecycle);

        let reopened = VaultLifecycle::open(&vault).unwrap();
        let current = reopened.current_authority_v1().unwrap();
        if expect_new {
            assert_eq!(current.authority, child_authority(child_incarnation));
            let replayed = reopened.replay_rebirth(&request).unwrap().unwrap();
            assert_eq!(replayed.state, RebirthResponseStateV1::Replayed);
            assert_eq!(replayed.receipt.unwrap().after_revision, 0);
        } else {
            assert_eq!(current, parent);
            assert!(reopened.replay_rebirth(&request).unwrap().is_none());
        }
    }
}

#[test]
fn changed_authority_after_prepare_is_fenced_stale_without_losing_identity() {
    let root = fixture_root("stale-fence");
    let legacy = root.join("legacy.sqlite");
    let vault = root.join("continuity-vault");
    let incarnation_id = [0x61; 32];
    let scope_token = create_legacy_authority(&legacy, incarnation_id, 14);
    let lifecycle = VaultLifecycle::open(&vault).unwrap();
    let parent = lifecycle.bootstrap_legacy_store_v1(&legacy).unwrap();
    let prepared = lifecycle
        .prepare_rebirth(RebirthPrepareRequestV1 {
            scope_token,
            expected_incarnation_id: incarnation_id,
            expected_revision: 14,
            action: RebirthActionV1::Rebirth,
        })
        .unwrap();
    let current_database = lifecycle.current_authority_database_path().unwrap();
    let conn = Connection::open(current_database).unwrap();
    conn.execute(
        "UPDATE active_bindings SET revision = 15 WHERE incarnation_id = ?1",
        params![incarnation_id.to_vec()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO snapshots VALUES (?1, ?2, ?3)",
        params![15_i64, scope_token.to_vec(), vec![0x25_u8; 32]],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO journal VALUES (?1, ?2, ?3)",
        params![scope_token.to_vec(), 15_i64, vec![0x26_u8; 32]],
    )
    .unwrap();
    drop(conn);

    let request = confirmation(
        scope_token,
        incarnation_id,
        14,
        RebirthActionV1::Rebirth,
        prepared.request_nonce,
    );
    assert_eq!(
        lifecycle
            .preflight_rebirth_confirmation(&request)
            .unwrap_err()
            .code(),
        "REBIRTH_FENCE_STALE"
    );
    let current = lifecycle.current_authority_v1().unwrap();
    assert_eq!(current.generation_id, parent.generation_id);
    assert_eq!(current.authority.incarnation_id, incarnation_id);
    assert_eq!(current.authority.revision, 15);
    assert_eq!(current.authority.state_digest, [0x25; 32]);
}
