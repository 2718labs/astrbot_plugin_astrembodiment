use ae_contracts::{
    wire, DeliveryKnowledgeV1, HostContractErrorV1, HostEffectDispositionV1, HostEffectV1,
    HostIngressKindV1, HostIngressV1, HostSettlementStatusV1, HostSettlementV1, PublicTextV1,
    HOST_SCHEMA_V1, LARK_PUBLIC_EFFECT_V1, PUBLIC_TEXT_V1,
};
use ae_runtime::{AstrRuntime, RuntimeError};

fn current_ingress(epoch: [u8; 16]) -> HostIngressV1 {
    HostIngressV1 {
        schema_version: HOST_SCHEMA_V1,
        kind: HostIngressKindV1::CurrentEvent,
        ingress_id: [1; 32],
        process_epoch_id: epoch,
        adapter_type: "lark".to_owned(),
        adapter_id_binding: [2; 32],
        scope_binding: [3; 32],
        session_binding: [4; 32],
        turn_binding: [5; 32],
        event_id: [6; 32],
        observed_at_ms: 1_000,
        base_revision: 0,
        current_event_text: Some("current user event".to_owned()),
        settlement: None,
    }
}

fn canonical_public_effect() -> HostEffectV1 {
    let mut effect = HostEffectV1 {
        schema_version: HOST_SCHEMA_V1,
        disposition: HostEffectDispositionV1::PublicEffect,
        effect_id: [0; 32],
        process_epoch_id: [7; 16],
        adapter_type: "lark".to_owned(),
        adapter_id_binding: [2; 32],
        scope_binding: [3; 32],
        session_binding: [4; 32],
        turn_binding: [5; 32],
        action_id: [17; 32],
        capability_id: LARK_PUBLIC_EFFECT_V1.to_owned(),
        authority_evidence_digest: [34; 32],
        policy_evidence_digest: [51; 32],
        authority_granted: true,
        policy_granted: true,
        payload_class: PUBLIC_TEXT_V1.to_owned(),
        public_payload: Some(PublicTextV1::new("canonical public reply".to_owned()).unwrap()),
        expires_at_ms: 10_000,
    };
    effect.effect_id = effect.recompute_effect_id();
    effect
}

