use ae_action_contract::{
    decode_action_contract_v1, encode_action_contract_v1, ActionContractV1, ActionDispositionV1,
    ActionRequirementsV1, CanonicalTokenSetV1, CanonicalTokenV1, UnitIntervalV1,
};

fn token(value: &str) -> CanonicalTokenV1 {
    CanonicalTokenV1::new(value.to_owned(), 128).expect("canonical fixture token")
}

fn set(values: &[&str], max_items: u16) -> CanonicalTokenSetV1 {
    CanonicalTokenSetV1::new(values.iter().map(|value| token(value)).collect(), max_items)
        .expect("canonical fixture set")
}

fn generated_set(count: usize, max_items: u16, prefix: &str) -> CanonicalTokenSetV1 {
    let values = (0..count)
        .map(|index| {
            CanonicalTokenV1::new(format!("{prefix}-{index:03}"), 200)
                .expect("generated canonical token")
        })
        .collect();
    CanonicalTokenSetV1::new(values, max_items.max(count as u16)).expect("generated canonical set")
}

fn hex_bytes(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).expect("golden hex"))
        .collect()
}

fn fixture() -> ActionContractV1 {
    ActionContractV1::from_evaluation(
        [7; 16],
        [2; 32],
        9,
        [1; 32],
        [3; 32],
        ActionDispositionV1::SpeechAndToolPlan,
        token("say"),
        ActionRequirementsV1::new(
            set(&["must-a", "must-b"], 64),
            set(&["should-a"], 64),
            set(&["may-a"], 64),
            set(&["must-not-a"], 64),
        ),
        set(&["calculator", "correction"], 32),
        set(&["profile"], 32),
        UnitIntervalV1::from_parts_per_million(800_000).expect("confidence"),
        2_000,
    )
    .expect("fixture contract")
}

fn contract_with(
    disposition: ActionDispositionV1,
    speech_act: CanonicalTokenV1,
    requirements: ActionRequirementsV1,
    allowed_tools: CanonicalTokenSetV1,
    allowed_disclosures: CanonicalTokenSetV1,
) -> ActionContractV1 {
    ActionContractV1::from_evaluation(
        [7; 16],
        [2; 32],
        9,
        [1; 32],
        [3; 32],
        disposition,
        speech_act,
        requirements,
        allowed_tools,
        allowed_disclosures,
        UnitIntervalV1::from_parts_per_million(800_000).expect("confidence"),
        2_000,
    )
    .expect("contract fixture")
}

#[test]
fn fixed_codec_known_answer_round_trip() {
    let contract = fixture();
    let encoded = encode_action_contract_v1(&contract).expect("encode");
    let decoded = decode_action_contract_v1(&encoded).expect("decode");
    assert_eq!(decoded, contract);
    assert_eq!(
        encode_action_contract_v1(&decoded).expect("re-encode"),
        encoded
    );

    // Independent wire oracle for the fixed fixture (header + LE integers + raw bytes).
    // Filled with the hand-authored bytes below; this must not be derived from `encoded`.
    let expected = hex_bytes(concat!(
        "4145414354563100010004010000",
        "07070707070707070707070707070707",
        "0202020202020202020202020202020202020202020202020202020202020202",
        "0900000000000000",
        "0101010101010101010101010101010101010101010101010101010101010101",
        "0303030303030303030303030303030303030303030303030303030303030303",
        "03",
        "0300736179",
        "020006006d7573742d6106006d7573742d620100080073686f756c642d61010005006d61792d6101000a006d7573742d6e6f742d61",
        "02000a0063616c63756c61746f720a00636f7272656374696f6e0100070070726f66696c65",
        "00350c00d007000000000000",
        "aadbb4177c211a85aa2b4d33f7cef43b4e15270c92920bc4e46c063827a45d13"
    ));
    assert_eq!(encoded, expected);
}

#[test]
fn disposition_codes_are_fixed_and_unknown_codes_fail_closed() {
    let mut encoded = encode_action_contract_v1(&fixture()).expect("encode");
    // Header (14 bytes) + action/turn/revision/source/constitution (120 bytes).
    assert_eq!(encoded[14 + 120], 3);
    encoded[14 + 120] = 0xff;
    assert!(decode_action_contract_v1(&encoded).is_err());
}

