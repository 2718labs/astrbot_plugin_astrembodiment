use ae_contracts::{
    wire, AllostaticSetpoints, EpistemicPriors, ExpressionPhenotype, GenesisManifestProposal,
    PersonaGenesisRequest, PersonaScopeRef, PersonaSelectionKind, PersonaSourceRef,
    PersonalityVector, ScopeRef, SocialPriors,
};
use ae_fixed::Fixed;
use ae_runtime::AstrRuntime;
use ae_store::{
    RebirthActionV1, RebirthOutcomeV1, RebirthPrepareRequestV1, RebirthResponseStateV1,
    UserAuthorizedRebirthV1,
};
use std::path::{Path, PathBuf};

fn task_temp_dir(label: &str) -> PathBuf {
    let root = std::env::var_os("CODEX_TASK_TEMP")
        .map(PathBuf::from)
        .expect("CODEX_TASK_TEMP must be set for rebirth integration tests");
    let path = root.join(format!("runtime-rebirth-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn request(seed: u8) -> PersonaGenesisRequest {
    let scope = PersonaScopeRef {
        bot_token: [seed; 16],
        persona_token: [seed.wrapping_add(1); 16],
    };
    let source = PersonaSourceRef {
        scope,
        source_digest: [seed.wrapping_add(2); 32],
        capability_digest: [seed.wrapping_add(3); 32],
        selection: PersonaSelectionKind::Conversation,
        prompt_chars: 10,
        begin_dialog_count: 1,
        mood_dialog_count: 0,
    };
    let proposal = GenesisManifestProposal {
        schema_version: 1,
        source: source.clone(),
        traits: PersonalityVector {
            baseline_warmth: Fixed::from_raw(700_000),
            ..PersonalityVector::default()
        },
        trait_confidence: PersonalityVector {
            baseline_warmth: Fixed::from_raw(500_000),
            ..PersonalityVector::default()
        },
        expression: ExpressionPhenotype::default(),
        allostasis: AllostaticSetpoints::default(),
        epistemic: EpistemicPriors::default(),
        social: SocialPriors::default(),
        compiler_protocol_digest: [seed.wrapping_add(4); 32],
        compiler_model_digest: [seed.wrapping_add(5); 32],
    };
    PersonaGenesisRequest {
        source,
        proposal,
        formula_digest: [seed.wrapping_add(6); 32],
        incarnation_nonce: [seed.wrapping_add(7); 32],
        parent_incarnation_id: None,
        observed_at_ms: 1_700_000_000_000,
    }
}

fn scope_for(request: &PersonaGenesisRequest) -> ScopeRef {
    ScopeRef {
        bot_token: request.source.scope.bot_token,
        persona_token: request.source.scope.persona_token,
        relation_token: None,
        session_token: [0x55; 16],
    }
}

fn prepare_request(
    scope: &ScopeRef,
    incarnation_id: [u8; 32],
    action: RebirthActionV1,
) -> RebirthPrepareRequestV1 {
    RebirthPrepareRequestV1 {
        scope_token: wire::persona_scope_digest(
            &scope.bot_token,
            &scope.persona_token,
            scope.relation_token.as_ref(),
        ),
        expected_incarnation_id: incarnation_id,
        expected_revision: 0,
        action: action.clone(),
    }
}

fn assert_commits_once_and_replays_after_restart(path: &Path, action: RebirthActionV1, seed: u8) {
    let request = request(seed);
    let scope = scope_for(&request);
    let mut runtime = AstrRuntime::open(path).unwrap();
    let genesis = runtime.ensure_genesis(&request).unwrap();
    let prepare = prepare_request(&scope, genesis.incarnation_id, action);

    let challenge = runtime.prepare_rebirth_v1(&scope, &prepare).unwrap();
    assert_eq!(challenge.state, RebirthResponseStateV1::ConfirmationPending);
    assert_ne!(challenge.request_nonce, [0; 32]);
    assert_ne!(challenge.request_nonce_digest, [0; 32]);
    assert_ne!(challenge.binding_digest, [0; 32]);
    runtime.flush_and_close().unwrap();
    drop(runtime);

    let confirmation = UserAuthorizedRebirthV1 {
        scope_token: prepare.scope_token,
        expected_incarnation_id: prepare.expected_incarnation_id,
        expected_revision: prepare.expected_revision,
        request_nonce: challenge.request_nonce,
        action: prepare.action.clone(),
        confirmed: true,
    };
    let mut reopened = AstrRuntime::open(path).unwrap();
    let committed = reopened.confirm_rebirth_v1(&scope, &confirmation).unwrap();
    assert_eq!(committed.state, RebirthResponseStateV1::Committed);
    let receipt = committed
        .receipt
        .expect("commit must issue an audit receipt");
    assert_eq!(receipt.before_revision, 0);
    assert_eq!(receipt.after_revision, 0);
    assert_eq!(receipt.outcome, RebirthOutcomeV1::Committed);
    assert!(receipt.audit_time_ms > 0);
    assert_ne!(
        receipt.parent_incarnation_short,
        receipt.child_incarnation_short
    );
    assert_eq!(reopened.current_revision(&scope).unwrap(), 0);
    reopened.flush_and_close().unwrap();
    drop(reopened);

    let mut replay_runtime = AstrRuntime::open(path).unwrap();
    let replayed = replay_runtime
        .confirm_rebirth_v1(&scope, &confirmation)
        .unwrap();
    assert_eq!(replayed.state, RebirthResponseStateV1::Replayed);
    assert_eq!(replayed.receipt, Some(receipt));
    assert_eq!(replay_runtime.current_revision(&scope).unwrap(), 0);
}

#[test]
fn rebirth_stages_one_child_then_replays_the_same_receipt_after_reopen() {
    let path = task_temp_dir("rebirth").join("runtime.sqlite3");
    assert_commits_once_and_replays_after_restart(&path, RebirthActionV1::Rebirth, 11);
}

#[test]
fn clear_active_state_uses_the_same_durable_runtime_commit_lane() {
    let path = task_temp_dir("clear").join("runtime.sqlite3");
    assert_commits_once_and_replays_after_restart(&path, RebirthActionV1::ClearActiveState, 21);
}
