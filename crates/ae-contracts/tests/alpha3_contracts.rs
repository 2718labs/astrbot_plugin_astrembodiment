use ae_contracts::{
    alpha3::{
        decode_execution_scoped_locator_v1, decode_intention_scoped_locator_v1,
        encode_execution_scoped_locator_v1, encode_intention_scoped_locator_v1,
        validate_source_ref_count, Alpha3ContractError, ExperienceProjectionV2, GateDecisionKindV2,
        GateDecisionV2, GateReasonV2, InteractionFactBatchV1, InteractionFactKindV1,
        InteractionFactV1, InteractionSourceAuthorityV1, ProjectionFieldV1, ALPHA3_SCHEMA_VERSION,
    },
    wire, CanonicalEvent, CausalRef, ScopeRef,
};

fn legacy_v2_ref_for_fixture(
    relation_scope: &[u8; 32],
    record_domain: &[u8],
    record_id: &[u8; 16],
) -> [u8; 32] {
    fn hash(
        relation_scope: &[u8; 32],
        purpose: &[u8],
        record_domain: &[u8],
        payload: &[u8],
    ) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new_keyed(relation_scope);
        for field in [
            b"ae.alpha3.authenticated-public-ref.v1".as_slice(),
            purpose,
            record_domain,
            payload,
        ] {
            hasher.update(&(field.len() as u64).to_le_bytes());
            hasher.update(field);
        }
        *hasher.finalize().as_bytes()
    }

    let mask = hash(relation_scope, b"mask", record_domain, &[]);
    let mut legacy_ref = [0; 32];
    for index in 0..record_id.len() {
        legacy_ref[index] = record_id[index] ^ mask[index];
    }
    let tag = hash(
        relation_scope,
        b"auth",
        record_domain,
        &legacy_ref[..record_id.len()],
    );
    legacy_ref[record_id.len()..].copy_from_slice(&tag[..record_id.len()]);
    legacy_ref
}
use ae_fixed::Fixed;

fn scope() -> ScopeRef {
    ScopeRef {
        bot_token: [2; 16],
        persona_token: [3; 16],
        relation_token: Some([4; 16]),
        session_token: [5; 16],
    }
}

fn fact(index: u8) -> InteractionFactV1 {
    InteractionFactV1 {
        fact_id: [index; 16],
        kind: InteractionFactKindV1::InboundObserved,
        observed_at_utc_ms: 1_700_000_000_000 + u64::from(index),
        source_authority: InteractionSourceAuthorityV1::AstrbotMetadata,
        source_digest: [index.wrapping_add(1); 32],
        extractor_digest: [index.wrapping_add(2); 32],
        confidence: Fixed::ONE,
        value_code: None,
        subject_public_ref: None,
        consent_terms: None,
        scheduled_at_utc_ms: None,
        expires_at_utc_ms: None,
    }
}

fn batch(count: usize) -> InteractionFactBatchV1 {
    InteractionFactBatchV1 {
        schema_version: ALPHA3_SCHEMA_VERSION,
        event_id: [9; 16],
        scope: scope(),
        causal: CausalRef {
            turn_id: [10; 16],
            action_id: None,
            delivery_id: None,
            claim_id: None,
            base_revision: 7,
        },
        facts: (0..count).map(|index| fact(index as u8 + 20)).collect(),
    }
}

