use ae_contracts::r7::Digest;
use ae_morph::{
    produce_native_morph_catalog_v1, MorphAffordanceCatalogV1, MorphAvailabilityV1,
    MorphClassificationVocabularyInputV1, MorphClassificationVocabularyV1,
    MorphConfirmationRequirementV1, MorphEffectorInputV1, MorphEffectorV1, MorphErrorV1,
    MorphStateBindingV1, MorphVocabularyBoundsV1, NativeMorphEffectorStateV1,
    MORPH_AFFORDANCE_MAX_ITEMS_V1,
};

#[test]
fn native_producer_uses_fixed_vocabulary_and_committed_binding() {
    let source =
        NativeMorphEffectorStateV1::new(17, digest(7), digest(8), vec!["effector.alpha".into()])
            .expect("native effector state");
    let first =
        produce_native_morph_catalog_v1(source.clone(), "morph.catalog.native.v1".into(), 32)
            .expect("native catalog");
    let second = produce_native_morph_catalog_v1(source, "morph.catalog.native.v1".into(), 32)
        .expect("deterministic native catalog");
    assert_eq!(first, second);
    assert_eq!(first.revision(), 17);
    assert_eq!(first.identity_constitution_digest(), &digest(7));
    assert_eq!(first.source_state_digest(), &digest(8));
    assert_eq!(
        first.classification_vocabulary().capability_classes(),
        &["capability_a".to_owned()]
    );
}

fn digest(tag: u8) -> Digest {
    [tag; 32]
}

fn bounds() -> MorphVocabularyBoundsV1 {
    MorphVocabularyBoundsV1::new(4, 32).expect("valid caller bounds")
}

fn vocabulary_input() -> MorphClassificationVocabularyInputV1 {
    MorphClassificationVocabularyInputV1 {
        capability_classes: vec!["capability_a".into(), "capability_b".into()],
        safety_classes: vec!["safety_a".into(), "safety_b".into()],
        reliability_classes: vec!["reliability_a".into(), "reliability_b".into()],
        side_effect_classes: vec!["side_effect_a".into(), "side_effect_b".into()],
        latency_classes: vec!["latency_a".into(), "latency_b".into()],
        cost_classes: vec!["cost_a".into(), "cost_b".into()],
        reversibility_classes: vec!["reversibility_a".into(), "reversibility_b".into()],
    }
}

fn vocabulary() -> MorphClassificationVocabularyV1 {
    MorphClassificationVocabularyV1::new(vocabulary_input(), bounds())
        .expect("valid caller-declared vocabulary")
}

fn binding(revision: u64, identity_tag: u8, state_tag: u8) -> MorphStateBindingV1 {
    MorphStateBindingV1::new(revision, digest(identity_tag), digest(state_tag))
        .expect("valid state binding")
}

fn effector_input(
    effector_id: &str,
    availability: MorphAvailabilityV1,
    safety_class: &str,
) -> MorphEffectorInputV1 {
    MorphEffectorInputV1 {
        effector_id: effector_id.into(),
        capability_class: "capability_a".into(),
        availability,
        safety_class: safety_class.into(),
        reliability_class: "reliability_a".into(),
        side_effect_class: "side_effect_a".into(),
        confirmation_requirement: MorphConfirmationRequirementV1::Required,
        latency_class: "latency_a".into(),
        cost_class: "cost_a".into(),
        reversibility_class: "reversibility_a".into(),
    }
}

fn effector(
    effector_id: &str,
    availability: MorphAvailabilityV1,
    safety_class: &str,
    vocabulary: &MorphClassificationVocabularyV1,
    binding: &MorphStateBindingV1,
) -> MorphEffectorV1 {
    MorphEffectorV1::new(
        effector_input(effector_id, availability, safety_class),
        32,
        vocabulary,
        binding,
    )
    .expect("valid classified effector")
}

