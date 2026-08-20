use ae_contracts::{ActionEffectStage, Digest, Id128, ScopeRef, ToolActionDescriptor};

fn scope_with_relation(relation_token: Option<Id128>) -> ScopeRef {
    ScopeRef {
        bot_token: [0x11; 16],
        persona_token: [0x22; 16],
        relation_token,
        session_token: [0x33; 16],
    }
}

fn descriptor_with_result(result_digest: Option<Digest>) -> ToolActionDescriptor {
    ToolActionDescriptor {
        action_id: [0xaa; 16],
        tool_class: 7,
        side_effect_class: 2,
        argument_digest: [0xbb; 32],
        authorization_digest: [0xcc; 32],
        result_digest,
        stage: ActionEffectStage::Executed,
    }
}

#[test]
fn optional_id128_none_is_exact_json_null_and_round_trips() {
    let original = scope_with_relation(None);
    let json = serde_json::to_value(&original).expect("ScopeRef should serialize");

    assert_eq!(json["relation_token"], serde_json::Value::Null);
    let decoded: ScopeRef = serde_json::from_value(json).expect("ScopeRef should deserialize");
    assert_eq!(decoded, original);
}

#[test]
fn optional_id128_some_is_lowercase_hex_and_round_trips() {
    let original = scope_with_relation(Some([
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f,
    ]));
    let json = serde_json::to_value(&original).expect("ScopeRef should serialize");

    assert_eq!(
        json["relation_token"],
        serde_json::json!("000102030405060708090a0b0c0d0e0f")
    );
    let decoded: ScopeRef = serde_json::from_value(json).expect("ScopeRef should deserialize");
    assert_eq!(decoded, original);
}

#[test]
fn optional_digest_none_is_exact_json_null_and_round_trips() {
    let original = descriptor_with_result(None);
    let json = serde_json::to_value(&original).expect("ToolActionDescriptor should serialize");

    assert_eq!(json["result_digest"], serde_json::Value::Null);
    let decoded: ToolActionDescriptor =
        serde_json::from_value(json).expect("ToolActionDescriptor should deserialize");
    assert_eq!(decoded, original);
}

#[test]
fn optional_digest_some_is_lowercase_hex_and_round_trips() {
    let original = descriptor_with_result(Some([
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
        0x1e, 0x1f,
    ]));
    let json = serde_json::to_value(&original).expect("ToolActionDescriptor should serialize");

    assert_eq!(
        json["result_digest"],
        serde_json::json!("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f")
    );
    let decoded: ToolActionDescriptor =
        serde_json::from_value(json).expect("ToolActionDescriptor should deserialize");
    assert_eq!(decoded, original);
}
