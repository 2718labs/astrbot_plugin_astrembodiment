use ae_action_contract::{
    ActionContractV1, ActionDispositionV1, ActionRealizationV1, ActionRequirementsV1,
    CanonicalTokenSetV1, CanonicalTokenV1, UnitIntervalV1,
};
use ae_cognitive_envelope::{
    compile_projection_v1, AffordanceCatalogV1, AffordanceV1, BoundedListV1, BoundedTextV1,
    CapsuleFieldV1, CognitiveKvViewV1, EpistemicsV1, ExactAnchorKindV1, ExactAnchorV1,
    ExactTurnAnchorsV1, OrganismSnapshotRefV1, PraxisV1, ProjectionErrorV1, ProjectionInput,
    ProjectionPreconditionsV1, ProjectionSourceKindV1, ProviderProfileV1, RelationScopeV1,
    RelationV1, SourceCapsuleV1, SourceProvenanceV1, TurnV1, COGNITIVE_ENVELOPE_SCHEMA_V1,
    MAX_PROJECTION_TOKENS,
};
use ae_contracts::r7::{
    wire, CausalRef, EvidenceVector, Id128, ScopeRef, SemanticEstimate, VerdictKind,
};
use ae_efference_copy::{EfferenceCopySourceV1, ExpectedDispositionV1, ObservedDispositionV1};
use ae_epistemic_state::{
    compile_epistemic_projection_v1, CallerProvidedEpistemicClassificationV1,
    EpistemicEvidenceGapV1, EpistemicProjectionInputV1, EpistemicSourceBindingV1, VerifierNeedV1,
};
use ae_fixed::Fixed;
use ae_genesis::r7::{
    AntiGoalsV1, CorrectionBoundaryConstitutionV1, ExpressionBasisV1, IdentityBoundsV1,
    IdentityConstitutionV1, IncarnationRefV1, OperationalCommitmentsV1, RelationalPlayLimitsV1,
    SeedCodeV1,
};
use ae_soma::{
    BoundedSomaSignalV1, CallerProvidedClassificationV1, SomaClassificationIngressV1,
    SomaFieldSetV1, SomaStateV1, SomaSubjectiveAxisV1,
};
use ae_subjective_present::{
    ConfidenceV1, DisclosureV1, SubjectiveBandV1, SubjectivePresentInputV1,
    SubjectivePresentProjectionV1, SubjectivePresentV1, SubjectiveTrendV1,
};

type Digest = [u8; 32];

const ACTION_ID: Id128 = [51; 16];
const TURN_ID: Id128 = [12; 16];
const TURN_BINDING: Digest = [12; 32];
const BASE_REVISION: u64 = 9;

fn digest(seed: u8) -> Digest {
    [seed; 32]
}

fn text(value: &str, max_chars: u32) -> BoundedTextV1 {
    BoundedTextV1::new(value.to_owned(), max_chars).expect("bounded fixture text")
}

fn list<T>(items: Vec<T>, max_items: u16) -> BoundedListV1<T> {
    BoundedListV1::new(items, max_items).expect("bounded fixture list")
}

fn fields(name: &str, value: &str, seed: u8) -> BoundedListV1<CapsuleFieldV1> {
    list(
        vec![
            CapsuleFieldV1::new(text(name, 64), text(value, 256), digest(seed))
                .expect("source-bound field"),
        ],
        8,
    )
}

fn identity_constitution(seed: u8) -> IdentityConstitutionV1 {
    let bounds = IdentityBoundsV1::new(8, 64).expect("identity bounds");
    let seed_code = SeedCodeV1::new(
        "persona.genesis.seed.v1".to_owned(),
        64,
        digest(seed),
        digest(seed.saturating_add(1)),
    )
    .expect("seed code");
    let incarnation = IncarnationRefV1::derive(&seed_code, digest(seed.saturating_add(2)), 1)
        .expect("incarnation");
    IdentityConstitutionV1::derive(
        &incarnation,
        OperationalCommitmentsV1::new(vec!["accept_verified_correction".to_owned()], bounds)
            .expect("commitments"),
        AntiGoalsV1::new(vec!["avoid_invented_memory".to_owned()], bounds).expect("anti-goals"),
        ExpressionBasisV1::new(vec!["directness:high".to_owned()], bounds).expect("expression"),
        CorrectionBoundaryConstitutionV1::new(vec!["respect_explicit_boundary".to_owned()], bounds)
            .expect("boundaries"),
        RelationalPlayLimitsV1::new(vec!["honor_current_scope".to_owned()], bounds)
            .expect("relational limits"),
    )
    .expect("identity constitution")
}

