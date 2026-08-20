// Migrated from the removed callback-based R7 API. These regressions keep the
// old user-stimulus atomicity assertions against the only supported typed-input
// entrypoint, while committed_semantic_projection_path.rs covers the matching
// successful payload path.

#[allow(unused_macros)]
macro_rules! user_stimulus_state_transition_test_contents {
    () => {
        use ae_action_contract::{
            ActionContractV1, ActionDispositionV1, ActionRequirementsV1, CanonicalTokenSetV1,
            CanonicalTokenV1, UnitIntervalV1,
        };
        use ae_cognitive_envelope::{
            AffordanceCatalogV1, AffordanceV1, BoundedListV1, BoundedTextV1, CapsuleFieldV1,
            CognitiveKvViewV1, EpistemicsV1, ExactAnchorKindV1, ExactAnchorV1, ExactTurnAnchorsV1,
            OrganismSnapshotRefV1, PraxisV1, ProjectionPreconditionsV1, ProjectionSourceKindV1,
            ProviderProfileV1, RelationScopeV1, RelationV1, SourceCapsuleV1, SourceProvenanceV1,
            TurnV1,
        };
        use ae_contracts::r7::{
            CausalRef, Digest, EvidenceVector, Id128, ScopeRef, SemanticEstimate, VerdictKind,
        };
        use ae_epistemic_state::{
            compile_epistemic_projection_v1, CallerProvidedEpistemicClassificationV1,
            EpistemicEvidenceGapV1, EpistemicProjectionInputV1, EpistemicSourceBindingV1,
            VerifierNeedV1,
        };
        use ae_fixed::Fixed;
        use ae_genesis::r7::{
            AntiGoalsV1, CorrectionBoundaryConstitutionV1, ExpressionBasisV1, IdentityBoundsV1,
            IdentityConstitutionV1, IncarnationRefV1, OperationalCommitmentsV1,
            RelationalPlayLimitsV1, SeedCodeV1,
        };
        use ae_soma::{
            BoundedSomaSignalV1, CallerProvidedClassificationV1, SomaClassificationIngressV1,
            SomaFieldSetV1, SomaStateV1, SomaSubjectiveAxisV1,
        };
        use ae_subjective_present::{
            ConfidenceV1, DisclosureV1, SubjectiveBandV1, SubjectivePresentInputV1,
            SubjectivePresentProjectionV1, SubjectivePresentV1, SubjectiveTrendV1,
        };

        // This fixture is included both by focused organism tests and by an unchanged
        // PyO3 regression fixture. Each consuming target needs a different subset.
        const ACTION_ID: Id128 = [51; 16];
        const TURN_ID: Id128 = [12; 16];

        fn digest(seed: u8) -> Digest {
            [seed; 32]
        }

        fn text(value: &str, max_chars: u32) -> BoundedTextV1 {
            BoundedTextV1::new(value.to_owned(), max_chars).expect("bounded fixture token")
        }

        fn list<T>(items: Vec<T>, max_items: u16) -> BoundedListV1<T> {
            BoundedListV1::new(items, max_items).expect("bounded fixture list")
        }

        fn fields(name: &str, value: &str, seed: u8) -> BoundedListV1<CapsuleFieldV1> {
            list(
                vec![
                    CapsuleFieldV1::new(text(name, 64), text(value, 256), digest(seed))
                        .expect("source-bound field"),
                ],
                8,
            )
        }

        pub(crate) fn identity(seed: u8) -> IdentityConstitutionV1 {
            let bounds = IdentityBoundsV1::new(8, 64).expect("identity bounds");
            let seed_code = SeedCodeV1::new(
                "persona.genesis.seed.v1".to_owned(),
                64,
                digest(seed),
                digest(seed + 1),
            )
            .expect("seed code");
            let incarnation =
                IncarnationRefV1::derive(&seed_code, digest(seed + 2), 1).expect("incarnation");
            IdentityConstitutionV1::derive(
                &incarnation,
                OperationalCommitmentsV1::new(
                    vec!["accept_verified_correction".to_owned()],
                    bounds,
                )
                .expect("commitments"),
                AntiGoalsV1::new(vec!["avoid_invented_memory".to_owned()], bounds)
                    .expect("anti-goals"),
                ExpressionBasisV1::new(vec!["directness:high".to_owned()], bounds)
                    .expect("expression"),
                CorrectionBoundaryConstitutionV1::new(
                    vec!["respect_explicit_boundary".to_owned()],
                    bounds,
                )
                .expect("boundaries"),
                RelationalPlayLimitsV1::new(vec!["honor_current_scope".to_owned()], bounds)
                    .expect("relational limits"),
            )
            .expect("identity constitution")
        }

        fn capsule<T>(
            kind: ProjectionSourceKindV1,
            seed: u8,
            revision: u64,
            content_digest: Digest,
            value: T,
        ) -> SourceCapsuleV1<T> {
            SourceCapsuleV1::new(
                SourceProvenanceV1::new(
                    kind,
                    text(&format!("source:{seed}"), 128),
                    revision,
                    digest(seed + 100),
                )
                .expect("provenance"),
                content_digest,
                value,
            )
            .expect("capsule")
        }

        fn legacy_subjective_present() -> SubjectivePresentProjectionV1 {
            let item = SubjectivePresentV1::try_from_input(SubjectivePresentInputV1 {
                axis: "irritation".to_owned(),
                band: SubjectiveBandV1::Moderate,
                trend: SubjectiveTrendV1::Stable,
                behavioral_effect: "prefer_concise_output".to_owned(),
                disclosure: DisclosureV1::BehavioralOnly,
                confidence: ConfidenceV1::High,
                cause_ref: None,
            })
            .expect("legacy typed subjective present");
            SubjectivePresentProjectionV1::new(vec![item]).expect("legacy subjective projection")
        }

        fn soma_field(signal_id: &str, value: f64) -> SomaFieldSetV1 {
            let signal = BoundedSomaSignalV1::new(signal_id.to_owned(), 64, value, 0.0, 1.0)
                .expect("bounded soma signal");
            SomaFieldSetV1::new(vec![signal], 8).expect("bounded soma field")
        }

        fn token(value: &str) -> CanonicalTokenV1 {
            CanonicalTokenV1::new(value.to_owned(), 64).expect("action token")
        }

        fn token_set(values: &[&str]) -> CanonicalTokenSetV1 {
            CanonicalTokenSetV1::new(values.iter().map(|value| token(value)).collect(), 8)
                .expect("token set")
        }

        fn unit(value: u32) -> UnitIntervalV1 {
            UnitIntervalV1::from_parts_per_million(value).expect("unit interval")
        }

        fn epistemic_projection(
            identity: IdentityConstitutionV1,
            state_digest: Digest,
            revision: u64,
            turn_id: Id128,
        ) -> ae_epistemic_state::EpistemicProjectionV1 {
            let binding = EpistemicSourceBindingV1::new(
                ScopeRef {
                    bot_token: [1; 16],
                    persona_token: [2; 16],
                    relation_token: Some([3; 16]),
                    session_token: [4; 16],
                },
                turn_id,
                state_digest,
                revision,
                identity,
            )
            .expect("bound epistemic source");
            let estimate = SemanticEstimate {
                schema_version: 1,
                dimensions: EvidenceVector {
                    epistemic_conflict: Fixed::from_raw(500_000),
                    new_information: Fixed::from_raw(250_000),
                    ..EvidenceVector::default()
                },
                estimator_confidence: Fixed::from_raw(800_000),
                estimator_digest: digest(71),
            };
            let classification = CallerProvidedEpistemicClassificationV1::new(
                VerdictKind::ConfirmedSelfError,
                vec![EpistemicEvidenceGapV1::ConflictingEvidence],
                VerifierNeedV1::Required,
                Fixed::from_raw(420_000),
                true,
                true,
            )
            .expect("caller-provided epistemic classification");
            compile_epistemic_projection_v1(&EpistemicProjectionInputV1::new(
                binding,
                CausalRef {
                    turn_id,
                    action_id: Some(ACTION_ID),
                    delivery_id: None,
                    claim_id: Some([81; 16]),
                    base_revision: revision,
                },
                estimate,
                classification,
            ))
            .expect("typed epistemic projection")
        }

        pub(crate) fn identity_capsule(
            identity: IdentityConstitutionV1,
        ) -> SourceCapsuleV1<IdentityConstitutionV1> {
            let identity_digest = *identity.constitution_digest();
            capsule(
                ProjectionSourceKindV1::IdentityConstitution,
                4,
                1,
                identity_digest,
                identity,
            )
        }

        use crate::r7::{
            AstrRuntime, BoundedProjectionReferencesV1, PreOutputProjectionUpdateV1,
            R7PreOutputProjectionInputV1, RuntimeError,
        };
        use ae_contracts::r7::{CanonicalEvent, UserStimulus};

        #[derive(Clone, Copy)]
        struct FixtureSemanticBinding {
            revision: u64,
            state_after: Digest,
            turn_id: Id128,
            scope_digest: Digest,
            turn_binding: Digest,
        }

        fn fixture_semantic_binding() -> FixtureSemanticBinding {
            FixtureSemanticBinding {
                revision: 1,
                state_after: digest(171),
                turn_id: TURN_ID,
                scope_digest: digest(172),
                turn_binding: digest(173),
            }
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

        fn pre_output_update(
            identity: IdentityConstitutionV1,
            transition: &FixtureSemanticBinding,
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
            let epistemic =
                epistemic_projection(identity, source_state_digest, revision, transition.turn_id);

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

        fn deliberately_nonmatching_typed_input() -> R7PreOutputProjectionInputV1 {
            let constitution = identity(41);
            let binding = fixture_semantic_binding();
            let update = pre_output_update(constitution.clone(), &binding, None, None, None, None);
            R7PreOutputProjectionInputV1::new(identity_capsule(constitution), update)
                .expect("valid typed identity")
        }

        #[allow(dead_code)]
        pub(crate) fn matching_pre_output_input(
            revision: u64,
            state_after: Digest,
            turn_id: Id128,
            scope_digest: Digest,
            turn_binding: Digest,
        ) -> R7PreOutputProjectionInputV1 {
            let constitution = identity(41);
            let binding = FixtureSemanticBinding {
                revision,
                state_after,
                turn_id,
                scope_digest,
                turn_binding,
            };
            let update = pre_output_update(constitution.clone(), &binding, None, None, None, None);
            R7PreOutputProjectionInputV1::new(identity_capsule(constitution), update)
                .expect("production-bound typed input")
        }

        fn closed_stimulus(base_revision: u64, positive: i64) -> CanonicalEvent {
            CanonicalEvent::UserStimulus(UserStimulus {
                event_id: [1; 16],
                scope: ScopeRef {
                    bot_token: [2; 16],
                    persona_token: [3; 16],
                    relation_token: Some([4; 16]),
                    session_token: [5; 16],
                },
                causal: CausalRef {
                    turn_id: [6; 16],
                    action_id: None,
                    delivery_id: None,
                    claim_id: None,
                    base_revision,
                },
                observed_at_ms: 1,
                evidence: SemanticEstimate {
                    schema_version: 1,
                    dimensions: EvidenceVector {
                        positive: Fixed::from_raw(110_000 + positive),
                        affiliation: Fixed::from_raw(120_000),
                        harm: Fixed::from_raw(130_000),
                        boundary: Fixed::from_raw(140_000),
                        repair: Fixed::from_raw(150_000),
                        repetition: Fixed::from_raw(160_000),
                        new_information: Fixed::from_raw(170_000),
                        constraint_instability: Fixed::from_raw(180_000),
                        epistemic_conflict: Fixed::from_raw(190_000),
                        self_responsibility: Fixed::from_raw(200_000),
                        other_responsibility: Fixed::from_raw(210_000),
                        hostility: Fixed::from_raw(220_000),
                        publicness: Fixed::from_raw(230_000),
                        engagement: Fixed::from_raw(240_000),
                        rejection: Fixed::from_raw(250_000),
                    },
                    estimator_confidence: Fixed::from_raw(800_000),
                    estimator_digest: [7; 32],
                },
            })
        }

        #[derive(Clone)]
        struct Snapshot {
            potential: Vec<Fixed>,
            excitation: Vec<Fixed>,
            revision: u64,
            formula_digest: [u8; 32],
        }

        fn snapshot(runtime: &AstrRuntime) -> Snapshot {
            Snapshot {
                potential: runtime.field.potential.clone(),
                excitation: runtime.field.excitation.clone(),
                revision: runtime.current_revision(),
                formula_digest: runtime.formula_digest,
            }
        }

        fn assert_unchanged(runtime: &AstrRuntime, before: &Snapshot) {
            assert_eq!(runtime.field.potential, before.potential);
            assert_eq!(runtime.field.excitation, before.excitation);
            assert_eq!(runtime.current_revision(), before.revision);
            assert_eq!(runtime.formula_digest, before.formula_digest);
        }

        #[test]
        fn invalid_semantics_do_not_reach_typed_projection_compilation_or_mutate_runtime() {
            let mut event = closed_stimulus(0, 0);
            let CanonicalEvent::UserStimulus(stimulus) = &mut event else {
                panic!("fixture is a user stimulus");
            };
            stimulus.evidence.estimator_confidence = Fixed::ZERO;
            let mut runtime = AstrRuntime::scaffold();
            let before = snapshot(&runtime);
            let input = deliberately_nonmatching_typed_input();

            assert!(matches!(
                runtime.apply_user_stimulus_with_private_projection_wire_v1(&event, &input),
                Err(RuntimeError::InvalidSemanticEstimate)
            ));
            assert_unchanged(&runtime, &before);
        }

        #[test]
        fn typed_projection_compilation_failure_is_atomic_before_semantic_commit() {
            let event = closed_stimulus(0, 0);
            let mut runtime = AstrRuntime::scaffold();
            let before = snapshot(&runtime);
            let input = deliberately_nonmatching_typed_input();

            assert!(matches!(
                runtime.apply_user_stimulus_with_private_projection_wire_v1(&event, &input),
                Err(RuntimeError::PrivateProjectionWireUnavailable)
            ));
            assert_unchanged(&runtime, &before);
        }

        #[test]
        fn stale_and_formula_failures_are_rejected_before_any_state_change() {
            let stale = closed_stimulus(1, 0);
            let mut runtime = AstrRuntime::scaffold();
            let before = snapshot(&runtime);
            let input = deliberately_nonmatching_typed_input();
            assert!(matches!(
                runtime.apply_user_stimulus_with_private_projection_wire_v1(&stale, &input),
                Err(RuntimeError::UserStimulusBaseRevisionMismatch)
            ));
            assert_unchanged(&runtime, &before);

            runtime.formula_digest = [0; 32];
            let formula_before = snapshot(&runtime);
            assert!(matches!(
                runtime.apply_user_stimulus_with_private_projection_wire_v1(
                    &closed_stimulus(0, 0),
                    &input,
                ),
                Err(RuntimeError::NativeFormulaDigestMismatch)
            ));
            assert_unchanged(&runtime, &formula_before);
        }
    };
}
