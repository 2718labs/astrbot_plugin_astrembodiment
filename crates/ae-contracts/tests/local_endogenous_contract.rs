use ae_contracts::{
    EndogenousIntentPhaseV1, EndogenousIntentStateV1, LocalCognitionContractErrorV1,
    LocalDreamResidueV1, LocalDreamTagV1, LOCAL_COGNITION_SCHEMA_VERSION,
    MAX_LOCAL_DREAM_TTL_MS_V1,
};
use ae_fixed::Fixed;

fn intent() -> EndogenousIntentStateV1 {
    EndogenousIntentStateV1 {
        schema_version: LOCAL_COGNITION_SCHEMA_VERSION,
        intent_id: [1; 16],
        persona_scope: [2; 32],
        state: EndogenousIntentPhaseV1::Incubating,
        salience: Fixed::from_raw(350_000),
        inhibition: Fixed::from_raw(250_000),
        urgency: Fixed::from_raw(150_000),
        revision: 1,
        source_semantic_revision: 3,
        updated_at_utc_ms: 1_000,
        expires_at_utc_ms: 2_000,
        source_event_id: [3; 16],
        commitment_digest: [4; 32],
    }
}

fn dream() -> LocalDreamResidueV1 {
    LocalDreamResidueV1 {
        schema_version: LOCAL_COGNITION_SCHEMA_VERSION,
        residue_id: [5; 16],
        persona_scope: [2; 32],
        tags: vec![LocalDreamTagV1::Continuity, LocalDreamTagV1::Threshold],
        affect_valence: Fixed::from_raw(-200_000),
        affect_arousal: Fixed::from_raw(300_000),
        source_event_ids: vec![[3; 16]],
        created_at_utc_ms: 1_000,
        expires_at_utc_ms: 2_000,
        non_fact: true,
        commitment_digest: [6; 32],
    }
}

#[test]
fn local_cognition_contracts_accept_only_bounded_persona_local_state() {
    assert_eq!(intent().validate_v1(), Ok(()));
    assert_eq!(dream().validate_v1(), Ok(()));

    let intent_json = serde_json::to_value(intent()).unwrap();
    let dream_json = serde_json::to_value(dream()).unwrap();
    for forbidden in [
        "relation_scope",
        "recipient",
        "message",
        "prompt",
        "provider",
        "platform",
        "send",
        "text",
        "factual_memory",
    ] {
        assert!(
            intent_json.get(forbidden).is_none(),
            "intent leaked {forbidden}"
        );
        assert!(
            dream_json.get(forbidden).is_none(),
            "dream leaked {forbidden}"
        );
    }

    let mut unknown = serde_json::to_value(intent()).unwrap();
    unknown["message"] = serde_json::json!("must not deserialize");
    assert!(serde_json::from_value::<EndogenousIntentStateV1>(unknown).is_err());
}

#[test]
fn local_dream_is_non_fact_unique_and_short_lived() {
    let mut factual = dream();
    factual.non_fact = false;
    assert_eq!(
        factual.validate_v1(),
        Err(LocalCognitionContractErrorV1::DreamMustBeNonFact)
    );

    let mut duplicate = dream();
    duplicate.tags.push(LocalDreamTagV1::Continuity);
    assert_eq!(
        duplicate.validate_v1(),
        Err(LocalCognitionContractErrorV1::DuplicateValue)
    );

    let mut long_lived = dream();
    long_lived.expires_at_utc_ms = long_lived.created_at_utc_ms + MAX_LOCAL_DREAM_TTL_MS_V1 + 1;
    assert_eq!(
        long_lived.validate_v1(),
        Err(LocalCognitionContractErrorV1::TtlExceeded)
    );
}