#[test]
fn catalog_is_deterministic_content_addressed_and_filters_available_effectors() {
    let vocabulary = vocabulary();
    let binding = binding(7, 1, 2);
    let entries = vec![
        effector(
            "effector.alpha",
            MorphAvailabilityV1::Available,
            "safety_a",
            &vocabulary,
            &binding,
        ),
        effector(
            "effector.beta",
            MorphAvailabilityV1::Unavailable,
            "safety_a",
            &vocabulary,
            &binding,
        ),
    ];

    let first = MorphAffordanceCatalogV1::new(
        "morph.catalog.v1".into(),
        32,
        binding.clone(),
        vocabulary.clone(),
        entries.clone(),
        MORPH_AFFORDANCE_MAX_ITEMS_V1,
    )
    .expect("valid catalog");
    let second = MorphAffordanceCatalogV1::new(
        "morph.catalog.v1".into(),
        32,
        binding.clone(),
        vocabulary.clone(),
        entries,
        MORPH_AFFORDANCE_MAX_ITEMS_V1,
    )
    .expect("same input is valid");

    assert_eq!(first.catalog_digest(), second.catalog_digest());
    assert_eq!(first.revision(), 7);
    assert_eq!(first.identity_constitution_digest(), &digest(1));
    assert_eq!(first.source_state_digest(), &digest(2));
    assert_eq!(
        first
            .available_effectors()
            .map(MorphEffectorV1::effector_id)
            .collect::<Vec<_>>(),
        vec!["effector.alpha"]
    );

    let changed = MorphAffordanceCatalogV1::new(
        "morph.catalog.v1".into(),
        32,
        binding.clone(),
        vocabulary.clone(),
        vec![
            effector(
                "effector.alpha",
                MorphAvailabilityV1::Available,
                "safety_a",
                &vocabulary,
                &binding,
            ),
            effector(
                "effector.beta",
                MorphAvailabilityV1::Unavailable,
                "safety_b",
                &vocabulary,
                &binding,
            ),
        ],
        MORPH_AFFORDANCE_MAX_ITEMS_V1,
    )
    .expect("changed constituent remains valid");
    assert_ne!(first.catalog_digest(), changed.catalog_digest());
}

#[test]
fn rejects_unknown_undeclared_and_raw_classifications() {
    assert_eq!(
        MorphAvailabilityV1::parse("maybe"),
        Err(MorphErrorV1::UnknownAvailability)
    );
    assert_eq!(
        MorphConfirmationRequirementV1::parse("sometimes"),
        Err(MorphErrorV1::UnknownConfirmationRequirement)
    );
    assert_eq!(
        MorphStateBindingV1::new(7, [0; 32], digest(2)),
        Err(MorphErrorV1::ZeroDigest {
            field: "identity_constitution_digest"
        })
    );

    let vocabulary = vocabulary();
    let binding = binding(7, 1, 2);
    let mut undeclared =
        effector_input("effector.alpha", MorphAvailabilityV1::Available, "safety_a");
    undeclared.capability_class = "capability_unknown".into();
    assert_eq!(
        MorphEffectorV1::new(undeclared, 32, &vocabulary, &binding),
        Err(MorphErrorV1::UndeclaredClassification {
            axis: "capability_class"
        })
    );

    let mut prohibited_class =
        effector_input("effector.alpha", MorphAvailabilityV1::Available, "safety_a");
    prohibited_class.capability_class = "provider_payload".into();
    assert_eq!(
        MorphEffectorV1::new(prohibited_class, 32, &vocabulary, &binding),
        Err(MorphErrorV1::ProhibitedToken {
            field: "capability_class"
        })
    );

    let free_text = effector_input(
        "user said hello",
        MorphAvailabilityV1::Available,
        "safety_a",
    );
    assert_eq!(
        MorphEffectorV1::new(free_text, 32, &vocabulary, &binding),
        Err(MorphErrorV1::NonCanonicalToken {
            field: "effector_id"
        })
    );

    for prohibited in [
        "raw_user_text",
        "provider_payload",
        "visible_text",
        "neural_array",
        "continuum_kv",
        "effect_payload",
    ] {
        let input = effector_input(prohibited, MorphAvailabilityV1::Available, "safety_a");
        assert_eq!(
            MorphEffectorV1::new(input, 32, &vocabulary, &binding),
            Err(MorphErrorV1::ProhibitedToken {
                field: "effector_id"
            })
        );
    }
}

