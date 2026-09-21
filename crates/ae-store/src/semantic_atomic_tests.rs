use crate as ae_store;
use crate::semantic::{
    full_replay_row_visits_v1, migrate_schema, reset_full_replay_row_visits_v1,
    semantic_evidence_digest_v1, PairedSemanticCommitV1, SemanticFaultPoint,
    SEMANTIC_APPRAISAL_SCHEMA_V1_SQL, SEMANTIC_APPRAISAL_SCHEMA_V2_SQL,
};
use crate::{ClaimOutcome, GenesisCommit, Store, StoreError};
use ae_continuum::CommitEnvelope;
use ae_contracts::{
    phase0_canonical_formula_digest_v1, phase0_semantic_route_digest_v1, wire, AllostaticSetpoints,
    CanonicalEvent, CausalRef, CommitStatus, EpistemicPriors, EvidenceVector, ExpressionPhenotype,
    GenesisManifest, GenesisReceipt, GenesisStatus, InvariantResiduals, PerceptionProposalV1,
    PersonaScopeRef, PersonaSelectionKind, PersonaSourceRef, PersonalityVector, ScopeRef,
    SemanticEstimate, SocialPriors, TransitionReceipt, UserStimulus,
};
use ae_fixed::Fixed;
use ae_neurofield::{
    graph_digest, initial_state_from_manifest, state_digest, NeuralField, SparseGraph, NEURON_SLOTS,
};
use ae_semantic_core::{
    decode_canonical_semantic_snapshot_v3, derive_user_stimulus_transition_v1,
    UserStimulusTransitionInputV1,
};
use rusqlite::{OptionalExtension, TransactionBehavior};

const GENESIS_FORMULA: [u8; 32] = [0x31; 32];

struct Fixture {
    store: Store,
    database: std::path::PathBuf,
    bot: [u8; 16],
    persona: [u8; 16],
    incarnation: [u8; 32],
    manifest: GenesisManifest,
    development_seed: [u8; 32],
    initial_field: NeuralField,
    initial_graph: SparseGraph,
    cleanup_on_drop: bool,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let database = std::env::temp_dir().join(format!(
            "ae-semantic-atomic-{label}-{}-{}.db",
            std::process::id(),
            ae_store::now_ms()
        ));
        let _ = std::fs::remove_file(&database);
        let mut store = Store::open(&database).expect("open test store");
        let bot = [0x11; 16];
        let persona = [0x12; 16];
        let incarnation = [0x13; 32];
        let mut manifest = GenesisManifest {
            schema_version: 1,
            traits: PersonalityVector {
                baseline_warmth: Fixed::from_raw(620_000),
                ..PersonalityVector::default()
            },
            expression: ExpressionPhenotype::default(),
            allostasis: AllostaticSetpoints::default(),
            epistemic: EpistemicPriors::default(),
            social: SocialPriors::default(),
            manifest_digest: [0; 32],
        };
        manifest.manifest_digest = wire::manifest_body_digest(&manifest);
        let seed_code_digest = ae_genesis::derive_seed_code_digest(&manifest.manifest_digest);
        let development_seed =
            ae_genesis::derive_development_seed(&seed_code_digest, &incarnation, &GENESIS_FORMULA);
        let (initial_field, initial_graph) =
            initial_state_from_manifest(&manifest, &GENESIS_FORMULA, &development_seed);
        let initial_snapshot_digest = state_digest(&initial_field, &GENESIS_FORMULA);
        let initial_graph_digest = graph_digest(&initial_graph);
        let source = PersonaSourceRef {
            scope: PersonaScopeRef {
                bot_token: bot,
                persona_token: persona,
            },
            source_digest: [0x14; 32],
            capability_digest: [0x15; 32],
            selection: PersonaSelectionKind::Conversation,
            prompt_chars: 1,
            begin_dialog_count: 0,
            mood_dialog_count: 0,
        };
        let scope_key =
            ae_genesis::genesis_scope_key(&bot, &persona, &source.source_digest, &GENESIS_FORMULA);
        let nonce = [0x16; 32];
        let ClaimOutcome::Claimed { lease_epoch, .. } =
            store.claim_lease(&scope_key, Some(nonce)).unwrap()
        else {
            panic!("fresh lease must be claimed");
        };
        let receipt = GenesisReceipt {
            schema_version: 1,
            seed_code_digest,
            manifest_digest: manifest.manifest_digest,
            incarnation_id: incarnation,
            formula_digest: GENESIS_FORMULA,
            persona_source_digest: source.source_digest,
            compiler_protocol_digest: [0x17; 32],
            compiler_model_digest: [0x18; 32],
            development_seed_digest: development_seed,
            initial_snapshot_digest,
            graph_digest: initial_graph_digest,
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
                manifest: manifest.clone(),
                manifest_body: wire::encode_manifest_body(&manifest),
                seed_code_digest,
                incarnation_id: incarnation,
                formula_digest: GENESIS_FORMULA,
                source,
                compiler_protocol_digest: [0x17; 32],
                compiler_model_digest: [0x18; 32],
                compiled_at_ms: 1,
                receipt,
                initial_snapshot_digest,
                state_bytes: encode_field(&initial_field),
                graph_digest: initial_graph_digest,
            })
            .unwrap();
        Self {
            store,
            database,
            bot,
            persona,
            incarnation,
            manifest,
            development_seed,
            initial_field,
            initial_graph,
            cleanup_on_drop: true,
        }
    }

    fn persona_scope(&self) -> [u8; 32] {
        wire::persona_scope_digest(&self.bot, &self.persona, None)
    }

    fn semantic_commit(
        &self,
        relation_byte: u8,
        event_byte: u8,
        evidence_raw: i64,
        journal_base: u64,
        semantic_base: u64,
        graph_before: [u8; 32],
    ) -> PairedSemanticCommitV1 {
        let relation = [relation_byte; 16];
        let scope = ScopeRef {
            bot_token: self.bot,
            persona_token: self.persona,
            relation_token: Some(relation),
            session_token: [0x21; 16],
        };
        let event = CanonicalEvent::UserStimulus(UserStimulus {
            event_id: [event_byte; 16],
            scope: scope.clone(),
            causal: CausalRef {
                turn_id: [event_byte.wrapping_add(1); 16],
                action_id: None,
                delivery_id: None,
                claim_id: None,
                base_revision: journal_base,
            },
            observed_at_ms: 1_700_000_000_000 + u64::from(event_byte),
            evidence: SemanticEstimate {
                schema_version: 1,
                dimensions: EvidenceVector {
                    positive: Fixed::from_raw(evidence_raw),
                    ..EvidenceVector::default()
                },
                estimator_confidence: Fixed::from_raw(800_000),
                estimator_digest: [0x29; 32],
            },
        });
        let persona_scope = self.persona_scope();
        let relation_scope = wire::persona_scope_digest(&self.bot, &self.persona, Some(&relation));
        let event_digest = wire::event_digest(&event);
        let evidence_digest = semantic_evidence_digest_v1(&event).unwrap();
        let route_digest = phase0_semantic_route_digest_v1();
        let formula_digest = phase0_canonical_formula_digest_v1(&GENESIS_FORMULA);
        let authority_digest = ae_authority::authority_projection_digest(&event);

        // Read only the predecessor snapshot needed to assemble an untrusted
        // Task-6 candidate. Production never takes this path: Store derives
        // the same transition from authenticated DB state internally.
        let predecessor = rusqlite::Connection::open(&self.database).unwrap();
        let snapshot: Option<Vec<u8>> = predecessor
            .query_row(
                "SELECT snapshot_bytes FROM semantic_snapshots
                 WHERE persona_scope=?1 AND semantic_revision<=?2
                 ORDER BY semantic_revision DESC LIMIT 1",
                rusqlite::params![
                    persona_scope.to_vec(),
                    i64::try_from(semantic_base).unwrap()
                ],
                |row| row.get(0),
            )
            .optional()
            .unwrap();
        let (field, graph) = match snapshot {
            Some(bytes) => {
                let decoded = decode_canonical_semantic_snapshot_v3(&bytes).unwrap();
                (decoded.field, decoded.graph)
            }
            None => (self.initial_field.clone(), self.initial_graph.clone()),
        };
        assert_eq!(graph_digest(&graph), graph_before);
        let event_bytes = wire::encode_event(&event);
        let semantic_base_bytes = semantic_base.to_le_bytes();
        let request_nonce_digest = wire::domain_hash(
            b"astr-embodiment/canonical-semantic-request-nonce-v1",
            &[
                &event_bytes,
                &persona_scope,
                &relation_scope,
                &self.incarnation,
                &semantic_base_bytes,
            ],
        );
        let proposal = PerceptionProposalV1 {
            schema_version: PerceptionProposalV1::SCHEMA_VERSION,
            origin_digest: event_digest,
            dimensions: match &event {
                CanonicalEvent::UserStimulus(stimulus) => stimulus.evidence.dimensions.clone(),
                _ => unreachable!(),
            },
            estimator_confidence: Fixed::from_raw(800_000),
            protocol_version: PerceptionProposalV1::PROTOCOL_VERSION,
            request_nonce_digest,
        };
        let derived = derive_user_stimulus_transition_v1(UserStimulusTransitionInputV1 {
            field: &field,
            baseline: &self.initial_field,
            graph: &graph,
            manifest_digest: self.manifest.manifest_digest,
            development_seed_digest: self.development_seed,
            proposal: &proposal,
            formula_digest,
            scope_digest: persona_scope,
            event_digest,
            source_digest: evidence_digest,
            authority_digest,
            semantic_base_revision: semantic_base,
        })
        .unwrap();
        let journal_receipt = TransitionReceipt {
            schema_version: 1,
            formula_digest,
            scope_digest: persona_scope,
            event_digest,
            authority_digest,
            base_revision: journal_base,
            next_revision: journal_base + 1,
            state_before: derived.state_before_digest,
            state_after: derived.state_after_digest,
            graph_after: derived.graph_after_digest,
            action_contract: None,
            active_nodes: derived.active_nodes,
            active_edges: derived.active_edges,
            residuals: derived.telemetry.residuals.clone(),
            status: CommitStatus::Committed,
        };
        let chain_seed = self
            .store
            .last_chain_digest(&persona_scope)
            .unwrap()
            .unwrap_or_else(|| state_digest(&self.initial_field, &GENESIS_FORMULA));
        PairedSemanticCommitV1 {
            journal: CommitEnvelope {
                event_kind: wire::event_kind_name(&event).to_owned(),
                event_bytes,
                receipt: journal_receipt,
                chain_seed,
                delta_bytes: Vec::new(),
            },
            persona_scope,
            relation_scope: Some(relation_scope),
            semantic_base_revision: semantic_base,
            event_id: [event_byte; 16],
            event_digest,
            evidence_digest,
            estimator_digest: [0x29; 32],
            incarnation_id: self.incarnation,
            manifest_digest: self.manifest.manifest_digest,
            route_digest,
            formula_digest,
            state_digest: derived.state_after_digest,
            snapshot_bytes: derived.snapshot_bytes,
            graph_digest: derived.graph_after_digest,
            receipt_bytes: derived.semantic_receipt_bytes,
            telemetry_bytes: derived.telemetry_bytes,
        }
    }

    fn commit_delivery(&mut self, event_byte: u8, journal_base: u64) {
        let event = CanonicalEvent::DeliveryOutcome(ae_contracts::DeliveryOutcome {
            event_id: [event_byte; 16],
            scope: ScopeRef {
                bot_token: self.bot,
                persona_token: self.persona,
                relation_token: Some([0x41; 16]),
                session_token: [0x42; 16],
            },
            causal: CausalRef {
                turn_id: [0x43; 16],
                action_id: None,
                delivery_id: None,
                claim_id: None,
                base_revision: journal_base,
            },
            delivered: true,
            visible_action_digest: [0x44; 32],
            delivered_at_ms: 1_700_000_100_000,
        });
        let event_digest = wire::event_digest(&event);
        let persona_scope = self.persona_scope();
        let state = state_digest(&self.initial_field, &GENESIS_FORMULA);
        let receipt = TransitionReceipt {
            schema_version: 1,
            formula_digest: GENESIS_FORMULA,
            scope_digest: persona_scope,
            event_digest,
            authority_digest: ae_authority::authority_projection_digest(&event),
            base_revision: journal_base,
            next_revision: journal_base + 1,
            state_before: state,
            state_after: state,
            graph_after: graph_digest(&self.initial_graph),
            action_contract: None,
            active_nodes: self.initial_field.active_node_count(),
            active_edges: 0,
            residuals: InvariantResiduals::default(),
            status: CommitStatus::Committed,
        };
        let chain_seed = self
            .store
            .last_chain_digest(&persona_scope)
            .unwrap()
            .unwrap_or(state);
        self.store
            .commit_journal(&CommitEnvelope {
                event_kind: wire::event_kind_name(&event).to_owned(),
                event_bytes: wire::encode_event(&event),
                receipt,
                chain_seed,
                delta_bytes: Vec::new(),
            })
            .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if !self.cleanup_on_drop {
            return;
        }
        let _ = std::fs::remove_file(&self.database);
        let _ = std::fs::remove_file(self.database.with_extension("db-wal"));
        let _ = std::fs::remove_file(self.database.with_extension("db-shm"));
    }
}