#[test]
fn malformed_headers_lengths_trailing_and_digest_are_rejected() {
    let encoded = encode_action_contract_v1(&fixture()).expect("encode");
    for end in 0..encoded.len() {
        assert!(
            decode_action_contract_v1(&encoded[..end]).is_err(),
            "offset {end}"
        );
    }

    let mut wrong_magic = encoded.clone();
    wrong_magic[0] ^= 1;
    assert!(decode_action_contract_v1(&wrong_magic).is_err());

    let mut wrong_version = encoded.clone();
    wrong_version[8] = 2;
    assert!(decode_action_contract_v1(&wrong_version).is_err());

    let mut wrong_length = encoded.clone();
    wrong_length[10..14].copy_from_slice(&0u32.to_le_bytes());
    assert!(decode_action_contract_v1(&wrong_length).is_err());

    let mut trailing = encoded.clone();
    trailing.push(0);
    assert!(decode_action_contract_v1(&trailing).is_err());

    let mut forged_digest = encoded;
    let digest_start = forged_digest.len() - 32;
    forged_digest[digest_start] ^= 1;
    assert!(decode_action_contract_v1(&forged_digest).is_err());
}

#[test]
fn wire_little_endian_fields_and_semantic_digest_change_together() {
    let contract = fixture();
    let bytes = encode_action_contract_v1(&contract).expect("encode");
    // base_revision follows the five fixed-width identity fields.
    let revision_start = 14 + 16 + 32;
    assert_eq!(
        &bytes[revision_start..revision_start + 8],
        &9u64.to_le_bytes()
    );

    let changed = ActionContractV1::from_evaluation(
        [7; 16],
        [2; 32],
        10,
        [1; 32],
        [3; 32],
        ActionDispositionV1::SpeechAndToolPlan,
        token("say"),
        ActionRequirementsV1::new(
            set(&["must-a", "must-b"], 64),
            set(&["should-a"], 64),
            set(&["may-a"], 64),
            set(&["must-not-a"], 64),
        ),
        set(&["calculator", "correction"], 32),
        set(&["profile"], 32),
        UnitIntervalV1::from_parts_per_million(800_000).expect("confidence"),
        2_000,
    )
    .expect("changed contract");
    let changed_bytes = encode_action_contract_v1(&changed).expect("encode changed");
    assert_ne!(bytes, changed_bytes);
    assert_ne!(contract.contract_digest(), changed.contract_digest());
}

#[test]
fn strict_token_set_bounds_order_utf8_and_shape_are_rejected() {
    let encoded = encode_action_contract_v1(&fixture()).expect("encode");
    let speech_len = 14 + 16 + 32 + 8 + 32 + 32 + 1;

    let mut invalid_utf8 = encoded.clone();
    invalid_utf8[speech_len + 2] = 0xff;
    assert!(decode_action_contract_v1(&invalid_utf8).is_err());

    let mut invalid_token = encoded.clone();
    invalid_token[speech_len + 2] = b'S';
    assert!(decode_action_contract_v1(&invalid_token).is_err());

    let mut too_long = encoded.clone();
    too_long[speech_len..speech_len + 2].copy_from_slice(&129u16.to_le_bytes());
    assert!(decode_action_contract_v1(&too_long).is_err());

    let must_count = speech_len + 2 + 3;
    let mut too_many = encoded.clone();
    too_many[must_count..must_count + 2].copy_from_slice(&65u16.to_le_bytes());
    assert!(decode_action_contract_v1(&too_many).is_err());

    let mut duplicate = encoded.clone();
    // `must-a` becomes `must-b`, colliding with the second sorted item.
    duplicate[must_count + 2 + 5] = b'b';
    assert!(decode_action_contract_v1(&duplicate).is_err());

    let mut reversed = encoded;
    // `must-a` becomes `z...`, violating strict ascending order.
    reversed[must_count + 2] = b'z';
    assert!(decode_action_contract_v1(&reversed).is_err());

    let mut shape = fixture();
    shape = ActionContractV1::from_evaluation(
        *shape.action_id(),
        *shape.turn_binding(),
        shape.base_revision(),
        *shape.source_state_digest(),
        *shape.identity_constitution_digest(),
        ActionDispositionV1::Silence,
        token("silence"),
        ActionRequirementsV1::new(set(&[], 64), set(&[], 64), set(&[], 64), set(&[], 64)),
        set(&[], 32),
        set(&[], 32),
        UnitIntervalV1::from_parts_per_million(800_000).expect("confidence"),
        2_000,
    )
    .expect("valid silence shape");
    let mut shape_bytes = encode_action_contract_v1(&shape).expect("encode silence");
    shape_bytes[14 + 120] = 3;
    assert!(decode_action_contract_v1(&shape_bytes).is_err());
}