fn legacy_subjective_present() -> SubjectivePresentProjectionV1 {
    let item = SubjectivePresentV1::try_from_input(SubjectivePresentInputV1 {
        axis: "irritation".to_owned(),
        band: SubjectiveBandV1::Moderate,
        trend: SubjectiveTrendV1::Stable,
        behavioral_effect: "prefer_concise_output".to_owned(),
        disclosure: DisclosureV1::BehavioralOnly,
        confidence: ConfidenceV1::High,
        cause_ref: None,
    })
    .expect("legacy typed subjective present");
    SubjectivePresentProjectionV1::new(vec![item]).expect("legacy subjective projection")
}

fn token(value: &str) -> CanonicalTokenV1 {
    CanonicalTokenV1::new(value.to_owned(), 64).expect("action token")
}

fn token_set(values: &[&str]) -> CanonicalTokenSetV1 {
    CanonicalTokenSetV1::new(values.iter().map(|value| token(value)).collect(), 8)
        .expect("token set")
}

fn unit(value: u32) -> UnitIntervalV1 {
    UnitIntervalV1::from_parts_per_million(value).expect("unit interval")
}

fn action_contract(
    identity_digest: Digest,
    turn_binding: Digest,
    base_revision: u64,
    state_digest: Digest,
) -> ActionContractV1 {
    ActionContractV1::from_evaluation(
        ACTION_ID,
        turn_binding,
        base_revision,
        state_digest,
        identity_digest,
        ActionDispositionV1::Speech,
        token("answer"),
        ActionRequirementsV1::new(
            token_set(&["respect_exact_boundary"]),
            token_set(&[]),
            token_set(&[]),
            token_set(&["invent_memory"]),
        ),
        token_set(&[]),
        token_set(&["correction"]),
        unit(800_000),
        2_000,
    )
    .expect("typed action contract")
}

fn capsule<T>(
    source_kind: ProjectionSourceKindV1,
    source_seed: u8,
    source_revision: u64,
    content_digest: Digest,
    value: T,
) -> SourceCapsuleV1<T> {
    SourceCapsuleV1::new(
        SourceProvenanceV1::new(
            source_kind,
            text(&format!("source:{source_seed}"), 128),
            source_revision,
            digest(source_seed.saturating_add(100)),
        )
        .expect("provenance"),
        content_digest,
        value,
    )
    .expect("capsule")
}

fn soma_signal(signal_id: &str, value: f64) -> BoundedSomaSignalV1 {
    BoundedSomaSignalV1::new(signal_id.to_owned(), 64, value, 0.0, 1.0)
        .expect("bounded soma signal")
}

fn soma_field(signal_id: &str, value: f64) -> SomaFieldSetV1 {
    SomaFieldSetV1::new(vec![soma_signal(signal_id, value)], 8).expect("bounded soma field")
}

fn semantic_state_digest(revision: u64) -> Digest {
    wire::domain_hash(
        b"astr-embodiment/r7/test/committed-semantic-source-state-v1",
        &[&revision.to_be_bytes()],
    )
}

fn soma_state(identity_digest: Digest, revision: u64) -> SomaStateV1 {
    SomaStateV1::new_bound_to_source_state(
        "soma:snapshot:9".to_owned(),
        128,
        revision,
        identity_digest,
        soma_field("energy_availability", 0.7),
        soma_field("mobilization_balance", 0.6),
        soma_field("endocrine_load", 0.2),
        soma_field("repair_pressure", 0.3),
        soma_field("circadian_phase", 0.4),
        semantic_state_digest(revision),
    )
    .expect("typed soma state")
}