#[test]
fn production_semantic_entry_point_never_accepts_caller_assembled_authority() {
    let mut fixture = Fixture::new("production-boundary");
    let candidate = fixture.semantic_commit(
        0x4d,
        0x4e,
        250_000,
        0,
        0,
        graph_digest(&fixture.initial_graph),
    );
    let arbitrary_origin = fixture
        .store
        .mint_perception_challenge_from_committed_inbound_v1(candidate.event_digest);
    assert!(arbitrary_origin.is_err());
    assert_eq!(fixture.store.count_journal().unwrap(), 0);
    assert_eq!(fixture.store.semantic_counts().unwrap(), (0, 0, 0, 0));
    assert_eq!(fixture.store.semantic_evidence_count().unwrap(), 0);
}

#[test]
fn canonical_event_and_semantic_sidecar_commit_together_or_not_at_all() {
    let mut fixture = Fixture::new("faults");
    let graph_before = graph_digest(&fixture.initial_graph);
    let commit = fixture.semantic_commit(0x51, 0x52, 250_000, 0, 0, graph_before);

    for fault in [
        SemanticFaultPoint::AfterJournalInsert,
        SemanticFaultPoint::AfterSemanticInsert,
    ] {
        assert!(fixture
            .store
            .commit_event_with_semantic_test_fault_v1(&commit, fault)
            .is_err());
        assert_eq!(fixture.store.semantic_counts().unwrap(), (0, 0, 0, 0));
        assert_eq!(fixture.store.semantic_evidence_count().unwrap(), 0);
        assert_eq!(fixture.store.count_journal().unwrap(), 0);
        assert_eq!(
            fixture
                .store
                .semantic_revision_v1(&fixture.persona_scope())
                .unwrap(),
            0
        );
    }

    let committed = fixture
        .store
        .commit_event_with_semantic_v1(&commit)
        .unwrap();
    assert_eq!(committed.semantic_revision, 1);
    assert_eq!(committed.journal_revision, 1);
    assert_eq!(fixture.store.semantic_counts().unwrap(), (1, 1, 1, 1));
    assert_eq!(fixture.store.semantic_evidence_count().unwrap(), 1);
    assert_eq!(fixture.store.count_journal().unwrap(), 1);
}

#[test]
fn append_verification_never_replays_existing_semantic_history() {
    let mut fixture = Fixture::new("incremental-append-verification");
    let mut graph = graph_digest(&fixture.initial_graph);
    for (offset, evidence) in [100_000, 200_000, 300_000].into_iter().enumerate() {
        let revision = u64::try_from(offset).unwrap();
        let candidate = fixture.semantic_commit(
            0x53,
            0x54 + u8::try_from(offset).unwrap(),
            evidence,
            revision,
            revision,
            graph,
        );
        graph = fixture
            .store
            .commit_event_with_semantic_v1(&candidate)
            .unwrap()
            .graph_digest;
    }

    reset_full_replay_row_visits_v1();
    let append = fixture.semantic_commit(0x53, 0x57, 400_000, 3, 3, graph);
    fixture
        .store
        .commit_event_with_semantic_v1(&append)
        .unwrap();
    assert_eq!(
        full_replay_row_visits_v1(),
        0,
        "an append may authenticate only the checkpoint and one predecessor"
    );

    fixture.store.audit_semantic_integrity_v1().unwrap();
    assert_eq!(
        full_replay_row_visits_v1(),
        4,
        "the explicit audit remains the full-history integrity boundary"
    );
}

#[test]
fn duplicate_stale_and_identity_conflict_never_split_cursors() {
    let mut fixture = Fixture::new("dedupe");
    fixture.commit_delivery(0x61, 0);
    assert_eq!(fixture.store.count_journal().unwrap(), 1);
    assert_eq!(
        fixture
            .store
            .semantic_revision_v1(&fixture.persona_scope())
            .unwrap(),
        0
    );

    let genesis_graph = graph_digest(&fixture.initial_graph);
    let first = fixture.semantic_commit(0x71, 0x72, 100_000, 1, 0, genesis_graph);
    let first_result = fixture.store.commit_event_with_semantic_v1(&first).unwrap();
    assert_eq!(
        (
            first_result.journal_revision,
            first_result.semantic_revision
        ),
        (2, 1)
    );

    // Exact retry wins before either now-stale CAS and returns stored bytes.
    let duplicate = fixture.store.commit_event_with_semantic_v1(&first).unwrap();
    assert_eq!(duplicate, first_result);
    assert_eq!(fixture.store.count_journal().unwrap(), 2);

    // Same private relation and same event ID, but different canonical event.
    let conflict = fixture.semantic_commit(0x71, 0x72, 900_000, 2, 1, first.graph_digest);
    assert!(matches!(
        fixture.store.commit_event_with_semantic_v1(&conflict),
        Err(StoreError::SemanticIdentityConflict)
    ));

    let stale = fixture.semantic_commit(0x71, 0x73, 200_000, 2, 99, first.graph_digest);
    assert!(matches!(
        fixture.store.commit_event_with_semantic_v1(&stale),
        Err(StoreError::StaleRevision {
            expected: 99,
            actual: 1
        })
    ));
    assert_eq!(fixture.store.count_journal().unwrap(), 2);
    assert_eq!(fixture.store.semantic_evidence_count().unwrap(), 1);

    // A second relation owns a disjoint event-ID namespace but advances the
    // same persona emotion cursor.
    let second = fixture.semantic_commit(0x74, 0x72, 300_000, 2, 1, first.graph_digest);
    let second_result = fixture
        .store
        .commit_event_with_semantic_v1(&second)
        .unwrap();
    assert_eq!(
        (
            second_result.journal_revision,
            second_result.semantic_revision
        ),
        (3, 2)
    );
    assert_eq!(fixture.store.semantic_evidence_count().unwrap(), 2);

    let database = fixture.database.clone();
    let persona_scope = fixture.persona_scope();
    fixture.cleanup_on_drop = false;
    drop(fixture);
    let reopened = Store::open(&database).unwrap();
    assert_eq!(reopened.current_revision(&persona_scope).unwrap(), 3);
    assert_eq!(reopened.semantic_revision_v1(&persona_scope).unwrap(), 2);
    assert_eq!(
        reopened
            .latest_semantic_v1(&persona_scope)
            .unwrap()
            .unwrap(),
        second_result
    );
    drop(reopened);
    let _ = std::fs::remove_file(database);
}

