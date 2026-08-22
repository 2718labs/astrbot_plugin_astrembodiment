use ae_agent::r7::{
    compile_epistemic_action_policy_v1, ActionAuthorityContextV1, ActionPolicyErrorV1,
    CallerSelectedFieldsV1, EpistemicActionInputV1, EpistemicSourceRefV1, PolicyArtifactV1,
    EPISTEMIC_SOURCE_PROVENANCE_DOMAIN_V1,
};
use ae_contracts::r7::{wire, Digest, Id128};

fn digest(seed: u8) -> Digest {
    [seed; 32]
}

fn id(seed: u8) -> Id128 {
    [seed; 16]
}

fn valid_source() -> Digest {
    let revision = 4u64.to_be_bytes();
    wire::domain_hash(
        EPISTEMIC_SOURCE_PROVENANCE_DOMAIN_V1,
        &[&digest(10), &digest(20), &digest(21), &id(7), &revision],
    )
}

fn input_for_evidence(seed: u8) -> EpistemicActionInputV1 {
    let state = digest(seed);
    let identity = digest(seed.wrapping_add(1));
    let scope = digest(seed.wrapping_add(2));
    let turn = id(seed.wrapping_add(3));
    let revision = 4u64.to_be_bytes();
    let source_digest = wire::domain_hash(
        EPISTEMIC_SOURCE_PROVENANCE_DOMAIN_V1,
        &[&state, &identity, &scope, &turn, &revision],
    );
    EpistemicActionInputV1 {
        source: Some(EpistemicSourceRefV1 {
            source_digest,
            state_digest: state,
            identity_digest: identity,
            scope_digest: scope,
            turn_id: turn,
            revision: 4,
        }),
        context: ActionAuthorityContextV1 {
            state_digest: state,
            identity_digest: identity,
            scope_digest: scope,
            turn_id: turn,
            revision: 4,
            now_ms: 1_000,
            policy: PolicyArtifactV1::derive(1, identity, scope, 100),
        },
        r7_available: true,
    }
}

fn policy() -> PolicyArtifactV1 {
    PolicyArtifactV1::derive(1, digest(20), digest(21), 100)
}

fn input(source_digest: Option<Digest>, r7_available: bool) -> EpistemicActionInputV1 {
    let policy = policy();
    EpistemicActionInputV1 {
        source: source_digest.map(|source_digest| EpistemicSourceRefV1 {
            source_digest,
            state_digest: digest(10),
            identity_digest: digest(20),
            scope_digest: digest(21),
            turn_id: id(7),
            revision: 4,
        }),
        context: ActionAuthorityContextV1 {
            state_digest: digest(10),
            identity_digest: digest(20),
            scope_digest: digest(21),
            turn_id: id(7),
            revision: 4,
            now_ms: 1_000,
            policy,
        },
        r7_available,
    }
}

#[test]
fn red_source_digest_is_nonzero_and_recomputed_from_typed_provenance() {
    assert!(compile_epistemic_action_policy_v1(
        &input(Some(valid_source()), true),
        CallerSelectedFieldsV1::default(),
    )
    .is_ok());
    assert_eq!(
        compile_epistemic_action_policy_v1(
            &input(Some([0; 32]), true),
            CallerSelectedFieldsV1::default(),
        ),
        Err(ActionPolicyErrorV1::ZeroSourceDigest)
    );

    assert_eq!(
        compile_epistemic_action_policy_v1(
            &input(Some(digest(30)), true),
            CallerSelectedFieldsV1::default(),
        ),
        Err(ActionPolicyErrorV1::SourceDigestMismatch)
    );
}

#[test]
fn red_two_evidence_vectors_make_deterministic_distinct_contracts() {
    let a = compile_epistemic_action_policy_v1(
        &input_for_evidence(30),
        CallerSelectedFieldsV1::default(),
    )
    .expect("first policy");
    let b = compile_epistemic_action_policy_v1(
        &input_for_evidence(31),
        CallerSelectedFieldsV1::default(),
    )
    .expect("second policy");
    let a_same = compile_epistemic_action_policy_v1(
        &input_for_evidence(30),
        CallerSelectedFieldsV1::default(),
    )
    .expect("same policy");

    assert_ne!(a.contract_digest(), b.contract_digest());
    assert_eq!(a.contract_digest(), a_same.contract_digest());
    assert_ne!(a.provenance_digest(), &[0; 32]);
}

