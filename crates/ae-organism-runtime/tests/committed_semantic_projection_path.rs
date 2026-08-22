// Reuse the established typed source fixtures; this test adds the atomic
// prepared semantic-transition origin and the pre-output boundary.
#[allow(unused_macros)]
macro_rules! committed_semantic_projection_path_test_contents {
    () => {
        private_projection_runtime_test_contents!();

        use crate::r7::{
            discard_private_projection_transfer_v1, AstrRuntime, NativeProjectionPayloadProducerV1,
            PreOutputProjectionUpdateV1, PrivateProjectionPayloadWireErrorV1,
            PrivateProjectionTransferReceiptV1, R7PreOutputProjectionInputV1, RuntimeError,
        };
        use ae_contracts::r7::{CanonicalEvent, UserStimulus};

        #[derive(Clone, Copy)]
        struct MatchingSemanticBinding {
            revision: u64,
            state_after: Digest,
            turn_id: Id128,
            scope_digest: Digest,
            turn_binding: Digest,
        }

        fn matching_semantic_binding(
            runtime: &AstrRuntime,
            event: &CanonicalEvent,
        ) -> Result<MatchingSemanticBinding, RuntimeError> {
            let binding = runtime.semantic_projection_binding_for_test(event)?;
            Ok(MatchingSemanticBinding {
                revision: binding.revision,
                state_after: binding.state_after,
                turn_id: binding.turn_id,
                scope_digest: binding.scope_digest,
                turn_binding: binding.turn_binding,
            })
        }

        fn scope_ref_for(scope_digest: &Digest) -> String {
            const HEX: &[u8; 16] = b"0123456789abcdef";
            let mut value = String::with_capacity("native-scope-v1:".len() + 64);
            value.push_str("native-scope-v1:");
            for byte in scope_digest {
                value.push(char::from(HEX[usize::from(byte >> 4)]));
                value.push(char::from(HEX[usize::from(byte & 0x0f)]));
            }
            value
        }

        fn user_event(event_id: Id128, positive: i64) -> CanonicalEvent {
            CanonicalEvent::UserStimulus(UserStimulus {
                event_id,
                scope: ScopeRef {
                    bot_token: [62; 16],
                    persona_token: [63; 16],
                    relation_token: Some([64; 16]),
                    session_token: [65; 16],
                },
                causal: CausalRef {
                    turn_id: TURN_ID,
                    action_id: None,
                    delivery_id: None,
                    claim_id: None,
                    base_revision: 0,
                },
                observed_at_ms: 1_000,
                evidence: SemanticEstimate {
                    schema_version: 1,
                    dimensions: EvidenceVector {
                        positive: Fixed::from_raw(300_000 + positive),
                        harm: Fixed::from_raw(100_000),
                        epistemic_conflict: Fixed::from_raw(200_000),
                        boundary: Fixed::from_raw(150_000),
                        ..EvidenceVector::default()
                    },
                    estimator_confidence: Fixed::from_raw(800_000),
                    estimator_digest: digest(66),
                },
            })
        }

        fn pre_output_update(
            identity: IdentityConstitutionV1,
            transition: &MatchingSemanticBinding,
            action_source_state_override: Option<Digest>,
            organism_turn_id_override: Option<Id128>,
            action_turn_binding_override: Option<Digest>,
            scope_ref_override: Option<String>,
        ) -> PreOutputProjectionUpdateV1 {
            let revision = transition.revision;
            let source_state_digest = transition.state_after;
            let causal_turn_binding = transition.turn_binding;
            let scope_ref =
                scope_ref_override.unwrap_or_else(|| scope_ref_for(&transition.scope_digest));
            let identity_digest = *identity.constitution_digest();
            let soma = SomaStateV1::new_bound_to_source_state(
                format!("soma:snapshot:{revision}"),
                128,
                revision,
                identity_digest,
                soma_field("energy_availability", 0.7),
                soma_field("mobilization_balance", 0.6),
                soma_field("endocrine_load", 0.2),
                soma_field("repair_pressure", 0.3),
                soma_field("circadian_phase", 0.4),
                source_state_digest,
            )
            .expect("SOMA remains distinct while binding the committed semantic source");
            assert_ne!(soma.state_digest(), &source_state_digest);
            let organism = OrganismSnapshotRefV1::new(
                text(&format!("organism:snapshot:{revision}"), 128),
                source_state_digest,
                causal_turn_binding,
                organism_turn_id_override.unwrap_or(transition.turn_id),
                TurnV1::new(
                    text(&format!("turn:{revision}"), 128),
                    text("incarnation:1", 128),
                    text(&scope_ref, 128),
                )
                .expect("turn"),
            )
            .expect("organism reference bound to the committed source state");
            let cognitive = CognitiveKvViewV1::new(
                text(&format!("continuum:view:{revision}"), 128),
                digest(21),
                legacy_subjective_present(),
                EpistemicsV1::new(fields("claim", "verified", 22)).expect("epistemics"),
                PraxisV1::new(fields("objective", "answer_current_turn", 23)).expect("praxis"),
            )
            .expect("cognitive view");
            let anchors = ExactTurnAnchorsV1::new(list(
                vec![ExactAnchorV1::new(
                    ExactAnchorKindV1::ActiveSafetyRequirement,
                    text("anchor:safety:1", 128),
                    text("do_not_disclose_private_control", 512),
                    digest(31),
                )
                .expect("anchor")],
                64,
            ))
            .expect("anchors");
            let relation = RelationScopeV1::new(
                text(&scope_ref, 128),
                RelationV1::new(fields("boundary", "current_scope_only", 52)).expect("relation"),
            )
            .expect("relation scope");
            let affordances = AffordanceCatalogV1::new(
                text(&scope_ref, 128),
                list(
                    vec![AffordanceV1::new(
                        text("answer", 64),
                        text("emit_bounded_answer", 256),
                        digest(81),
                        digest(82),
                    )
                    .expect("affordance")],
                    64,
                ),
            )
            .expect("affordances");
            let provider = ProviderProfileV1::new(text("provider:fixture", 128), 3_200)
                .expect("provider profile");
            let action = committed_action_contract(
                identity_digest,
                revision,
                action_source_state_override.unwrap_or(source_state_digest),
                action_turn_binding_override.unwrap_or(causal_turn_binding),
            );
            let soma_ingress = SomaClassificationIngressV1::new(
                *soma.state_digest(),
                revision,
                identity_digest,
                vec![CallerProvidedClassificationV1::new(
                    SomaSubjectiveAxisV1::Energy,
                    SubjectiveBandV1::Moderate,
                    SubjectiveTrendV1::Stable,
                    "prefer_bounded_effort".to_owned(),
                    DisclosureV1::BehavioralOnly,
                    ConfidenceV1::High,
                    Some(format!("soma:snapshot:{revision}")),
                )
                .expect("caller-provided soma classification")],
                8,
            )
            .expect("soma classification ingress");
            let epistemic = epistemic_projection(identity, source_state_digest, revision);

            let update = PreOutputProjectionUpdateV1::new(
                revision,
                BoundedProjectionReferencesV1::new(
                    capsule(
                        ProjectionSourceKindV1::OrganismSnapshot,
                        1,
                        revision,
                        source_state_digest,
                        organism,
                    ),
                    capsule(
                        ProjectionSourceKindV1::CognitiveKvView,
                        2,
                        revision,
                        digest(20),
                        cognitive,
                    ),
                    capsule(
                        ProjectionSourceKindV1::ExactTurnAnchors,
                        3,
                        revision,
                        digest(30),
                        anchors,
                    ),
                    capsule(
                        ProjectionSourceKindV1::RelationScope,
                        5,
                        revision,
                        digest(50),
                        relation,
                    ),
                    capsule(
                        ProjectionSourceKindV1::AffordanceCatalog,
                        8,
                        revision,
                        digest(80),
                        affordances,
                    ),
                    capsule(
                        ProjectionSourceKindV1::ProviderProfile,
                        9,
                        revision,
                        digest(90),
                        provider,
                    ),
                ),
                capsule(
                    ProjectionSourceKindV1::ActionContract,
                    6,
                    revision,
                    *action.contract_digest(),
                    action,
                ),
                capsule(
                    ProjectionSourceKindV1::SomaState,
                    10,
                    revision,
                    *soma.state_digest(),
                    soma,
                ),
                soma_ingress,
                epistemic,
                ProjectionPreconditionsV1::new(1_000, 0, 0, 7, 7, 0, 512),
            );
            update
        }

        fn committed_action_contract(
            identity_digest: Digest,
            revision: u64,
            source_state_digest: Digest,
            turn_binding: Digest,
        ) -> ActionContractV1 {
            ActionContractV1::from_evaluation(
                ACTION_ID,
                turn_binding,
                revision,
                source_state_digest,
                identity_digest,
                ActionDispositionV1::Speech,
                token("answer"),
                ActionRequirementsV1::new(
                    token_set(&["respect_exact_boundary"]),
                    token_set(&[]),
                    token_set(&[]),
                    token_set(&["invent_memory"]),
                ),
                token_set(&[]),
                token_set(&["correction"]),
                unit(800_000),
                2_000,
            )
            .expect("typed action contract")
        }

        fn atomically_compile(
            runtime: &mut AstrRuntime,
            identity: IdentityConstitutionV1,
            event: &CanonicalEvent,
        ) -> Result<crate::r7::RuntimeDecision, RuntimeError> {
            let binding = matching_semantic_binding(runtime, event)?;
            let update = pre_output_update(identity.clone(), &binding, None, None, None, None);
            let input = R7PreOutputProjectionInputV1::new(identity_capsule(identity), update)
                .expect("immutable identity");
            runtime.apply_user_stimulus_with_private_projection_wire_v1(event, &input)
        }

        #[test]
        fn prepared_semantic_transition_commits_only_with_a_matching_pre_output_wire() {
            let constitution = identity(41);
            let mut runtime = AstrRuntime::scaffold();
            let decision =
                atomically_compile(&mut runtime, constitution.clone(), &user_event([61; 16], 0))
                    .expect("typed prepared transition seals before final commit");
            assert_eq!(runtime.current_revision(), 1);
            assert_ne!(decision.receipt.state_before, decision.receipt.state_after);
            assert_ne!(decision.receipt.event_digest, [0; 32]);
            assert_ne!(decision.receipt.authority_digest, [0; 32]);
            let mut wire = decision.into_private_projection_wire();
            let mut repeated_runtime = AstrRuntime::scaffold();
            let repeated_wire_digest = *atomically_compile(
                &mut repeated_runtime,
                constitution.clone(),
                &user_event([61; 16], 0),
            )
            .expect("same closed evidence has a deterministic prepared wire")
            .into_private_projection_wire()
            .wire_digest();
            assert_eq!(
                wire.wire_digest(),
                &repeated_wire_digest,
                "same closed evidence produces the same AER7PPW1 wire"
            );
            let transfer = wire
                .begin_transfer_once_v1()
                .expect("exactly one crate-private native transfer");
            assert!(matches!(
                wire.begin_transfer_once_v1(),
                Err(PrivateProjectionPayloadWireErrorV1::AlreadyConsumed)
            ));
            assert_eq!(
                discard_private_projection_transfer_v1(transfer),
                PrivateProjectionTransferReceiptV1::Discarded
            );
            assert!(matches!(
                atomically_compile(&mut runtime, constitution, &user_event([61; 16], 0)),
                Err(RuntimeError::UserStimulusBaseRevisionMismatch)
            ));
            assert_eq!(runtime.current_revision(), 1);
        }

        #[test]
        fn mismatched_pre_output_source_cannot_advance_the_target_runtime() {
            let constitution = identity(41);
            let event = user_event([61; 16], 0);
            let mut target_runtime = AstrRuntime::scaffold();
            let field_before = target_runtime.field.potential.clone();
            let binding = matching_semantic_binding(&target_runtime, &event)
                .expect("derive the matching binding");
            let update = pre_output_update(
                constitution.clone(),
                &binding,
                Some(digest(99)),
                None,
                None,
                None,
            );
            let input = R7PreOutputProjectionInputV1::new(identity_capsule(constitution), update)
                .expect("identity");
            assert!(matches!(
                target_runtime.apply_user_stimulus_with_private_projection_wire_v1(&event, &input),
                Err(RuntimeError::PrivateProjectionWireUnavailable)
            ));
            assert_eq!(target_runtime.current_revision(), 0);
            assert_eq!(target_runtime.field.potential, field_before);
        }

        #[test]
        fn failed_pre_output_source_compilation_does_not_consume_a_runtime_transition() {
            let constitution = identity(41);
            let event = user_event([61; 16], 0);
            let mut runtime = AstrRuntime::scaffold();
            let field_before = runtime.field.potential.clone();
            let binding =
                matching_semantic_binding(&runtime, &event).expect("derive the matching binding");
            let update = pre_output_update(
                constitution.clone(),
                &binding,
                Some(digest(99)),
                None,
                None,
                None,
            );
            let input = R7PreOutputProjectionInputV1::new(identity_capsule(constitution), update)
                .expect("identity");
            assert!(matches!(
                runtime.apply_user_stimulus_with_private_projection_wire_v1(&event, &input),
                Err(RuntimeError::PrivateProjectionWireUnavailable)
            ));
            assert_eq!(runtime.current_revision(), 0);
            assert_eq!(runtime.field.potential, field_before);
        }

        #[test]
        fn failed_r7_input_leaves_the_same_runtime_retryable_then_good_input_commits() {
            let constitution = identity(41);
            let event = user_event([61; 16], 0);
            let mut runtime = AstrRuntime::scaffold();
            let field_before = runtime.field.potential.clone();
            let binding =
                matching_semantic_binding(&runtime, &event).expect("derive the matching binding");
            let rejected = pre_output_update(
                constitution.clone(),
                &binding,
                Some(digest(99)),
                None,
                None,
                None,
            );
            let rejected_input =
                R7PreOutputProjectionInputV1::new(identity_capsule(constitution.clone()), rejected)
                    .expect("identity");
            assert!(matches!(
                runtime
                    .apply_user_stimulus_with_private_projection_wire_v1(&event, &rejected_input),
                Err(RuntimeError::PrivateProjectionWireUnavailable)
            ));
            assert_eq!(runtime.current_revision(), 0);
            assert_eq!(runtime.field.potential, field_before);

            let update = pre_output_update(constitution.clone(), &binding, None, None, None, None);
            let input = R7PreOutputProjectionInputV1::new(identity_capsule(constitution), update)
                .expect("identity");
            runtime
                .apply_user_stimulus_with_private_projection_wire_v1(&event, &input)
                .expect("the same runtime remains retryable after a rejected input");
            assert_eq!(runtime.current_revision(), 1);
        }

        #[test]
        fn new_r7_commit_does_not_read_or_advance_a_legacy_producer_watermark() {
            let constitution = identity(41);
            let mut runtime = AstrRuntime::scaffold();
            let legacy =
                NativeProjectionPayloadProducerV1::new(identity_capsule(constitution.clone()))
                    .expect("legacy producer owns its own independent watermark");

            atomically_compile(&mut runtime, constitution, &user_event([61; 16], 0))
                .expect("new R7 semantic path commits its own revision");

            assert_eq!(runtime.current_revision(), 1);
            assert_eq!(legacy.current_revision(), None);
        }

        #[test]
        fn migrated_legacy_distinct_evidence_cannot_reuse_a_semantic_transition() {
            let constitution = identity(41);
            let event = user_event([61; 16], 0);
            let mut runtime = AstrRuntime::scaffold();
            let field_before = runtime.field.potential.clone();
            let binding =
                matching_semantic_binding(&runtime, &event).expect("derive the matching binding");
            let mismatched_update =
                pre_output_update(constitution.clone(), &binding, None, None, None, None);
            let mismatched_input = R7PreOutputProjectionInputV1::new(
                identity_capsule(constitution.clone()),
                mismatched_update,
            )
            .expect("identity");

            assert!(matches!(
                runtime.apply_user_stimulus_with_private_projection_wire_v1(
                    &user_event([61; 16], 100_000),
                    &mismatched_input,
                ),
                Err(RuntimeError::PrivateProjectionWireUnavailable)
            ));
            assert_eq!(runtime.current_revision(), 0);
            assert_eq!(runtime.field.potential, field_before);

            atomically_compile(&mut runtime, constitution, &event)
                .expect("the original matching evidence remains retryable");
            assert_eq!(runtime.current_revision(), 1);
        }

        #[test]
        fn migrated_legacy_causal_binding_mismatches_do_not_consume_the_transition() {
            let constitution = identity(41);
            let event = user_event([61; 16], 0);

            for (turn_override, scope_override) in [
                (Some([98; 16]), None),
                (None, Some("native-scope-v1:wrong".to_owned())),
            ] {
                let mut runtime = AstrRuntime::scaffold();
                let field_before = runtime.field.potential.clone();
                let binding = matching_semantic_binding(&runtime, &event)
                    .expect("derive the matching binding");
                let update = pre_output_update(
                    constitution.clone(),
                    &binding,
                    None,
                    turn_override,
                    None,
                    scope_override,
                );
                let input = R7PreOutputProjectionInputV1::new(
                    identity_capsule(constitution.clone()),
                    update,
                )
                .expect("identity");

                assert!(matches!(
                    runtime.apply_user_stimulus_with_private_projection_wire_v1(&event, &input),
                    Err(RuntimeError::PrivateProjectionWireUnavailable)
                ));
                assert_eq!(runtime.current_revision(), 0);
                assert_eq!(runtime.field.potential, field_before);
            }
        }
    };
}
