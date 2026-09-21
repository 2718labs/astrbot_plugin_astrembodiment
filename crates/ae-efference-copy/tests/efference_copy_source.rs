use ae_action_contract::{
    ActionContractV1, ActionDispositionV1, ActionRealizationV1, ActionRequirementsV1,
    CanonicalTokenSetV1, CanonicalTokenV1, DisclosureUseV1, ToolProposalV1, UnitIntervalV1,
};
use ae_efference_copy::{
    EffectClassV1, EffectDigestRecordV1, EffectPhaseV1, EfferenceCopyErrorV1,
    EfferenceCopySourceV1, ExpectedDispositionV1, ObservedDispositionV1, MAX_EFFECT_DIGEST_RECORDS,
};

type Digest = [u8; 32];

fn digest(seed: u8) -> Digest {
    [seed; 32]
}

fn token(value: &str) -> CanonicalTokenV1 {
    CanonicalTokenV1::new(value.to_owned(), 64).expect("canonical fixture token")
}

fn token_set(values: &[&str]) -> CanonicalTokenSetV1 {
    CanonicalTokenSetV1::new(values.iter().map(|value| token(value)).collect(), 8)
        .expect("canonical fixture token set")
}

fn unit(parts_per_million: u32) -> UnitIntervalV1 {
    UnitIntervalV1::from_parts_per_million(parts_per_million).expect("unit fixture")
}

fn contract_with(
    action_seed: u8,
    turn_seed: u8,
    base_revision: u64,
    state_seed: u8,
    identity_seed: u8,
) -> ActionContractV1 {
    ActionContractV1::from_evaluation(
        [action_seed; 16],
        digest(turn_seed),
        base_revision,
        digest(state_seed),
        digest(identity_seed),
        ActionDispositionV1::SpeechAndToolPlan,
        token("answer_with_tool_plan"),
        ActionRequirementsV1::new(
            token_set(&["answer_current_turn"]),
            token_set(&["prefer_concise_output"]),
            token_set(&["state_verified_correction"]),
            token_set(&["invent_memory"]),
        ),
        token_set(&["calculator"]),
        token_set(&["correction"]),
        unit(800_000),
        2_000,
    )
    .expect("valid action contract")
}

fn contract() -> ActionContractV1 {
    contract_with(7, 1, 9, 2, 3)
}

fn realization(contract: &ActionContractV1, manifest_confidence: u32) -> ActionRealizationV1 {
    ActionRealizationV1::for_contract(
        contract,
        vec![],
        vec![ToolProposalV1::new(token("calculator"), digest(30)).expect("tool proposal")],
        vec![DisclosureUseV1::new(token("correction"), digest(40)).expect("disclosure use")],
        unit(manifest_confidence),
    )
    .expect("valid action realization")
}

fn effect(
    ordinal: u16,
    phase: EffectPhaseV1,
    class: EffectClassV1,
    seed: u8,
) -> EffectDigestRecordV1 {
    EffectDigestRecordV1::new(ordinal, phase, class, digest(seed))
        .expect("valid effect digest record")
}

fn effects() -> Vec<EffectDigestRecordV1> {
    vec![
        effect(0, EffectPhaseV1::Expected, EffectClassV1::VisibleOutput, 50),
        effect(1, EffectPhaseV1::Observed, EffectClassV1::VisibleOutput, 50),
        effect(2, EffectPhaseV1::Expected, EffectClassV1::ToolEffect, 51),
        effect(3, EffectPhaseV1::Observed, EffectClassV1::ToolEffect, 52),
    ]
}

#[test]
fn equal_typed_inputs_produce_equal_domain_separated_copy_identity() {
    let contract = contract();
    let realization = realization(&contract, 700_000);
    let first = EfferenceCopySourceV1::default()
        .form(
            &contract,
            &realization,
            ExpectedDispositionV1::SpeechAndToolPlan,
            ObservedDispositionV1::SpeechAndToolEffect,
            effects(),
        )
        .expect("first copy");
    let second = EfferenceCopySourceV1::default()
        .form(
            &contract,
            &realization,
            ExpectedDispositionV1::SpeechAndToolPlan,
            ObservedDispositionV1::SpeechAndToolEffect,
            effects(),
        )
        .expect("second copy from independent source");

    assert_eq!(first, second);
    assert_eq!(first.action_id(), contract.action_id());
    assert_eq!(first.contract_digest(), contract.contract_digest());
    assert_eq!(first.realization_digest(), realization.realization_digest());
    assert_eq!(first.effect_records(), effects());
    assert_ne!(first.copy_digest(), &[0; 32]);
}

#[test]
fn explicit_dispositions_are_retained_without_inferred_settlement() {
    let contract = contract();
    let realization = realization(&contract, 700_000);
    let copy = EfferenceCopySourceV1::default()
        .form(
            &contract,
            &realization,
            ExpectedDispositionV1::SpeechAndToolPlan,
            ObservedDispositionV1::NoEffect,
            vec![],
        )
        .expect("explicit disposition copy");

    assert_eq!(
        copy.expected_disposition(),
        ExpectedDispositionV1::SpeechAndToolPlan
    );
    assert_eq!(copy.observed_disposition(), ObservedDispositionV1::NoEffect);
    assert!(copy.effect_records().is_empty());
}

