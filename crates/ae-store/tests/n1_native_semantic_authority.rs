use ae_continuum::CommitEnvelope;
use ae_contracts::{
    wire, AllostaticSetpoints, CanonicalEvent, CommitStatus, EpistemicPriors, ExpressionPhenotype,
    GenesisManifest, GenesisReceipt, GenesisStatus, InvariantResiduals, PersonaScopeRef,
    PersonaSelectionKind, PersonaSourceRef, PersonalityVector, ScopeRef, SocialPriors, TimeAdvance,
    TransitionReceipt,
};
use ae_fixed::Fixed;
use ae_store::{
    decode_n1_native_bundle_v1, encode_n1_native_bundle_v1, n1_native_bundle_digest_v1,
    n1_state_bytes_digest_v1, n1_transition_receipt_digest_v1, ActionBindingV1,
    ClosedEstimateBindingV1, GenesisCommit, KvReferenceV1, MorphBindingV1, N1IdentityBindingV1,
    N1NativeSemanticBundleV1, N1ScopeBindingV1, N1StateBindingV1, N1TurnBindingV1, PolicyBindingV1,
    SomaBindingV1, StatefulNativeSemanticCommitV1, Store, StoreError,
};
use rusqlite::params;
use std::path::Path;

fn digest(seed: u8) -> [u8; 32] {
    [seed; 32]
}

fn id(seed: u8) -> [u8; 16] {
    [seed; 16]
}

fn bundle() -> N1NativeSemanticBundleV1 {
    let scope = ScopeRef {
        bot_token: id(1),
        persona_token: id(2),
        relation_token: Some(id(3)),
        session_token: id(4),
    };
    N1NativeSemanticBundleV1 {
        schema_version: 1,
        identity: N1IdentityBindingV1 {
            incarnation_id: digest(10),
            manifest_digest: digest(11),
            seed_code_digest: digest(12),
            formula_digest: digest(13),
            constitution_digest: digest(14),
            genesis_receipt_digest: digest(15),
        },
        scope: N1ScopeBindingV1 {
            scope,
            writer_scope_digest: digest(16),
            turn_scope_digest: digest(17),
        },
        state: N1StateBindingV1 {
            base_revision: 0,
            next_revision: 1,
            state_before_digest: digest(18),
            state_after_digest: digest(19),
            state_bytes_digest: digest(20),
            graph_after_digest: digest(21),
        },
        turn: N1TurnBindingV1 {
            turn_id: id(5),
            turn_binding_digest: digest(22),
            session_binding_digest: digest(23),
            exact_anchor_set_digest: digest(24),
            relation_scope_digest: digest(25),
            owner_attestation_digest: digest(26),
        },
        event_digest: digest(27),
        receipt_digest: digest(28),
        kv_refs: vec![KvReferenceV1 {
            key_digest: digest(29),
            value_digest: digest(30),
            canonical_value_digest: digest(31),
            canonical_value_len: 9,
            kv_stream_revision: 7,
        }],
        soma: SomaBindingV1 {
            source_state_digest: digest(32),
            soma_state_digest: digest(33),
            source_owner_attestation_digest: digest(34),
        },
        morph: MorphBindingV1 {
            source_state_digest: digest(35),
            state_binding_digest: digest(36),
            catalog_digest: digest(37),
            source_owner_attestation_digest: digest(38),
        },
        estimate: ClosedEstimateBindingV1 {
            estimate_digest: digest(39),
            evidence_vector_digest: digest(40),
            estimator_digest: digest(41),
            estimator_confidence: Fixed::from_raw(800_000),
            source_owner_attestation_digest: digest(42),
        },
        policy: PolicyBindingV1 {
            policy_version: 1,
            policy_digest: digest(43),
            policy_expires_at_ms: 99,
            policy_owner_attestation_digest: digest(44),
        },
        action: None,
        provenance_digest: digest(47),
        bundle_digest: digest(48),
    }
}