#[test]
fn missing_cursor_and_orphan_origin_fail_closed() {
    let mut missing_cursor = Fixture::new("missing-cursor");
    let genesis_graph = graph_digest(&missing_cursor.initial_graph);
    let first = missing_cursor.semantic_commit(0x81, 0x82, 100_000, 0, 0, genesis_graph);
    missing_cursor
        .store
        .commit_event_with_semantic_v1(&first)
        .unwrap();
    rusqlite::Connection::open(&missing_cursor.database)
        .unwrap()
        .execute(
            "DELETE FROM semantic_cursor WHERE persona_scope=?1",
            rusqlite::params![missing_cursor.persona_scope().to_vec()],
        )
        .unwrap();

    assert!(matches!(
        missing_cursor.store.commit_event_with_semantic_v1(&first),
        Err(StoreError::ContinuityFence("semantic_cursor_missing"))
    ));
    assert_eq!(missing_cursor.store.count_journal().unwrap(), 1);

    let mut orphan_origin = Fixture::new("orphan-origin");
    let genesis_graph = graph_digest(&orphan_origin.initial_graph);
    let committed = orphan_origin.semantic_commit(0x83, 0x84, 100_000, 0, 0, genesis_graph);
    orphan_origin
        .store
        .commit_event_with_semantic_v1(&committed)
        .unwrap();
    rusqlite::Connection::open(&orphan_origin.database)
        .unwrap()
        .execute_batch(
            "PRAGMA foreign_keys=OFF;
             DELETE FROM semantic_cursor;
             DELETE FROM semantic_evidence_authority;
             DELETE FROM semantic_telemetry;
             DELETE FROM semantic_receipts;
             DELETE FROM semantic_graphs;
             DELETE FROM semantic_snapshots;
             DELETE FROM semantic_commits;",
        )
        .unwrap();
    let append = orphan_origin.semantic_commit(0x85, 0x86, 200_000, 1, 0, genesis_graph);

    assert!(matches!(
        orphan_origin.store.commit_event_with_semantic_v1(&append),
        Err(StoreError::ContinuityFence("semantic_cursor_missing"))
    ));
    assert_eq!(orphan_origin.store.count_journal().unwrap(), 1);
}

#[test]
fn journal_only_lane_rejects_user_stimulus_without_writing() {
    let mut fixture = Fixture::new("journal-only-user-stimulus");
    let genesis_graph = graph_digest(&fixture.initial_graph);
    let candidate = fixture.semantic_commit(0x91, 0x92, 100_000, 0, 0, genesis_graph);

    assert!(matches!(
        fixture.store.commit_journal(&candidate.journal),
        Err(StoreError::SemanticInvalid(
            "user_stimulus_requires_paired_semantic_commit"
        ))
    ));
    assert_eq!(fixture.store.count_journal().unwrap(), 0);
    assert_eq!(fixture.store.semantic_counts().unwrap(), (0, 0, 0, 0));
    assert_eq!(fixture.store.semantic_evidence_count().unwrap(), 0);
}

#[test]
fn paired_store_rebinds_all_event_and_scope_identity() {
    let mut fixture = Fixture::new("store-rebinds-identity");
    let genesis_graph = graph_digest(&fixture.initial_graph);
    let mut candidate = fixture.semantic_commit(0x93, 0x94, 150_000, 0, 0, genesis_graph);
    let canonical_event = wire::decode_event(&candidate.journal.event_bytes).unwrap();
    let CanonicalEvent::UserStimulus(stimulus) = &canonical_event else {
        panic!("fixture must create a user stimulus");
    };
    let canonical_persona = wire::persona_scope_digest(
        &stimulus.scope.bot_token,
        &stimulus.scope.persona_token,
        None,
    );
    let canonical_relation = stimulus.scope.relation_token.as_ref().map(|relation| {
        wire::persona_scope_digest(
            &stimulus.scope.bot_token,
            &stimulus.scope.persona_token,
            Some(relation),
        )
    });
    let canonical_event_digest = wire::event_digest(&canonical_event);
    let canonical_evidence_digest = semantic_evidence_digest_v1(&canonical_event).unwrap();
    let canonical_estimator = stimulus.evidence.estimator_digest;
    let canonical_authority = ae_authority::authority_projection_digest(&canonical_event);

    candidate.persona_scope = [0xa1; 32];
    candidate.relation_scope = None;
    candidate.event_id = [0xa2; 16];
    candidate.event_digest = [0xa3; 32];
    candidate.evidence_digest = [0xa4; 32];
    candidate.estimator_digest = [0xa5; 32];
    candidate.journal.event_kind = "CallerClaim".to_owned();
    candidate.journal.receipt.scope_digest = [0xa6; 32];
    candidate.journal.receipt.event_digest = [0xa7; 32];
    candidate.journal.receipt.authority_digest = [0xa8; 32];
    candidate.journal.receipt.base_revision = 77;
    candidate.journal.receipt.next_revision = 78;

    let committed = fixture
        .store
        .commit_event_with_semantic_v1(&candidate)
        .unwrap();
    let persisted_receipt = wire::decode_transition_receipt(&committed.journal.receipt_bytes)
        .expect("persisted journal receipt must remain canonical");
    assert_eq!(committed.persona_scope, canonical_persona);
    assert_eq!(committed.relation_scope, canonical_relation);
    assert_eq!(committed.event_id, stimulus.event_id);
    assert_eq!(committed.event_digest, canonical_event_digest);
    assert_eq!(committed.evidence_digest, Some(canonical_evidence_digest));
    assert_eq!(committed.estimator_digest, Some(canonical_estimator));
    assert_eq!(
        committed.journal.event_kind,
        wire::event_kind_name(&canonical_event)
    );
    assert_eq!(persisted_receipt.scope_digest, canonical_persona);
    assert_eq!(persisted_receipt.event_digest, canonical_event_digest);
    assert_eq!(persisted_receipt.authority_digest, canonical_authority);
    assert_eq!(
        (
            persisted_receipt.base_revision,
            persisted_receipt.next_revision
        ),
        (0, 1)
    );
}

#[test]
fn relation_evidence_identity_drift_fails_explicit_audit_and_reopen() {
    let mut fixture = Fixture::new("relation-evidence-drift");
    let genesis_graph = graph_digest(&fixture.initial_graph);
    let first = fixture.semantic_commit(0x95, 0x96, 175_000, 0, 0, genesis_graph);
    fixture.store.commit_event_with_semantic_v1(&first).unwrap();
    rusqlite::Connection::open(&fixture.database)
        .unwrap()
        .execute_batch(
            "PRAGMA foreign_keys=OFF;
             UPDATE semantic_evidence_authority
             SET relation_present=0, relation_scope=persona_scope, event_id=zeroblob(16)
             WHERE semantic_revision=1;",
        )
        .unwrap();

    assert!(matches!(
        fixture.store.audit_semantic_integrity_v1(),
        Err(StoreError::ContinuityFence(
            "semantic_relation_identity_set"
        ))
    ));

    let database = fixture.database.clone();
    fixture.cleanup_on_drop = false;
    drop(fixture);
    assert!(matches!(
        Store::open(&database),
        Err(StoreError::ContinuityFence(
            "semantic_relation_identity_set"
        ))
    ));
    let _ = std::fs::remove_file(database);
}

#[test]
fn cursor_rewind_fails_incrementally_and_historical_deletion_fails_explicit_audit() {
    let mut rewind = Fixture::new("cursor-rewind");
    let genesis_graph = graph_digest(&rewind.initial_graph);
    let first = rewind.semantic_commit(0x97, 0x98, 100_000, 0, 0, genesis_graph);
    let first_result = rewind.store.commit_event_with_semantic_v1(&first).unwrap();
    let second = rewind.semantic_commit(0x99, 0x9a, 200_000, 1, 1, first_result.graph_digest);
    rewind.store.commit_event_with_semantic_v1(&second).unwrap();
    rusqlite::Connection::open(&rewind.database)
        .unwrap()
        .execute_batch(
            "PRAGMA foreign_keys=OFF;
             UPDATE semantic_cursor SET
               (semantic_revision,journal_revision,incarnation_id,manifest_digest,
                route_digest,formula_digest,graph_digest,state_digest,evidence_digest,
                estimator_digest,event_digest,commitment_digest) =
               (SELECT semantic_revision,journal_revision,incarnation_id,manifest_digest,
                       route_digest,formula_digest,graph_digest,state_digest,evidence_digest,
                       estimator_digest,event_digest,commitment_digest
                FROM semantic_commits
                WHERE semantic_commits.persona_scope=semantic_cursor.persona_scope
                  AND semantic_revision=1);",
        )
        .unwrap();
    assert!(matches!(
        rewind.store.latest_semantic_v1(&rewind.persona_scope()),
        Err(StoreError::ContinuityFence(
            "semantic_cursor_not_canonical_head"
        ))
    ));

    let mut missing = Fixture::new("historical-sidecar-delete");
    let genesis_graph = graph_digest(&missing.initial_graph);
    let first = missing.semantic_commit(0x9b, 0x9c, 100_000, 0, 0, genesis_graph);
    let first_result = missing.store.commit_event_with_semantic_v1(&first).unwrap();
    let second = missing.semantic_commit(0x9d, 0x9e, 200_000, 1, 1, first_result.graph_digest);
    let second_result = missing
        .store
        .commit_event_with_semantic_v1(&second)
        .unwrap();
    rusqlite::Connection::open(&missing.database)
        .unwrap()
        .execute_batch(
            "PRAGMA foreign_keys=OFF;
             DELETE FROM semantic_receipts WHERE semantic_revision=1;",
        )
        .unwrap();
    let append = missing.semantic_commit(0x9f, 0xa0, 300_000, 2, 2, second_result.graph_digest);
    missing
        .store
        .commit_event_with_semantic_v1(&append)
        .expect("incremental append must not scan historical sidecars");
    assert_eq!(missing.store.count_journal().unwrap(), 3);
    assert!(missing
        .store
        .latest_semantic_v1(&missing.persona_scope())
        .unwrap()
        .is_some());
    assert!(matches!(
        missing.store.audit_semantic_integrity_v1(),
        Err(StoreError::ContinuityFence("semantic_lane_sidecar_set"))
    ));

    let database = missing.database.clone();
    missing.cleanup_on_drop = false;
    drop(missing);
    assert!(matches!(
        Store::open(&database),
        Err(StoreError::ContinuityFence("semantic_revision_set"))
            | Err(StoreError::ContinuityFence("semantic_lane_sidecar_set"))
            | Err(StoreError::ContinuityFence("semantic_budget_checkpoint"))
    ));
    let _ = std::fs::remove_file(database);
}

