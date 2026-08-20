use ae_contracts::{
    CausalRef, Digest, EvidenceVector, Id128, ScopeRef, SemanticEstimate, VerdictKind,
};
use ae_epistemic_state::{
    compile_epistemic_projection_v1, CallerProvidedEpistemicClassificationV1,
    EpistemicEvidenceGapV1, EpistemicProjectionInputV1, EpistemicSourceBindingV1,
    EpistemicStateErrorV1, VerifierNeedV1, EPISTEMIC_EVIDENCE_DIMENSION_COUNT_V1,
};
use ae_fixed::Fixed;
use ae_genesis::{
    AntiGoalsV1, CorrectionBoundaryConstitutionV1, ExpressionBasisV1, IdentityBoundsV1,
    IdentityConstitutionV1, IncarnationRefV1, OperationalCommitmentsV1, RelationalPlayLimitsV1,
    SeedCodeV1,
};

fn digest(seed: u8) -> Digest {
    [seed; 32]
}

fn id(seed: u8) -> Id128 {
    [seed; 16]
}

fn scope() -> ScopeRef {
    ScopeRef {
        bot_token: id(1),
        persona_token: id(2),
        relation_token: Some(id(3)),
        session_token: id(4),
    }
}

fn identity() -> IdentityConstitutionV1 {
    let bounds = IdentityBoundsV1::new(1, 64).expect("identity bounds");
    let seed = SeedCodeV1::new(
        "epistemic.projection.seed".to_owned(),
        64,
        digest(20),
        digest(21),
    )
    .expect("seed");
    let incarnation = IncarnationRefV1::derive(&seed, digest(22), 1).expect("incarnation");
    IdentityConstitutionV1::derive(
        &incarnation,
        OperationalCommitmentsV1::new(vec!["truth_over_appeasement".to_owned()], bounds)
            .expect("commitments"),
        AntiGoalsV1::new(vec!["avoid_invented_memory".to_owned()], bounds).expect("anti goals"),
        ExpressionBasisV1::new(vec!["directness:high".to_owned()], bounds).expect("expression"),
        CorrectionBoundaryConstitutionV1::new(
            vec!["acknowledge_confirmed_error".to_owned()],
            bounds,
        )
        .expect("correction boundary"),
        RelationalPlayLimitsV1::new(vec!["honor_current_scope".to_owned()], bounds)
            .expect("relational limits"),
    )
    .expect("identity constitution")
}

fn binding(turn_id: Id128, revision: u64) -> EpistemicSourceBindingV1 {
    EpistemicSourceBindingV1::new(scope(), turn_id, digest(5), revision, identity())
        .expect("valid bound source")
}

fn estimate(conflict: Fixed) -> SemanticEstimate {
    SemanticEstimate {
        schema_version: 1,
        dimensions: EvidenceVector {
            epistemic_conflict: conflict,
            new_information: Fixed::from_raw(250_000),
            ..EvidenceVector::default()
        },
        estimator_confidence: Fixed::from_raw(800_000),
        estimator_digest: digest(7),
    }
}

fn classification() -> CallerProvidedEpistemicClassificationV1 {
    CallerProvidedEpistemicClassificationV1::new(
        VerdictKind::ConfirmedSelfError,
        vec![
            EpistemicEvidenceGapV1::ConflictingEvidence,
            EpistemicEvidenceGapV1::VerifierPending,
        ],
        VerifierNeedV1::Required,
        Fixed::from_raw(420_000),
        true,
        true,
    )
    .expect("canonical caller-provided classification")
}

fn input(turn_id: Id128, revision: u64) -> EpistemicProjectionInputV1 {
    EpistemicProjectionInputV1::new(
        binding(turn_id, revision),
        CausalRef {
            turn_id,
            action_id: Some(id(8)),
            delivery_id: None,
            claim_id: Some(id(9)),
            base_revision: revision,
        },
        estimate(Fixed::from_raw(500_000)),
        classification(),
    )
}

