#[allow(unused_macros)]
macro_rules! astrbot_v4273_tool_private_boundary_test_contents {
    () => {
        use crate::r7::{
            AstrBotPublicSignalV1, AstrBotToolDispositionV1, AstrBotToolIngressV1, AstrRuntime,
            RuntimeError,
        };
        use ae_contracts::r7::wire;

        const NATIVE_CANARY: &str = "[[AE_NATIVE_PUBLIC_EFFECT_V1]]";

        fn ingress(
            epoch: [u8; 32],
            turn: [u8; 32],
            base_revision: u64,
            observed_at_ms: u64,
            text: &str,
        ) -> AstrBotToolIngressV1 {
            let mut ingress = AstrBotToolIngressV1 {
                schema_version: 1,
                invocation_id: [0; 32],
                process_epoch_id: epoch,
                adapter_binding: [2; 32],
                session_binding: [3; 32],
                turn_binding: turn,
                event_binding: [4; 32],
                observed_at_ms,
                base_revision,
                current_event_text: text.to_owned(),
            };
            ingress.invocation_id = ingress.recompute_invocation_id();
            ingress
        }

        #[test]
        fn v4273_tool_native_boundary_is_typed_bounded_and_private() {
            let mut runtime = AstrRuntime::scaffold();
            let positive = ingress([1; 32], [5; 32], 0, 1_000, NATIVE_CANARY);
            let observed = runtime
                .apply_astrbot_tool_v1(positive.clone())
                .expect("valid canary ingress");

            assert_eq!(observed.schema_version, 1);
            assert_eq!(observed.invocation_id, positive.invocation_id);
            assert_eq!(observed.process_epoch_id, positive.process_epoch_id);
            assert_eq!(observed.adapter_binding, positive.adapter_binding);
            assert_eq!(observed.session_binding, positive.session_binding);
            assert_eq!(observed.turn_binding, positive.turn_binding);
            assert_eq!(observed.event_binding, positive.event_binding);
            assert_eq!(observed.revision, 1);
            assert_eq!(observed.disposition, AstrBotToolDispositionV1::PublicSignal);
            assert_eq!(
                observed.public_signal,
                Some(AstrBotPublicSignalV1::Observed)
            );
            observed.validate_shape().expect("valid typed outcome");

            let repeated = runtime
                .apply_astrbot_tool_v1(positive)
                .expect("unexpired duplicate returns cached outcome");
            assert_eq!(repeated, observed);
            assert_eq!(runtime.current_revision(), 1);

            let ordinary = ingress([1; 32], [6; 32], 1, 1_001, "ordinary current turn");
            let silence = runtime
                .apply_astrbot_tool_v1(ordinary)
                .expect("ordinary ingress is valid");
            assert_eq!(silence.revision, 2);
            assert_eq!(silence.disposition, AstrBotToolDispositionV1::Silence);
            assert_eq!(silence.public_signal, None);
            silence.validate_shape().expect("valid typed silence");
        }

        #[test]
        fn v4273_invocation_domain_uses_sha256_text_leaf_and_big_endian_revision() {
            let value = ingress(
                [0x11; 32],
                [0x44; 32],
                0x0102_0304_0506_0708,
                9_000,
                "v4.27.3 digest fixture",
            );
            let text_sha256 = [
                0x57, 0xee, 0x01, 0x5b, 0x35, 0xa6, 0xc0, 0x0b, 0x10, 0x49, 0x00, 0x25, 0x4a, 0x65,
                0xde, 0x49, 0x01, 0x0b, 0x14, 0x51, 0x69, 0x20, 0x5d, 0xc2, 0x8a, 0xd5, 0x0f, 0x2b,
                0xec, 0x10, 0xd7, 0xf3,
            ];
            let revision_be = value.base_revision.to_be_bytes();
            let expected = wire::domain_hash(
                b"astr-embodiment/astrbot-v4273-tool-invocation-v1",
                &[
                    &value.process_epoch_id,
                    &value.adapter_binding,
                    &value.session_binding,
                    &value.turn_binding,
                    &value.event_binding,
                    &text_sha256,
                    &revision_be,
                ],
            );
            let stale_domain = wire::domain_hash(
                b"astr-embodiment/astrbot-v4268-tool-invocation-v1",
                &[
                    &value.process_epoch_id,
                    &value.adapter_binding,
                    &value.session_binding,
                    &value.turn_binding,
                    &value.event_binding,
                    &text_sha256,
                    &revision_be,
                ],
            );

            assert_eq!(value.invocation_id, expected);
            assert_ne!(value.invocation_id, stale_domain);
        }

        #[test]
        fn v4273_tool_rejects_conflict_invalid_epoch_expiry_and_revision() {
            let mut runtime = AstrRuntime::scaffold();
            let first = ingress([1; 32], [5; 32], 0, 1_000, "ordinary current turn");
            runtime
                .apply_astrbot_tool_v1(first.clone())
                .expect("first valid evaluation");

            let mut conflict = ingress([1; 32], [5; 32], 0, 1_001, "different private text");
            conflict.invocation_id = first.invocation_id;
            assert_eq!(
                runtime.apply_astrbot_tool_v1(conflict),
                Err(RuntimeError::AstrBotToolIdentityConflict)
            );

            let mut bad_identity = ingress([1; 32], [6; 32], 1, 1_001, "ordinary current turn");
            bad_identity.turn_binding = [7; 32];
            assert_eq!(
                runtime.apply_astrbot_tool_v1(bad_identity),
                Err(RuntimeError::InvalidAstrBotToolIngress)
            );

            let old_epoch = ingress([2; 32], [6; 32], 1, 1_001, "ordinary current turn");
            assert_eq!(
                runtime.apply_astrbot_tool_v1(old_epoch),
                Err(RuntimeError::AstrBotToolProcessEpochMismatch)
            );

            let stale_revision = ingress([1; 32], [6; 32], 0, 1_001, "ordinary current turn");
            assert_eq!(
                runtime.apply_astrbot_tool_v1(stale_revision),
                Err(RuntimeError::AstrBotToolBaseRevisionMismatch)
            );

            let mut expired = first;
            expired.observed_at_ms = 31_001;
            assert_eq!(
                runtime.apply_astrbot_tool_v1(expired),
                Err(RuntimeError::AstrBotToolInvocationExpired)
            );

            let invalid_nul = ingress([1; 32], [6; 32], 1, 1_001, "invalid\0text");
            assert_eq!(
                runtime.apply_astrbot_tool_v1(invalid_nul),
                Err(RuntimeError::InvalidAstrBotToolIngress)
            );
        }

        #[test]
        fn v4273_tool_enforces_exact_text_and_shape_bounds() {
            let exact_utf8_limit = ingress([1; 32], [5; 32], 0, 1, &"\u{10ffff}".repeat(16_384));
            assert_eq!(exact_utf8_limit.current_event_text.len(), 65_536);
            assert!(exact_utf8_limit.validate_shape().is_ok());

            let scalar_overflow = ingress([1; 32], [5; 32], 0, 1, &"a".repeat(16_385));
            assert!(scalar_overflow.validate_shape().is_err());

            let mut zero_binding = ingress([1; 32], [5; 32], 0, 1, "ordinary current turn");
            zero_binding.event_binding = [0; 32];
            zero_binding.invocation_id = zero_binding.recompute_invocation_id();
            assert!(zero_binding.validate_shape().is_err());

            let observed_at_zero = ingress([1; 32], [5; 32], 0, 0, "ordinary current turn");
            assert!(observed_at_zero.validate_shape().is_err());
        }

        #[test]
        fn v4273_tool_registry_refuses_unexpired_eviction_but_prunes_expired_records() {
            let mut runtime = AstrRuntime::scaffold();
            for index in 0..1_024_u64 {
                let mut turn = [0; 32];
                turn[..8].copy_from_slice(&(index + 1).to_be_bytes());
                let entry = ingress([1; 32], turn, index, 10_000, "ordinary current turn");
                runtime
                    .apply_astrbot_tool_v1(entry)
                    .expect("registry entry within capacity");
            }

            let mut overflow_turn = [0; 32];
            overflow_turn[..8].copy_from_slice(&1_025_u64.to_be_bytes());
            let overflow = ingress(
                [1; 32],
                overflow_turn,
                1_024,
                10_000,
                "ordinary current turn",
            );
            assert_eq!(
                runtime.apply_astrbot_tool_v1(overflow),
                Err(RuntimeError::AstrBotToolRegistryFull)
            );
            assert_eq!(runtime.current_revision(), 1_024);

            let after_expiry = ingress([1; 32], [0x77; 32], 1_024, 40_001, "ordinary current turn");
            runtime
                .apply_astrbot_tool_v1(after_expiry)
                .expect("expired records may be pruned");
            assert_eq!(runtime.current_revision(), 1_025);
        }

        #[test]
        fn v4273_tool_registry_and_public_dtos_exclude_raw_text() {
            const PRIVATE_REGISTRY_SENTINEL: &str =
                "PRIVATE_REGISTRY_7be1f2557ee84c978f3f675fbccf8379";
            let private_ingress = ingress([1; 32], [5; 32], 0, 1_000, PRIVATE_REGISTRY_SENTINEL);
            let ingress_repr = format!("{private_ingress:?}");
            assert!(!ingress_repr.contains(PRIVATE_REGISTRY_SENTINEL));

            let mut runtime = AstrRuntime::scaffold();
            let outcome = runtime
                .apply_astrbot_tool_v1(private_ingress)
                .expect("private ordinary text maps to typed silence");
            assert_eq!(outcome.disposition, AstrBotToolDispositionV1::Silence);
            assert!(!format!("{outcome:?}").contains(PRIVATE_REGISTRY_SENTINEL));
            assert!(!format!("{runtime:?}").contains(PRIVATE_REGISTRY_SENTINEL));
        }
    };
}