#[test]
fn corrupted_historical_payload_fails_explicit_audit_and_reopen() {
    let mut fixture = Fixture::new("historical-payload-corruption");
    let genesis_graph = graph_digest(&fixture.initial_graph);
    let first = fixture.semantic_commit(0xb1, 0xb2, 100_000, 0, 0, genesis_graph);
    let first_result = fixture.store.commit_event_with_semantic_v1(&first).unwrap();
    let second = fixture.semantic_commit(0xb3, 0xb4, 200_000, 1, 1, first_result.graph_digest);
    let second_result = fixture
        .store
        .commit_event_with_semantic_v1(&second)
        .unwrap();

    let tamper = rusqlite::Connection::open(&fixture.database).unwrap();
    let mut historical: Vec<u8> = tamper
        .query_row(
            "SELECT snapshot_bytes FROM semantic_snapshots WHERE semantic_revision=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let last = historical.len() - 1;
    historical[last] ^= 1;
    tamper
        .execute(
            "UPDATE semantic_snapshots SET snapshot_bytes=?1 WHERE semantic_revision=1",
            rusqlite::params![historical],
        )
        .unwrap();
    drop(tamper);

    let append = fixture.semantic_commit(0xb5, 0xb6, 300_000, 2, 2, second_result.graph_digest);
    fixture
        .store
        .commit_event_with_semantic_v1(&append)
        .expect("incremental append must not scan historical payloads");
    assert_eq!(fixture.store.count_journal().unwrap(), 3);
    assert!(matches!(
        fixture.store.audit_semantic_integrity_v1(),
        Err(StoreError::ContinuityFence("semantic_sidecar_digest"))
    ));

    let database = fixture.database.clone();
    fixture.cleanup_on_drop = false;
    drop(fixture);
    assert!(matches!(
        Store::open(&database),
        Err(StoreError::ContinuityFence("semantic_sidecar_digest"))
    ));
    let _ = std::fs::remove_file(database);
}

fn sqlite_table_sql(conn: &rusqlite::Connection, table: &str) -> String {
    conn.query_row(
        "SELECT sql FROM sqlite_schema WHERE type='table' AND name=?1",
        rusqlite::params![table],
        |row| row.get(0),
    )
    .unwrap()
}

fn pragma_foreign_key_from_columns(conn: &rusqlite::Connection, table: &str) -> Vec<String> {
    let mut statement = conn
        .prepare(&format!("PRAGMA foreign_key_list('{table}')"))
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn semantic_persistence_shape(
    conn: &rusqlite::Connection,
) -> (
    Vec<(String, String, String, Option<String>)>,
    i64,
    i64,
    Option<Vec<u8>>,
    i64,
) {
    let mut statement = conn
        .prepare(
            "SELECT type,name,tbl_name,sql FROM sqlite_schema
             WHERE name NOT LIKE 'sqlite_%'
             ORDER BY type,name",
        )
        .unwrap();
    let schema = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let (origin_rows, origin_bytes) = conn
        .query_row(
            "SELECT COUNT(*),COALESCE(SUM(
                length(persona_scope)+length(source_scope_digest)+length(incarnation_id)+
                length(manifest_digest)+length(route_digest)+length(formula_digest)+
                length(state_digest)+length(graph_digest)+length(origin_digest)
             ),0) FROM semantic_origins",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let semantic_version = conn
        .query_row(
            "SELECT value FROM meta WHERE key='semantic_schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()
        .unwrap();
    let user_version = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    (
        schema,
        origin_rows,
        origin_bytes,
        semantic_version,
        user_version,
    )
}

type AppraisalPersistenceShape = (
    Vec<(String, String, String, Option<String>)>,
    i64,
    i64,
    i64,
    i64,
    Option<Vec<u8>>,
    i64,
);

fn appraisal_persistence_shape(conn: &rusqlite::Connection) -> AppraisalPersistenceShape {
    let mut statement = conn
        .prepare(
            "SELECT type,name,tbl_name,sql FROM sqlite_schema
             WHERE tbl_name IN ('semantic_appraisal_budget','semantic_appraisal_claim')
             ORDER BY type,name",
        )
        .unwrap();
    let schema = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let (budget_rows, budget_bytes): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*),COALESCE(SUM(length(persona_scope)),0)
             FROM semantic_appraisal_budget",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let (claim_rows, claim_bytes): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*),COALESCE(SUM(
                 length(request_nonce_digest)+length(persona_scope)+
                 length(origin_event_digest)+length(origin_digest)+length(provider_digest)+
                 COALESCE(length(CAST(outcome_code AS BLOB)),0)
             ),0) FROM semantic_appraisal_claim",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let version = conn
        .query_row(
            "SELECT value FROM meta WHERE key='semantic_appraisal_schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()
        .unwrap();
    let user_version = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    (
        schema,
        budget_rows,
        claim_rows,
        budget_bytes,
        claim_bytes,
        version,
        user_version,
    )
}

fn downgrade_to_appraisal_v1(database: &std::path::Path) {
    let connection = rusqlite::Connection::open(database).unwrap();
    connection
        .execute_batch(
            "DROP TABLE semantic_appraisal_rollup;
             DROP TABLE semantic_appraisal_claim;
             DROP TABLE semantic_appraisal_budget;
             DELETE FROM meta WHERE key='semantic_appraisal_schema_version';",
        )
        .unwrap();
    connection
        .execute_batch(SEMANTIC_APPRAISAL_SCHEMA_V1_SQL)
        .unwrap();
    connection
        .execute(
            "INSERT INTO meta(key,value) VALUES('semantic_appraisal_schema_version',X'01')",
            [],
        )
        .unwrap();
}

fn indexed_digest(tag: u8, ordinal: u64) -> Vec<u8> {
    let mut digest = [tag; 32];
    digest[24..].copy_from_slice(&ordinal.to_be_bytes());
    digest.to_vec()
}

#[derive(Clone, Copy)]
enum AppraisalV1GateCase {
    SettledClaimLimit,
    BudgetPersonaLimit,
    BudgetRowLimit,
    AggregateBytes,
    UsageOverflow,
    PendingPerPersona,
}

impl AppraisalV1GateCase {
    fn label(self) -> &'static str {
        match self {
            Self::SettledClaimLimit => "settled-claim-limit",
            Self::BudgetPersonaLimit => "budget-persona-limit",
            Self::BudgetRowLimit => "budget-row-limit",
            Self::AggregateBytes => "aggregate-bytes",
            Self::UsageOverflow => "usage-overflow",
            Self::PendingPerPersona => "pending-per-persona",
        }
    }

    fn expected(self) -> (&'static str, u64, u64) {
        match self {
            Self::SettledClaimLimit => ("semantic_appraisal.claim_rows", 4_096, 4_097),
            Self::BudgetPersonaLimit => ("semantic_appraisal.personas", 4_096, 4_097),
            Self::BudgetRowLimit => ("semantic_appraisal.budget_rows", 4_096, 4_097),
            Self::AggregateBytes => (
                "semantic_appraisal.aggregate_bytes",
                8 * 1024 * 1024,
                8 * 1024 * 1024 + 1,
            ),
            Self::UsageOverflow => ("semantic_appraisal.usage_tokens", u64::MAX, u64::MAX),
            Self::PendingPerPersona => ("semantic_appraisal.pending_rows_per_persona", 64, 65),
        }
    }
}