#[test]
fn compiles_existing_typed_evidence_into_a_bound_projection() {
    let projection = compile_epistemic_projection_v1(&input(id(10), 11))
        .expect("projection from typed evidence");

    assert_eq!(projection.turn_id(), &id(10));
    assert_eq!(projection.revision(), 11);
    assert_eq!(projection.claim_under_challenge(), Some(&id(9)));
    assert!(projection.classification_is_caller_provided());
    assert_ne!(projection.identity_digest(), &[0; 32]);
    assert_ne!(projection.source_estimate_digest(), &[0; 32]);
    assert_ne!(projection.projection_digest(), &[0; 32]);
    assert_eq!(EPISTEMIC_EVIDENCE_DIMENSION_COUNT_V1, 15);
}

#[test]
fn rejects_turn_and_revision_binding_mismatches() {
    let turn_mismatch = EpistemicProjectionInputV1::new(
        binding(id(10), 11),
        CausalRef {
            turn_id: id(12),
            action_id: None,
            delivery_id: None,
            claim_id: None,
            base_revision: 11,
        },
        estimate(Fixed::ZERO),
        classification(),
    );
    assert_eq!(
        compile_epistemic_projection_v1(&turn_mismatch),
        Err(EpistemicStateErrorV1::TurnBindingMismatch)
    );

    let revision_mismatch = EpistemicProjectionInputV1::new(
        binding(id(10), 11),
        CausalRef {
            turn_id: id(10),
            action_id: None,
            delivery_id: None,
            claim_id: None,
            base_revision: 12,
        },
        estimate(Fixed::ZERO),
        classification(),
    );
    assert_eq!(
        compile_epistemic_projection_v1(&revision_mismatch),
        Err(EpistemicStateErrorV1::RevisionBindingMismatch)
    );
}

#[test]
fn rejects_noncanonical_or_unbounded_caller_classification() {
    assert_eq!(
        CallerProvidedEpistemicClassificationV1::new(
            VerdictKind::Unresolved,
            vec![
                EpistemicEvidenceGapV1::VerifierPending,
                EpistemicEvidenceGapV1::ConflictingEvidence,
            ],
            VerifierNeedV1::Required,
            Fixed::from_raw(500_000),
            false,
            false,
        ),
        Err(EpistemicStateErrorV1::NonCanonicalEvidenceGapOrder { index: 1 })
    );
    assert_eq!(
        CallerProvidedEpistemicClassificationV1::new(
            VerdictKind::Unresolved,
            vec![
                EpistemicEvidenceGapV1::InsufficientEvidence,
                EpistemicEvidenceGapV1::ConflictingEvidence,
                EpistemicEvidenceGapV1::VerifierPending,
                EpistemicEvidenceGapV1::VerifierPending,
            ],
            VerifierNeedV1::Required,
            Fixed::from_raw(500_000),
            false,
            false,
        ),
        Err(EpistemicStateErrorV1::TooManyEvidenceGaps {
            max_items: 3,
            actual_items: 4,
        })
    );
    assert_eq!(
        CallerProvidedEpistemicClassificationV1::new(
            VerdictKind::ConfirmedSelfError,
            Vec::new(),
            VerifierNeedV1::NotRequired,
            Fixed::from_raw(500_000),
            false,
            true,
        ),
        Err(EpistemicStateErrorV1::CorrectionRequiresAcknowledgement)
    );
}

#[test]
fn projection_digest_is_deterministic_and_binds_the_typed_estimate() {
    let first = compile_epistemic_projection_v1(&input(id(10), 11)).expect("first");
    let same = compile_epistemic_projection_v1(&input(id(10), 11)).expect("same");
    assert_eq!(first.projection_digest(), same.projection_digest());
    assert_eq!(
        first.source_estimate_digest(),
        same.source_estimate_digest()
    );

    let mut changed_input = input(id(10), 11);
    changed_input.estimate = estimate(Fixed::from_raw(600_000));
    let changed = compile_epistemic_projection_v1(&changed_input).expect("changed");
    assert_ne!(
        first.source_estimate_digest(),
        changed.source_estimate_digest()
    );
    assert_ne!(first.projection_digest(), changed.projection_digest());
}
