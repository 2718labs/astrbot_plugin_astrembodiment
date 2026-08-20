#[allow(unused_macros)]
macro_rules! private_projection_runtime_test_contents {
    () => {
use ae_action_contract::{
    ActionContractV1, ActionDispositionV1, ActionRealizationV1, ActionRequirementsV1,
    CanonicalTokenSetV1, CanonicalTokenV1, UnitIntervalV1,
};
use ae_cognitive_envelope::{
    AffordanceCatalogV1, AffordanceV1, BoundedListV1, BoundedTextV1, CapsuleFieldV1,
    CognitiveKvViewV1, EpistemicsV1, ExactAnchorKindV1, ExactAnchorV1, ExactTurnAnchorsV1,
    OrganismSnapshotRefV1, PraxisV1, ProjectionPreconditionsV1, ProjectionSourceKindV1,
    ProviderProfileV1, RelationScopeV1, RelationV1, SourceCapsuleV1, SourceProvenanceV1, TurnV1,
};
use ae_contracts::r7::{
    wire, CausalRef, Digest, EvidenceVector, Id128, ScopeRef, SemanticEstimate, VerdictKind,
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
use crate::r7::{BoundedProjectionReferencesV1, NativeProjectionUpdateV1};
use ae_soma::{
    BoundedSomaSignalV1, CallerProvidedClassificationV1, SomaClassificationIngressV1,
    SomaFieldSetV1, SomaStateV1, SomaSubjectiveAxisV1,
};
use ae_subjective_present::{
    ConfidenceV1, DisclosureV1, SubjectiveBandV1, SubjectivePresentInputV1,
    SubjectivePresentProjectionV1, SubjectivePresentV1, SubjectiveTrendV1,
};

// This fixture is included both by focused organism tests and by an unchanged
// PyO3 regression fixture. Each consuming target needs a different subset.
#[allow(dead_code)]
const ACTION_ID: Id128 = [51; 16];
#[allow(dead_code)]
const TURN_ID: Id128 = [12; 16];
#[allow(dead_code)]
const TURN_BINDING: Digest = [12; 32];

#[allow(dead_code)]
fn digest(seed: u8) -> Digest {
    [seed; 32]
}

#[allow(dead_code)]
fn text(value: &str, max_chars: u32) -> BoundedTextV1 {
    BoundedTextV1::new(value.to_owned(), max_chars).expect("bounded fixture token")
}

#[allow(dead_code)]
fn list<T>(items: Vec<T>, max_items: u16) -> BoundedListV1<T> {
    BoundedListV1::new(items, max_items).expect("bounded fixture list")
}

#[allow(dead_code)]
fn fields(name: &str, value: &str, seed: u8) -> BoundedListV1<CapsuleFieldV1> {
    list(
        vec![
            CapsuleFieldV1::new(text(name, 64), text(value, 256), digest(seed))
                .expect("source-bound field"),
        ],
        8,
    )
}

#[allow(dead_code)]
pub(crate) fn identity(seed: u8) -> IdentityConstitutionV1 {
    let bounds = IdentityBoundsV1::new(8, 64).expect("identity bounds");
    let seed_code = SeedCodeV1::new(
        "persona.genesis.seed.v1".to_owned(),
        64,
        digest(seed),
        digest(seed + 1),
    )
    .expect("seed code");
    let incarnation =
        IncarnationRefV1::derive(&seed_code, digest(seed + 2), 1).expect("incarnation");
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

#[allow(dead_code)]
fn capsule<T>(
    kind: ProjectionSourceKindV1,
    seed: u8,
    revision: u64,
    content_digest: Digest,
    value: T,
) -> SourceCapsuleV1<T> {
    SourceCapsuleV1::new(
        SourceProvenanceV1::new(
            kind,
            text(&format!("source:{seed}"), 128),
            revision,
            digest(seed + 100),
        )
        .expect("provenance"),
        content_digest,
        value,
    )
    .expect("capsule")
}

#[allow(dead_code)]
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

#[allow(dead_code)]
fn soma_field(signal_id: &str, value: f64) -> SomaFieldSetV1 {
    let signal = BoundedSomaSignalV1::new(signal_id.to_owned(), 64, value, 0.0, 1.0)
        .expect("bounded soma signal");
    SomaFieldSetV1::new(vec![signal], 8).expect("bounded soma field")
}

#[allow(dead_code)]
fn semantic_state_digest(revision: u64) -> Digest {
    wire::domain_hash(
        b"astr-embodiment/r7/test/committed-semantic-source-state-v1",
        &[&revision.to_be_bytes()],
    )
}

#[allow(dead_code)]
fn soma_state(identity_digest: Digest, revision: u64) -> SomaStateV1 {
    SomaStateV1::new_bound_to_source_state(
        format!("soma:snapshot:{revision}"),
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

#[allow(dead_code)]
fn token(value: &str) -> CanonicalTokenV1 {
    CanonicalTokenV1::new(value.to_owned(), 64).expect("action token")
}

#[allow(dead_code)]
fn token_set(values: &[&str]) -> CanonicalTokenSetV1 {
    CanonicalTokenSetV1::new(values.iter().map(|value| token(value)).collect(), 8)
        .expect("token set")
}

#[allow(dead_code)]
fn unit(value: u32) -> UnitIntervalV1 {
    UnitIntervalV1::from_parts_per_million(value).expect("unit interval")
}

#[allow(dead_code)]
fn action_contract(
    identity_digest: Digest,
    revision: u64,
    state_digest: Digest,
) -> ActionContractV1 {
    ActionContractV1::from_evaluation(
        ACTION_ID,
        TURN_BINDING,
        revision,
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

#[allow(dead_code)]
fn epistemic_projection(
    identity: IdentityConstitutionV1,
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
        TURN_ID,
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
        vec![EpistemicEvidenceGapV1::ConflictingEvidence],
        VerifierNeedV1::Required,
        Fixed::from_raw(420_000),
        true,
        true,
    )
    .expect("caller-provided epistemic classification");
    compile_epistemic_projection_v1(&EpistemicProjectionInputV1::new(
        binding,
        CausalRef {
            turn_id: TURN_ID,
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

#[allow(dead_code)]
pub(crate) fn identity_capsule(
    identity: IdentityConstitutionV1,
) -> SourceCapsuleV1<IdentityConstitutionV1> {
    let identity_digest = *identity.constitution_digest();
    capsule(
        ProjectionSourceKindV1::IdentityConstitution,
        4,
        1,
        identity_digest,
        identity,
    )
}

#[allow(dead_code)]
fn update(
    identity: IdentityConstitutionV1,
    revision: u64,
    anchors_revision: u64,
) -> NativeProjectionUpdateV1 {
    let identity_digest = *identity.constitution_digest();
    let soma = soma_state(identity_digest, revision);
    let state_digest = *soma
        .source_state_digest()
        .expect("fixture SOMA binds a distinct semantic source state");
    let soma_state_digest = *soma.state_digest();
    let organism = OrganismSnapshotRefV1::new(
        text(&format!("organism:snapshot:{revision}"), 128),
        state_digest,
        TURN_BINDING,
        TURN_ID,
        TurnV1::new(
            text(&format!("turn:{revision}"), 128),
            text("incarnation:1", 128),
            text("scope:current", 128),
        )
        .expect("turn"),
    )
    .expect("organism reference");
    let cognitive = CognitiveKvViewV1::new(
        text(&format!("continuum:view:{revision}"), 128),
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
    let provider =
        ProviderProfileV1::new(text("provider:fixture", 128), 3_200).expect("provider profile");
    let action = action_contract(identity_digest, revision, state_digest);
    let realization =
        ActionRealizationV1::for_contract(&action, vec![], vec![], vec![], unit(700_000))
            .expect("typed realization");
    let efference = EfferenceCopySourceV1::default()
        .form(
            &action,
            &realization,
            ExpectedDispositionV1::Speech,
            ObservedDispositionV1::Speech,
            vec![],
        )
        .expect("typed efference copy");
    let soma_ingress = SomaClassificationIngressV1::new(
        soma_state_digest,
        revision,
        identity_digest,
        vec![CallerProvidedClassificationV1::new(
            SomaSubjectiveAxisV1::Energy,
            SubjectiveBandV1::Moderate,
            SubjectiveTrendV1::Stable,
            "prefer_bounded_effort".to_owned(),
            DisclosureV1::BehavioralOnly,
            ConfidenceV1::High,
            Some(format!("soma:snapshot:{revision}")),
        )
        .expect("caller-provided soma classification")],
        8,
    )
    .expect("soma classification ingress");
    let epistemic = epistemic_projection(identity, state_digest, revision);

    NativeProjectionUpdateV1::new(
        revision,
        BoundedProjectionReferencesV1::new(
            capsule(
                ProjectionSourceKindV1::OrganismSnapshot,
                1,
                revision,
                state_digest,
                organism,
            ),
            capsule(
                ProjectionSourceKindV1::CognitiveKvView,
                2,
                revision,
                digest(20),
                cognitive,
            ),
            capsule(
                ProjectionSourceKindV1::ExactTurnAnchors,
                3,
                anchors_revision,
                digest(30),
                anchors,
            ),
            capsule(
                ProjectionSourceKindV1::RelationScope,
                5,
                revision,
                digest(50),
                relation,
            ),
            capsule(
                ProjectionSourceKindV1::AffordanceCatalog,
                8,
                revision,
                digest(80),
                affordances,
            ),
            capsule(
                ProjectionSourceKindV1::ProviderProfile,
                9,
                revision,
                digest(90),
                provider,
            ),
        ),
        capsule(
            ProjectionSourceKindV1::ActionContract,
            6,
            revision,
            *action.contract_digest(),
            action,
        ),
        capsule(
            ProjectionSourceKindV1::SomaState,
            10,
            revision,
            soma_state_digest,
            soma,
        ),
        soma_ingress,
        epistemic,
        realization,
        efference,
        ProjectionPreconditionsV1::new(1_000, 0, 0, 7, 7, 0, 512),
    )
}
    };
}