#[test]
fn canonical_bundle_round_trips_and_digest_is_stable() {
    let mut source = bundle();
    source.bundle_digest = n1_native_bundle_digest_v1(&source).unwrap();
    let expected_digest = source.bundle_digest;
    let bytes = encode_n1_native_bundle_v1(&source).unwrap();
    let decoded = decode_n1_native_bundle_v1(&bytes).unwrap();
    assert_eq!(decoded, source);
    assert_eq!(
        n1_native_bundle_digest_v1(&decoded).unwrap(),
        expected_digest
    );
}

#[test]
fn codec_rejects_trailing_bytes_and_zero_bindings() {
    let mut source = bundle();
    source.bundle_digest = n1_native_bundle_digest_v1(&source).unwrap();
    let mut bytes = encode_n1_native_bundle_v1(&source).unwrap();
    bytes.push(0);
    assert!(decode_n1_native_bundle_v1(&bytes).is_err());

    let mut zero = source;
    zero.identity.formula_digest = [0; 32];
    assert!(encode_n1_native_bundle_v1(&zero).is_err());
}

#[test]
fn codec_rejects_noncanonical_kv_order() {
    let mut source = bundle();
    let mut second = source.kv_refs[0].clone();
    second.key_digest = digest(1);
    source.kv_refs.push(second);
    assert!(encode_n1_native_bundle_v1(&source).is_err());
}

#[test]
fn codec_rejects_action_bearing_bundle_until_fixed_codec_exists() {
    let mut source = bundle();
    source.action = Some(ActionBindingV1 {
        action_id: id(6),
        action_contract_digest: digest(45),
        action_contract_bytes: vec![0xde, 0xad, 0xbe, 0xef],
        action_owner_attestation_digest: digest(46),
    });
    assert!(encode_n1_native_bundle_v1(&source).is_err());
}

fn native_commit_fixture() -> (Store, StatefulNativeSemanticCommitV1, ScopeRef) {
    native_commit_fixture_with_store(Store::open_in_memory().unwrap())
}