fn insert_appraisal_v1_gate_case(connection: &rusqlite::Connection, case: AppraisalV1GateCase) {
    let persona = indexed_digest(0xa1, 0);
    match case {
        AppraisalV1GateCase::SettledClaimLimit => {
            connection
                .execute(
                    "INSERT INTO semantic_appraisal_budget VALUES(?1,0,1000000,4097,0,0,1)",
                    [&persona],
                )
                .unwrap();
            let mut insert = connection
                .prepare(
                    "INSERT INTO semantic_appraisal_claim(
                       request_nonce_digest,persona_scope,utc_day,origin_event_digest,
                       origin_digest,provider_digest,reserved_tokens,created_at_ms,
                       settled_at_ms,charged_tokens,outcome_code,canonical_revision,
                       semantic_revision
                     ) VALUES(?1,?2,0,?3,?4,?5,1,1,1,1,'success',0,NULL)",
                )
                .unwrap();
            for ordinal in 0_u64..=4_096 {
                insert
                    .execute(rusqlite::params![
                        indexed_digest(0xa2, ordinal),
                        &persona,
                        indexed_digest(0xa3, ordinal),
                        indexed_digest(0xa4, ordinal),
                        indexed_digest(0xa5, ordinal),
                    ])
                    .unwrap();
            }
        }
        AppraisalV1GateCase::BudgetPersonaLimit => {
            let mut insert = connection
                .prepare("INSERT INTO semantic_appraisal_budget VALUES(?1,0,1000000,0,0,0,1)")
                .unwrap();
            for ordinal in 0_u64..=4_096 {
                insert.execute([indexed_digest(0xa6, ordinal)]).unwrap();
            }
        }
        AppraisalV1GateCase::BudgetRowLimit => {
            let mut insert = connection
                .prepare("INSERT INTO semantic_appraisal_budget VALUES(?1,?2,1000000,0,0,0,1)")
                .unwrap();
            for day in 0_i64..=4_096 {
                insert.execute(rusqlite::params![&persona, day]).unwrap();
            }
        }
        AppraisalV1GateCase::AggregateBytes => {
            connection
                .pragma_update(None, "ignore_check_constraints", "ON")
                .unwrap();
            connection
                .execute(
                    "INSERT INTO semantic_appraisal_budget VALUES(?1,0,1000000,1,0,0,1)",
                    [&persona],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO semantic_appraisal_claim(
                       request_nonce_digest,persona_scope,utc_day,origin_event_digest,
                       origin_digest,provider_digest,reserved_tokens,created_at_ms,
                       settled_at_ms,charged_tokens,outcome_code,canonical_revision,
                       semantic_revision
                     ) VALUES(?1,?2,0,?3,?4,?5,1,1,1,1,
                              CAST(zeroblob(?6) AS TEXT),0,NULL)",
                    rusqlite::params![
                        indexed_digest(0xa7, 0),
                        &persona,
                        indexed_digest(0xa8, 0),
                        indexed_digest(0xa9, 0),
                        indexed_digest(0xaa, 0),
                        8_i64 * 1024 * 1024 + 1,
                    ],
                )
                .unwrap();
        }
        AppraisalV1GateCase::UsageOverflow => {
            connection
                .execute(
                    "INSERT INTO semantic_appraisal_budget VALUES(?1,0,1000000,?2,0,1,1)",
                    rusqlite::params![&persona, i64::MAX],
                )
                .unwrap();
            let mut insert = connection
                .prepare(
                    "INSERT INTO semantic_appraisal_claim(
                       request_nonce_digest,persona_scope,utc_day,origin_event_digest,
                       origin_digest,provider_digest,reserved_tokens,created_at_ms,
                       settled_at_ms,charged_tokens,outcome_code,canonical_revision,
                       semantic_revision
                     ) VALUES(?1,?2,0,?3,?4,?5,1,1,1,?6,'success',0,NULL)",
                )
                .unwrap();
            for ordinal in 0_u64..3 {
                insert
                    .execute(rusqlite::params![
                        indexed_digest(0xab, ordinal),
                        &persona,
                        indexed_digest(0xac, ordinal),
                        indexed_digest(0xad, ordinal),
                        indexed_digest(0xae, ordinal),
                        i64::MAX,
                    ])
                    .unwrap();
            }
        }
        AppraisalV1GateCase::PendingPerPersona => {
            connection
                .execute(
                    "INSERT INTO semantic_appraisal_budget VALUES(?1,0,1000000,0,49920,0,1)",
                    [&persona],
                )
                .unwrap();
            let mut insert = connection
                .prepare(
                    "INSERT INTO semantic_appraisal_claim(
                       request_nonce_digest,persona_scope,utc_day,origin_event_digest,
                       origin_digest,provider_digest,reserved_tokens,created_at_ms
                     ) VALUES(?1,?2,0,?3,?4,?5,768,1)",
                )
                .unwrap();
            for ordinal in 0_u64..65 {
                insert
                    .execute(rusqlite::params![
                        indexed_digest(0xaf, ordinal),
                        &persona,
                        indexed_digest(0xb0, ordinal),
                        indexed_digest(0xb1, ordinal),
                        indexed_digest(0xb2, ordinal),
                    ])
                    .unwrap();
            }
        }
    }
}

fn assert_appraisal_v1_gate_is_pre_ddl_and_byte_exact(case: AppraisalV1GateCase) {
    let mut fixture = Fixture::new(case.label());
    let database = fixture.database.clone();
    fixture.cleanup_on_drop = false;
    drop(fixture);
    downgrade_to_appraisal_v1(&database);

    let mut connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "OFF")
        .unwrap();
    insert_appraisal_v1_gate_case(&connection, case);
    let shape_before = appraisal_persistence_shape(&connection);
    assert_eq!(shape_before.5, Some(vec![1]));
    let (resource, limit, actual) = case.expected();

    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let result = migrate_schema(&tx);
    assert!(
        matches!(
            result,
            Err(StoreError::StorageBudgetExceeded {
                resource: actual_resource,
                limit: actual_limit,
                actual: actual_value,
            }) if actual_resource == resource && actual_limit == limit && actual_value == actual
        ),
        "unexpected {} live migration result: {result:?}",
        case.label()
    );
    let v2_columns: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('semantic_appraisal_claim')
             WHERE name='terminal_receipt_digest'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(v2_columns, 0, "appraisal V2 DDL ran before admission");
    assert_eq!(appraisal_persistence_shape(&tx), shape_before);
    drop(tx);
    drop(connection);

    let file_digest_before = blake3::hash(&std::fs::read(&database).unwrap());
    let result = Store::open(&database);
    assert!(
        matches!(
            result,
            Err(StoreError::StorageBudgetExceeded {
                resource: actual_resource,
                limit: actual_limit,
                actual: actual_value,
            }) if actual_resource == resource && actual_limit == limit && actual_value == actual
        ),
        "unexpected {} Store::open result",
        case.label()
    );
    assert_eq!(
        blake3::hash(&std::fs::read(&database).unwrap()),
        file_digest_before
    );
    let inspect = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(appraisal_persistence_shape(&inspect), shape_before);
    drop(inspect);
    let _ = std::fs::remove_file(database);
}

#[test]
fn appraisal_v1_resource_gates_precede_every_schema_rewrite() {
    for case in [
        AppraisalV1GateCase::SettledClaimLimit,
        AppraisalV1GateCase::BudgetPersonaLimit,
        AppraisalV1GateCase::BudgetRowLimit,
        AppraisalV1GateCase::AggregateBytes,
        AppraisalV1GateCase::UsageOverflow,
        AppraisalV1GateCase::PendingPerPersona,
    ] {
        assert_appraisal_v1_gate_is_pre_ddl_and_byte_exact(case);
    }
}

#[test]
fn appraisal_v3_terminal_receipt_retention_is_bounded_on_every_open() {
    let mut fixture = Fixture::new("semantic-appraisal-v3-retention");
    let database = fixture.database.clone();
    fixture.cleanup_on_drop = false;
    drop(fixture);

    let connection = rusqlite::Connection::open(&database).unwrap();
    let persona = indexed_digest(0xb3, 0);
    connection
        .execute(
            "INSERT INTO semantic_appraisal_budget
             VALUES(?1,0,1000000,0,0,0,1,0,0,zeroblob(32))",
            [&persona],
        )
        .unwrap();
    let mut insert = connection
        .prepare(
            "INSERT INTO semantic_appraisal_claim(
               request_nonce_digest,persona_scope,utc_day,origin_event_digest,
               origin_digest,provider_digest,reserved_tokens,created_at_ms,
               settled_at_ms,charged_tokens,outcome_code,canonical_revision,
               semantic_revision,usage_known,usage_tokens,proposal_identity_digest,
               settlement_identity_digest,reply_affect_bytes,reply_affect_digest,
               terminal_receipt_bytes,terminal_receipt_digest
             ) VALUES(?1,?2,0,?3,?4,?5,0,1,1,0,'timeout',0,NULL,0,NULL,NULL,
                      ?6,zeroblob(16384),?7,zeroblob(16384),?8)",
        )
        .unwrap();
    for ordinal in 0_u64..256 {
        insert
            .execute(rusqlite::params![
                indexed_digest(0xb4, ordinal),
                &persona,
                indexed_digest(0xb5, ordinal),
                indexed_digest(0xb6, ordinal),
                indexed_digest(0xb7, ordinal),
                indexed_digest(0xb8, ordinal),
                indexed_digest(0xb9, ordinal),
                indexed_digest(0xba, ordinal),
            ])
            .unwrap();
    }
    drop(insert);
    let shape_before = appraisal_persistence_shape(&connection);
    drop(connection);
    let file_digest_before = blake3::hash(&std::fs::read(&database).unwrap());

    let result = Store::open(&database);
    assert!(
        matches!(
            result,
            Err(StoreError::StorageBudgetExceeded {
                resource: "semantic_appraisal.aggregate_bytes",
                limit,
                actual,
            }) if limit == 8 * 1024 * 1024 && actual > limit
        ),
        "unexpected V3 retained-receipt admission result"
    );
    assert_eq!(
        blake3::hash(&std::fs::read(&database).unwrap()),
        file_digest_before
    );
    let inspect = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(appraisal_persistence_shape(&inspect), shape_before);
    drop(inspect);
    let _ = std::fs::remove_file(database);
}