#[test]
fn fixed_bounds_and_zero_bindings_fail_closed_on_encode_or_decode() {
    let long = "a".repeat(129);
    let oversized_token = CanonicalTokenV1::new(long, 200).expect("fixture token");
    let contract = ActionContractV1::from_evaluation(
        [7; 16],
        [2; 32],
        9,
        [1; 32],
        [3; 32],
        ActionDispositionV1::SpeechAndToolPlan,
        token("say"),
        ActionRequirementsV1::new(
            CanonicalTokenSetV1::new(vec![oversized_token], 64).expect("set"),
            set(&[], 64),
            set(&[], 64),
            set(&[], 64),
        ),
        set(&["tool"], 32),
        set(&[], 32),
        UnitIntervalV1::from_parts_per_million(800_000).expect("confidence"),
        2_000,
    )
    .expect("semantic contract");
    assert!(encode_action_contract_v1(&contract).is_err());

    let encoded = encode_action_contract_v1(&fixture()).expect("encode");
    let mut zero_id = encoded.clone();
    zero_id[14] = 0;
    assert!(decode_action_contract_v1(&zero_id).is_err());

    let mut over_confidence = encoded.clone();
    let confidence_start = over_confidence.len() - 32 - 8 - 4;
    over_confidence[confidence_start..confidence_start + 4]
        .copy_from_slice(&1_000_001u32.to_le_bytes());
    assert!(decode_action_contract_v1(&over_confidence).is_err());

    let mut missing_expiry = encoded.clone();
    let expiry_start = missing_expiry.len() - 32 - 8;
    missing_expiry[expiry_start..expiry_start + 8].fill(0);
    assert!(decode_action_contract_v1(&missing_expiry).is_err());

    let mut huge_body = encoded;
    huge_body[10..14].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode_action_contract_v1(&huge_body).is_err());
}

#[test]
fn every_disposition_wire_code_round_trips_and_unknown_four_fails() {
    let cases = [
        (
            ActionDispositionV1::Silence,
            token("silence"),
            set(&[], 32),
            0u8,
        ),
        (ActionDispositionV1::Speech, token("say"), set(&[], 32), 1u8),
        (
            ActionDispositionV1::ToolPlan,
            token("plan"),
            set(&["tool"], 32),
            2u8,
        ),
        (
            ActionDispositionV1::SpeechAndToolPlan,
            token("plan"),
            set(&["tool"], 32),
            3u8,
        ),
    ];
    for (disposition, speech_act, tools, code) in cases {
        let contract = contract_with(
            disposition,
            speech_act,
            ActionRequirementsV1::new(set(&[], 64), set(&[], 64), set(&[], 64), set(&[], 64)),
            tools,
            set(&[], 32),
        );
        let mut encoded = encode_action_contract_v1(&contract).expect("encode disposition");
        assert_eq!(encoded[14 + 120], code);
        assert_eq!(
            decode_action_contract_v1(&encoded).expect("decode disposition"),
            contract
        );
        encoded[14 + 120] = 4;
        assert!(decode_action_contract_v1(&encoded).is_err());
    }
}