fn native_commit_fixture_with_store(
    mut store: Store,
) -> (Store, StatefulNativeSemanticCommitV1, ScopeRef) {
    let scope = ScopeRef {
        bot_token: id(11),
        persona_token: id(12),
        relation_token: None,
        session_token: id(13),
    };
    let formula_digest = digest(13);
    let source = PersonaSourceRef {
        scope: PersonaScopeRef {
            bot_token: scope.bot_token,
            persona_token: scope.persona_token,
        },
        source_digest: digest(30),
        capability_digest: digest(31),
        selection: PersonaSelectionKind::ExplicitDefault,
        prompt_chars: 0,
        begin_dialog_count: 0,
        mood_dialog_count: 0,
    };
    let mut manifest = GenesisManifest {
        schema_version: 1,
        traits: PersonalityVector::default(),
        expression: ExpressionPhenotype::default(),
        allostasis: AllostaticSetpoints::default(),
        epistemic: EpistemicPriors::default(),
        social: SocialPriors::default(),
        manifest_digest: [0; 32],
    };
    manifest.manifest_digest = wire::manifest_body_digest(&manifest);
    let manifest_body = wire::encode_manifest_body(&manifest);
    let seed_code_digest = ae_genesis::derive_seed_code_digest(&manifest.manifest_digest);
    let incarnation_id = digest(32);
    let nonce_digest = digest(33);
    let scope_key = ae_genesis::genesis_scope_key(
        &scope.bot_token,
        &scope.persona_token,
        &source.source_digest,
        &formula_digest,
    );
    let lease_epoch = match store.claim_lease(&scope_key, Some(nonce_digest)).unwrap() {
        ae_store::ClaimOutcome::Claimed { lease_epoch, .. } => lease_epoch,
        other => panic!("unexpected genesis lease outcome: {other:?}"),
    };
    let genesis_receipt = GenesisReceipt {
        schema_version: 1,
        seed_code_digest,
        manifest_digest: manifest.manifest_digest,
        incarnation_id,
        formula_digest,
        persona_source_digest: source.source_digest,
        compiler_protocol_digest: digest(34),
        compiler_model_digest: digest(35),
        development_seed_digest: digest(36),
        initial_snapshot_digest: digest(15),
        graph_digest: digest(17),
        equilibrium_residual: Fixed::ZERO,
        energy_residual: Fixed::ZERO,
        capacity_residual: Fixed::ZERO,
        sample_fit_residual: Fixed::ZERO,
        status: GenesisStatus::Committed,
    };
    store
        .commit_genesis(&GenesisCommit {
            scope_key,
            lease_epoch,
            nonce_digest,
            manifest: manifest.clone(),
            manifest_body,
            seed_code_digest,
            incarnation_id,
            formula_digest,
            source: source.clone(),
            compiler_protocol_digest: genesis_receipt.compiler_protocol_digest,
            compiler_model_digest: genesis_receipt.compiler_model_digest,
            compiled_at_ms: 1,
            receipt: genesis_receipt.clone(),
            initial_snapshot_digest: genesis_receipt.initial_snapshot_digest,
            state_bytes: b"genesis-state".to_vec(),
            graph_digest: genesis_receipt.graph_digest,
        })
        .unwrap();
    let genesis_context = store.read_n1_authority_context_v1(&scope).unwrap();
    assert_eq!(genesis_context.current_revision, 0);
    assert_eq!(genesis_context.state_bytes, b"genesis-state");
    let writer_scope_digest = wire::persona_scope_digest(
        &scope.bot_token,
        &scope.persona_token,
        scope.relation_token.as_ref(),
    );
    let event = CanonicalEvent::TimeAdvance(TimeAdvance {
        event_id: id(14),
        scope: scope.clone(),
        elapsed_ms: 5,
    });
    let event_bytes = wire::encode_event(&event);
    let event_digest = wire::event_digest(&event);
    let receipt = TransitionReceipt {
        schema_version: 1,
        formula_digest,
        scope_digest: writer_scope_digest,
        event_digest,
        authority_digest: digest(14),
        base_revision: 0,
        next_revision: 1,
        state_before: digest(15),
        state_after: digest(16),
        graph_after: digest(17),
        action_contract: None,
        active_nodes: 1,
        active_edges: 0,
        residuals: InvariantResiduals::default(),
        status: CommitStatus::Committed,
    };
    let mut bundle = bundle();
    bundle.scope.scope = scope.clone();
    bundle.scope.writer_scope_digest = writer_scope_digest;
    bundle.scope.turn_scope_digest = wire::scope_digest(&scope);
    bundle.state.base_revision = 0;
    bundle.state.next_revision = 1;
    bundle.state.state_before_digest = receipt.state_before;
    bundle.state.state_after_digest = receipt.state_after;
    bundle.state.graph_after_digest = receipt.graph_after;
    bundle.event_digest = event_digest;
    bundle.identity.formula_digest = receipt.formula_digest;
    bundle.identity.incarnation_id = incarnation_id;
    bundle.identity.manifest_digest = manifest.manifest_digest;
    bundle.identity.seed_code_digest = seed_code_digest;
    bundle.identity.constitution_digest = genesis_context.identity.constitution_digest;
    bundle.identity.genesis_receipt_digest = wire::genesis_receipt_digest(&genesis_receipt);
    bundle.receipt_digest = n1_transition_receipt_digest_v1(&receipt);
    bundle.state.state_bytes_digest = n1_state_bytes_digest_v1(b"native-state");
    bundle.bundle_digest = n1_native_bundle_digest_v1(&bundle).unwrap();
    let journal = CommitEnvelope {
        event_kind: "time_advance".to_owned(),
        event_bytes,
        receipt,
        chain_seed: genesis_receipt.initial_snapshot_digest,
        delta_bytes: vec![],
    };
    let commit = StatefulNativeSemanticCommitV1 {
        journal,
        state_bytes: b"native-state".to_vec(),
        bundle,
    };
    (store, commit, scope)
}

