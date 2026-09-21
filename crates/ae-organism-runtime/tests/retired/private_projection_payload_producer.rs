// The established runtime fixture supplies only fully-bound typed source values.
// Reusing it keeps this test focused on the producer transaction boundary.
#[allow(unused_macros)]
macro_rules! private_projection_payload_producer_test_contents {
    () => {
private_projection_runtime_test_contents!();

use ae_morph::{
    MorphAffordanceCatalogV1, MorphAvailabilityV1, MorphClassificationVocabularyInputV1,
    MorphClassificationVocabularyV1, MorphConfirmationRequirementV1, MorphEffectorInputV1,
    MorphEffectorV1, MorphStateBindingV1, MorphVocabularyBoundsV1, MORPH_AFFORDANCE_MAX_ITEMS_V1,
};
use crate::r7::{
    discard_private_projection_transfer_v1,
    NativeProjectionPayloadIngressV1, NativeProjectionPayloadProducerErrorV1,
    NativeProjectionPayloadProducerInputV1, NativeProjectionPayloadProducerV1,
    OrganismRuntimeErrorV1, PrivateProjectionPayloadWireErrorV1,
    PrivateProjectionTransferReceiptV1,
};

fn morph_catalog(
    revision: u64,
    identity_digest: Digest,
    state_digest: Digest,
) -> MorphAffordanceCatalogV1 {
    let binding = MorphStateBindingV1::new(revision, identity_digest, state_digest)
        .expect("typed morph binding");
    let vocabulary = MorphClassificationVocabularyV1::new(
        MorphClassificationVocabularyInputV1 {
            capability_classes: vec!["capability_a".into()],
            safety_classes: vec!["safety_a".into()],
            reliability_classes: vec!["reliability_a".into()],
            side_effect_classes: vec!["side_effect_a".into()],
            latency_classes: vec!["latency_a".into()],
            cost_classes: vec!["cost_a".into()],
            reversibility_classes: vec!["reversibility_a".into()],
        },
        MorphVocabularyBoundsV1::new(4, 32).expect("typed morph vocabulary bounds"),
    )
    .expect("typed morph vocabulary");
    let effector = MorphEffectorV1::new(
        MorphEffectorInputV1 {
            effector_id: "effector.alpha".into(),
            capability_class: "capability_a".into(),
            availability: MorphAvailabilityV1::Available,
            safety_class: "safety_a".into(),
            reliability_class: "reliability_a".into(),
            side_effect_class: "side_effect_a".into(),
            confirmation_requirement: MorphConfirmationRequirementV1::Required,
            latency_class: "latency_a".into(),
            cost_class: "cost_a".into(),
            reversibility_class: "reversibility_a".into(),
        },
        32,
        &vocabulary,
        &binding,
    )
    .expect("typed morph effector");
    MorphAffordanceCatalogV1::new(
        "morph.catalog.v1".into(),
        32,
        binding,
        vocabulary,
        vec![effector],
        MORPH_AFFORDANCE_MAX_ITEMS_V1,
    )
    .expect("typed morph catalog")
}

fn producer_input(
    identity: IdentityConstitutionV1,
    revision: u64,
    anchors_revision: u64,
    kv_snapshot_digest: Digest,
    morph_identity_digest: Digest,
    morph_state_digest: Digest,
    morph_revision: u64,
) -> NativeProjectionPayloadIngressV1 {
    NativeProjectionPayloadIngressV1::ready(NativeProjectionPayloadProducerInputV1::new(
        update(identity, revision, anchors_revision),
        kv_snapshot_digest,
        morph_catalog(morph_revision, morph_identity_digest, morph_state_digest),
    ))
}

fn update_with_epistemic_state_mismatch(
    identity: IdentityConstitutionV1,
    revision: u64,
) -> NativeProjectionUpdateV1 {
    let identity_digest = *identity.constitution_digest();
    let soma = soma_state(identity_digest, revision);
    let state_digest = *soma
        .source_state_digest()
        .expect("fixture SOMA binds semantic source state");
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
        state_digest,
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
    let epistemic = epistemic_projection(identity, digest(87), revision);

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
                revision,
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
            state_digest,
            soma,
        ),
        soma_ingress,
        epistemic,
        realization,
        efference,
        ProjectionPreconditionsV1::new(1_000, 0, 0, 7, 7, 0, 512),
    )
}