#[test]
fn changed_explicit_classification_or_effect_record_changes_copy_identity() {
    let contract = contract();
    let realization = realization(&contract, 700_000);
    let baseline = EfferenceCopySourceV1::default()
        .form(
            &contract,
            &realization,
            ExpectedDispositionV1::SpeechAndToolPlan,
            ObservedDispositionV1::SpeechAndToolEffect,
            effects(),
        )
        .expect("baseline copy");
    let changed_disposition = EfferenceCopySourceV1::default()
        .form(
            &contract,
            &realization,
            ExpectedDispositionV1::SpeechAndToolPlan,
            ObservedDispositionV1::NoEffect,
            effects(),
        )
        .expect("changed disposition copy");
    let mut changed_effects = effects();
    changed_effects[3] = effect(3, EffectPhaseV1::Observed, EffectClassV1::Disclosure, 53);
    let changed_effect = EfferenceCopySourceV1::default()
        .form(
            &contract,
            &realization,
            ExpectedDispositionV1::SpeechAndToolPlan,
            ObservedDispositionV1::SpeechAndToolEffect,
            changed_effects,
        )
        .expect("changed effect copy");

    assert_ne!(baseline.copy_digest(), changed_disposition.copy_digest());
    assert_ne!(baseline.copy_digest(), changed_effect.copy_digest());
}

#[test]
fn rejects_contracts_with_changed_turn_state_revision_or_identity_binding() {
    let selected_contract = contract();
    let realization = realization(&selected_contract, 700_000);
    let mismatched_contracts = [
        contract_with(7, 8, 9, 2, 3),
        contract_with(7, 1, 9, 8, 3),
        contract_with(7, 1, 10, 2, 3),
        contract_with(7, 1, 9, 2, 8),
    ];

    for mismatched_contract in mismatched_contracts {
        let mut source = EfferenceCopySourceV1::default();
        assert_eq!(
            source.form(
                &mismatched_contract,
                &realization,
                ExpectedDispositionV1::SpeechAndToolPlan,
                ObservedDispositionV1::NoEffect,
                vec![],
            ),
            Err(EfferenceCopyErrorV1::ContractDigestMismatch)
        );
        source
            .form(
                &selected_contract,
                &realization,
                ExpectedDispositionV1::SpeechAndToolPlan,
                ObservedDispositionV1::NoEffect,
                vec![],
            )
            .expect("rejected mismatch must not consume the selected action");
    }
}

#[test]
fn rejects_action_identity_mismatch() {
    let selected_contract = contract();
    let realization = realization(&selected_contract, 700_000);
    let mismatched_action = contract_with(8, 1, 9, 2, 3);

    assert_eq!(
        EfferenceCopySourceV1::default().form(
            &mismatched_action,
            &realization,
            ExpectedDispositionV1::SpeechAndToolPlan,
            ObservedDispositionV1::NoEffect,
            vec![],
        ),
        Err(EfferenceCopyErrorV1::ActionIdMismatch)
    );
}

#[test]
fn rejects_unordered_unbounded_and_zero_effect_digests() {
    assert_eq!(
        EffectDigestRecordV1::new(
            0,
            EffectPhaseV1::Expected,
            EffectClassV1::VisibleOutput,
            [0; 32],
        ),
        Err(EfferenceCopyErrorV1::ZeroEffectDigest)
    );

    let contract = contract();
    let realization = realization(&contract, 700_000);
    assert_eq!(
        EfferenceCopySourceV1::default().form(
            &contract,
            &realization,
            ExpectedDispositionV1::SpeechAndToolPlan,
            ObservedDispositionV1::NoEffect,
            vec![
                effect(0, EffectPhaseV1::Expected, EffectClassV1::VisibleOutput, 50,),
                effect(2, EffectPhaseV1::Observed, EffectClassV1::VisibleOutput, 50,),
            ],
        ),
        Err(EfferenceCopyErrorV1::NonContiguousEffectOrdinal {
            index: 1,
            expected: 1,
            actual: 2,
        })
    );

    let too_many = (0..=MAX_EFFECT_DIGEST_RECORDS)
        .map(|ordinal| {
            effect(
                ordinal,
                EffectPhaseV1::Observed,
                EffectClassV1::ToolEffect,
                60,
            )
        })
        .collect();
    assert_eq!(
        EfferenceCopySourceV1::default().form(
            &contract,
            &realization,
            ExpectedDispositionV1::SpeechAndToolPlan,
            ObservedDispositionV1::ToolEffect,
            too_many,
        ),
        Err(EfferenceCopyErrorV1::TooManyEffectRecords {
            max: MAX_EFFECT_DIGEST_RECORDS,
            actual: usize::from(MAX_EFFECT_DIGEST_RECORDS) + 1,
        })
    );
}

#[test]
fn rejects_replay_of_an_action_even_with_a_different_realization_or_feedback() {
    let contract = contract();
    let first_realization = realization(&contract, 700_000);
    let second_realization = realization(&contract, 600_000);
    let mut source = EfferenceCopySourceV1::default();
    source
        .form(
            &contract,
            &first_realization,
            ExpectedDispositionV1::SpeechAndToolPlan,
            ObservedDispositionV1::SpeechAndToolEffect,
            effects(),
        )
        .expect("first feedback");

    assert_eq!(
        source.form(
            &contract,
            &second_realization,
            ExpectedDispositionV1::SpeechAndToolPlan,
            ObservedDispositionV1::NoEffect,
            vec![],
        ),
        Err(EfferenceCopyErrorV1::ReplayedAction)
    );
}
