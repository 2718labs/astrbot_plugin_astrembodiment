use ae_contracts::*;
#[test]
fn vendored_clock_matches_host_sample_and_kind11_is_separate() {
    let frozen = freeze_embodiment_time_v1("America/Los_Angeles", 1773000000000).unwrap();
    assert_eq!(frozen.persona_utc_offset_seconds, -25200);
    assert_eq!(frozen.persona_local_minute, 780);
    assert_eq!(frozen.persona_day_ordinal, 20520);
    assert_eq!(frozen.next_timezone_transition_utc_ms, Some(1793523600000));
    assert!(frozen.validate_v1());
    assert_eq!(
        freeze_embodiment_time_v1("UTC-07:00", 1773000000000)
            .unwrap()
            .next_timezone_transition_utc_ms,
        None
    );
    assert!(canonical_embodiment_timezone("UTC+00:00").is_err());
    let mut request = EmbodimentTimeAdvanceRequestV1 {
        schema_version: 1,
        operation_id: [0; 16],
        scope: PersonaScopeRef {
            bot_token: [1; 16],
            persona_token: [2; 16],
        },
        profile_revision: 1,
        schedule_revision: 1,
        frozen,
    };
    request.operation_id = request.expected_operation_id();
    let bytes = request.encode_wire_v1().unwrap();
    assert_eq!(
        EmbodimentTimeAdvanceRequestV1::decode_wire_v1(&bytes).unwrap(),
        request
    );
    assert!(wire::decode_event(&bytes).is_err());
}