fn soma_classification(axis: SomaSubjectiveAxisV1) -> CallerProvidedClassificationV1 {
    CallerProvidedClassificationV1::new(
        axis,
        SubjectiveBandV1::Moderate,
        SubjectiveTrendV1::Stable,
        "prefer_bounded_effort".to_owned(),
        DisclosureV1::BehavioralOnly,
        ConfidenceV1::High,
        Some("soma:snapshot:9".to_owned()),
    )
    .expect("caller-provided soma classification")
}

fn epistemic_projection(
    identity: IdentityConstitutionV1,
    turn_id: Id128,
    state_digest: Digest,
    revision: u64,
) -> ae_epistemic_state::EpistemicProjectionV1 {
    let binding = EpistemicSourceBindingV1::new(
        ScopeRef {
            bot_token: [1; 16],
            persona_token: [2; 16],
            relation_token: Some([3; 16]),
            session_token: [4; 16],
        },
        turn_id,
        state_digest,
        revision,
        identity,
    )
    .expect("bound epistemic source");
    let estimate = SemanticEstimate {
        schema_version: 1,
        dimensions: EvidenceVector {
            epistemic_conflict: Fixed::from_raw(500_000),
            new_information: Fixed::from_raw(250_000),
            ..EvidenceVector::default()
        },
        estimator_confidence: Fixed::from_raw(800_000),
        estimator_digest: digest(71),
    };
    let classification = CallerProvidedEpistemicClassificationV1::new(
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
    .expect("caller-provided epistemic classification");
    compile_epistemic_projection_v1(&EpistemicProjectionInputV1::new(
        binding,
        CausalRef {
            turn_id,
            action_id: Some(ACTION_ID),
            delivery_id: None,
            claim_id: Some([81; 16]),
            base_revision: revision,
        },
        estimate,
        classification,
    ))
    .expect("typed epistemic projection")
}

#[derive(Clone, Copy)]
struct Variant {
    action_turn_binding: Option<Digest>,
    action_revision: Option<u64>,
    action_state_digest: Option<Digest>,
    action_identity_digest: Option<Digest>,
    action_content_digest_override: Option<Digest>,
    replayed_realization: bool,
    replayed_efference_copy: bool,
    mismatched_efference_realization: bool,
    soma_state_identity: Option<Digest>,
    soma_state_revision: Option<u64>,
    soma_capsule_digest_override: Option<Digest>,
    organism_state_digest_override: Option<Digest>,
    soma_ingress_state_digest: Option<Digest>,
    soma_ingress_revision: Option<u64>,
    soma_ingress_identity: Option<Digest>,
    epistemic_turn_id: Option<Id128>,
    epistemic_state_digest: Option<Digest>,
    epistemic_revision: Option<u64>,
    epistemic_other_identity: bool,
    soma_axis: SomaSubjectiveAxisV1,
}

impl Default for Variant {
    fn default() -> Self {
        Self {
            action_turn_binding: None,
            action_revision: None,
            action_state_digest: None,
            action_identity_digest: None,
            action_content_digest_override: None,
            replayed_realization: false,
            replayed_efference_copy: false,
            mismatched_efference_realization: false,
            soma_state_identity: None,
            soma_state_revision: None,
            soma_capsule_digest_override: None,
            organism_state_digest_override: None,
            soma_ingress_state_digest: None,
            soma_ingress_revision: None,
            soma_ingress_identity: None,
            epistemic_turn_id: None,
            epistemic_state_digest: None,
            epistemic_revision: None,
            epistemic_other_identity: false,
            soma_axis: SomaSubjectiveAxisV1::Energy,
        }
    }
}

fn input(variant: Variant) -> ProjectionInput {
    let identity = identity_constitution(41);
    let identity_digest = *identity.constitution_digest();
    let soma = soma_state(
        variant.soma_state_identity.unwrap_or(identity_digest),
        variant.soma_state_revision.unwrap_or(BASE_REVISION),
    );
    let organism_state_digest = variant
        .organism_state_digest_override
        .unwrap_or(*soma.source_state_digest().expect("fixture source binding"));
    let action = action_contract(
        variant.action_identity_digest.unwrap_or(identity_digest),
        variant.action_turn_binding.unwrap_or(TURN_BINDING),
        variant.action_revision.unwrap_or(BASE_REVISION),
        variant.action_state_digest.unwrap_or(organism_state_digest),
    );
    let realization_source = if variant.replayed_realization {
        action_contract(
            identity_digest,
            TURN_BINDING,
            BASE_REVISION + 1,
            organism_state_digest,
        )
    } else {
        action.clone()
    };
    let realization = ActionRealizationV1::for_contract(
        &realization_source,
        vec![],
        vec![],
        vec![],
        unit(700_000),
    )
    .expect("typed realization");
    let efference_contract = if variant.replayed_efference_copy || variant.replayed_realization {
        action_contract(
            identity_digest,
            TURN_BINDING,
            BASE_REVISION + 1,
            organism_state_digest,
        )
    } else {
        action.clone()
    };
    let efference_realization = if variant.mismatched_efference_realization {
        ActionRealizationV1::for_contract(&action, vec![], vec![], vec![], unit(600_000))
            .expect("mismatched typed realization")
    } else if variant.replayed_efference_copy || variant.replayed_realization {
        ActionRealizationV1::for_contract(
            &efference_contract,
            vec![],
            vec![],
            vec![],
            unit(700_000),
        )
        .expect("replayed efference realization")
    } else {
        realization.clone()
    };
    let efference_copy = EfferenceCopySourceV1::default()
        .form(
            &efference_contract,
            &efference_realization,
            ExpectedDispositionV1::Speech,
            ObservedDispositionV1::Speech,
            vec![],
        )
        .expect("typed efference copy");
    let soma_ingress = SomaClassificationIngressV1::new(
        variant
            .soma_ingress_state_digest
            .unwrap_or(*soma.state_digest()),
        variant.soma_ingress_revision.unwrap_or(soma.revision()),
        variant
            .soma_ingress_identity
            .unwrap_or(*soma.identity_constitution_digest()),
        vec![soma_classification(variant.soma_axis)],
        8,
    )
    .expect("soma classification ingress");
    let epistemic_identity = if variant.epistemic_other_identity {
        identity_constitution(91)
    } else {
        identity.clone()
    };
    let epistemic = epistemic_projection(
        epistemic_identity,
        variant.epistemic_turn_id.unwrap_or(TURN_ID),
        variant
            .epistemic_state_digest
            .unwrap_or(organism_state_digest),
        variant.epistemic_revision.unwrap_or(BASE_REVISION),
    );

    let organism = OrganismSnapshotRefV1::new(
        text("organism:snapshot:9", 128),
        organism_state_digest,
        TURN_BINDING,
        TURN_ID,
        TurnV1::new(
            text("turn:41", 128),
            text("incarnation:9", 128),
            text("scope:current", 128),
        )
        .expect("turn"),
    )
    .expect("organism reference");
    let cognitive = CognitiveKvViewV1::new(
        text("continuum:view:9", 128),
        digest(21),
        legacy_subjective_present(),
        EpistemicsV1::new(fields("claim", "verified", 22)).expect("epistemics"),
        PraxisV1::new(fields("objective", "answer_current_turn", 23)).expect("praxis"),
    )
    .expect("cognitive view");
    let anchors = ExactTurnAnchorsV1::new(list(
        vec![ExactAnchorV1::new(
            ExactAnchorKindV1::ActiveSafetyRequirement,
            text("anchor:safety:1", 128),
            text("do_not_disclose_private_control", 512),
            digest(31),
        )
        .expect("anchor")],
        64,
    ))
    .expect("anchors");
    let relation = RelationScopeV1::new(
        text("scope:current", 128),
        RelationV1::new(fields("boundary", "current_scope_only", 52)).expect("relation"),
    )
    .expect("relation scope");
    let affordances = AffordanceCatalogV1::new(
        text("scope:current", 128),
        list(
            vec![AffordanceV1::new(
                text("answer", 64),
                text("emit_bounded_answer", 256),
                digest(81),
                digest(82),
            )
            .expect("affordance")],
            64,
        ),
    )
    .expect("affordances");
    let provider = ProviderProfileV1::new(text("provider:fixture", 128), MAX_PROJECTION_TOKENS)
        .expect("provider");

    ProjectionInput::new(
        capsule(
            ProjectionSourceKindV1::OrganismSnapshot,
            1,
            BASE_REVISION,
            organism_state_digest,
            organism,
        ),
        capsule(
            ProjectionSourceKindV1::CognitiveKvView,
            2,
            BASE_REVISION,
            digest(20),
            cognitive,
        ),
        capsule(
            ProjectionSourceKindV1::ExactTurnAnchors,
            3,
            BASE_REVISION,
            digest(30),
            anchors,
        ),
        capsule(
            ProjectionSourceKindV1::IdentityConstitution,
            4,
            BASE_REVISION,
            identity_digest,
            identity,
        ),
        capsule(
            ProjectionSourceKindV1::RelationScope,
            5,
            BASE_REVISION,
            digest(50),
            relation,
        ),
        capsule(
            ProjectionSourceKindV1::ActionContract,
            6,
            variant.action_revision.unwrap_or(BASE_REVISION),
            variant
                .action_content_digest_override
                .unwrap_or(*action.contract_digest()),
            action,
        ),
        capsule(
            ProjectionSourceKindV1::AffordanceCatalog,
            8,
            BASE_REVISION,
            digest(80),
            affordances,
        ),
        capsule(
            ProjectionSourceKindV1::ProviderProfile,
            9,
            BASE_REVISION,
            digest(90),
            provider,
        ),
        capsule(
            ProjectionSourceKindV1::SomaState,
            10,
            variant.soma_state_revision.unwrap_or(BASE_REVISION),
            variant
                .soma_capsule_digest_override
                .unwrap_or(*soma.state_digest()),
            soma,
        ),
        soma_ingress,
        epistemic,
        realization,
        efference_copy,
    )
}

fn preconditions() -> ProjectionPreconditionsV1 {
    ProjectionPreconditionsV1::new(1_000, 0, 0, 7, 7, 0, MAX_PROJECTION_TOKENS)
}

#[test]
fn assembles_direct_soma_epistemic_and_efference_sources() {
    let input = input(Variant::default());
    let (envelope, certificate) =
        compile_projection_v1(&input, &preconditions()).expect("valid direct assembly");

    assert_eq!(envelope.schema(), COGNITIVE_ENVELOPE_SCHEMA_V1);
    assert_eq!(envelope.subjective_present()[0].axis(), "energy");
    let encoded = serde_json::to_string(&envelope).expect("bounded envelope serialization");
    assert!(!encoded.contains("irritation"));
    assert!(!encoded.contains("epistemic_conflict"));
    assert_eq!(
        envelope.epistemics().projection_digest(),
        input.epistemic_projection().projection_digest()
    );
    assert_eq!(
        certificate.soma_state_digest(),
        input.soma_state().value().state_digest()
    );
    assert_eq!(
        certificate.efference_copy_digest(),
        input.efference_copy().copy_digest()
    );
    assert_eq!(envelope.projection_certificate(), &certificate);
    assert_ne!(envelope.envelope_digest(), &[0; 32]);
}

#[test]
fn rejects_soma_capsule_and_ingress_state_revision_and_identity_mismatches() {
    for (variant, expected) in [
        (
            Variant {
                soma_capsule_digest_override: Some(digest(99)),
                ..Variant::default()
            },
            ProjectionErrorV1::SomaStateDigestMismatch,
        ),
        (
            Variant {
                soma_ingress_state_digest: Some(digest(98)),
                ..Variant::default()
            },
            ProjectionErrorV1::SomaSubjectiveStateMismatch,
        ),
        (
            Variant {
                soma_ingress_revision: Some(BASE_REVISION + 1),
                ..Variant::default()
            },
            ProjectionErrorV1::SomaSubjectiveRevisionMismatch,
        ),
        (
            Variant {
                soma_ingress_identity: Some(digest(97)),
                ..Variant::default()
            },
            ProjectionErrorV1::SomaSubjectiveIdentityMismatch,
        ),
    ] {
        assert_eq!(
            compile_projection_v1(&input(variant), &preconditions()),
            Err(expected)
        );
    }
}

#[test]
fn rejects_cross_source_soma_and_epistemic_state_revision_identity_and_turn() {
    for (variant, expected) in [
        (
            Variant {
                organism_state_digest_override: Some(digest(96)),
                ..Variant::default()
            },
            ProjectionErrorV1::SomaSourceStateDigestMismatch,
        ),
        (
            Variant {
                soma_state_revision: Some(BASE_REVISION + 1),
                ..Variant::default()
            },
            ProjectionErrorV1::SomaSourceRevisionMismatch,
        ),
        (
            Variant {
                soma_state_identity: Some(digest(95)),
                ..Variant::default()
            },
            ProjectionErrorV1::SomaIdentityConstitutionDigestMismatch,
        ),
        (
            Variant {
                epistemic_turn_id: Some([94; 16]),
                ..Variant::default()
            },
            ProjectionErrorV1::EpistemicTurnBindingMismatch,
        ),
        (
            Variant {
                epistemic_state_digest: Some(digest(93)),
                ..Variant::default()
            },
            ProjectionErrorV1::EpistemicSourceStateDigestMismatch,
        ),
        (
            Variant {
                epistemic_revision: Some(BASE_REVISION + 1),
                ..Variant::default()
            },
            ProjectionErrorV1::EpistemicRevisionMismatch,
        ),
        (
            Variant {
                epistemic_other_identity: true,
                ..Variant::default()
            },
            ProjectionErrorV1::EpistemicIdentityConstitutionDigestMismatch,
        ),
    ] {
        assert_eq!(
            compile_projection_v1(&input(variant), &preconditions()),
            Err(expected)
        );
    }
}

#[test]
fn rejects_action_and_direct_efference_replay_bindings() {
    assert_eq!(
        compile_projection_v1(
            &input(Variant {
                action_content_digest_override: Some(digest(99)),
                ..Variant::default()
            }),
            &preconditions(),
        ),
        Err(ProjectionErrorV1::ActionContractDigestMismatch)
    );
    assert_eq!(
        compile_projection_v1(
            &input(Variant {
                replayed_realization: true,
                ..Variant::default()
            }),
            &preconditions(),
        ),
        Err(ProjectionErrorV1::ActionRealizationContractDigestMismatch)
    );
    assert_eq!(
        compile_projection_v1(
            &input(Variant {
                replayed_efference_copy: true,
                ..Variant::default()
            }),
            &preconditions(),
        ),
        Err(ProjectionErrorV1::EfferenceCopyContractDigestMismatch)
    );
    assert_eq!(
        compile_projection_v1(
            &input(Variant {
                mismatched_efference_realization: true,
                ..Variant::default()
            }),
            &preconditions(),
        ),
        Err(ProjectionErrorV1::EfferenceCopyRealizationDigestMismatch)
    );
}

#[test]
fn direct_sources_change_envelope_identity_and_do_not_admit_raw_text() {
    let baseline = compile_projection_v1(&input(Variant::default()), &preconditions())
        .expect("baseline")
        .0;
    let changed = compile_projection_v1(
        &input(Variant {
            soma_axis: SomaSubjectiveAxisV1::Fatigue,
            ..Variant::default()
        }),
        &preconditions(),
    )
    .expect("changed direct source")
    .0;
    assert_ne!(baseline.envelope_digest(), changed.envelope_digest());
    assert!(BoundedTextV1::new("raw user conversation".to_owned(), 128).is_err());
    assert!(CanonicalTokenV1::new("raw neural array".to_owned(), 128).is_err());
}