#[test]
fn all_fixed_sets_cover_empty_one_max_and_max_plus_one() {
    let requirement_cases = [
        ("must", 0usize),
        ("should", 1usize),
        ("may", 2usize),
        ("must-not", 3usize),
    ];
    for (prefix, selected) in requirement_cases {
        for count in [0usize, 1, 64, 65] {
            let sets = [
                generated_set(if selected == 0 { count } else { 0 }, 64, "must"),
                generated_set(if selected == 1 { count } else { 0 }, 64, "should"),
                generated_set(if selected == 2 { count } else { 0 }, 64, "may"),
                generated_set(if selected == 3 { count } else { 0 }, 64, "must-not"),
            ];
            let contract = contract_with(
                ActionDispositionV1::SpeechAndToolPlan,
                token("plan"),
                ActionRequirementsV1::new(
                    sets[0].clone(),
                    sets[1].clone(),
                    sets[2].clone(),
                    sets[3].clone(),
                ),
                set(&["tool"], 32),
                set(&[], 32),
            );
            let result = encode_action_contract_v1(&contract);
            assert_eq!(
                result.is_ok(),
                count <= 64,
                "requirements.{prefix} count {count}"
            );
        }
    }

    for (prefix, allowed_tools, max) in [
        ("allowed-tools", true, 32usize),
        ("allowed-disclosures", false, 32usize),
    ] {
        for count in [0usize, 1, 32, 33] {
            let tools = if allowed_tools {
                if count == 0 {
                    set(&[], 32)
                } else {
                    generated_set(count, 32, prefix)
                }
            } else {
                set(&["tool"], 32)
            };
            let disclosures = if allowed_tools || count == 0 {
                set(&[], 32)
            } else {
                generated_set(count, 32, prefix)
            };
            let disposition = if allowed_tools && count == 0 {
                ActionDispositionV1::Speech
            } else {
                ActionDispositionV1::SpeechAndToolPlan
            };
            let contract = contract_with(
                disposition,
                token("say"),
                ActionRequirementsV1::new(set(&[], 64), set(&[], 64), set(&[], 64), set(&[], 64)),
                tools,
                disclosures,
            );
            let result = encode_action_contract_v1(&contract);
            assert_eq!(result.is_ok(), count <= max, "{prefix} count {count}");
        }
    }
}

#[test]
fn zero_state_constitution_and_turn_digests_are_rejected_on_wire() {
    let encoded = encode_action_contract_v1(&fixture()).expect("encode");
    for start in [14usize + 16, 14 + 16 + 32 + 8, 14 + 16 + 32 + 8 + 32] {
        let mut zero_digest = encoded.clone();
        zero_digest[start..start + 32].fill(0);
        assert!(
            decode_action_contract_v1(&zero_digest).is_err(),
            "offset {start}"
        );
    }
}

#[test]
fn speech_and_ordinary_token_length_boundaries_are_enforced() {
    let speech_offset = 14 + 16 + 32 + 8 + 32 + 32 + 1;
    let encoded = encode_action_contract_v1(&fixture()).expect("encode");
    let mut speech_zero = encoded.clone();
    speech_zero[speech_offset..speech_offset + 2].copy_from_slice(&0u16.to_le_bytes());
    assert!(decode_action_contract_v1(&speech_zero).is_err());
    let mut speech_65 = encoded.clone();
    speech_65[speech_offset..speech_offset + 2].copy_from_slice(&65u16.to_le_bytes());
    assert!(decode_action_contract_v1(&speech_65).is_err());

    let speech_64 = CanonicalTokenV1::new("a".repeat(64), 128).expect("speech 64");
    let contract_64 = contract_with(
        ActionDispositionV1::SpeechAndToolPlan,
        speech_64,
        ActionRequirementsV1::new(set(&[], 64), set(&[], 64), set(&[], 64), set(&[], 64)),
        set(&["tool"], 32),
        set(&[], 32),
    );
    assert!(decode_action_contract_v1(
        &encode_action_contract_v1(&contract_64).expect("encode 64")
    )
    .is_ok());

    for length in [128usize, 129] {
        let ordinary = CanonicalTokenV1::new("a".repeat(length), 200).expect("ordinary token");
        let contract = contract_with(
            ActionDispositionV1::SpeechAndToolPlan,
            token("plan"),
            ActionRequirementsV1::new(
                CanonicalTokenSetV1::new(vec![ordinary], 64).expect("ordinary set"),
                set(&[], 64),
                set(&[], 64),
                set(&[], 64),
            ),
            set(&["tool"], 32),
            set(&[], 32),
        );
        assert_eq!(encode_action_contract_v1(&contract).is_ok(), length == 128);
    }
}