#[test]
fn scoped_locators_are_transparent_checksums_bound_to_relation_and_domain() {
    let relation_scope = [41; 32];
    let other_relation_scope = [42; 32];
    let record_id = [43; 16];
    let intention_ref = encode_intention_scoped_locator_v1(&relation_scope, &record_id);
    let execution_ref = encode_execution_scoped_locator_v1(&relation_scope, &record_id);

    assert_eq!(&intention_ref[..16], record_id.as_slice());
    assert_eq!(&execution_ref[..16], record_id.as_slice());
    assert_ne!(intention_ref, execution_ref);
    assert_eq!(
        decode_intention_scoped_locator_v1(&relation_scope, &intention_ref).unwrap(),
        record_id
    );
    assert_eq!(
        decode_execution_scoped_locator_v1(&relation_scope, &execution_ref).unwrap(),
        record_id
    );
    assert!(decode_intention_scoped_locator_v1(&other_relation_scope, &intention_ref).is_err());
    assert!(decode_execution_scoped_locator_v1(&other_relation_scope, &execution_ref).is_err());
    assert!(decode_execution_scoped_locator_v1(&relation_scope, &intention_ref).is_err());
    assert!(decode_intention_scoped_locator_v1(&relation_scope, &execution_ref).is_err());

    for byte_index in [0, 15, 16, 31] {
        let mut tampered = execution_ref;
        tampered[byte_index] ^= 1;
        assert!(decode_execution_scoped_locator_v1(&relation_scope, &tampered).is_err());
    }

    let legacy_intention_ref = legacy_v2_ref_for_fixture(
        &relation_scope,
        b"ae.alpha3.intention-public-ref.v1",
        &record_id,
    );
    let legacy_execution_ref = legacy_v2_ref_for_fixture(
        &relation_scope,
        b"ae.alpha3.execution-public-ref.v1",
        &record_id,
    );
    assert!(
        ae_contracts::alpha3::legacy_v2_intention_public_ref_matches(
            &relation_scope,
            &record_id,
            &legacy_intention_ref,
        )
    );
    assert!(
        ae_contracts::alpha3::legacy_v2_execution_public_ref_matches(
            &relation_scope,
            &record_id,
            &legacy_execution_ref,
        )
    );
    assert!(decode_intention_scoped_locator_v1(&relation_scope, &legacy_intention_ref).is_err());
    assert!(decode_execution_scoped_locator_v1(&relation_scope, &legacy_execution_ref).is_err());
}

#[test]
fn historical_gate_without_authority_attestation_remains_readable() {
    let historical = GateDecisionV2 {
        decision: GateDecisionKindV2::Allowed,
        reason: GateReasonV2::AllRequirementsSatisfied,
        evaluated_at_utc_ms: 1_700_000_000_000,
        retry_at_utc_ms: None,
        consent_epoch: 2,
        consent_revision: 3,
        policy_revision: 4,
        budget_day_start_utc_ms: 1_699_977_600_000,
        intention_public_ref: [5; 32],
        cause_public_refs: vec![[6; 32]],
        capability_snapshot_digest: [7; 32],
        authority_snapshot_digest: None,
        authority_expires_at_utc_ms: None,
        authority_budget_expires_at_utc_ms: None,
        effective_frequency: None,
    };
    let mut historical_json = serde_json::to_value(&historical).unwrap();
    let object = historical_json.as_object_mut().unwrap();
    object.remove("authority_snapshot_digest");
    object.remove("authority_expires_at_utc_ms");
    object.remove("authority_budget_expires_at_utc_ms");
    object.remove("effective_frequency");

    let decoded: GateDecisionV2 = serde_json::from_value(historical_json).unwrap();
    assert_eq!(decoded.authority_snapshot_digest, None);
    assert_eq!(decoded.authority_expires_at_utc_ms, None);
    assert_eq!(decoded.authority_budget_expires_at_utc_ms, None);
    assert_eq!(decoded.effective_frequency, None);
    decoded.validate().unwrap();
}

#[test]
fn historical_dispatch_claim_without_gate_snapshot_remains_readable() {
    let historical_json = serde_json::json!({
        "claim_token": ae_contracts::hex::encode32(&[1; 32]),
        "outbound_public_ref": ae_contracts::hex::encode32(&[2; 32]),
        "consent_epoch": 3,
        "consent_revision": 4,
        "policy_revision": 5,
        "target_binding_digest": ae_contracts::hex::encode32(&[6; 32]),
        "capability_snapshot_digest": ae_contracts::hex::encode32(&[7; 32]),
        "lease_deadline_utc_ms": 8,
    });

    let decoded: ae_contracts::alpha3::DispatchClaimV2 =
        serde_json::from_value(historical_json).unwrap();
    let reserialized = serde_json::to_value(decoded).unwrap();
    assert!(reserialized
        .as_object()
        .unwrap()
        .contains_key("gate_decision_snapshot"));
    assert_eq!(
        reserialized["gate_decision_snapshot"],
        serde_json::Value::Null
    );
}

fn push_string(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u32).to_le_bytes());
    out.extend_from_slice(value.as_bytes());
}

