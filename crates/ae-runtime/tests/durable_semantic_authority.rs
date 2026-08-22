#[allow(unused_macros)]
macro_rules! durable_semantic_authority_test_contents {
    () => {
        use crate::r7::PrivateProjectionTransferReceiptV1;
        use crate::AstrRuntime;
        use ae_attention::r7::assemble_load;
        use ae_authority::authority_projection_digest;
        use ae_continuum::CommitEnvelope;
        use ae_contracts::r7 as r7_contracts;
        use ae_contracts::{
            wire, AllostaticSetpoints, CanonicalEvent, CausalRef, CommitStatus, EpistemicPriors,
            EvidenceVector, ExpressionPhenotype, GenesisManifestProposal, GenesisReceipt,
            GenesisStatus, InvariantResiduals, PersonaGenesisRequest, PersonaScopeRef,
            PersonaSelectionKind, PersonaSourceRef, PersonalityVector, ScopeRef, SemanticEstimate,
            SocialPriors, TransitionReceipt, UserStimulus,
        };
        use ae_fixed::Fixed;
        use ae_genesis::{derive_identity, genesis_scope_key, GenesisPrior};
        use ae_neurofield::{
            graph_digest, initial_state_from_manifest, state_digest, NeuralField, SparseGraph,
            Synapse, EDGE_CAPACITY, NEURON_SLOTS,
        };
        use ae_store::{ClaimOutcome, GenesisCommit, StatefulCommit, Store};
        use std::path::PathBuf;
        use std::sync::atomic::{AtomicUsize, Ordering};

        mod r7_projection_fixture {
            user_stimulus_state_transition_test_contents!();
        }

        const CANONICAL_HOT_STATE_MAGIC_V1: [u8; 8] = *b"AEHOTST\0";
        const CANONICAL_HOT_STATE_SCHEMA_V1: u16 = 1;
        const R7_SEMANTIC_PERSONA_SCOPE_DOMAIN_V1: &[u8] =
            b"astr-embodiment/runtime/r7-semantic-persona-scope-v1";

        static NEXT_DATABASE_ID: AtomicUsize = AtomicUsize::new(0);

        #[derive(Clone)]
        struct Fixture {
            database: PathBuf,
            scope: ScopeRef,
            scope_digest: [u8; 32],
            formula_digest: [u8; 32],
            field: NeuralField,
            graph: SparseGraph,
            state_digest: [u8; 32],
            graph_digest: [u8; 32],
            state_bytes: Vec<u8>,
            layout: CanonicalLayout,
        }

        #[derive(Clone, Copy)]
        struct CanonicalLayout {
            magic_end: usize,
            version_end: usize,
            formula_end: usize,
            vector_count_offsets: [usize; 8],
            vector_value_offsets: [usize; 8],
            vector_ends: [usize; 8],
            row_count_offset: usize,
            row_value_offset: usize,
            row_end: usize,
            edge_count_offset: usize,
            edge_value_offset: usize,
            edge_end: usize,
        }

        fn request(seed: u8) -> PersonaGenesisRequest {
            let scope = PersonaScopeRef {
                bot_token: [seed; 16],
                persona_token: [seed.wrapping_add(1); 16],
            };
            let source = PersonaSourceRef {
                scope,
                source_digest: [seed.wrapping_add(2); 32],
                capability_digest: [seed.wrapping_add(3); 32],
                selection: PersonaSelectionKind::Conversation,
                prompt_chars: 10,
                begin_dialog_count: 1,
                mood_dialog_count: 0,
            };
            let proposal = GenesisManifestProposal {
                schema_version: 1,
                source: source.clone(),
                traits: PersonalityVector {
                    baseline_warmth: Fixed::from_raw(700_000),
                    ..PersonalityVector::default()
                },
                trait_confidence: PersonalityVector {
                    baseline_warmth: Fixed::from_raw(500_000),
                    ..PersonalityVector::default()
                },
                expression: ExpressionPhenotype::default(),
                allostasis: AllostaticSetpoints::default(),
                epistemic: EpistemicPriors::default(),
                social: SocialPriors::default(),
                compiler_protocol_digest: [seed.wrapping_add(4); 32],
                compiler_model_digest: [seed.wrapping_add(5); 32],
            };
            PersonaGenesisRequest {
                source,
                proposal,
                formula_digest: [seed.wrapping_add(6); 32],
                incarnation_nonce: [seed.wrapping_add(7); 32],
                parent_incarnation_id: None,
                observed_at_ms: 1_700_000_000_000,
            }
        }

        fn scope_for(request: &PersonaGenesisRequest) -> ScopeRef {
            ScopeRef {
                bot_token: request.source.scope.bot_token,
                persona_token: request.source.scope.persona_token,
                relation_token: None,
                session_token: [0; 16],
            }
        }

        fn semantic_persona_scope(scope: &ScopeRef) -> [u8; 32] {
            let root_persona_scope =
                wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
            wire::domain_hash(R7_SEMANTIC_PERSONA_SCOPE_DOMAIN_V1, &[&root_persona_scope])
        }

        fn unique_database(name: &str) -> PathBuf {
            let id = NEXT_DATABASE_ID.fetch_add(1, Ordering::Relaxed);
            std::env::temp_dir().join(format!(
                "ae-runtime-canonical-hot-state-{name}-{}-{id}.db",
                std::process::id()
            ))
        }

        fn fixture(name: &str) -> Fixture {
            fixture_with_first_row_offset(name, 0)
        }

        fn fixture_with_first_row_offset(name: &str, first_row_offset: u32) -> Fixture {
            let request = request(41);
            let identity = derive_identity(&request, &GenesisPrior::default()).expect("identity");
            let formula_digest = request.formula_digest;
            let (mut field, mut graph) = initial_state_from_manifest(
                &identity.manifest,
                &formula_digest,
                &identity.development_seed_digest,
            );

            field.potential[0] = Fixed::from_raw(-1_000_001);
            field.excitation[1] = Fixed::from_raw(2_000_002);
            field.inhibition[2] = Fixed::from_raw(-3_000_003);
            field.adaptation[3] = Fixed::from_raw(4_000_004);
            field.precision[4] = Fixed::from_raw(-5_000_005);
            field.prediction_error[5] = Fixed::from_raw(6_000_006);
            field.eligibility[6] = Fixed::from_raw(-7_000_007);
            field.metabolic_reserve[7] = Fixed::from_raw(8_000_008);

            graph.edges = vec![
                Synapse {
                    target: (NEURON_SLOTS - 1) as u32,
                    weight: -123,
                    eligibility: 456,
                    stability: 789,
                    last_used_epoch: 321,
                    operator_id: 17,
                    delay_class: 9,
                    flags: 0xa5a5,
                },
                Synapse {
                    target: 7,
                    weight: 234,
                    eligibility: -567,
                    stability: 890,
                    last_used_epoch: 654,
                    operator_id: 23,
                    delay_class: 11,
                    flags: 0x5a5a,
                },
            ];
            graph.row_offsets[1..].fill(graph.edges.len() as u32);
            graph.row_offsets[1] = 1;
            graph.row_offsets[0] = first_row_offset;
            assert!(field.validate());
            assert!(graph.validate());

            let state_digest = state_digest(&field, &formula_digest);
            let graph_digest = graph_digest(&graph);
            let (state_bytes, layout) =
                encode_test_canonical_hot_state(&formula_digest, &field, &graph);
            let database = unique_database(name);
            let scope_key = genesis_scope_key(
                &request.source.scope.bot_token,
                &request.source.scope.persona_token,
                &request.source.source_digest,
                &formula_digest,
            );
            let mut store = Store::open(&database).expect("open fixture store");
            let ClaimOutcome::Claimed { lease_epoch, nonce } = store
                .claim_lease(&scope_key, Some(request.incarnation_nonce))
                .expect("claim fixture lease")
            else {
                panic!("fixture lease must be claimed");
            };
            let receipt = GenesisReceipt {
                schema_version: 1,
                seed_code_digest: identity.seed_code_digest,
                manifest_digest: identity.manifest_digest,
                incarnation_id: identity.incarnation_id,
                formula_digest,
                persona_source_digest: request.source.source_digest,
                compiler_protocol_digest: request.proposal.compiler_protocol_digest,
                compiler_model_digest: request.proposal.compiler_model_digest,
                development_seed_digest: identity.development_seed_digest,
                initial_snapshot_digest: state_digest,
                graph_digest,
                equilibrium_residual: Fixed::ZERO,
                energy_residual: Fixed::ZERO,
                capacity_residual: Fixed::ZERO,
                sample_fit_residual: Fixed::ZERO,
                status: GenesisStatus::Committed,
            };
            store
                .commit_genesis(&GenesisCommit {
                    scope_key,
                    lease_epoch,
                    nonce_digest: nonce,
                    manifest: identity.manifest,
                    manifest_body: wire::encode_manifest_body(&receipt_manifest(&request)),
                    seed_code_digest: identity.seed_code_digest,
                    incarnation_id: identity.incarnation_id,
                    formula_digest,
                    source: request.source.clone(),
                    compiler_protocol_digest: request.proposal.compiler_protocol_digest,
                    compiler_model_digest: request.proposal.compiler_model_digest,
                    compiled_at_ms: request.observed_at_ms,
                    receipt,
                    initial_snapshot_digest: state_digest,
                    state_bytes: state_bytes.clone(),
                    graph_digest,
                })
                .expect("commit custom genesis");
            drop(store);

            let scope = scope_for(&request);
            Fixture {
                database,
                scope_digest: wire::persona_scope_digest(
                    &request.source.scope.bot_token,
                    &request.source.scope.persona_token,
                    None,
                ),
                scope,
                formula_digest,
                field,
                graph,
                state_digest,
                graph_digest,
                state_bytes,
                layout,
            }
        }

        fn receipt_manifest(request: &PersonaGenesisRequest) -> ae_contracts::GenesisManifest {
            derive_identity(request, &GenesisPrior::default())
                .expect("identity")
                .manifest
        }

        fn encode_test_canonical_hot_state(
            formula_digest: &[u8; 32],
            field: &NeuralField,
            graph: &SparseGraph,
        ) -> (Vec<u8>, CanonicalLayout) {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&CANONICAL_HOT_STATE_MAGIC_V1);
            let magic_end = bytes.len();
            bytes.extend_from_slice(&CANONICAL_HOT_STATE_SCHEMA_V1.to_le_bytes());
            let version_end = bytes.len();
            bytes.extend_from_slice(formula_digest);
            let formula_end = bytes.len();

            let mut vector_count_offsets = [0; 8];
            let mut vector_value_offsets = [0; 8];
            let mut vector_ends = [0; 8];
            for (index, values) in [
                &field.potential,
                &field.excitation,
                &field.inhibition,
                &field.adaptation,
                &field.precision,
                &field.prediction_error,
                &field.eligibility,
                &field.metabolic_reserve,
            ]
            .into_iter()
            .enumerate()
            {
                vector_count_offsets[index] = bytes.len();
                bytes.extend_from_slice(&(values.len() as u32).to_le_bytes());
                vector_value_offsets[index] = bytes.len();
                for value in values {
                    bytes.extend_from_slice(&value.encode());
                }
                vector_ends[index] = bytes.len();
            }

            let row_count_offset = bytes.len();
            bytes.extend_from_slice(&(graph.row_offsets.len() as u32).to_le_bytes());
            let row_value_offset = bytes.len();
            for offset in &graph.row_offsets {
                bytes.extend_from_slice(&offset.to_le_bytes());
            }
            let row_end = bytes.len();
            let edge_count_offset = bytes.len();
            bytes.extend_from_slice(&(graph.edges.len() as u32).to_le_bytes());
            let edge_value_offset = bytes.len();
            for edge in &graph.edges {
                bytes.extend_from_slice(&edge.target.to_le_bytes());
                bytes.extend_from_slice(&edge.weight.to_le_bytes());
                bytes.extend_from_slice(&edge.eligibility.to_le_bytes());
                bytes.extend_from_slice(&edge.stability.to_le_bytes());
                bytes.extend_from_slice(&edge.last_used_epoch.to_le_bytes());
                bytes.push(edge.operator_id);
                bytes.push(edge.delay_class);
                bytes.extend_from_slice(&edge.flags.to_le_bytes());
            }
            let edge_end = bytes.len();
            (
                bytes,
                CanonicalLayout {
                    magic_end,
                    version_end,
                    formula_end,
                    vector_count_offsets,
                    vector_value_offsets,
                    vector_ends,
                    row_count_offset,
                    row_value_offset,
                    row_end,
                    edge_count_offset,
                    edge_value_offset,
                    edge_end,
                },
            )
        }

        fn take<const N: usize>(bytes: &[u8], cursor: &mut usize) -> [u8; N] {
            let end = cursor.checked_add(N).expect("fixture cursor overflow");
            let value = bytes
                .get(*cursor..end)
                .expect("fixture canonical bytes have expected length")
                .try_into()
                .expect("fixed array");
            *cursor = end;
            value
        }

        fn decode_test_canonical_hot_state(bytes: &[u8]) -> ([u8; 32], NeuralField, SparseGraph) {
            let mut cursor = 0;
            assert_eq!(take::<8>(bytes, &mut cursor), CANONICAL_HOT_STATE_MAGIC_V1);
            assert_eq!(
                u16::from_le_bytes(take::<2>(bytes, &mut cursor)),
                CANONICAL_HOT_STATE_SCHEMA_V1
            );
            let formula_digest = take::<32>(bytes, &mut cursor);
            let mut vectors = Vec::new();
            for _ in 0..8 {
                let count = u32::from_le_bytes(take::<4>(bytes, &mut cursor)) as usize;
                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    values.push(Fixed::decode(take::<8>(bytes, &mut cursor)));
                }
                vectors.push(values);
            }
            let row_count = u32::from_le_bytes(take::<4>(bytes, &mut cursor)) as usize;
            let mut row_offsets = Vec::with_capacity(row_count);
            for _ in 0..row_count {
                row_offsets.push(u32::from_le_bytes(take::<4>(bytes, &mut cursor)));
            }
            let edge_count = u32::from_le_bytes(take::<4>(bytes, &mut cursor)) as usize;
            let mut edges = Vec::with_capacity(edge_count);
            for _ in 0..edge_count {
                edges.push(Synapse {
                    target: u32::from_le_bytes(take::<4>(bytes, &mut cursor)),
                    weight: i16::from_le_bytes(take::<2>(bytes, &mut cursor)),
                    eligibility: i16::from_le_bytes(take::<2>(bytes, &mut cursor)),
                    stability: u16::from_le_bytes(take::<2>(bytes, &mut cursor)),
                    last_used_epoch: u16::from_le_bytes(take::<2>(bytes, &mut cursor)),
                    operator_id: take::<1>(bytes, &mut cursor)[0],
                    delay_class: take::<1>(bytes, &mut cursor)[0],
                    flags: u16::from_le_bytes(take::<2>(bytes, &mut cursor)),
                });
            }
            assert_eq!(cursor, bytes.len(), "fixture decode reaches exact EOF");
            let mut vectors = vectors.into_iter();
            (
                formula_digest,
                NeuralField {
                    potential: vectors.next().expect("potential"),
                    excitation: vectors.next().expect("excitation"),
                    inhibition: vectors.next().expect("inhibition"),
                    adaptation: vectors.next().expect("adaptation"),
                    precision: vectors.next().expect("precision"),
                    prediction_error: vectors.next().expect("prediction error"),
                    eligibility: vectors.next().expect("eligibility"),
                    metabolic_reserve: vectors.next().expect("metabolic reserve"),
                },
                SparseGraph { row_offsets, edges },
            )
        }

        fn assert_synapse_eq(actual: &Synapse, expected: &Synapse) {
            assert_eq!(actual.target, expected.target);
            assert_eq!(actual.weight, expected.weight);
            assert_eq!(actual.eligibility, expected.eligibility);
            assert_eq!(actual.stability, expected.stability);
            assert_eq!(actual.last_used_epoch, expected.last_used_epoch);
            assert_eq!(actual.operator_id, expected.operator_id);
            assert_eq!(actual.delay_class, expected.delay_class);
            assert_eq!(actual.flags, expected.flags);
        }

        #[test]
        fn canonical_hot_state_round_trips_all_field_and_graph_bytes() {
            let fixture = fixture("round-trip");
            let mut runtime = AstrRuntime::open(&fixture.database).expect("open runtime");
            assert_eq!(
                runtime
                    .current_revision(&fixture.scope)
                    .expect("bind hot state"),
                0
            );
            runtime.flush_and_close().expect("flush decoded hot state");
            drop(runtime);

            let store = Store::open(&fixture.database).expect("reopen fixture store");
            let stored = store
                .read_snapshot(&fixture.scope_digest, 0)
                .expect("read snapshot")
                .expect("revision-zero snapshot");
            assert_eq!(stored.state_bytes, fixture.state_bytes);
            assert_eq!(stored.state_digest, fixture.state_digest);

            let (formula_digest, field, graph) =
                decode_test_canonical_hot_state(&stored.state_bytes);
            assert_eq!(formula_digest, fixture.formula_digest);
            assert_eq!(field.potential, fixture.field.potential);
            assert_eq!(field.excitation, fixture.field.excitation);
            assert_eq!(field.inhibition, fixture.field.inhibition);
            assert_eq!(field.adaptation, fixture.field.adaptation);
            assert_eq!(field.precision, fixture.field.precision);
            assert_eq!(field.prediction_error, fixture.field.prediction_error);
            assert_eq!(field.eligibility, fixture.field.eligibility);
            assert_eq!(field.metabolic_reserve, fixture.field.metabolic_reserve);
            assert_eq!(graph.row_offsets, fixture.graph.row_offsets);
            assert_eq!(graph.edges.len(), fixture.graph.edges.len());
            for (actual, expected) in graph.edges.iter().zip(&fixture.graph.edges) {
                assert_synapse_eq(actual, expected);
            }
            assert_eq!(state_digest(&field, &formula_digest), fixture.state_digest);
            assert_eq!(graph_digest(&graph), fixture.graph_digest);
        }

        fn assert_rejected(label: &str, bytes: Vec<u8>) {
            let fixture = fixture(label);
            let mut store =
                Store::open(&fixture.database).expect("open fixture store for corruption");
            store
                .write_snapshot(&fixture.scope_digest, 0, &fixture.state_digest, &bytes)
                .expect("install corrupted state bytes");
            drop(store);

            let mut runtime = AstrRuntime::open(&fixture.database).expect("open corrupted runtime");
            assert!(
                runtime.current_revision(&fixture.scope).is_err(),
                "{label} bytes must not bind HotBrain"
            );
        }

        fn root_event_from_r7(event: &r7_contracts::CanonicalEvent) -> CanonicalEvent {
            let r7_contracts::CanonicalEvent::UserStimulus(stimulus) = event else {
                panic!("fixture must contain a user stimulus");
            };
            CanonicalEvent::UserStimulus(UserStimulus {
                event_id: stimulus.event_id,
                scope: ScopeRef {
                    bot_token: stimulus.scope.bot_token,
                    persona_token: stimulus.scope.persona_token,
                    relation_token: stimulus.scope.relation_token,
                    session_token: stimulus.scope.session_token,
                },
                causal: CausalRef {
                    turn_id: stimulus.causal.turn_id,
                    action_id: stimulus.causal.action_id,
                    delivery_id: stimulus.causal.delivery_id,
                    claim_id: stimulus.causal.claim_id,
                    base_revision: stimulus.causal.base_revision,
                },
                observed_at_ms: stimulus.observed_at_ms,
                evidence: SemanticEstimate {
                    schema_version: stimulus.evidence.schema_version,
                    dimensions: EvidenceVector {
                        positive: stimulus.evidence.dimensions.positive,
                        affiliation: stimulus.evidence.dimensions.affiliation,
                        harm: stimulus.evidence.dimensions.harm,
                        boundary: stimulus.evidence.dimensions.boundary,
                        repair: stimulus.evidence.dimensions.repair,
                        repetition: stimulus.evidence.dimensions.repetition,
                        new_information: stimulus.evidence.dimensions.new_information,
                        constraint_instability: stimulus.evidence.dimensions.constraint_instability,
                        epistemic_conflict: stimulus.evidence.dimensions.epistemic_conflict,
                        self_responsibility: stimulus.evidence.dimensions.self_responsibility,
                        other_responsibility: stimulus.evidence.dimensions.other_responsibility,
                        hostility: stimulus.evidence.dimensions.hostility,
                        publicness: stimulus.evidence.dimensions.publicness,
                        engagement: stimulus.evidence.dimensions.engagement,
                        rejection: stimulus.evidence.dimensions.rejection,
                    },
                    estimator_confidence: stimulus.evidence.estimator_confidence,
                    estimator_digest: stimulus.evidence.estimator_digest,
                },
            })
        }

        fn production_projection_turn_binding(
            next_revision: u64,
            state_after: &[u8; 32],
            turn_id: &[u8; 16],
            scope_digest: &[u8; 32],
            event_digest: &[u8; 32],
            authority_digest: &[u8; 32],
        ) -> [u8; 32] {
            let revision = next_revision.to_be_bytes();
            r7_contracts::wire::domain_hash(
                b"astr-embodiment/r7/committed-semantic-transition-binding-v1",
                &[
                    &revision,
                    state_after,
                    turn_id,
                    scope_digest,
                    event_digest,
                    authority_digest,
                ],
            )
        }

        fn apply_stimulus_to_field(
            mut field: NeuralField,
            event: &r7_contracts::CanonicalEvent,
        ) -> NeuralField {
            let r7_contracts::CanonicalEvent::UserStimulus(stimulus) = event else {
                panic!("fixture must contain a user stimulus");
            };
            let load = assemble_load(&stimulus.evidence.dimensions, NEURON_SLOTS as u32);
            assert_eq!(load.active_nodes.len(), load.node_loads.len());
            for (&node, &node_load) in load.active_nodes.iter().zip(&load.node_loads) {
                let regional_load = node_load
                    .checked_mul(stimulus.evidence.estimator_confidence)
                    .expect("bounded fixture estimate");
                let index = node as usize;
                field.potential[index] = field.potential[index].saturating_add(regional_load);
                field.excitation[index] = field.excitation[index].saturating_add(regional_load);
            }
            field
        }

        fn expected_production_next_field(
            request: &PersonaGenesisRequest,
            event: &r7_contracts::CanonicalEvent,
        ) -> NeuralField {
            let identity =
                derive_identity(request, &GenesisPrior::default()).expect("derive genesis");
            let (field, _) = initial_state_from_manifest(
                &identity.manifest,
                &request.formula_digest,
                &identity.development_seed_digest,
            );
            apply_stimulus_to_field(field, event)
        }

        #[test]
        fn semantic_journal_without_same_revision_snapshot_fails_closed() {
            let request = request(70);
            let database = unique_database("semantic-missing-snapshot");
            let mut runtime = AstrRuntime::open(&database).expect("open runtime");
            runtime.ensure_genesis(&request).expect("commit genesis");
            drop(runtime);

            let mut store = Store::open(&database).expect("open store");
            let committed = store
                .lookup_bound_genesis(
                    &request.source.scope.bot_token,
                    &request.source.scope.persona_token,
                )
                .expect("lookup bound genesis")
                .expect("genesis");
            let scope = ScopeRef {
                bot_token: request.source.scope.bot_token,
                persona_token: request.source.scope.persona_token,
                relation_token: None,
                session_token: [0; 16],
            };
            let semantic_scope = semantic_persona_scope(&scope);
            let legacy_scope =
                wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
            let state_bytes = store
                .read_snapshot(&legacy_scope, 0)
                .expect("read genesis snapshot")
                .expect("genesis snapshot")
                .state_bytes;
            let receipt_for = |event: &CanonicalEvent, base_revision| TransitionReceipt {
                schema_version: 1,
                formula_digest: committed.receipt.formula_digest,
                scope_digest: semantic_scope,
                event_digest: wire::event_digest(event),
                authority_digest: [72; 32],
                base_revision,
                next_revision: base_revision + 1,
                state_before: committed.receipt.initial_snapshot_digest,
                state_after: committed.receipt.initial_snapshot_digest,
                graph_after: committed.receipt.graph_digest,
                action_contract: None,
                active_nodes: 0,
                active_edges: 0,
                residuals: InvariantResiduals::default(),
                status: CommitStatus::Committed,
            };
            let first_event = CanonicalEvent::TimeAdvance(ae_contracts::TimeAdvance {
                event_id: [71; 16],
                scope: scope.clone(),
                elapsed_ms: 1,
            });
            let first_receipt = receipt_for(&first_event, 0);
            let (_, first_row) = store
                .commit_stateful_journal(&StatefulCommit {
                    journal: CommitEnvelope {
                        event_kind: "time_advance".to_owned(),
                        event_bytes: wire::encode_event(&first_event),
                        receipt: first_receipt,
                        chain_seed: committed.receipt.initial_snapshot_digest,
                        delta_bytes: Vec::new(),
                    },
                    state_bytes,
                })
                .expect("install paired semantic journal row");
            let second_event = CanonicalEvent::TimeAdvance(ae_contracts::TimeAdvance {
                event_id: [73; 16],
                scope: scope.clone(),
                elapsed_ms: 1,
            });
            store
                .commit_journal(&CommitEnvelope {
                    event_kind: "time_advance".to_owned(),
                    event_bytes: wire::encode_event(&second_event),
                    receipt: receipt_for(&second_event, 1),
                    chain_seed: first_row.chain_digest,
                    delta_bytes: Vec::new(),
                })
                .expect("install malformed semantic journal row");
            drop(store);

            let mut reopened = AstrRuntime::open(&database).expect("reopen runtime");
            assert!(
                reopened.current_revision(&scope).is_err(),
                "semantic journal rows require a same-revision snapshot"
            );
        }

        #[test]
        fn hydrate_rejects_older_semantic_row_without_snapshot_even_with_later_snapshot() {
            let request = request(72);
            let database = unique_database("semantic-older-missing-snapshot");
            let mut runtime = AstrRuntime::open(&database).expect("open runtime");
            runtime.ensure_genesis(&request).expect("commit genesis");
            drop(runtime);

            let mut store = Store::open(&database).expect("open store");
            let committed = store
                .lookup_bound_genesis(
                    &request.source.scope.bot_token,
                    &request.source.scope.persona_token,
                )
                .expect("lookup bound genesis")
                .expect("genesis");
            let scope = ScopeRef {
                bot_token: request.source.scope.bot_token,
                persona_token: request.source.scope.persona_token,
                relation_token: None,
                session_token: [0; 16],
            };
            let semantic_scope = semantic_persona_scope(&scope);
            let legacy_scope =
                wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
            let state_bytes = store
                .read_snapshot(&legacy_scope, 0)
                .expect("read genesis snapshot")
                .expect("genesis snapshot")
                .state_bytes;
            let receipt_for = |event: &CanonicalEvent, base_revision| TransitionReceipt {
                schema_version: 1,
                formula_digest: committed.receipt.formula_digest,
                scope_digest: semantic_scope,
                event_digest: wire::event_digest(event),
                authority_digest: [74; 32],
                base_revision,
                next_revision: base_revision + 1,
                state_before: committed.receipt.initial_snapshot_digest,
                state_after: committed.receipt.initial_snapshot_digest,
                graph_after: committed.receipt.graph_digest,
                action_contract: None,
                active_nodes: 0,
                active_edges: 0,
                residuals: InvariantResiduals::default(),
                status: CommitStatus::Committed,
            };
            let first_event = CanonicalEvent::TimeAdvance(ae_contracts::TimeAdvance {
                event_id: [75; 16],
                scope: scope.clone(),
                elapsed_ms: 1,
            });
            let (_, first_row) = store
                .commit_journal(&CommitEnvelope {
                    event_kind: "time_advance".to_owned(),
                    event_bytes: wire::encode_event(&first_event),
                    receipt: receipt_for(&first_event, 0),
                    chain_seed: committed.receipt.initial_snapshot_digest,
                    delta_bytes: Vec::new(),
                })
                .expect("install older semantic journal row without snapshot");
            let second_event = CanonicalEvent::TimeAdvance(ae_contracts::TimeAdvance {
                event_id: [76; 16],
                scope: scope.clone(),
                elapsed_ms: 1,
            });
            store
                .commit_stateful_journal(&StatefulCommit {
                    journal: CommitEnvelope {
                        event_kind: "time_advance".to_owned(),
                        event_bytes: wire::encode_event(&second_event),
                        receipt: receipt_for(&second_event, 1),
                        chain_seed: first_row.chain_digest,
                        delta_bytes: Vec::new(),
                    },
                    state_bytes,
                })
                .expect("install later semantic journal row and snapshot");
            drop(store);

            let mut reopened = AstrRuntime::open(&database).expect("reopen runtime");
            assert!(
                reopened.current_revision(&scope).is_err(),
                "hydrate must audit every semantic journal row before selecting latest snapshot"
            );
        }

        #[test]
        fn hydrate_replay_close_preserves_committed_semantic_state() {
            let request = request(71);
            let scope = ScopeRef {
                bot_token: request.source.scope.bot_token,
                persona_token: request.source.scope.persona_token,
                relation_token: Some([74; 16]),
                session_token: [75; 16],
            };
            let event = r7_contracts::CanonicalEvent::UserStimulus(r7_contracts::UserStimulus {
                event_id: [76; 16],
                scope: r7_contracts::ScopeRef {
                    bot_token: scope.bot_token,
                    persona_token: scope.persona_token,
                    relation_token: scope.relation_token,
                    session_token: scope.session_token,
                },
                causal: r7_contracts::CausalRef {
                    turn_id: [77; 16],
                    action_id: None,
                    delivery_id: None,
                    claim_id: None,
                    base_revision: 0,
                },
                observed_at_ms: request.observed_at_ms,
                evidence: r7_contracts::SemanticEstimate {
                    schema_version: 1,
                    dimensions: r7_contracts::EvidenceVector {
                        positive: Fixed::from_raw(700_000),
                        affiliation: Fixed::from_raw(600_000),
                        harm: Fixed::from_raw(100_000),
                        boundary: Fixed::from_raw(200_000),
                        repair: Fixed::from_raw(400_000),
                        repetition: Fixed::from_raw(100_000),
                        new_information: Fixed::from_raw(500_000),
                        constraint_instability: Fixed::from_raw(100_000),
                        epistemic_conflict: Fixed::from_raw(300_000),
                        self_responsibility: Fixed::from_raw(300_000),
                        other_responsibility: Fixed::from_raw(200_000),
                        hostility: Fixed::from_raw(100_000),
                        publicness: Fixed::from_raw(100_000),
                        engagement: Fixed::from_raw(600_000),
                        rejection: Fixed::from_raw(100_000),
                    },
                    estimator_confidence: Fixed::from_raw(800_000),
                    estimator_digest: [78; 32],
                },
            });
            let root_event = root_event_from_r7(&event);
            let next_field = expected_production_next_field(&request, &event);
            let state_after = state_digest(&next_field, &request.formula_digest);
            let legacy_persona_scope =
                wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
            let semantic_persona_scope = semantic_persona_scope(&scope);
            let full_scope_digest = wire::scope_digest(&scope);
            let event_digest = wire::event_digest(&root_event);
            let authority_digest = authority_projection_digest(&root_event);
            let turn_binding = production_projection_turn_binding(
                1,
                &state_after,
                &[77; 16],
                &full_scope_digest,
                &event_digest,
                &authority_digest,
            );
            let input = r7_projection_fixture::matching_pre_output_input(
                1,
                state_after,
                [77; 16],
                full_scope_digest,
                turn_binding,
            );
            let database = unique_database("production-authority");
            let mut runtime = AstrRuntime::open(&database).expect("open runtime");
            runtime.ensure_genesis(&request).expect("commit genesis");

            let legacy = runtime
                .apply_event(&scope, &root_event)
                .expect("legacy G0 accepts the exact root user stimulus first");
            assert!(!legacy.deduplicated);
            assert_eq!(legacy.receipt.scope_digest, legacy_persona_scope);
            assert_eq!(legacy.receipt.event_digest, event_digest);
            assert_eq!(
                runtime
                    .current_revision(&scope)
                    .expect("legacy current revision remains visible before R7 starts"),
                1
            );

            let decision = runtime
                .apply_user_stimulus_with_private_projection_wire_v1(&event, &input)
                .expect("R7 must not deduplicate a legacy G0 no-op record");
            assert!(
                !decision.deduplicated,
                "R7 must commit its stateful semantic transition after a legacy G0 record"
            );
            assert_eq!(decision.receipt.event_digest, event_digest);
            assert_eq!(decision.receipt.scope_digest, semantic_persona_scope);
            assert_ne!(decision.receipt.scope_digest, legacy.receipt.scope_digest);
            assert_eq!(decision.receipt.state_after, state_after);
            assert_eq!(runtime.current_revision(&scope).expect("hot revision"), 1);
            assert_eq!(
                decision
                    .discard_private_projection_v1()
                    .expect("native discard is the only current terminal"),
                Some(PrivateProjectionTransferReceiptV1::Discarded)
            );

            runtime
                .flush_and_close()
                .expect("close without an extra snapshot write");
            drop(runtime);

            let mut reopened = AstrRuntime::open(&database).expect("reopen runtime");
            assert_eq!(
                reopened
                    .current_revision(&scope)
                    .expect("hydrate committed state"),
                1
            );

            let mut second_event = event.clone();
            let r7_contracts::CanonicalEvent::UserStimulus(second_stimulus) = &mut second_event
            else {
                panic!("fixture must contain a user stimulus");
            };
            second_stimulus.event_id = [79; 16];
            second_stimulus.causal.turn_id = [80; 16];
            second_stimulus.causal.base_revision = 1;
            let second_root_event = root_event_from_r7(&second_event);
            let second_field = apply_stimulus_to_field(next_field, &second_event);
            let second_state_after = state_digest(&second_field, &request.formula_digest);
            let second_event_digest = wire::event_digest(&second_root_event);
            let second_authority_digest = authority_projection_digest(&second_root_event);
            let second_turn_binding = production_projection_turn_binding(
                2,
                &second_state_after,
                &[80; 16],
                &full_scope_digest,
                &second_event_digest,
                &second_authority_digest,
            );
            let second_input = r7_projection_fixture::matching_pre_output_input(
                2,
                second_state_after,
                [80; 16],
                full_scope_digest,
                second_turn_binding,
            );
            let second_decision = reopened
                .apply_user_stimulus_with_private_projection_wire_v1(&second_event, &second_input)
                .expect("reopened runtime must prepare from its committed semantic snapshot");
            assert_eq!(second_decision.revision, 2);
            assert_eq!(second_decision.receipt.state_after, second_state_after);

            let public_g0_base = reopened
                .current_revision(&scope)
                .expect("read the public ordinary-G0 revision after semantic divergence");
            let mut third_event = second_event.clone();
            let r7_contracts::CanonicalEvent::UserStimulus(third_stimulus) = &mut third_event
            else {
                panic!("fixture must contain a user stimulus");
            };
            third_stimulus.event_id = [81; 16];
            third_stimulus.causal.turn_id = [82; 16];
            third_stimulus.causal.base_revision = second_decision.revision;
            assert_eq!(
                second_decision
                    .discard_private_projection_v1()
                    .expect("discard second private projection"),
                Some(PrivateProjectionTransferReceiptV1::Discarded)
            );
            let third_root_event = root_event_from_r7(&third_event);
            let third_field = apply_stimulus_to_field(second_field, &third_event);
            let third_state_after = state_digest(&third_field, &request.formula_digest);
            let third_event_digest = wire::event_digest(&third_root_event);
            let third_authority_digest = authority_projection_digest(&third_root_event);
            let third_turn_binding = production_projection_turn_binding(
                3,
                &third_state_after,
                &[82; 16],
                &full_scope_digest,
                &third_event_digest,
                &third_authority_digest,
            );
            let third_input = r7_projection_fixture::matching_pre_output_input(
                3,
                third_state_after,
                [82; 16],
                full_scope_digest,
                third_turn_binding,
            );
            let third_decision = reopened
                .apply_user_stimulus_with_private_projection_wire_v1(&third_event, &third_input)
                .expect("R7 uses the explicit semantic revision rather than public G0 revision");
            assert_eq!(third_decision.revision, 3);
            assert_eq!(
                third_decision
                    .discard_private_projection_v1()
                    .expect("discard third private projection"),
                Some(PrivateProjectionTransferReceiptV1::Discarded)
            );

            let mut legacy_after_semantic = root_event.clone();
            let CanonicalEvent::UserStimulus(legacy_stimulus) = &mut legacy_after_semantic else {
                panic!("fixture must contain a root user stimulus");
            };
            legacy_stimulus.event_id = [83; 16];
            legacy_stimulus.causal.turn_id = [84; 16];
            legacy_stimulus.causal.base_revision = public_g0_base;
            let legacy_after = reopened
                .apply_event(&scope, &legacy_after_semantic)
                .expect("ordinary G0 accepts the causal base returned by current_revision");
            assert_eq!(legacy_after.revision, 2);
            assert_eq!(
                reopened
                    .current_revision(&scope)
                    .expect("public G0 revision remains the legacy lane"),
                2
            );

            reopened
                .flush_and_close()
                .expect("close divergent durable lanes");
            drop(reopened);

            let mut final_reopen =
                AstrRuntime::open(&database).expect("reopen divergent durable lanes");
            final_reopen
                .audit_durable_histories_v1(&scope.bot_token, &scope.persona_token)
                .expect("crate-private audit independently validates both durable histories");
            assert_eq!(
                final_reopen
                    .current_revision(&scope)
                    .expect("public G0 revision after reopen"),
                2
            );
            let inspect = final_reopen
                .inspect(&scope.bot_token, &scope.persona_token)
                .expect("public inspect remains on the legacy G0 authority lane");
            assert_eq!(inspect.revision, 2);
            assert_eq!(inspect.journal_count, 2);
            let replay = final_reopen
                .verify_replay(&scope.bot_token, &scope.persona_token)
                .expect("public replay remains on the legacy G0 authority lane");
            assert!(replay.ok);
            assert_eq!(replay.checked, 2);
            assert_eq!(replay.final_revision, 2);

            let retry = final_reopen
                .apply_user_stimulus_with_private_projection_wire_v1(&event, &input)
                .expect("exact event is deduplicated before stale or projection work");
            assert!(retry.deduplicated);
            assert_eq!(retry.receipt.event_digest, event_digest);
            assert_eq!(
                retry
                    .discard_private_projection_v1()
                    .expect("deduplicated transition has no payload"),
                None
            );
            let legacy_retry = final_reopen
                .apply_event(&scope, &legacy_after_semantic)
                .expect("exact legacy G0 retry remains idempotent");
            assert!(legacy_retry.deduplicated);

            let store = Store::open(&database).expect("open durable authority");
            let canonical_event_bytes = wire::encode_event(&root_event);
            let legacy_row = store
                .lookup_event(&legacy_persona_scope, &event_digest)
                .expect("read legacy G0 row")
                .expect("legacy G0 row");
            let semantic_row = store
                .lookup_event(&semantic_persona_scope, &event_digest)
                .expect("read semantic R7 row")
                .expect("semantic R7 row");
            assert_eq!(legacy_row.event_bytes, canonical_event_bytes);
            assert_eq!(semantic_row.event_bytes, canonical_event_bytes);
            assert_eq!(legacy_row.event_digest, event_digest);
            assert_eq!(semantic_row.event_digest, event_digest);
            assert_eq!(
                store
                    .current_revision(&legacy_persona_scope)
                    .expect("legacy G0 revision"),
                2
            );
            assert_eq!(
                store
                    .current_revision(&semantic_persona_scope)
                    .expect("semantic durable revision"),
                3
            );
            assert!(
                store
                    .read_snapshot(&legacy_persona_scope, 2)
                    .expect("read legacy G0 snapshot")
                    .is_none(),
                "the legacy no-op must not alias a semantic snapshot"
            );
            let snapshot = store
                .read_snapshot(&semantic_persona_scope, 3)
                .expect("read durable semantic snapshot")
                .expect("semantic commit writes its snapshot atomically");
            assert_eq!(snapshot.state_digest, third_state_after);
        }

        #[test]
        fn r7_and_legacy_user_stimulus_do_not_substitute_each_other_in_reverse_order() {
            let request = request(91);
            let scope = ScopeRef {
                bot_token: request.source.scope.bot_token,
                persona_token: request.source.scope.persona_token,
                relation_token: Some([94; 16]),
                session_token: [95; 16],
            };
            let event = r7_contracts::CanonicalEvent::UserStimulus(r7_contracts::UserStimulus {
                event_id: [96; 16],
                scope: r7_contracts::ScopeRef {
                    bot_token: scope.bot_token,
                    persona_token: scope.persona_token,
                    relation_token: scope.relation_token,
                    session_token: scope.session_token,
                },
                causal: r7_contracts::CausalRef {
                    turn_id: [97; 16],
                    action_id: None,
                    delivery_id: None,
                    claim_id: None,
                    base_revision: 0,
                },
                observed_at_ms: request.observed_at_ms,
                evidence: r7_contracts::SemanticEstimate {
                    schema_version: 1,
                    dimensions: r7_contracts::EvidenceVector {
                        positive: Fixed::from_raw(700_000),
                        affiliation: Fixed::from_raw(600_000),
                        harm: Fixed::from_raw(100_000),
                        boundary: Fixed::from_raw(200_000),
                        repair: Fixed::from_raw(400_000),
                        repetition: Fixed::from_raw(100_000),
                        new_information: Fixed::from_raw(500_000),
                        constraint_instability: Fixed::from_raw(100_000),
                        epistemic_conflict: Fixed::from_raw(300_000),
                        self_responsibility: Fixed::from_raw(300_000),
                        other_responsibility: Fixed::from_raw(200_000),
                        hostility: Fixed::from_raw(100_000),
                        publicness: Fixed::from_raw(100_000),
                        engagement: Fixed::from_raw(600_000),
                        rejection: Fixed::from_raw(100_000),
                    },
                    estimator_confidence: Fixed::from_raw(800_000),
                    estimator_digest: [98; 32],
                },
            });
            let root_event = root_event_from_r7(&event);
            let next_field = expected_production_next_field(&request, &event);
            let state_after = state_digest(&next_field, &request.formula_digest);
            let legacy_persona_scope =
                wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
            let semantic_persona_scope = semantic_persona_scope(&scope);
            let full_scope_digest = wire::scope_digest(&scope);
            let event_digest = wire::event_digest(&root_event);
            let authority_digest = authority_projection_digest(&root_event);
            let turn_binding = production_projection_turn_binding(
                1,
                &state_after,
                &[97; 16],
                &full_scope_digest,
                &event_digest,
                &authority_digest,
            );
            let input = r7_projection_fixture::matching_pre_output_input(
                1,
                state_after,
                [97; 16],
                full_scope_digest,
                turn_binding,
            );
            let database = unique_database("production-authority-reverse-order");
            let mut runtime = AstrRuntime::open(&database).expect("open runtime");
            runtime.ensure_genesis(&request).expect("commit genesis");

            let semantic = runtime
                .apply_user_stimulus_with_private_projection_wire_v1(&event, &input)
                .expect("R7 commits the first semantic transition");
            assert!(!semantic.deduplicated);
            assert_eq!(semantic.receipt.scope_digest, semantic_persona_scope);
            assert_eq!(
                semantic
                    .discard_private_projection_v1()
                    .expect("discard private semantic projection"),
                Some(PrivateProjectionTransferReceiptV1::Discarded)
            );

            let legacy = runtime
                .apply_event(&scope, &root_event)
                .expect("legacy G0 remains independently commit-able after R7");
            assert!(!legacy.deduplicated);
            assert_eq!(legacy.receipt.scope_digest, legacy_persona_scope);
            assert_eq!(legacy.receipt.event_digest, event_digest);
            assert_eq!(
                runtime
                    .current_revision(&scope)
                    .expect("semantic current revision"),
                1
            );

            runtime.flush_and_close().expect("close both durable lanes");
            drop(runtime);

            let mut reopened = AstrRuntime::open(&database).expect("reopen runtime");
            assert_eq!(
                reopened
                    .current_revision(&scope)
                    .expect("rebind R7 semantic lane"),
                1
            );
            reopened
                .audit_durable_histories_v1(&scope.bot_token, &scope.persona_token)
                .expect("crate-private audit validates both reverse-order histories");
            let replay = reopened
                .verify_replay(&scope.bot_token, &scope.persona_token)
                .expect("public replay remains on the legacy G0 lane");
            assert!(replay.ok);
            assert_eq!(replay.checked, 1);

            let retry = reopened
                .apply_user_stimulus_with_private_projection_wire_v1(&event, &input)
                .expect("exact R7 retry remains idempotent");
            assert!(retry.deduplicated);
            assert_eq!(
                retry
                    .discard_private_projection_v1()
                    .expect("deduplicated transition has no payload"),
                None
            );

            let legacy_retry = reopened
                .apply_event(&scope, &root_event)
                .expect("exact legacy retry remains idempotent");
            assert!(legacy_retry.deduplicated);

            let store = Store::open(&database).expect("open durable lanes");
            assert_eq!(store.current_revision(&legacy_persona_scope).unwrap(), 1);
            assert_eq!(store.current_revision(&semantic_persona_scope).unwrap(), 1);
            assert!(store
                .read_snapshot(&legacy_persona_scope, 1)
                .unwrap()
                .is_none());
            assert_eq!(
                store
                    .read_snapshot(&semantic_persona_scope, 1)
                    .unwrap()
                    .expect("semantic snapshot")
                    .state_digest,
                state_after
            );
        }

        #[test]
        fn semantic_expression_is_committed_deduplicated_and_restored() {
            let request = request(101);
            let mut scope = scope_for(&request);
            scope.session_token = [108; 16];
            let database = unique_database("semantic-expression");
            let mut runtime = AstrRuntime::open(&database).expect("open runtime");
            runtime.ensure_genesis(&request).expect("commit genesis");

            let proposal = r7_contracts::PerceptionProposalV1 {
                schema_version: r7_contracts::PerceptionProposalV1::SCHEMA_VERSION,
                event_id: [102; 16],
                turn_id: [103; 16],
                observed_at_ms: request.observed_at_ms,
                base_revision: 0,
                dimensions: r7_contracts::EvidenceVector {
                    positive: Fixed::from_raw(600_000),
                    affiliation: Fixed::from_raw(400_000),
                    engagement: Fixed::from_raw(500_000),
                    ..r7_contracts::EvidenceVector::default()
                },
                estimator_confidence: Fixed::from_raw(900_000),
                protocol_version: r7_contracts::PerceptionProposalV1::PROTOCOL_VERSION,
                request_nonce_digest: [104; 32],
            };
            let first = runtime
                .apply_perception_proposal_v1(&scope, &proposal)
                .expect("commit first semantic proposal");
            assert!(!first.deduplicated);
            assert_eq!(first.expression_projection.revision, first.revision);
            assert!(first
                .expression_projection
                .profile_fxp6
                .values()
                .into_iter()
                .all(|value| value <= 1_000_000));

            let duplicate = runtime
                .apply_perception_proposal_v1(&scope, &proposal)
                .expect("deduplicate the exact semantic proposal");
            assert!(duplicate.deduplicated);
            assert_eq!(duplicate.revision, first.revision);
            assert_eq!(duplicate.expression_projection, first.expression_projection);

            runtime.flush_and_close().expect("close committed field");
            drop(runtime);

            let mut reopened = AstrRuntime::open(&database).expect("reopen runtime");
            let mut second_proposal = proposal.clone();
            second_proposal.event_id = [105; 16];
            second_proposal.turn_id = [106; 16];
            second_proposal.base_revision = first.revision;
            second_proposal.request_nonce_digest = [107; 32];
            second_proposal.dimensions = r7_contracts::EvidenceVector {
                harm: Fixed::from_raw(700_000),
                boundary: Fixed::from_raw(500_000),
                hostility: Fixed::from_raw(400_000),
                rejection: Fixed::from_raw(300_000),
                ..r7_contracts::EvidenceVector::default()
            };
            let second = reopened
                .apply_perception_proposal_v1(&scope, &second_proposal)
                .expect("continue from the restored semantic field");
            assert_eq!(second.revision, first.revision + 1);
            assert_ne!(second.expression_projection, first.expression_projection);
            reopened.flush_and_close().expect("close restored field");
        }

        #[test]
        fn canonical_hot_state_rejects_truncation_counts_invalid_graph_formula_and_trailing_bytes()
        {
            let fixture = fixture("corruption-layout");
            let section_endpoints = [
                ("magic", fixture.layout.magic_end),
                ("schema-version", fixture.layout.version_end),
                ("formula", fixture.layout.formula_end),
                ("potential", fixture.layout.vector_ends[0]),
                ("excitation", fixture.layout.vector_ends[1]),
                ("inhibition", fixture.layout.vector_ends[2]),
                ("adaptation", fixture.layout.vector_ends[3]),
                ("precision", fixture.layout.vector_ends[4]),
                ("prediction-error", fixture.layout.vector_ends[5]),
                ("eligibility", fixture.layout.vector_ends[6]),
                ("metabolic-reserve", fixture.layout.vector_ends[7]),
                ("row-offsets", fixture.layout.row_end),
                ("edges", fixture.layout.edge_end - 1),
            ];
            for (section, endpoint) in section_endpoints {
                assert_rejected(
                    &format!("truncated-{section}"),
                    fixture.state_bytes[..endpoint].to_vec(),
                );
            }

            let mut oversized_vector = fixture.state_bytes.clone();
            oversized_vector[fixture.layout.vector_count_offsets[0]
                ..fixture.layout.vector_count_offsets[0] + 4]
                .copy_from_slice(&u32::MAX.to_le_bytes());
            assert_rejected("oversized-vector-count", oversized_vector);

            let mut oversized_edge = fixture.state_bytes.clone();
            oversized_edge[fixture.layout.edge_count_offset..fixture.layout.edge_count_offset + 4]
                .copy_from_slice(&u32::MAX.to_le_bytes());
            assert_rejected("oversized-edge-count", oversized_edge);

            let mut capacity_overflow = fixture.state_bytes.clone();
            capacity_overflow
                [fixture.layout.edge_count_offset..fixture.layout.edge_count_offset + 4]
                .copy_from_slice(&((EDGE_CAPACITY + 1) as u32).to_le_bytes());
            assert_rejected("edge-capacity-overflow", capacity_overflow);

            let mut non_monotonic_offsets = fixture.state_bytes.clone();
            non_monotonic_offsets
                [fixture.layout.row_value_offset + 4..fixture.layout.row_value_offset + 8]
                .copy_from_slice(&2u32.to_le_bytes());
            non_monotonic_offsets
                [fixture.layout.row_value_offset + 8..fixture.layout.row_value_offset + 12]
                .copy_from_slice(&1u32.to_le_bytes());
            assert_rejected("non-monotonic-row-offsets", non_monotonic_offsets);

            let malformed_first_row_offset =
                fixture_with_first_row_offset("nonzero-first-row-offset", 1);
            let mut malformed_runtime = AstrRuntime::open(&malformed_first_row_offset.database)
                .expect("open malformed runtime");
            assert!(
                malformed_runtime
                    .current_revision(&malformed_first_row_offset.scope)
                    .is_err(),
                "nonzero first row offset bytes must not bind HotBrain"
            );

            let mut target_out_of_bounds = fixture.state_bytes.clone();
            target_out_of_bounds
                [fixture.layout.edge_value_offset..fixture.layout.edge_value_offset + 4]
                .copy_from_slice(&(NEURON_SLOTS as u32).to_le_bytes());
            assert_rejected("edge-target-out-of-bounds", target_out_of_bounds);

            let mut formula_mismatch = fixture.state_bytes.clone();
            formula_mismatch[fixture.layout.version_end] ^= 1;
            assert_rejected("formula-mismatch", formula_mismatch);

            let mut state_digest_mismatch = fixture.state_bytes.clone();
            state_digest_mismatch[fixture.layout.vector_value_offsets[0]] ^= 1;
            assert_rejected("state-digest-mismatch", state_digest_mismatch);

            let mut graph_digest_mismatch = fixture.state_bytes.clone();
            graph_digest_mismatch[fixture.layout.edge_value_offset + 4] ^= 1;
            assert_rejected("graph-digest-mismatch", graph_digest_mismatch);

            let mut trailing_byte = fixture.state_bytes.clone();
            trailing_byte.push(0xff);
            assert_rejected("trailing-byte", trailing_byte);

            assert!(fixture.layout.row_count_offset < fixture.layout.row_value_offset);
        }
    };
}