#[test]
fn red_policy_binds_identity_scope_state_turn_and_revision() {
    let result = compile_epistemic_action_policy_v1(
        &input(Some(valid_source()), true),
        CallerSelectedFieldsV1::default(),
    )
    .expect("policy");
    assert_eq!(result.contract().expect("contract").turn_id, id(7));

    let mut foreign = input(Some(valid_source()), true);
    foreign.source.as_mut().expect("source").identity_digest = digest(99);
    assert_eq!(
        compile_epistemic_action_policy_v1(&foreign, CallerSelectedFieldsV1::default()),
        Err(ActionPolicyErrorV1::ForeignIdentity)
    );

    let mut foreign_state = input(Some(valid_source()), true);
    foreign_state.source.as_mut().expect("source").state_digest = digest(98);
    assert_eq!(
        compile_epistemic_action_policy_v1(&foreign_state, CallerSelectedFieldsV1::default()),
        Err(ActionPolicyErrorV1::ForeignState)
    );

    let mut foreign_scope = input(Some(valid_source()), true);
    foreign_scope.source.as_mut().expect("source").scope_digest = digest(97);
    assert_eq!(
        compile_epistemic_action_policy_v1(&foreign_scope, CallerSelectedFieldsV1::default()),
        Err(ActionPolicyErrorV1::ForeignScope)
    );

    let mut forged_policy = input(Some(valid_source()), true);
    forged_policy.context.policy.digest = digest(96);
    assert_eq!(
        compile_epistemic_action_policy_v1(&forged_policy, CallerSelectedFieldsV1::default()),
        Err(ActionPolicyErrorV1::PolicyDigestMismatch)
    );
}

#[test]
fn red_rejects_missing_source_foreign_revision_turn_overflow_expiry_and_caller_fields() {
    assert_eq!(
        compile_epistemic_action_policy_v1(&input(None, true), CallerSelectedFieldsV1::default()),
        Err(ActionPolicyErrorV1::MissingEpistemicSource)
    );

    let mut foreign_revision = input(Some(valid_source()), true);
    foreign_revision.source.as_mut().expect("source").revision = 5;
    assert_eq!(
        compile_epistemic_action_policy_v1(&foreign_revision, CallerSelectedFieldsV1::default()),
        Err(ActionPolicyErrorV1::ForeignRevision)
    );

    let mut foreign_turn = input(Some(valid_source()), true);
    foreign_turn.source.as_mut().expect("source").turn_id = id(8);
    assert_eq!(
        compile_epistemic_action_policy_v1(&foreign_turn, CallerSelectedFieldsV1::default()),
        Err(ActionPolicyErrorV1::ForeignTurn)
    );

    let mut overflow = input(Some(valid_source()), true);
    overflow.context.now_ms = u64::MAX;
    assert_eq!(
        compile_epistemic_action_policy_v1(&overflow, CallerSelectedFieldsV1::default()),
        Err(ActionPolicyErrorV1::TimeOverflow)
    );

    let mut expired = input(Some(valid_source()), true);
    expired.context.policy = PolicyArtifactV1::derive(1, digest(20), digest(21), 0);
    assert_eq!(
        compile_epistemic_action_policy_v1(&expired, CallerSelectedFieldsV1::default()),
        Err(ActionPolicyErrorV1::Expired)
    );

    for caller in [
        CallerSelectedFieldsV1 {
            classification: true,
            ..CallerSelectedFieldsV1::default()
        },
        CallerSelectedFieldsV1 {
            action: true,
            ..CallerSelectedFieldsV1::default()
        },
        CallerSelectedFieldsV1 {
            text: true,
            ..CallerSelectedFieldsV1::default()
        },
        CallerSelectedFieldsV1 {
            provider: true,
            ..CallerSelectedFieldsV1::default()
        },
        CallerSelectedFieldsV1 {
            control: true,
            ..CallerSelectedFieldsV1::default()
        },
    ] {
        assert_eq!(
            compile_epistemic_action_policy_v1(&input(Some(valid_source()), true), caller),
            Err(ActionPolicyErrorV1::CallerSelectedFieldRejected)
        );
    }
}

#[test]
fn red_missing_r7_keeps_g0_only_without_synthetic_action() {
    let result = compile_epistemic_action_policy_v1(
        &input(Some(valid_source()), false),
        CallerSelectedFieldsV1::default(),
    )
    .expect("G0-only result");
    assert!(result.g0_only());
    assert!(result.contract().is_none());
}