#[test]
fn challenge_global_row_limit_rejects_before_cleanup_and_open_is_byte_exact() {
    let mut fixture = Fixture::new("challenge-global-row-gate");
    fixture.cleanup_on_drop = false;
    let database = fixture.database.clone();
    drop(fixture);

    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "OFF")
        .unwrap();
    let mut insert = connection
        .prepare(
            "INSERT INTO perception_challenges(
                request_nonce_digest,challenge_secret,origin_digest,origin_bytes,persona_scope,
                bot_token,persona_token,origin_event_digest,origin_journal_revision,base_revision,
                incarnation_id,manifest_digest,created_at_ms,expires_at_ms
             ) VALUES(?1,?2,?3,X'7B7D',?4,?5,?6,?7,1,0,?8,?9,1,300001)",
        )
        .unwrap();
    for ordinal in 0_u64..=4_096 {
        let persona_ordinal = ordinal / 64;
        insert
            .execute(rusqlite::params![
                indexed_digest(0x81, ordinal),
                vec![0x82_u8; 32],
                indexed_digest(0x83, ordinal),
                indexed_digest(0x84, persona_ordinal),
                vec![0x85_u8; 16],
                vec![0x86_u8; 16],
                indexed_digest(0x87, ordinal),
                vec![0x88_u8; 32],
                vec![0x89_u8; 32],
            ])
            .unwrap();
    }
    drop(insert);
    drop(connection);
    let file_digest_before = blake3::hash(&std::fs::read(&database).unwrap());

    assert!(matches!(
        Store::open(&database),
        Err(StoreError::StorageBudgetExceeded {
            resource: "perception_challenge.rows",
            limit: 4_096,
            actual: 4_097,
        })
    ));
    assert_eq!(
        blake3::hash(&std::fs::read(&database).unwrap()),
        file_digest_before,
        "failed open must not delete or rewrite a challenge"
    );

    let _ = std::fs::remove_file(&database);
    let _ = std::fs::remove_file(database.with_extension("db-wal"));
    let _ = std::fs::remove_file(database.with_extension("db-shm"));
}

#[test]
fn public_audit_rejects_external_challenge_overflow_without_writing() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let mut fixture = Fixture::new("audit-challenge-overflow-read-only");
    let database = fixture.database.clone();
    let external = rusqlite::Connection::open(&database).unwrap();
    external.pragma_update(None, "foreign_keys", "OFF").unwrap();
    let mut insert = external
        .prepare(
            "INSERT INTO perception_challenges(
                request_nonce_digest,challenge_secret,origin_digest,origin_bytes,persona_scope,
                bot_token,persona_token,origin_event_digest,origin_journal_revision,base_revision,
                incarnation_id,manifest_digest,created_at_ms,expires_at_ms
             ) VALUES(?1,?2,?3,X'7B7D',?4,?5,?6,?7,1,0,?8,?9,1,300001)",
        )
        .unwrap();
    for ordinal in 0_u64..=4_096 {
        insert
            .execute(rusqlite::params![
                indexed_digest(0xc1, ordinal),
                vec![0xc2_u8; 32],
                indexed_digest(0xc3, ordinal),
                indexed_digest(0xc4, ordinal / 64),
                vec![0xc5_u8; 16],
                vec![0xc6_u8; 16],
                indexed_digest(0xc7, ordinal),
                vec![0xc8_u8; 32],
                vec![0xc9_u8; 32],
            ])
            .unwrap();
    }
    drop(insert);
    drop(external);

    let writes = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&writes);
    fixture.store.connection().unwrap().update_hook(Some(
        move |_: rusqlite::hooks::Action, _: &str, _: &str, _: i64| {
            observed.fetch_add(1, Ordering::SeqCst);
        },
    ));
    let wal = database.with_extension("db-wal");
    let wal_before = std::fs::read(&wal).unwrap_or_default();
    assert!(matches!(
        fixture.store.audit_semantic_integrity_v1(),
        Err(StoreError::StorageBudgetExceeded {
            resource: "perception_challenge.rows",
            limit: 4_096,
            actual: 4_097,
        })
    ));
    assert_eq!(writes.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read(&wal).unwrap_or_default(), wal_before);
}

fn downgrade_to_898b_semantic_schema(database: &std::path::Path) {
    let conn = rusqlite::Connection::open(database).unwrap();
    conn.pragma_update(None, "foreign_keys", "OFF").unwrap();

    // The fixture starts from the current schema. Remove the V4-only lane
    // projection before reconstructing the historical 898b tables.
    conn.execute_batch(
        "DROP TABLE semantic_time_authority;
         ALTER TABLE semantic_commits DROP COLUMN transition_kind;
         ALTER TABLE semantic_cursor DROP COLUMN transition_kind;
         ALTER TABLE semantic_cursor DROP COLUMN time_anchor_semantic_revision;
         ALTER TABLE semantic_cursor DROP COLUMN time_anchor_state_digest;
         ALTER TABLE semantic_cursor DROP COLUMN time_awake_ticks;
         ALTER TABLE semantic_cursor DROP COLUMN time_drowsy_ticks;
         ALTER TABLE semantic_cursor DROP COLUMN time_asleep_ticks;
         ALTER TABLE semantic_cursor DROP COLUMN time_awake_remainder_ms;
         ALTER TABLE semantic_cursor DROP COLUMN time_drowsy_remainder_ms;
         ALTER TABLE semantic_cursor DROP COLUMN time_asleep_remainder_ms;",
    )
    .unwrap();

    let commit_sql = sqlite_table_sql(&conn, "semantic_commits");
    let weak_commit_sql = commit_sql
        .replacen(
            "CREATE TABLE semantic_commits",
            "CREATE TABLE semantic_commits_898b",
            1,
        )
        .replace("            UNIQUE(persona_scope, journal_revision),\n", "")
        .replace(
            "            UNIQUE(persona_scope, semantic_revision, relation_present, relation_scope, event_id, incarnation_id, manifest_digest, route_digest, formula_digest, graph_digest, state_digest, evidence_digest, estimator_digest, event_digest, commitment_digest),\n",
            "",
        );
    assert_ne!(weak_commit_sql, commit_sql);
    conn.execute_batch(&weak_commit_sql).unwrap();
    conn.execute(
        "INSERT INTO semantic_commits_898b SELECT * FROM semantic_commits",
        [],
    )
    .unwrap();

    // Recreate the actual pre-V2 898b evidence layout.  V3 adds four
    // perception receipt columns and two supplemental tables; carrying those
    // backwards made this fixture an impossible hybrid rather than a valid
    // legacy database.
    conn.execute_batch(
        "CREATE TABLE semantic_evidence_authority_898b (
            persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
            semantic_revision INTEGER NOT NULL CHECK(typeof(semantic_revision)='integer' AND semantic_revision>0),
            relation_present INTEGER NOT NULL CHECK(typeof(relation_present)='integer' AND relation_present IN (0,1)),
            relation_scope BLOB NOT NULL CHECK(typeof(relation_scope)='blob' AND length(relation_scope)=32),
            event_id BLOB NOT NULL CHECK(typeof(event_id)='blob' AND length(event_id)=16),
            incarnation_id BLOB NOT NULL CHECK(typeof(incarnation_id)='blob' AND length(incarnation_id)=32),
            manifest_digest BLOB NOT NULL CHECK(typeof(manifest_digest)='blob' AND length(manifest_digest)=32),
            route_digest BLOB NOT NULL CHECK(typeof(route_digest)='blob' AND length(route_digest)=32),
            formula_digest BLOB NOT NULL CHECK(typeof(formula_digest)='blob' AND length(formula_digest)=32),
            graph_digest BLOB NOT NULL CHECK(typeof(graph_digest)='blob' AND length(graph_digest)=32),
            state_digest BLOB NOT NULL CHECK(typeof(state_digest)='blob' AND length(state_digest)=32),
            evidence_digest BLOB NOT NULL CHECK(typeof(evidence_digest)='blob' AND length(evidence_digest)=32),
            estimator_digest BLOB NOT NULL CHECK(typeof(estimator_digest)='blob' AND length(estimator_digest)=32),
            event_digest BLOB NOT NULL CHECK(typeof(event_digest)='blob' AND length(event_digest)=32),
            commitment_digest BLOB NOT NULL CHECK(typeof(commitment_digest)='blob' AND length(commitment_digest)=32),
            evidence_bytes BLOB NOT NULL CHECK(typeof(evidence_bytes)='blob' AND length(evidence_bytes)<=262144),
            PRIMARY KEY(persona_scope,relation_present,relation_scope,event_id),
            UNIQUE(persona_scope,semantic_revision),
            CHECK(relation_present=1 OR relation_scope=persona_scope),
            FOREIGN KEY(persona_scope,semantic_revision,incarnation_id,manifest_digest,
                        route_digest,formula_digest,graph_digest,state_digest,evidence_digest,
                        estimator_digest,event_digest,commitment_digest)
              REFERENCES semantic_commits(persona_scope,semantic_revision,incarnation_id,
                        manifest_digest,route_digest,formula_digest,graph_digest,state_digest,
                        evidence_digest,estimator_digest,event_digest,commitment_digest)
         );
         INSERT INTO semantic_evidence_authority_898b(
            persona_scope,semantic_revision,relation_present,relation_scope,event_id,
            incarnation_id,manifest_digest,route_digest,formula_digest,graph_digest,
            state_digest,evidence_digest,estimator_digest,event_digest,commitment_digest,
            evidence_bytes)
         SELECT persona_scope,semantic_revision,relation_present,relation_scope,event_id,
            incarnation_id,manifest_digest,route_digest,formula_digest,graph_digest,
            state_digest,evidence_digest,estimator_digest,event_digest,commitment_digest,
            evidence_bytes
         FROM semantic_evidence_authority;",
    )
    .unwrap();

    conn.execute_batch(
        "DROP INDEX IF EXISTS semantic_commit_journal_identity_v1;
         DROP INDEX IF EXISTS semantic_commit_relation_identity_v1;
         DROP INDEX IF EXISTS semantic_evidence_relation_revision;
         DROP INDEX IF EXISTS semantic_evidence_perception_nonce_v1;
         DROP INDEX IF EXISTS perception_challenge_context_v1;
         DROP INDEX IF EXISTS perception_challenge_persona_pending_v1;
         DROP INDEX IF EXISTS perception_challenge_expiry_v1;
         DROP TABLE perception_challenges;
         DROP TABLE semantic_budget_checkpoint;
         DROP TABLE semantic_evidence_authority;
         DROP TABLE semantic_commits;
         ALTER TABLE semantic_commits_898b RENAME TO semantic_commits;
         ALTER TABLE semantic_evidence_authority_898b RENAME TO semantic_evidence_authority;
         CREATE UNIQUE INDEX semantic_commit_checkpoint_identity_v1
           ON semantic_commits(persona_scope,semantic_revision,commitment_digest);
         CREATE INDEX semantic_evidence_relation_revision
           ON semantic_evidence_authority(persona_scope, relation_present, relation_scope, semantic_revision);
         DELETE FROM meta WHERE key='semantic_schema_version';",
    )
    .unwrap();
    assert_eq!(
        pragma_foreign_key_from_columns(&conn, "semantic_evidence_authority").len(),
        12
    );
}