#[test]
fn rejects_duplicate_unordered_and_unbounded_vocabularies_and_entries() {
    let mut duplicate = vocabulary_input();
    duplicate.capability_classes[1] = "capability_a".into();
    assert_eq!(
        MorphClassificationVocabularyV1::new(duplicate, bounds()),
        Err(MorphErrorV1::DuplicateClassification {
            axis: "capability_class",
            index: 1
        })
    );

    let mut unordered = vocabulary_input();
    unordered.safety_classes.swap(0, 1);
    assert_eq!(
        MorphClassificationVocabularyV1::new(unordered, bounds()),
        Err(MorphErrorV1::NonCanonicalClassificationOrder {
            axis: "safety_class",
            index: 1
        })
    );

    assert_eq!(
        MorphClassificationVocabularyV1::new(
            vocabulary_input(),
            MorphVocabularyBoundsV1::new(1, 32).unwrap()
        ),
        Err(MorphErrorV1::TooManyClassifications {
            axis: "capability_class",
            max_items: 1,
            actual_items: 2
        })
    );

    let vocabulary = vocabulary();
    let binding = binding(7, 1, 2);
    let alpha = effector(
        "effector.alpha",
        MorphAvailabilityV1::Available,
        "safety_a",
        &vocabulary,
        &binding,
    );
    let beta = effector(
        "effector.beta",
        MorphAvailabilityV1::Available,
        "safety_a",
        &vocabulary,
        &binding,
    );
    assert!(matches!(
        MorphAffordanceCatalogV1::new(
            "morph.catalog.v1".into(),
            32,
            binding.clone(),
            vocabulary.clone(),
            vec![alpha.clone(), alpha.clone()],
            2,
        ),
        Err(MorphErrorV1::DuplicateEffector { index: 1 })
    ));
    assert!(matches!(
        MorphAffordanceCatalogV1::new(
            "morph.catalog.v1".into(),
            32,
            binding.clone(),
            vocabulary.clone(),
            vec![beta.clone(), alpha.clone()],
            2,
        ),
        Err(MorphErrorV1::NonCanonicalEffectorOrder { index: 1 })
    ));
    assert_eq!(
        MorphAffordanceCatalogV1::new(
            "morph.catalog.v1".into(),
            32,
            binding.clone(),
            vocabulary.clone(),
            vec![alpha, beta],
            1,
        ),
        Err(MorphErrorV1::TooManyEntries {
            max_items: 1,
            actual_items: 2
        })
    );
    assert_eq!(
        MorphAffordanceCatalogV1::new(
            "morph.catalog.v1".into(),
            32,
            binding,
            vocabulary,
            vec![],
            MORPH_AFFORDANCE_MAX_ITEMS_V1 + 1,
        ),
        Err(MorphErrorV1::CatalogBoundExceedsSchema {
            max_items: MORPH_AFFORDANCE_MAX_ITEMS_V1,
            actual_bound: MORPH_AFFORDANCE_MAX_ITEMS_V1 + 1
        })
    );
}

#[test]
fn rejects_cross_state_and_vocabulary_bound_entries() {
    let vocabulary = vocabulary();
    let first_binding = binding(7, 1, 2);
    let cross_state_entry = effector(
        "effector.alpha",
        MorphAvailabilityV1::Available,
        "safety_a",
        &vocabulary,
        &first_binding,
    );
    assert!(matches!(
        MorphAffordanceCatalogV1::new(
            "morph.catalog.v1".into(),
            32,
            binding(8, 1, 3),
            vocabulary.clone(),
            vec![cross_state_entry],
            1,
        ),
        Err(MorphErrorV1::StateBindingMismatch { index: 0 })
    ));

    let entry = effector(
        "effector.alpha",
        MorphAvailabilityV1::Available,
        "safety_a",
        &vocabulary,
        &first_binding,
    );
    let mut expanded_input = vocabulary_input();
    expanded_input
        .capability_classes
        .push("capability_c".into());
    let expanded = MorphClassificationVocabularyV1::new(expanded_input, bounds()).unwrap();
    assert!(matches!(
        MorphAffordanceCatalogV1::new(
            "morph.catalog.v1".into(),
            32,
            first_binding,
            expanded,
            vec![entry],
            1,
        ),
        Err(MorphErrorV1::VocabularyBindingMismatch { index: 0 })
    ));
}