#[test]
fn producer_seals_a_canonical_one_shot_wire_only_after_a_fully_bound_update() {
    let identity = identity(41);
    let identity_digest = *identity.constitution_digest();
    let state_digest = semantic_state_digest(9);
    let mut producer = NativeProjectionPayloadProducerV1::new(identity_capsule(identity.clone()))
        .expect("immutable typed identity");
    let mut equivalent = NativeProjectionPayloadProducerV1::new(identity_capsule(identity.clone()))
        .expect("independent immutable typed identity");

    let mut wire = producer
        .produce(producer_input(
            identity.clone(),
            9,
            9,
            digest(21),
            identity_digest,
            state_digest,
            9,
        ))
        .expect("fully bound native source ingress");
    let equivalent_wire = equivalent
        .produce(producer_input(
            identity.clone(),
            9,
            9,
            digest(21),
            identity_digest,
            state_digest,
            9,
        ))
        .expect("equivalent fully bound native source ingress");
    assert_eq!(producer.current_revision(), Some(9));
    assert_eq!(wire.wire_digest(), equivalent_wire.wire_digest());
    let transfer = wire
        .begin_transfer_once_v1()
        .expect("one crate-private native transfer");
    assert!(matches!(
        wire.begin_transfer_once_v1(),
        Err(PrivateProjectionPayloadWireErrorV1::AlreadyConsumed)
    ));
    assert_eq!(
        discard_private_projection_transfer_v1(transfer),
        PrivateProjectionTransferReceiptV1::Discarded
    );
    assert_eq!(
        equivalent_wire.cancel_v1(),
        PrivateProjectionTransferReceiptV1::Cancelled
    );
    assert!(matches!(
        producer.produce(producer_input(
            identity.clone(),
            9,
            9,
            digest(21),
            identity_digest,
            state_digest,
            9,
        )),
        Err(NativeProjectionPayloadProducerErrorV1::Runtime(
            OrganismRuntimeErrorV1::StaleOrReplayedRevision {
                current: 9,
                incoming: 9
            }
        ))
    ));
    assert_eq!(producer.current_revision(), Some(9));
}

#[test]
fn unavailable_or_mismatched_typed_ingress_fails_without_consuming_the_revision() {
    let identity = identity(41);
    let identity_digest = *identity.constitution_digest();
    let state_digest = semantic_state_digest(9);
    let mut producer = NativeProjectionPayloadProducerV1::new(identity_capsule(identity.clone()))
        .expect("immutable typed identity");

    assert!(matches!(
        producer.produce(NativeProjectionPayloadIngressV1::unavailable()),
        Err(NativeProjectionPayloadProducerErrorV1::InputUnavailable)
    ));
    assert_eq!(producer.current_revision(), None);

    assert!(matches!(
        producer.produce(producer_input(
            identity.clone(),
            9,
            8,
            digest(21),
            identity_digest,
            state_digest,
            9,
        )),
        Err(NativeProjectionPayloadProducerErrorV1::Runtime(
            OrganismRuntimeErrorV1::RevisionBindingMismatch {
                field: "exact_turn_anchors",
                ..
            }
        ))
    ));
    assert_eq!(producer.current_revision(), None);

    assert!(matches!(
        producer.produce(producer_input(
            identity.clone(),
            9,
            9,
            digest(99),
            identity_digest,
            state_digest,
            9,
        )),
        Err(NativeProjectionPayloadProducerErrorV1::KvSnapshotDigestMismatch)
    ));
    assert_eq!(producer.current_revision(), None);

    for (morph_identity, morph_state, morph_revision, field) in [
        (digest(91), state_digest, 9, "identity_constitution_digest"),
        (identity_digest, digest(92), 9, "source_state_digest"),
        (identity_digest, state_digest, 8, "revision"),
    ] {
        assert!(matches!(
            producer.produce(producer_input(
                identity.clone(),
                9,
                9,
                digest(21),
                morph_identity,
                morph_state,
                morph_revision,
            )),
            Err(NativeProjectionPayloadProducerErrorV1::MorphBindingMismatch { field: actual })
                if actual == field
        ));
        assert_eq!(producer.current_revision(), None);
    }
}

#[test]
fn legacy_success_advances_one_watermark_and_replay_is_refused() {
    let identity = identity(41);
    let identity_digest = *identity.constitution_digest();
    let state_digest = semantic_state_digest(9);
    let mut producer = NativeProjectionPayloadProducerV1::new(identity_capsule(identity.clone()))
        .expect("immutable typed identity");
    producer
        .produce(producer_input(
            identity.clone(),
            9,
            9,
            digest(21),
            identity_digest,
            state_digest,
            9,
        ))
        .expect("canonical legacy issue advances its private watermark once");
    assert_eq!(producer.current_revision(), Some(9));
    assert!(matches!(
        producer.produce(producer_input(
            identity,
            9,
            9,
            digest(21),
            identity_digest,
            state_digest,
            9,
        )),
        Err(NativeProjectionPayloadProducerErrorV1::Runtime(
            OrganismRuntimeErrorV1::StaleOrReplayedRevision { .. }
        ))
    ));
}

#[test]
fn compilation_failure_does_not_advance_the_transaction_revision() {
    let identity = identity(41);
    let identity_digest = *identity.constitution_digest();
    let state_digest = semantic_state_digest(9);
    let mut producer = NativeProjectionPayloadProducerV1::new(identity_capsule(identity.clone()))
        .expect("immutable typed identity");

    assert!(matches!(
        producer.produce(NativeProjectionPayloadIngressV1::ready(
            NativeProjectionPayloadProducerInputV1::new(
                update_with_epistemic_state_mismatch(identity.clone(), 9),
                digest(21),
                morph_catalog(9, identity_digest, state_digest),
            )
        )),
        Err(NativeProjectionPayloadProducerErrorV1::Runtime(_))
    ));
    assert_eq!(producer.current_revision(), None);

    producer
        .produce(producer_input(
            identity,
            9,
            9,
            digest(21),
            identity_digest,
            state_digest,
            9,
        ))
        .expect("compile failure did not consume revision");
}
    };
}