fn downgrade_to_v4_semantic_schema(database: &std::path::Path) {
    let conn = rusqlite::Connection::open(database).unwrap();
    conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
    conn.execute_batch(
        "DROP TABLE semantic_time_authority;
         CREATE TABLE semantic_time_authority (
            persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
            semantic_revision INTEGER NOT NULL CHECK(typeof(semantic_revision)='integer' AND semantic_revision>0),
            journal_revision INTEGER NOT NULL UNIQUE CHECK(typeof(journal_revision)='integer' AND journal_revision>0),
            event_id BLOB NOT NULL CHECK(typeof(event_id)='blob' AND length(event_id)=16),
            event_digest BLOB NOT NULL CHECK(typeof(event_digest)='blob' AND length(event_digest)=32),
            authority_digest BLOB NOT NULL CHECK(typeof(authority_digest)='blob' AND length(authority_digest)=32),
            authority_bytes BLOB NOT NULL CHECK(typeof(authority_bytes)='blob' AND length(authority_bytes)<=16384),
            PRIMARY KEY(persona_scope,semantic_revision),
            UNIQUE(persona_scope,event_id),
            UNIQUE(persona_scope,event_digest),
            FOREIGN KEY(persona_scope,semantic_revision)
              REFERENCES semantic_commits(persona_scope,semantic_revision)
         );
         CREATE INDEX semantic_time_journal_v1
           ON semantic_time_authority(persona_scope,journal_revision);
         UPDATE meta SET value=X'04' WHERE key='semantic_schema_version';",
    )
    .unwrap();
}

fn downgrade_to_v3_semantic_schema(database: &std::path::Path) {
    let conn = rusqlite::Connection::open(database).unwrap();
    conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
    conn.execute_batch(
        "DROP TABLE semantic_time_authority;
         ALTER TABLE semantic_commits DROP COLUMN transition_kind;
         ALTER TABLE semantic_cursor DROP COLUMN transition_kind;
         ALTER TABLE semantic_cursor DROP COLUMN time_anchor_semantic_revision;
         ALTER TABLE semantic_cursor DROP COLUMN time_anchor_state_digest;
         ALTER TABLE semantic_cursor DROP COLUMN time_awake_ticks;
         ALTER TABLE semantic_cursor DROP COLUMN time_drowsy_ticks;
         ALTER TABLE semantic_cursor DROP COLUMN time_asleep_ticks;
         ALTER TABLE semantic_cursor DROP COLUMN time_awake_remainder_ms;
         ALTER TABLE semantic_cursor DROP COLUMN time_drowsy_remainder_ms;
         ALTER TABLE semantic_cursor DROP COLUMN time_asleep_remainder_ms;
         UPDATE meta SET value=X'03' WHERE key='semantic_schema_version';",
    )
    .unwrap();
}

#[test]
fn over_budget_direct_v3_fails_before_v4_ddl_and_open_is_byte_exact() {
    let mut fixture = Fixture::new("v3-schema-budget-preflight");
    let database = fixture.database.clone();
    fixture.cleanup_on_drop = false;
    drop(fixture);

    downgrade_to_v3_semantic_schema(&database);
    let mut connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "OFF")
        .unwrap();
    {
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        {
            let mut insert = tx
                .prepare(
                    "INSERT INTO semantic_budget_checkpoint(
                       persona_scope,row_count,aggregate_bytes,head_revision,
                       head_commitment_digest
                     ) VALUES(?1,1,0,1,?2)",
                )
                .unwrap();
            for ordinal in 0_u64..=4_096 {
                let mut persona_scope = [0xec_u8; 32];
                persona_scope[24..].copy_from_slice(&ordinal.to_be_bytes());
                insert
                    .execute(rusqlite::params![persona_scope.to_vec(), vec![0x91_u8; 32]])
                    .unwrap();
            }
        }
        tx.commit().unwrap();
    }
    let shape_before = semantic_persistence_shape(&connection);
    let checkpoint_rows_before: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM semantic_budget_checkpoint",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(checkpoint_rows_before, 4_097);
    assert_eq!(shape_before.3, Some(vec![3]));

    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(matches!(
        migrate_schema(&tx),
        Err(StoreError::StorageBudgetExceeded {
            resource: "semantic.history.global_personas",
            limit: 4_096,
            actual: 4_097,
        })
    ));
    let transition_kind_columns: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('semantic_commits')
             WHERE name='transition_kind'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let time_tables: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema
             WHERE type='table' AND name='semantic_time_authority'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(transition_kind_columns, 0, "V4 ALTER ran before admission");
    assert_eq!(time_tables, 0, "V4 CREATE ran before admission");
    assert_eq!(semantic_persistence_shape(&tx), shape_before);
    drop(tx);
    drop(connection);

    let file_digest_before = blake3::hash(&std::fs::read(&database).unwrap());
    assert!(matches!(
        Store::open(&database),
        Err(StoreError::StorageBudgetExceeded {
            resource: "semantic.history.global_personas",
            limit: 4_096,
            actual: 4_097,
        })
    ));
    assert_eq!(
        blake3::hash(&std::fs::read(&database).unwrap()),
        file_digest_before
    );
    let inspect = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(semantic_persistence_shape(&inspect), shape_before);
    assert_eq!(
        inspect
            .query_row(
                "SELECT COUNT(*) FROM semantic_budget_checkpoint",
                [],
                |row| { row.get::<_, i64>(0) }
            )
            .unwrap(),
        checkpoint_rows_before
    );
    drop(inspect);
    let _ = std::fs::remove_file(database);
}