fn digest_hex(digest: [u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn current_event_returns_typed_silence_and_revision() {
    let epoch = [7; 16];
    let mut runtime = AstrRuntime::scaffold();
    let effect = runtime
        .apply_host_ingress_v1(current_ingress(epoch))
        .unwrap()
        .unwrap();
    assert_eq!(effect.schema_version, HOST_SCHEMA_V1);
    assert_eq!(effect.disposition, HostEffectDispositionV1::Silence);
    assert_eq!(effect.process_epoch_id, epoch);
    assert_eq!(effect.public_payload, None);
    assert_eq!(runtime.current_revision(), 1);
}

#[test]
fn settlement_reenters_runtime_as_typed_ingress() {
    let epoch = [7; 16];
    let mut runtime = AstrRuntime::scaffold();
    let effect = runtime
        .apply_host_ingress_v1(current_ingress(epoch))
        .unwrap()
        .unwrap();
    let settlement = HostSettlementV1::for_effect(
        &effect,
        HostSettlementStatusV1::Silenced,
        DeliveryKnowledgeV1::NotDispatched,
        1_001,
    );
    let ingress = HostIngressV1::for_settlement(settlement.clone(), runtime.current_revision());
    assert_eq!(runtime.apply_host_ingress_v1(ingress).unwrap(), None);
    assert_eq!(runtime.last_host_settlement(), Some(&settlement));
}

#[test]
fn old_epoch_and_bad_shape_fail_closed() {
    let mut runtime = AstrRuntime::scaffold();
    runtime
        .apply_host_ingress_v1(current_ingress([7; 16]))
        .unwrap();
    let mut old_epoch = current_ingress([8; 16]);
    old_epoch.base_revision = runtime.current_revision();
    let err = runtime.apply_host_ingress_v1(old_epoch).unwrap_err();
    assert_eq!(err, RuntimeError::HostProcessEpochMismatch);

    let mut bad = current_ingress([7; 16]);
    bad.current_event_text = None;
    assert_eq!(
        runtime.apply_host_ingress_v1(bad).unwrap_err(),
        RuntimeError::InvalidHostIngress,
    );
}

#[test]
fn silence_shape_rejects_non_lark_and_stale_bindings() {
    let ingress = current_ingress([7; 16]);
    let silence = HostEffectV1::silence_for_ingress(&ingress, [17; 32]);
    assert_eq!(silence.validate_shape(), Ok(()));

    let mut wrong_adapter = silence.clone();
    wrong_adapter.adapter_type = "discord".to_owned();
    wrong_adapter.effect_id = wrong_adapter.recompute_effect_id();
    assert_eq!(
        wrong_adapter.validate_shape(),
        Err(HostContractErrorV1::InvalidEffectShape)
    );

    let mut wrong_adapter_binding = silence.clone();
    wrong_adapter_binding.adapter_id_binding = [66; 32];
    assert_eq!(
        wrong_adapter_binding.validate_shape(),
        Err(HostContractErrorV1::InvalidEffectShape)
    );

    let mut wrong_epoch = silence.clone();
    wrong_epoch.process_epoch_id = [85; 16];
    assert_eq!(
        wrong_epoch.validate_shape(),
        Err(HostContractErrorV1::InvalidEffectShape)
    );

    let mut wrong_scope = silence.clone();
    wrong_scope.scope_binding = [119; 32];
    assert_eq!(
        wrong_scope.validate_shape(),
        Err(HostContractErrorV1::InvalidEffectShape)
    );

    let mut wrong_session = silence.clone();
    wrong_session.session_binding = [68; 32];
    assert_eq!(
        wrong_session.validate_shape(),
        Err(HostContractErrorV1::InvalidEffectShape)
    );

    let mut wrong_turn = silence.clone();
    wrong_turn.turn_binding = [136; 32];
    assert_eq!(
        wrong_turn.validate_shape(),
        Err(HostContractErrorV1::InvalidEffectShape)
    );

    let mut zero_action = silence.clone();
    zero_action.action_id = [0; 32];
    assert_eq!(
        zero_action.validate_shape(),
        Err(HostContractErrorV1::InvalidEffectShape)
    );

    let mut wrong_effect_id = silence;
    wrong_effect_id.effect_id = [0; 32];
    assert_eq!(
        wrong_effect_id.validate_shape(),
        Err(HostContractErrorV1::InvalidEffectShape)
    );
}

#[test]
fn canonical_effect_id_matches_cross_language_fixture() {
    let effect = canonical_public_effect();
    assert_eq!(
        digest_hex(effect.recompute_effect_id()),
        "62d122635d48e44ba5c5b98b4bb7f0f318f085c652b71ebce68470a8b2aa04ae"
    );
    assert_eq!(effect.validate_shape(), Ok(()));
}

#[test]
fn exact_native_trigger_returns_fixed_public_effect() {
    const TRIGGER: &str = "[[AE_NATIVE_PUBLIC_EFFECT_V1]]";
    const FIXED_TEXT: &str = "AstrEmbodiment native public-effect v1.";

    let epoch = [7; 16];
    let mut ingress = current_ingress(epoch);
    ingress.current_event_text = Some(TRIGGER.to_owned());
    let expected_action_id = wire::domain_hash(
        b"astr-embodiment/native-public-action-v1",
        &[
            &ingress.process_epoch_id,
            &ingress.adapter_id_binding,
            &ingress.scope_binding,
            &ingress.session_binding,
            &ingress.turn_binding,
            &ingress.event_id,
        ],
    );
    let expected_authority = wire::domain_hash(
        b"astr-embodiment/explicit-public-trigger-authority-v1",
        &[&expected_action_id, &ingress.event_id],
    );
    let expected_policy = wire::domain_hash(
        b"astr-embodiment/fixed-public-text-policy-v1",
        &[&expected_action_id, FIXED_TEXT.as_bytes()],
    );

    let mut runtime = AstrRuntime::scaffold();
    let effect = runtime.apply_host_ingress_v1(ingress).unwrap().unwrap();

    assert_eq!(effect.disposition, HostEffectDispositionV1::PublicEffect);
    assert_eq!(
        effect.public_payload,
        Some(PublicTextV1::new(FIXED_TEXT.to_owned()).unwrap())
    );
    assert_eq!(effect.capability_id, LARK_PUBLIC_EFFECT_V1);
    assert_eq!(effect.payload_class, PUBLIC_TEXT_V1);
    assert!(effect.authority_granted);
    assert!(effect.policy_granted);
    assert_eq!(effect.action_id, expected_action_id);
    assert_eq!(effect.authority_evidence_digest, expected_authority);
    assert_eq!(effect.policy_evidence_digest, expected_policy);
    assert_eq!(effect.expires_at_ms, 31_000);
    assert_eq!(effect.effect_id, effect.recompute_effect_id());
    assert_eq!(effect.validate_shape(), Ok(()));
}

#[test]
fn native_public_effect_replay_keeps_action_and_effect_identity() {
    const TRIGGER: &str = "[[AE_NATIVE_PUBLIC_EFFECT_V1]]";

    let epoch = [7; 16];
    let mut first_ingress = current_ingress(epoch);
    first_ingress.current_event_text = Some(TRIGGER.to_owned());
    let mut runtime = AstrRuntime::scaffold();
    let first = runtime
        .apply_host_ingress_v1(first_ingress)
        .unwrap()
        .unwrap();
    assert_eq!(first.disposition, HostEffectDispositionV1::PublicEffect);

    let mut duplicate_ingress = current_ingress(epoch);
    duplicate_ingress.current_event_text = Some(TRIGGER.to_owned());
    duplicate_ingress.base_revision = runtime.current_revision();
    let duplicate = runtime
        .apply_host_ingress_v1(duplicate_ingress)
        .unwrap()
        .unwrap();

    assert_eq!(duplicate.disposition, HostEffectDispositionV1::PublicEffect);
    assert_eq!(duplicate.action_id, first.action_id);
    assert_eq!(duplicate.effect_id, first.effect_id);
}

#[test]
fn current_ingress_rejects_non_lark_and_zero_bindings_before_revision() {
    let epoch = [7; 16];
    let mut runtime = AstrRuntime::scaffold();

    for mut ingress in [
        {
            let mut ingress = current_ingress(epoch);
            ingress.adapter_type = "discord".to_owned();
            ingress
        },
        {
            let mut ingress = current_ingress(epoch);
            ingress.process_epoch_id = [0; 16];
            ingress
        },
        {
            let mut ingress = current_ingress(epoch);
            ingress.adapter_id_binding = [0; 32];
            ingress
        },
    ] {
        ingress.base_revision = runtime.current_revision();
        assert_eq!(
            runtime.apply_host_ingress_v1(ingress),
            Err(RuntimeError::InvalidHostIngress)
        );
        assert_eq!(runtime.current_revision(), 0);
    }
}

#[test]
fn issued_effect_settlement_requires_exact_correlation() {
    const TRIGGER: &str = "[[AE_NATIVE_PUBLIC_EFFECT_V1]]";

    let epoch = [7; 16];
    let mut ingress = current_ingress(epoch);
    ingress.current_event_text = Some(TRIGGER.to_owned());
    let mut runtime = AstrRuntime::scaffold();
    let effect = runtime.apply_host_ingress_v1(ingress).unwrap().unwrap();
    let mut mismatched = HostSettlementV1::for_effect(
        &effect,
        HostSettlementStatusV1::DispatchReturnedNoTypedReceipt,
        DeliveryKnowledgeV1::Unknown,
        1_001,
    );
    mismatched.action_id = [99; 32];
    let mismatched_ingress = HostIngressV1::for_settlement(mismatched, runtime.current_revision());
    assert_eq!(
        runtime.apply_host_ingress_v1(mismatched_ingress),
        Err(RuntimeError::InvalidHostSettlement)
    );
    assert_eq!(runtime.current_revision(), 1);

    let matching = HostSettlementV1::for_effect(
        &effect,
        HostSettlementStatusV1::DispatchReturnedNoTypedReceipt,
        DeliveryKnowledgeV1::Unknown,
        1_001,
    );
    let matching_ingress =
        HostIngressV1::for_settlement(matching.clone(), runtime.current_revision());
    assert_eq!(runtime.apply_host_ingress_v1(matching_ingress), Ok(None));
    assert_eq!(runtime.last_host_settlement(), Some(&matching));
}

#[test]
fn issued_effect_registry_has_a_fixed_bound() {
    const TRIGGER: &str = "[[AE_NATIVE_PUBLIC_EFFECT_V1]]";

    let epoch = [7; 16];
    let mut runtime = AstrRuntime::scaffold();
    for sequence in 1..=1_024_u64 {
        let mut ingress = current_ingress(epoch);
        ingress.current_event_text = Some(TRIGGER.to_owned());
        ingress.base_revision = runtime.current_revision();
        ingress.event_id[..8].copy_from_slice(&sequence.to_le_bytes());
        ingress.ingress_id[..8].copy_from_slice(&sequence.to_le_bytes());
        let effect = runtime.apply_host_ingress_v1(ingress).unwrap().unwrap();
        assert_eq!(effect.disposition, HostEffectDispositionV1::PublicEffect);
    }

    let mut overflow = current_ingress(epoch);
    overflow.current_event_text = Some(TRIGGER.to_owned());
    overflow.base_revision = runtime.current_revision();
    overflow.event_id[..8].copy_from_slice(&1_025_u64.to_le_bytes());
    overflow.ingress_id[..8].copy_from_slice(&1_025_u64.to_le_bytes());
    let error = runtime.apply_host_ingress_v1(overflow).unwrap_err();
    assert_eq!(error.to_string(), "host effect registry full");
    assert_eq!(runtime.current_revision(), 1_024);
}

#[test]
fn near_match_silences_and_timestamp_overflow_fails_before_revision() {
    const TRIGGER: &str = "[[AE_NATIVE_PUBLIC_EFFECT_V1]]";

    let epoch = [7; 16];
    let mut near_match = current_ingress(epoch);
    near_match.current_event_text = Some(format!("{TRIGGER} "));
    let mut near_runtime = AstrRuntime::scaffold();
    let silence = near_runtime
        .apply_host_ingress_v1(near_match)
        .unwrap()
        .unwrap();
    assert_eq!(silence.disposition, HostEffectDispositionV1::Silence);
    assert_eq!(near_runtime.current_revision(), 1);

    let mut overflow = current_ingress(epoch);
    overflow.current_event_text = Some(TRIGGER.to_owned());
    overflow.observed_at_ms = u64::MAX;
    let mut overflow_runtime = AstrRuntime::scaffold();
    assert_eq!(
        overflow_runtime.apply_host_ingress_v1(overflow),
        Err(RuntimeError::InvalidHostPublicEffect)
    );
    assert_eq!(overflow_runtime.current_revision(), 0);
}

#[test]
fn duplicate_suppressed_settlement_is_the_only_later_settlement_update() {
    const TRIGGER: &str = "[[AE_NATIVE_PUBLIC_EFFECT_V1]]";

    let epoch = [7; 16];
    let mut first_ingress = current_ingress(epoch);
    first_ingress.current_event_text = Some(TRIGGER.to_owned());
    let mut runtime = AstrRuntime::scaffold();
    let effect = runtime
        .apply_host_ingress_v1(first_ingress)
        .unwrap()
        .unwrap();
    let first = HostSettlementV1::for_effect(
        &effect,
        HostSettlementStatusV1::DispatchReturnedNoTypedReceipt,
        DeliveryKnowledgeV1::Unknown,
        1_001,
    );
    assert_eq!(
        runtime.apply_host_ingress_v1(HostIngressV1::for_settlement(
            first,
            runtime.current_revision()
        )),
        Ok(None)
    );

    let mut duplicate_ingress = current_ingress(epoch);
    duplicate_ingress.current_event_text = Some(TRIGGER.to_owned());
    duplicate_ingress.base_revision = runtime.current_revision();
    let duplicate_effect = runtime
        .apply_host_ingress_v1(duplicate_ingress)
        .unwrap()
        .unwrap();
    assert_eq!(duplicate_effect.effect_id, effect.effect_id);
    let duplicate = HostSettlementV1::for_effect(
        &duplicate_effect,
        HostSettlementStatusV1::DuplicateSuppressed,
        DeliveryKnowledgeV1::Unknown,
        1_002,
    );
    assert_eq!(
        runtime.apply_host_ingress_v1(HostIngressV1::for_settlement(
            duplicate.clone(),
            runtime.current_revision()
        )),
        Ok(None)
    );
    assert_eq!(runtime.last_host_settlement(), Some(&duplicate));

    let invalid_later = HostSettlementV1::for_effect(
        &effect,
        HostSettlementStatusV1::DeliveryUnknown,
        DeliveryKnowledgeV1::Unknown,
        1_003,
    );
    assert_eq!(
        runtime.apply_host_ingress_v1(HostIngressV1::for_settlement(
            invalid_later,
            runtime.current_revision()
        )),
        Err(RuntimeError::InvalidHostSettlement)
    );
}