fn v4_time_advance_fixture() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&4_u16.to_le_bytes());
    out.push(wire::KIND_TIME_ADVANCE);
    out.extend_from_slice(&[31; 16]);
    out.extend_from_slice(&[32; 16]);
    out.extend_from_slice(&[33; 16]);
    out.push(0);
    out.extend_from_slice(&[34; 16]);
    out.extend_from_slice(&5_u64.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&1_700_000_000_000_u64.to_le_bytes());
    out.extend_from_slice(&1_700_000_000_001_u64.to_le_bytes());
    push_string(&mut out, "Asia/Shanghai");
    out.extend_from_slice(&28_800_i32.to_le_bytes());
    out.extend_from_slice(&600_u16.to_le_bytes());
    out.extend_from_slice(&19_000_i32.to_le_bytes());
    push_string(&mut out, "Asia/Shanghai");
    out.extend_from_slice(&28_800_i32.to_le_bytes());
    out.extend_from_slice(&600_u16.to_le_bytes());
    out.extend_from_slice(&19_000_i32.to_le_bytes());
    out.extend_from_slice(&1_699_977_600_000_u64.to_le_bytes());
    out.extend_from_slice(&1_700_064_000_000_u64.to_le_bytes());
    out.push(0);
    out.extend_from_slice(&[35; 32]);
    out.extend_from_slice(&[36; 32]);
    out.extend_from_slice(&Fixed::ZERO.encode());
    out.extend_from_slice(&Fixed::ZERO.encode());
    out.push(0);
    out.extend_from_slice(&[37; 32]);
    out
}

#[test]
fn wire_v5_is_closed_and_v4_remains_decodable() {
    let expected = CanonicalEvent::InteractionFactBatch(batch(2));
    let encoded = wire::encode_event_checked(&expected).unwrap();
    assert_eq!(&encoded[..2], &5_u16.to_le_bytes());
    assert_eq!(wire::decode_event(&encoded).unwrap(), expected);
    let first_fact_id = [20_u8; 16];
    let first_fact_offset = encoded
        .windows(first_fact_id.len())
        .position(|window| window == first_fact_id)
        .unwrap();
    let mut unknown_wire_enum = encoded.clone();
    unknown_wire_enum[first_fact_offset + first_fact_id.len()] = u8::MAX;
    assert!(matches!(
        wire::decode_event(&unknown_wire_enum),
        Err(wire::WireError::InvalidEnum("interaction fact kind"))
    ));

    assert!(matches!(
        wire::encode_event_checked(&CanonicalEvent::InteractionFactBatch(batch(0))),
        Err(wire::WireError::EmptyInteractionFacts)
    ));
    assert!(matches!(
        wire::encode_event_checked(&CanonicalEvent::InteractionFactBatch(batch(17))),
        Err(wire::WireError::TooManyInteractionFacts)
    ));
    assert!(matches!(
        validate_source_ref_count(&[[0; 16]; 9]),
        Err(Alpha3ContractError::TooManySourceRefs)
    ));

    let mut unknown_enum = serde_json::to_value(fact(90)).unwrap();
    unknown_enum["kind"] = serde_json::json!("invented_fact");
    assert!(serde_json::from_value::<InteractionFactV1>(unknown_enum).is_err());
    let mut unknown_field = serde_json::to_value(fact(91)).unwrap();
    unknown_field["free_text"] = serde_json::json!("must not enter native state");
    assert!(serde_json::from_value::<InteractionFactV1>(unknown_field).is_err());

    let v4_fixture = v4_time_advance_fixture();
    let decoded_v4 = wire::decode_event(&v4_fixture).unwrap();
    assert!(matches!(&decoded_v4, CanonicalEvent::TimeAdvance(_)));
    assert_eq!(wire::encode_event(&decoded_v4), v4_fixture);
}

#[test]
fn legacy_experience_json_without_affect_defaults_to_not_initialized() {
    let unavailable = serde_json::json!({"availability": "unavailable_on_host"});
    let not_initialized = serde_json::json!({"availability": "not_initialized"});
    let historical = serde_json::json!({
        "world_mode": unavailable,
        "world_layer": {"availability": "unavailable_on_host"},
        "activity": {"availability": "unavailable_on_host"},
        "goal": {"availability": "unavailable_on_host"},
        "sleep": {"availability": "not_initialized"},
        "notable_nodes": {"availability": "not_initialized"},
        "contact_explanation": not_initialized,
        "disclosure": "hidden_by_user"
    });

    let decoded: ExperienceProjectionV2 = serde_json::from_value(historical).unwrap();
    assert_eq!(decoded.affect, ProjectionFieldV1::NotInitialized);
}