#[test]
fn direct_v3_with_authenticated_history_migrates_normally_to_v5() {
    let mut fixture = Fixture::new("v3-normal-migration");
    let candidate = fixture.semantic_commit(
        0xeb,
        0xea,
        350_000,
        0,
        0,
        graph_digest(&fixture.initial_graph),
    );
    let committed = fixture
        .store
        .commit_event_with_semantic_v1(&candidate)
        .unwrap();
    let persona_scope = fixture.persona_scope();
    let database = fixture.database.clone();
    fixture.cleanup_on_drop = false;
    drop(fixture);

    downgrade_to_v3_semantic_schema(&database);
    let reopened = Store::open(&database).expect("bounded exact V3 history must migrate");
    assert_eq!(
        reopened.latest_semantic_v1(&persona_scope).unwrap(),
        Some(committed)
    );
    reopened.close().unwrap();
    let inspect = rusqlite::Connection::open(&database).unwrap();
    let version: Vec<u8> = inspect
        .query_row(
            "SELECT value FROM meta WHERE key='semantic_schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, vec![5]);
    drop(inspect);
    let _ = std::fs::remove_file(database);
}

#[test]
fn weak_898b_schema_is_validated_rebuilt_and_preserves_semantic_identity() {
    let mut fixture = Fixture::new("weak-schema-migration");
    let candidate = fixture.semantic_commit(
        0xc1,
        0xc2,
        350_000,
        0,
        0,
        graph_digest(&fixture.initial_graph),
    );
    let committed = fixture
        .store
        .commit_event_with_semantic_v1(&candidate)
        .unwrap();
    let persona_scope = fixture.persona_scope();
    let database = fixture.database.clone();
    fixture.cleanup_on_drop = false;
    drop(fixture);

    downgrade_to_898b_semantic_schema(&database);
    let reopened = Store::open(&database).expect("known weak schema must migrate");
    assert_eq!(
        reopened.latest_semantic_v1(&persona_scope).unwrap(),
        Some(committed)
    );

    let inspect = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        pragma_foreign_key_from_columns(&inspect, "semantic_cursor").len(),
        12
    );
    assert_eq!(
        pragma_foreign_key_from_columns(&inspect, "semantic_evidence_authority").len(),
        15
    );
    let foreign_key_failures: i64 = inspect
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(foreign_key_failures, 0);
    let schema_version: Vec<u8> = inspect
        .query_row(
            "SELECT value FROM meta WHERE key='semantic_schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(schema_version, vec![5]);
    drop(inspect);
    drop(reopened);
    let _ = std::fs::remove_file(database);
}

#[test]
fn over_budget_weak_schema_failed_open_preserves_bytes_rows_schema_and_versions() {
    let mut fixture = Fixture::new("weak-schema-budget-preflight");
    let candidate = fixture.semantic_commit(
        0xd1,
        0xd2,
        350_000,
        0,
        0,
        graph_digest(&fixture.initial_graph),
    );
    fixture
        .store
        .commit_event_with_semantic_v1(&candidate)
        .unwrap();
    let database = fixture.database.clone();
    fixture.cleanup_on_drop = false;
    drop(fixture);

    downgrade_to_898b_semantic_schema(&database);
    let mut connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "OFF")
        .unwrap();
    {
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        {
            let mut insert = tx
                .prepare(
                    "INSERT INTO semantic_origins(
                       persona_scope,source_scope_digest,source_revision,legacy_migrated,
                       incarnation_id,manifest_digest,route_digest,formula_digest,
                       state_digest,graph_digest,origin_digest
                     ) SELECT ?1,source_scope_digest,source_revision,legacy_migrated,
                              incarnation_id,manifest_digest,route_digest,formula_digest,
                              state_digest,graph_digest,origin_digest
                       FROM semantic_origins LIMIT 1",
                )
                .unwrap();
            for ordinal in 0_u64..4_096 {
                let mut persona_scope = [0xee_u8; 32];
                persona_scope[24..].copy_from_slice(&ordinal.to_be_bytes());
                insert.execute([persona_scope.to_vec()]).unwrap();
            }
        }
        tx.commit().unwrap();
    }
    let shape_before = semantic_persistence_shape(&connection);
    assert_eq!(shape_before.1, 4_097);
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(matches!(
        migrate_schema(&tx),
        Err(StoreError::StorageBudgetExceeded {
            resource: "semantic.history.global_personas",
            limit: 4_096,
            actual: 4_097,
        })
    ));
    assert_eq!(semantic_persistence_shape(&tx), shape_before);
    drop(tx);
    drop(connection);

    let file_digest_before = blake3::hash(&std::fs::read(&database).unwrap());
    assert!(matches!(
        Store::open(&database),
        Err(StoreError::StorageBudgetExceeded {
            resource: "semantic.history.global_personas",
            limit: 4_096,
            actual: 4_097,
        })
    ));
    assert_eq!(
        blake3::hash(&std::fs::read(&database).unwrap()),
        file_digest_before
    );
    let inspect = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(semantic_persistence_shape(&inspect), shape_before);
    drop(inspect);
    let _ = std::fs::remove_file(database);
}

#[test]
fn over_budget_v4_failed_open_preserves_bytes_rows_schema_and_versions() {
    let mut fixture = Fixture::new("v4-schema-budget-preflight");
    let candidate = fixture.semantic_commit(
        0xe1,
        0xe2,
        350_000,
        0,
        0,
        graph_digest(&fixture.initial_graph),
    );
    fixture
        .store
        .commit_event_with_semantic_v1(&candidate)
        .unwrap();
    let database = fixture.database.clone();
    fixture.cleanup_on_drop = false;
    drop(fixture);

    downgrade_to_v4_semantic_schema(&database);
    let mut connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "OFF")
        .unwrap();
    {
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        {
            let mut insert = tx
                .prepare(
                    "INSERT INTO semantic_origins(
                       persona_scope,source_scope_digest,source_revision,legacy_migrated,
                       incarnation_id,manifest_digest,route_digest,formula_digest,
                       state_digest,graph_digest,origin_digest
                     ) SELECT ?1,source_scope_digest,source_revision,legacy_migrated,
                              incarnation_id,manifest_digest,route_digest,formula_digest,
                              state_digest,graph_digest,origin_digest
                       FROM semantic_origins LIMIT 1",
                )
                .unwrap();
            for ordinal in 0_u64..4_096 {
                let mut persona_scope = [0xed_u8; 32];
                persona_scope[24..].copy_from_slice(&ordinal.to_be_bytes());
                insert.execute([persona_scope.to_vec()]).unwrap();
            }
        }
        tx.commit().unwrap();
    }
    let shape_before = semantic_persistence_shape(&connection);
    assert_eq!(shape_before.1, 4_097);
    assert_eq!(shape_before.3, Some(vec![4]));
    drop(connection);

    let file_digest_before = blake3::hash(&std::fs::read(&database).unwrap());
    assert!(matches!(
        Store::open(&database),
        Err(StoreError::StorageBudgetExceeded {
            resource: "semantic.history.global_personas",
            limit: 4_096,
            actual: 4_097,
        })
    ));
    assert_eq!(
        blake3::hash(&std::fs::read(&database).unwrap()),
        file_digest_before
    );
    let inspect = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(semantic_persistence_shape(&inspect), shape_before);
    drop(inspect);
    let _ = std::fs::remove_file(database);
}

#[test]
fn semantic_appraisal_v1_and_v2_schemas_rebuild_to_frozen_v3() {
    for (label, schema, version_byte) in [
        (
            "semantic-appraisal-v1-to-v3",
            SEMANTIC_APPRAISAL_SCHEMA_V1_SQL,
            1_u8,
        ),
        (
            "semantic-appraisal-v2-to-v3",
            SEMANTIC_APPRAISAL_SCHEMA_V2_SQL,
            2_u8,
        ),
    ] {
        let mut fixture = Fixture::new(label);
        let database = fixture.database.clone();
        fixture.cleanup_on_drop = false;
        drop(fixture);

        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "DROP TABLE semantic_appraisal_rollup;
                 DROP TABLE semantic_appraisal_claim;
                 DROP TABLE semantic_appraisal_budget;
                 DELETE FROM meta WHERE key='semantic_appraisal_schema_version';",
            )
            .unwrap();
        connection.execute_batch(schema).unwrap();
        connection
            .execute(
                "INSERT INTO meta(key,value) VALUES('semantic_appraisal_schema_version',?1)",
                [vec![version_byte]],
            )
            .unwrap();
        drop(connection);

        let reopened = Store::open(&database).expect("exact appraisal schema must migrate");
        reopened.close().unwrap();
        let inspect = rusqlite::Connection::open(&database).unwrap();
        let version: Vec<u8> = inspect
            .query_row(
                "SELECT value FROM meta WHERE key='semantic_appraisal_schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, vec![3]);
        let receipt_columns: i64 = inspect
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('semantic_appraisal_claim')
                 WHERE name IN ('usage_known','usage_tokens','proposal_identity_digest',
                                'settlement_identity_digest','reply_affect_bytes',
                                'reply_affect_digest','terminal_receipt_bytes',
                                'terminal_receipt_digest')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(receipt_columns, 8);
        let compact_columns: i64 = inspect
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('semantic_appraisal_budget')
                 WHERE name IN ('compacted_claim_rows','compacted_charged_tokens',
                                'compacted_chain_digest')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(compact_columns, 3);
        assert_eq!(
            inspect
                .query_row(
                    "SELECT COUNT(*) FROM semantic_appraisal_rollup",
                    [],
                    |row| { row.get::<_, i64>(0) }
                )
                .unwrap(),
            1
        );
        drop(inspect);
        let _ = std::fs::remove_file(database);
    }
}

#[test]
fn unattested_semantic_appraisal_v1_terminal_is_rejected_before_migration_ddl() {
    let mut fixture = Fixture::new("semantic-appraisal-v1-unattested");
    let database = fixture.database.clone();
    fixture.cleanup_on_drop = false;
    drop(fixture);
    downgrade_to_appraisal_v1(&database);

    let connection = rusqlite::Connection::open(&database).unwrap();
    let persona = indexed_digest(0xc1, 0);
    connection
        .execute(
            "INSERT INTO semantic_appraisal_budget VALUES(?1,0,1000000,7,0,0,1)",
            [&persona],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO semantic_appraisal_claim(
               request_nonce_digest,persona_scope,utc_day,origin_event_digest,
               origin_digest,provider_digest,reserved_tokens,created_at_ms,
               settled_at_ms,charged_tokens,outcome_code,canonical_revision,
               semantic_revision
             ) VALUES(?1,?2,0,?3,?4,?5,7,1,2,7,'success',0,NULL)",
            rusqlite::params![
                indexed_digest(0xc2, 0),
                &persona,
                indexed_digest(0xc3, 0),
                indexed_digest(0xc4, 0),
                indexed_digest(0xc5, 0),
            ],
        )
        .unwrap();
    let before = appraisal_persistence_shape(&connection);
    drop(connection);

    assert!(Store::open(&database).is_err());
    let inspect = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(appraisal_persistence_shape(&inspect), before);
    let migrated_columns: i64 = inspect
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('semantic_appraisal_claim')
             WHERE name='terminal_receipt_digest'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(migrated_columns, 0);
    drop(inspect);
    let _ = std::fs::remove_file(database);
}

fn encode_field(field: &NeuralField) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 * (4 + NEURON_SLOTS * 8));
    for values in [
        &field.potential,
        &field.excitation,
        &field.inhibition,
        &field.adaptation,
        &field.precision,
        &field.prediction_error,
        &field.eligibility,
        &field.metabolic_reserve,
    ] {
        out.extend_from_slice(&(values.len() as u32).to_le_bytes());
        for value in values {
            out.extend_from_slice(&value.encode());
        }
    }
    out
}