#[test]
fn native_commit_is_atomic_and_reopenable() {
    let (mut store, commit, scope) = native_commit_fixture();
    let committed = store
        .commit_stateful_n1_native_semantic_v1(&commit)
        .unwrap();
    assert_eq!(committed.revision, 1);
    assert_eq!(store.count_journal().unwrap(), 1);
    assert_eq!(
        store
            .current_revision(&committed.bundle.scope.writer_scope_digest)
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .read_n1_native_semantic_v1(&committed.bundle.scope.writer_scope_digest, 1)
            .unwrap(),
        Some(committed.bundle.clone())
    );
    let report = store
        .replay_n1_native_semantic_v1(&committed.bundle.scope.writer_scope_digest, 1)
        .unwrap();
    assert!(report.ok);
    assert_eq!(report.final_revision, 1);
    let context = store.read_n1_authority_context_v1(&scope).unwrap();
    assert_eq!(context.current_revision, 1);
    assert_eq!(context.state_bytes, b"native-state");

    // Exact retry is a read-only replay, not a second journal/snapshot write.
    let replayed = store
        .commit_stateful_n1_native_semantic_v1(&commit)
        .unwrap();
    assert_eq!(replayed, committed);
    assert_eq!(store.count_journal().unwrap(), 1);
}

#[test]
fn native_commit_rejects_empty_state_before_any_write() {
    let (mut store, mut commit, _) = native_commit_fixture();
    commit.state_bytes.clear();
    assert!(store
        .commit_stateful_n1_native_semantic_v1(&commit)
        .is_err());
    assert_eq!(store.count_journal().unwrap(), 0);
}

#[test]
fn native_commit_rejects_forged_genesis_identity_before_any_write() {
    let (mut store, mut commit, _) = native_commit_fixture();
    commit.bundle.identity.incarnation_id = digest(99);
    commit.bundle.bundle_digest = n1_native_bundle_digest_v1(&commit.bundle).unwrap();

    let error = store
        .commit_stateful_n1_native_semantic_v1(&commit)
        .unwrap_err();
    assert!(matches!(error, StoreError::N1BundleInvalid(_)));
    assert_eq!(store.count_journal().unwrap(), 0);
    assert_eq!(
        store
            .current_revision(&commit.bundle.scope.writer_scope_digest)
            .unwrap(),
        0
    );
}

#[test]
fn native_commit_rejects_caller_selected_constitution_before_any_write() {
    let (mut store, mut commit, _) = native_commit_fixture();
    commit.bundle.identity.constitution_digest = digest(98);
    commit.bundle.bundle_digest = n1_native_bundle_digest_v1(&commit.bundle).unwrap();

    let error = store
        .commit_stateful_n1_native_semantic_v1(&commit)
        .unwrap_err();
    assert!(matches!(error, StoreError::N1BundleInvalid(_)));
    assert_eq!(store.count_journal().unwrap(), 0);
}

#[test]
fn native_commit_rejects_forged_genesis_chain_seed_before_any_write() {
    let (mut store, mut commit, _) = native_commit_fixture();
    commit.journal.chain_seed = digest(99);

    let error = store
        .commit_stateful_n1_native_semantic_v1(&commit)
        .unwrap_err();
    assert!(matches!(error, StoreError::N1BundleInvalid(_)));
    assert_eq!(store.count_journal().unwrap(), 0);
    assert_eq!(
        store
            .current_revision(&commit.bundle.scope.writer_scope_digest)
            .unwrap(),
        0
    );
}

#[test]
fn native_commit_rejects_action_bearing_bundle_before_any_write() {
    let (mut store, mut commit, _) = native_commit_fixture();
    commit.bundle.action = Some(ActionBindingV1 {
        action_id: id(6),
        action_contract_digest: digest(99),
        action_contract_bytes: vec![0xde, 0xad, 0xbe, 0xef],
        action_owner_attestation_digest: digest(46),
    });
    commit.journal.receipt.action_contract = Some(digest(99));

    let error = store
        .commit_stateful_n1_native_semantic_v1(&commit)
        .unwrap_err();
    assert!(matches!(error, StoreError::N1BundleInvalid(_)));
    assert_eq!(store.count_journal().unwrap(), 0);
    assert_eq!(
        store
            .current_revision(&commit.bundle.scope.writer_scope_digest)
            .unwrap(),
        0
    );
}

