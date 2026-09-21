use ae_action_contract::{
    ActionContractV1, ActionCoreErrorV1, ActionDispositionV1, ActionRealizationV1,
    ActionRequirementsV1, CanonicalTokenSetV1, CanonicalTokenV1, DisclosureUseV1, OwnedClaimV1,
    ToolProposalV1, UnitIntervalV1, ACTION_REALIZATION_SCHEMA_V1,
};
use std::collections::BTreeSet;

type Digest = [u8; 32];

fn digest(seed: u8) -> Digest {
    [seed; 32]
}

fn token(value: &str) -> CanonicalTokenV1 {
    CanonicalTokenV1::new(value.to_owned(), 64).expect("canonical fixture token")
}

fn token_set(values: &[&str], max_items: u16) -> CanonicalTokenSetV1 {
    CanonicalTokenSetV1::new(values.iter().map(|value| token(value)).collect(), max_items)
        .expect("canonical fixture token set")
}

fn unit(parts_per_million: u32) -> UnitIntervalV1 {
    UnitIntervalV1::from_parts_per_million(parts_per_million).expect("unit interval fixture")
}

fn requirements() -> ActionRequirementsV1 {
    ActionRequirementsV1::new(
        token_set(&["answer_current_turn", "respect_exact_boundary"], 8),
        token_set(&["prefer_concise_output"], 8),
        token_set(&["state_verified_correction"], 8),
        token_set(&["invent_memory", "seek_reassurance"], 8),
    )
}

fn contract(base_revision: u64) -> ActionContractV1 {
    ActionContractV1::from_evaluation(
        [7; 16],
        digest(1),
        base_revision,
        digest(2),
        digest(3),
        ActionDispositionV1::SpeechAndToolPlan,
        token("answer_with_tool_plan"),
        requirements(),
        token_set(&["calculator"], 8),
        token_set(&["correction"], 8),
        unit(800_000),
        2_000,
    )
    .expect("valid action contract")
}

fn owned_claim(seed: u8) -> OwnedClaimV1 {
    OwnedClaimV1::new(
        digest(seed),
        Some(token(&format!("span:{seed}"))),
        unit(750_000),
        unit(600_000),
        unit(400_000),
        true,
    )
    .expect("valid owned claim")
}

fn realization(contract: &ActionContractV1) -> ActionRealizationV1 {
    ActionRealizationV1::for_contract(
        contract,
        vec![owned_claim(20)],
        vec![ToolProposalV1::new(token("calculator"), digest(30)).expect("tool proposal")],
        vec![DisclosureUseV1::new(token("correction"), digest(40)).expect("disclosure use")],
        unit(700_000),
    )
    .expect("valid action realization")
}

#[test]
fn equal_evaluation_inputs_have_equal_contract_identity() {
    let first = contract(9);
    let second = contract(9);

    assert_eq!(first, second);
    assert_eq!(first.contract_digest(), second.contract_digest());
}

#[test]
fn changed_revision_changes_contract_identity() {
    assert_ne!(
        contract(9).contract_digest(),
        contract(10).contract_digest()
    );
}

#[test]
fn realization_copies_exact_contract_bindings() {
    let contract = contract(9);
    let first = realization(&contract);
    let second = realization(&contract);

    assert_eq!(first, second);
    assert_eq!(first.action_id(), contract.action_id());
    assert_eq!(first.contract_digest(), contract.contract_digest());
    assert_eq!(first.speech_act(), contract.speech_act());
    assert_eq!(first.realization_digest(), second.realization_digest());
}

#[test]
fn realization_json_uses_only_authoritative_schema_fields() {
    let value = serde_json::to_value(realization(&contract(9))).expect("serialize realization");
    let object = value.as_object().expect("realization object");
    let actual: BTreeSet<&str> = object.keys().map(String::as_str).collect();
    let expected = BTreeSet::from([
        "action_id",
        "contract_digest",
        "disclosures_used",
        "manifest_confidence",
        "owned_claims",
        "proposed_tools",
        "schema",
        "speech_act",
    ]);

    assert_eq!(actual, expected);
    assert_eq!(object["schema"], ACTION_REALIZATION_SCHEMA_V1);
    assert_eq!(object["action_id"].as_str().expect("action id").len(), 32);
    assert_eq!(
        object["contract_digest"]
            .as_str()
            .expect("contract digest")
            .len(),
        64
    );
    for forbidden in [
        "utterance",
        "provider_profile_digest",
        "visible_text_digest",
        "contract_adherence",
        "source_basis",
        "realization_digest",
        "disposition",
    ] {
        assert!(!object.contains_key(forbidden), "unexpected {forbidden}");
    }

    let owned_claim = object["owned_claims"][0]
        .as_object()
        .expect("owned claim object");
    let claim_fields: BTreeSet<&str> = owned_claim.keys().map(String::as_str).collect();
    assert_eq!(
        claim_fields,
        BTreeSet::from([
            "assertiveness",
            "claim_digest",
            "confidence",
            "span_ref",
            "stakes",
            "verifiable",
        ])
    );
}

#[test]
fn rejects_an_unknown_disposition() {
    assert_eq!(
        ActionDispositionV1::parse("broadcast"),
        Err(ActionCoreErrorV1::UnknownDisposition)
    );
}

#[test]
fn rejects_raw_text_tokens() {
    assert_eq!(
        CanonicalTokenV1::new("Tell the user everything\nnow".to_owned(), 128),
        Err(ActionCoreErrorV1::NonCanonicalToken)
    );
}

#[test]
fn rejects_duplicate_contract_requirements() {
    assert_eq!(
        CanonicalTokenSetV1::new(vec![token("answer"), token("answer")], 8),
        Err(ActionCoreErrorV1::DuplicateToken { index: 1 })
    );
}

#[test]
fn rejects_unbounded_contract_sets() {
    assert_eq!(
        CanonicalTokenSetV1::new(vec![token("answer"), token("verify")], 1),
        Err(ActionCoreErrorV1::TooManyItems {
            field: "canonical_token_set",
            max_items: 1,
            actual_items: 2,
        })
    );
}

#[test]
fn rejects_duplicate_owned_claims() {
    let contract = contract(9);
    assert_eq!(
        ActionRealizationV1::for_contract(
            &contract,
            vec![owned_claim(20), owned_claim(20)],
            vec![],
            vec![],
            unit(700_000),
        ),
        Err(ActionCoreErrorV1::DuplicateOwnedClaim { index: 1 })
    );
}

#[test]
fn rejects_a_tool_not_allowed_by_the_fixed_contract() {
    let contract = contract(9);
    assert_eq!(
        ActionRealizationV1::for_contract(
            &contract,
            vec![],
            vec![ToolProposalV1::new(token("browser"), digest(31)).expect("tool proposal")],
            vec![],
            unit(700_000),
        ),
        Err(ActionCoreErrorV1::ToolNotAllowed { index: 0 })
    );
}

#[test]
fn rejects_tools_for_a_speech_only_disposition() {
    assert_eq!(
        ActionContractV1::from_evaluation(
            [7; 16],
            digest(1),
            9,
            digest(2),
            digest(3),
            ActionDispositionV1::Speech,
            token("answer"),
            requirements(),
            token_set(&["calculator"], 8),
            token_set(&["correction"], 8),
            unit(800_000),
            2_000,
        ),
        Err(ActionCoreErrorV1::InvalidDispositionShape)
    );
}