#[test]
fn native_replay_checks_a_second_revision_and_state_chain() {
    let (mut store, first, _scope) = native_commit_fixture();
    let first_committed = store.commit_stateful_n1_native_semantic_v1(&first).unwrap();

    let scope = first.bundle.scope.scope.clone();
    let second_event = CanonicalEvent::TimeAdvance(TimeAdvance {
        event_id: id(55),
        scope: scope.clone(),
        elapsed_ms: 7,
    });
    let second_event_bytes = wire::encode_event(&second_event);
    let second_event_digest = wire::event_digest(&second_event);
    let second_state = b"native-state-2".to_vec();
    let mut second = first.clone();
    second.state_bytes = second_state.clone();
    second.journal.event_bytes = second_event_bytes;
    second.journal.receipt.base_revision = 1;
    second.journal.receipt.next_revision = 2;
    second.journal.receipt.event_digest = second_event_digest;
    second.journal.receipt.state_before = first.bundle.state.state_after_digest;
    second.journal.receipt.state_after = digest(56);
    second.journal.receipt.graph_after = digest(57);
    second.journal.chain_seed = store
        .last_chain_digest(&first.bundle.scope.writer_scope_digest)
        .unwrap()
        .unwrap();
    second.bundle.state.base_revision = 1;
    second.bundle.state.next_revision = 2;
    second.bundle.state.state_before_digest = first.bundle.state.state_after_digest;
    second.bundle.state.state_after_digest = second.journal.receipt.state_after;
    second.bundle.state.state_bytes_digest = n1_state_bytes_digest_v1(&second_state);
    second.bundle.state.graph_after_digest = second.journal.receipt.graph_after;
    second.bundle.event_digest = second_event_digest;
    second.bundle.receipt_digest = n1_transition_receipt_digest_v1(&second.journal.receipt);
    second.bundle.provenance_digest = digest(58);
    second.bundle.bundle_digest = n1_native_bundle_digest_v1(&second.bundle).unwrap();

    let second_committed = store
        .commit_stateful_n1_native_semantic_v1(&second)
        .unwrap();
    assert_eq!(first_committed.revision, 1);
    assert_eq!(second_committed.revision, 2);
    let report = store
        .replay_n1_native_semantic_v1(&first.bundle.scope.writer_scope_digest, 2)
        .unwrap();
    assert!(report.ok, "{report:?}");
    assert_eq!(report.checked, 2);
    assert_eq!(report.final_revision, 2);
}

#[test]
fn native_replay_rejects_malformed_persisted_digest_without_panicking() {
    let root = std::env::var("AE_RC1_TASK_TEMP").expect("G-local task root");
    let path = Path::new(&root).join("n1-native-replay-tamper.sqlite");
    let _ = std::fs::remove_file(&path);
    let (mut store, commit, _scope) = native_commit_fixture_with_store(Store::open(&path).unwrap());
    let committed = store
        .commit_stateful_n1_native_semantic_v1(&commit)
        .unwrap();
    store.close().unwrap();

    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE journal SET chain_digest = ?1 WHERE scope_digest = ?2 AND logical_revision = 1",
        params![
            vec![0u8; 3],
            committed.bundle.scope.writer_scope_digest.to_vec()
        ],
    )
    .unwrap();
    drop(conn);

    let reopened = Store::open(&path).unwrap();
    let read = reopened.read_n1_native_semantic_v1(&committed.bundle.scope.writer_scope_digest, 1);
    assert!(matches!(read, Err(StoreError::N1BundleInvalid(_))));
    let report = reopened
        .replay_n1_native_semantic_v1(&committed.bundle.scope.writer_scope_digest, 1)
        .unwrap();
    assert!(!report.ok);
    assert!(report.first_error.is_some());
}
