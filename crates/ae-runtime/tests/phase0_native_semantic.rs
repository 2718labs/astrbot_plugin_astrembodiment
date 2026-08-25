use ae_contracts::{
    evidence_vector_from_values, hex, wire, AllostaticSetpoints, EpistemicPriors, EvidenceVector,
    ExpressionPhenotype, GenesisManifestProposal, PerceptionProposalV1, PersonaGenesisRequest,
    PersonaScopeRef, PersonaSelectionKind, PersonaSourceRef, PersonalityVector, ScopeRef,
    SemanticLearningCompensationApplyV1, SemanticLearningCompensationClaimV1,
    SemanticLearningCompensationEnqueueV1, SocialPriors, TransitionReceiptV2,
};
use ae_fixed::Fixed;
use ae_neurofield::{NeuralField, SparseGraph, Synapse, NEURON_SLOTS, REGION_LAYOUT};
use ae_runtime::semantic_dynamics_v2::{propagate_semantic_dynamics_v2, DynamicsInputV2};
use ae_runtime::{
    AstrRuntime, LearningCompensationApplyDecisionV1, LearningCompensationClaimStatusV1,
    LearningCompensationEnqueueAvailabilityV1, LearningCompensationEnqueueStatusV1,
    SemanticClosureAvailabilityV1,
};
use ae_store::{Store, VaultLifecycle};
use sha2::{Digest as Sha2Digest, Sha256};
use std::path::PathBuf;

const REQUEST_NONCE_BINDING_DOMAIN_V1: &[u8] = b"astr-embodiment/spc1-request-nonce-binding-v1";
const SEMANTIC_NAMESPACE_DOMAIN_V1: &[u8] = b"astr-embodiment/semantic-lane-namespace-v1";

fn task_temp_dir(label: &str) -> PathBuf {
    let root = std::env::var_os("CODEX_TASK_TEMP")
        .map(PathBuf::from)
        .expect("CODEX_TASK_TEMP must be set for Phase 0 focused probes");
    let path = root.join(format!("phase0-native-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&path).expect("focused probe directory must be writable");
    path
}

fn genesis_request(seed: u8) -> PersonaGenesisRequest {
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
    PersonaGenesisRequest {
        source: source.clone(),
        proposal: GenesisManifestProposal {
            schema_version: 1,
            source,
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
        },
        formula_digest: [seed.wrapping_add(6); 32],
        incarnation_nonce: [seed.wrapping_add(7); 32],
        parent_incarnation_id: None,
        observed_at_ms: 1_700_000_000_000,
    }
}

fn perception_scope(request: &PersonaGenesisRequest) -> ScopeRef {
    ScopeRef {
        bot_token: request.source.scope.bot_token,
        persona_token: request.source.scope.persona_token,
        relation_token: None,
        session_token: [0x71; 16],
    }
}

fn canonical_request_nonce(scope: &ScopeRef, proposal: &PerceptionProposalV1) -> [u8; 32] {
    let relation_token = scope
        .relation_token
        .as_ref()
        .map(|token| format!("\"{}\"", hex::encode16(token)))
        .unwrap_or_else(|| "null".to_owned());
    let scope_json = format!(
        "{{\"bot_token\":\"{}\",\"persona_token\":\"{}\",\"relation_token\":{},\"session_token\":\"{}\"}}",
        hex::encode16(&scope.bot_token),
        hex::encode16(&scope.persona_token),
        relation_token,
        hex::encode16(&scope.session_token),
    );
    let binding_json = format!(
        "{{\"base_revision\":{},\"event_id\":\"{}\",\"observed_at_ms\":{},\"scope\":{},\"turn_id\":\"{}\"}}",
        proposal.base_revision,
        hex::encode16(&proposal.event_id),
        proposal.observed_at_ms,
        scope_json,
        hex::encode16(&proposal.turn_id),
    );
    let mut hasher = Sha256::new();
    hasher.update(REQUEST_NONCE_BINDING_DOMAIN_V1);
    hasher.update([0]);
    hasher.update(binding_json.as_bytes());
    let digest: [u8; 32] = hasher.finalize().into();
    if digest != [0; 32] {
        return digest;
    }
    let mut fallback = Sha256::new();
    fallback.update(REQUEST_NONCE_BINDING_DOMAIN_V1);
    fallback.update([1]);
    fallback.update(binding_json.as_bytes());
    fallback.finalize().into()
}

fn perception_proposal(
    scope: &ScopeRef,
    seed: u8,
    base_revision: u64,
    dimensions: EvidenceVector,
) -> PerceptionProposalV1 {
    let mut proposal = PerceptionProposalV1 {
        schema_version: 1,
        event_id: [seed; 16],
        turn_id: [seed.wrapping_add(1); 16],
        observed_at_ms: 1_700_000_000_200 + u64::from(seed),
        base_revision,
        dimensions,
        estimator_confidence: Fixed::ONE,
        protocol_version: 1,
        request_nonce_digest: [1; 32],
    };
    proposal.request_nonce_digest = canonical_request_nonce(scope, &proposal);
    proposal
}

fn semantic_scope(request: &PersonaGenesisRequest, incarnation_id: &[u8; 32]) -> [u8; 32] {
    let root_scope = wire::persona_scope_digest(
        &request.source.scope.bot_token,
        &request.source.scope.persona_token,
        None,
    );
    let binding = wire::domain_hash(
        SEMANTIC_NAMESPACE_DOMAIN_V1,
        &[&root_scope, incarnation_id, &request.formula_digest],
    );
    let mut relation_token = [0; 16];
    relation_token.copy_from_slice(&binding[..16]);
    wire::persona_scope_digest(
        &request.source.scope.bot_token,
        &request.source.scope.persona_token,
        Some(&relation_token),
    )
}

fn take_snapshot_block(bytes: &[u8], offset: &mut usize) -> Vec<u8> {
    let length_end = offset.checked_add(4).expect("block length overflow");
    let mut length = [0u8; 4];
    length.copy_from_slice(&bytes[*offset..length_end]);
    *offset = length_end;
    let length = usize::try_from(u32::from_le_bytes(length)).expect("block length is usize");
    let end = offset.checked_add(length).expect("block body overflow");
    let body = bytes[*offset..end].to_vec();
    *offset = end;
    body
}

fn aesem2_from_aesem3(v3: &[u8], semantic_receipt: &TransitionReceiptV2) -> Vec<u8> {
    assert!(v3.starts_with(b"AESEM3\0"));
    assert_eq!(u16::from_le_bytes([v3[7], v3[8]]), 3);
    let mut offset = 9;
    let field = take_snapshot_block(v3, &mut offset);
    let graph = take_snapshot_block(v3, &mut offset);
    let _telemetry = take_snapshot_block(v3, &mut offset);
    let _compensation = take_snapshot_block(v3, &mut offset);
    assert_eq!(offset, v3.len());

    let receipt = wire::encode_transition_receipt_v2(semantic_receipt);
    let mut v2 = Vec::new();
    v2.extend_from_slice(b"AESEM2\0");
    v2.extend_from_slice(&2u16.to_le_bytes());
    for block in [&field, &graph, &receipt] {
        v2.extend_from_slice(
            &(u32::try_from(block.len()).expect("fixture block fits u32")).to_le_bytes(),
        );
        v2.extend_from_slice(block);
    }
    v2
}

fn apply_from_claim(
    job_id: [u8; 32],
    request_digest: [u8; 32],
    claim: &ae_runtime::LearningCompensationClaimDecisionV1,
    provider_digest: [u8; 32],
    model_digest: [u8; 32],
    prompt_digest: [u8; 32],
) -> SemanticLearningCompensationApplyV1 {
    let mut teacher_values = [Fixed::ZERO; 15];
    teacher_values[0] = Fixed::ONE;
    let mut teacher_confidence = [Fixed::ZERO; 15];
    teacher_confidence[0] = Fixed::ONE;
    SemanticLearningCompensationApplyV1 {
        schema_version: 1,
        job_id,
        lease_token: claim.lease_token.expect("claimed lease token"),
        expected_request_digest: request_digest,
        expected_base_revision: claim.base_revision.expect("claimed base revision"),
        expected_formula_digest: claim.formula_digest.expect("claimed formula digest"),
        expected_telemetry_digest: claim.telemetry_digest.expect("claimed telemetry digest"),
        expected_checkpoint_digest: claim.checkpoint_digest.expect("claimed checkpoint digest"),
        teacher_vector: evidence_vector_from_values(teacher_values),
        teacher_confidence_vector: evidence_vector_from_values(teacher_confidence),
        provider_digest,
        model_digest,
        prompt_digest,
    }
}

#[test]
fn sparse_edge_propagates_source_signal_to_target_with_immutable_jacobi_state() {
    let mut field = NeuralField::zeroed();
    let baseline = NeuralField::zeroed();
    let source = 0_usize;
    let target = REGION_LAYOUT[1].0;
    field.potential[source] = Fixed::ONE;

    let mut graph = SparseGraph::empty();
    graph.edges.push(Synapse {
        target: u32::try_from(target).expect("target fits u32"),
        // Synapse i16 weights use the fixed native [-1000, 1000] scale.
        weight: 1_000,
        ..Synapse::default()
    });
    for offset in graph.row_offsets.iter_mut().skip(1) {
        *offset = 1;
    }
    assert!(graph.validate());
    assert_eq!(graph.row_offsets.len(), NEURON_SLOTS + 1);

    let result = propagate_semantic_dynamics_v2(DynamicsInputV2 {
        field: &field,
        baseline: &baseline,
        graph: &graph,
        local_by_region: [Fixed::ZERO; REGION_LAYOUT.len()],
        compensation_by_region: [Fixed::ZERO; REGION_LAYOUT.len()],
        local_confidence_by_region: [Fixed::ZERO; REGION_LAYOUT.len()],
    })
    .expect("valid edge fixture must prepare");

    assert_eq!(result.propagated_edge_count, 1);
    assert_eq!(
        result.next_field.potential[target],
        Fixed::from_raw(125_000)
    );
    assert_eq!(
        result.next_field.excitation[target],
        Fixed::from_raw(125_000)
    );
}

#[test]
fn aesem2_dedup_is_read_only_and_explicitly_unavailable() {
    let directory = task_temp_dir("aesem2");
    let path = directory.join("runtime.sqlite3");
    let request = genesis_request(61);
    let scope = perception_scope(&request);
    let proposal = perception_proposal(&scope, 62, 0, EvidenceVector::default());

    let mut runtime = AstrRuntime::open(&path).expect("runtime opens");
    let genesis = runtime.ensure_genesis(&request).expect("genesis commits");
    let first = runtime
        .apply_perception_proposal_v1(&scope, &proposal)
        .expect("Phase 0 proposal commits");
    let semantic_receipt = first
        .semantic_vector_receipt
        .clone()
        .expect("current event has semantic receipt");
    let semantic_scope = semantic_scope(&request, &genesis.incarnation_id);
    let revision = first.revision;
    runtime.flush_and_close().expect("runtime closes");
    drop(runtime);

    let authority_path = VaultLifecycle::open(directory.join("continuity-vault"))
        .expect("vault opens")
        .current_authority_database_path()
        .expect("authoritative database path");
    let mut store = Store::open(&authority_path).expect("authoritative store reopens");
    let snapshot = store
        .read_snapshot(&semantic_scope, revision)
        .expect("snapshot read")
        .expect("snapshot exists");
    let aesem2 = aesem2_from_aesem3(&snapshot.state_bytes, &semantic_receipt);
    store
        .write_snapshot(&semantic_scope, revision, &snapshot.state_digest, &aesem2)
        .expect("legacy fixture write");
    drop(store);

    let mut reopened = AstrRuntime::open(&path).expect("runtime reopens");
    let duplicate = reopened
        .apply_perception_proposal_v1(&scope, &proposal)
        .expect("legacy duplicate returns closed unavailable result");
    assert!(duplicate.deduplicated);
    assert_eq!(
        duplicate.availability,
        SemanticClosureAvailabilityV1::UnavailableLegacy
    );
    assert!(duplicate.semantic_vector_receipt.is_none());
    assert!(duplicate.semantic_telemetry_receipt.is_none());
    assert!(duplicate.node_observability.is_none());
    assert_eq!(duplicate.revision, revision);
    reopened.flush_and_close().expect("runtime closes");
    drop(reopened);

    let store = Store::open(&authority_path).expect("authoritative store reopens for readback");
    let after = store
        .read_snapshot(&semantic_scope, revision)
        .expect("snapshot readback")
        .expect("legacy snapshot remains");
    assert!(after.state_bytes.starts_with(b"AESEM2\0"));
    drop(store);
    std::fs::remove_dir_all(&directory).expect("owned focused fixture cleanup");
}

#[test]
fn compensation_append_is_cas_bound_idempotent_and_does_not_mutate_semantic_state() {
    let directory = task_temp_dir("compensation");
    let path = directory.join("runtime.sqlite3");
    let request = genesis_request(71);
    let scope = perception_scope(&request);
    let first_proposal = perception_proposal(&scope, 72, 0, EvidenceVector::default());
    let mut runtime = AstrRuntime::open(&path).expect("runtime opens");
    runtime.ensure_genesis(&request).expect("genesis commits");
    let first = runtime
        .apply_perception_proposal_v1(&scope, &first_proposal)
        .expect("source proposal commits");
    let telemetry = first
        .semantic_telemetry_receipt
        .clone()
        .expect("source telemetry is attested");

    let provider_digest = [0x91; 32];
    let model_digest = [0x92; 32];
    let prompt_digest = [0x93; 32];
    let enqueue = SemanticLearningCompensationEnqueueV1 {
        schema_version: 1,
        source_event_digest: telemetry.event_digest,
        source_text_digest: [0x94; 32],
        source_revision: first.revision,
        local_vector: EvidenceVector::default(),
        local_confidence_vector: Some(evidence_vector_from_values([Fixed::ONE; 15])),
        policy_digest: hex::decode32(
            "2d87c71d5250baaf3bb0661e2484da93991989a2f81eccf9177133e8d20536b4",
        )
        .expect("canonical all-ONE policy digest"),
        provider_digest,
        model_digest,
        prompt_digest,
        schema_digest: [0x96; 32],
        formula_digest: telemetry.formula_digest,
        local_estimator_formula_digest: [0x97; 32],
        source_telemetry_digest: telemetry.telemetry_digest,
        source_checkpoint_digest: telemetry.checkpoint_digest,
    };
    let queued = runtime
        .enqueue_learning_compensation_v1(&scope, &enqueue)
        .expect("request queues");
    assert_eq!(
        queued.availability,
        LearningCompensationEnqueueAvailabilityV1::Available
    );
    assert_eq!(queued.status, LearningCompensationEnqueueStatusV1::Queued);
    let job_id = queued.job_id.expect("queue returns native job id");
    let request_digest = queued.request_digest.expect("queue returns request digest");
    let enqueue_receipt_digest = queued
        .receipt_digest
        .expect("first queue seals a durable enqueue receipt");
    assert_eq!(queued.terminal_status, None);

    let claim = runtime
        .claim_learning_compensation_v1(
            &scope,
            &SemanticLearningCompensationClaimV1 {
                schema_version: 1,
                job_id,
                expected_request_digest: request_digest,
                previous_lease_token: None,
            },
        )
        .expect("first claim succeeds");
    assert_eq!(claim.status, LearningCompensationClaimStatusV1::Claimed);
    let deduplicated = runtime
        .enqueue_learning_compensation_v1(&scope, &enqueue)
        .expect("claimed job re-enqueue remains a queued receipt-backed job");
    assert_eq!(
        deduplicated.availability,
        LearningCompensationEnqueueAvailabilityV1::Available
    );
    assert_eq!(
        deduplicated.status,
        LearningCompensationEnqueueStatusV1::Queued
    );
    assert_eq!(deduplicated.job_id, Some(job_id));
    assert_eq!(deduplicated.request_digest, Some(request_digest));
    assert_eq!(deduplicated.receipt_digest, Some(enqueue_receipt_digest));
    assert_eq!(deduplicated.terminal_status, None);
    let stale_apply = apply_from_claim(
        job_id,
        request_digest,
        &claim,
        provider_digest,
        model_digest,
        prompt_digest,
    );

    let second_proposal = perception_proposal(
        &scope,
        73,
        first.revision,
        EvidenceVector {
            positive: Fixed::from_raw(250_000),
            ..EvidenceVector::default()
        },
    );
    let second = runtime
        .apply_perception_proposal_v1(&scope, &second_proposal)
        .expect("intervening semantic event commits");
    assert_eq!(second.revision, first.revision + 1);
    assert!(matches!(
        runtime
            .apply_learning_compensation_v1(&scope, &stale_apply)
            .expect("stale attempt is an outcome, not a mutation"),
        LearningCompensationApplyDecisionV1::StaleRetry { .. }
    ));

    let refreshed = runtime
        .claim_learning_compensation_v1(
            &scope,
            &SemanticLearningCompensationClaimV1 {
                schema_version: 1,
                job_id,
                expected_request_digest: request_digest,
                previous_lease_token: claim.lease_token,
            },
        )
        .expect("stale retry reclaims with prior lease");
    assert_eq!(refreshed.status, LearningCompensationClaimStatusV1::Claimed);
    let committed_apply = apply_from_claim(
        job_id,
        request_digest,
        &refreshed,
        provider_digest,
        model_digest,
        prompt_digest,
    );
    let committed = runtime
        .apply_learning_compensation_v1(&scope, &committed_apply)
        .expect("fresh apply commits");
    match committed {
        LearningCompensationApplyDecisionV1::Committed(receipt) => {
            assert!(receipt.changed_dimension_count >= 1);
            assert_ne!(receipt.u_next.positive, Fixed::ZERO);
        }
        other => panic!("expected compensation commit, got {other:?}"),
    }
    assert_eq!(
        runtime
            .semantic_revision_v1(&scope)
            .expect("semantic cursor"),
        second.revision
    );

    assert!(matches!(
        runtime
            .apply_learning_compensation_v1(&scope, &committed_apply)
            .expect("same apply replays durable receipt"),
        LearningCompensationApplyDecisionV1::Replayed(_)
    ));
    assert_eq!(
        runtime
            .semantic_revision_v1(&scope)
            .expect("semantic cursor"),
        second.revision
    );

    let after_compensation =
        perception_proposal(&scope, 74, second.revision, EvidenceVector::default());
    let observed = runtime
        .apply_perception_proposal_v1(&scope, &after_compensation)
        .expect("normal perception observes committed compensation");
    assert_ne!(
        observed
            .semantic_telemetry_receipt
            .expect("telemetry remains attested")
            .compensation_digest,
        telemetry.compensation_digest
    );

    // EXPIRED is a first-class native terminal, not an abandonment reason.
    let mut expiry_enqueue = enqueue.clone();
    expiry_enqueue.source_text_digest = [0xa1; 32];
    let expiry_queued = runtime
        .enqueue_learning_compensation_v1(&scope, &expiry_enqueue)
        .expect("expiry job queues");
    let expiry_job_id = expiry_queued.job_id.expect("expiry job id");
    let expiry_request_digest = expiry_queued.request_digest.expect("expiry request digest");
    let expiry_claim = runtime
        .claim_learning_compensation_v1(
            &scope,
            &SemanticLearningCompensationClaimV1 {
                schema_version: 1,
                job_id: expiry_job_id,
                expected_request_digest: expiry_request_digest,
                previous_lease_token: None,
            },
        )
        .expect("expiry job claims");
    let expired = runtime
        .expire_learning_compensation_v1(
            &scope,
            &ae_contracts::SemanticLearningCompensationTerminalV1 {
                schema_version: 1,
                job_id: expiry_job_id,
                lease_token: expiry_claim.lease_token.expect("expiry lease"),
                expected_request_digest: expiry_request_digest,
                reason_digest: [0xa2; 32],
                checkpoint_digest: expiry_claim
                    .checkpoint_digest
                    .expect("expiry checkpoint digest"),
            },
        )
        .expect("expiry seals terminal receipt");
    assert_eq!(
        expired.receipt.status,
        ae_contracts::LearningCompensationTerminalStatusV1::Expired
    );

    // A process restart has no raw text and therefore seals a pending job as a
    // durable abandonment receipt; no provider/retry is permitted afterward.
    let mut restart_enqueue = enqueue.clone();
    restart_enqueue.source_text_digest = [0xa3; 32];
    let restart_queued = runtime
        .enqueue_learning_compensation_v1(&scope, &restart_enqueue)
        .expect("restart job queues");
    let restart_job_id = restart_queued.job_id.expect("restart job id");
    let restart_request_digest = restart_queued
        .request_digest
        .expect("restart request digest");
    let restart_enqueue_receipt = restart_queued
        .receipt_digest
        .expect("restart queue seals acceptance receipt");

    runtime.flush_and_close().expect("runtime closes");
    drop(runtime);

    let mut reopened = AstrRuntime::open(&path).expect("runtime reopens");
    let recovered_enqueue = reopened
        .enqueue_learning_compensation_v1(&scope, &restart_enqueue)
        .expect("recovered terminal deterministically replays on enqueue");
    assert_eq!(
        recovered_enqueue.status,
        LearningCompensationEnqueueStatusV1::Replayed
    );
    assert_eq!(recovered_enqueue.job_id, Some(restart_job_id));
    assert_eq!(
        recovered_enqueue.terminal_status,
        Some(ae_contracts::LearningCompensationTerminalStatusV1::AbandonedInputUnavailable)
    );
    assert!(recovered_enqueue.receipt_digest.is_some());
    assert_ne!(
        recovered_enqueue.receipt_digest,
        Some(restart_enqueue_receipt)
    );
    let recovered = reopened
        .claim_learning_compensation_v1(
            &scope,
            &SemanticLearningCompensationClaimV1 {
                schema_version: 1,
                job_id: restart_job_id,
                expected_request_digest: restart_request_digest,
                previous_lease_token: None,
            },
        )
        .expect("recovered job returns sealed terminal");
    assert_eq!(
        recovered.status,
        LearningCompensationClaimStatusV1::Terminal
    );
    assert_eq!(
        recovered.terminal_status,
        Some(ae_contracts::LearningCompensationTerminalStatusV1::AbandonedInputUnavailable)
    );
    assert!(recovered.receipt_digest.is_some());
    reopened.flush_and_close().expect("reopened runtime closes");
    drop(reopened);
    std::fs::remove_dir_all(&directory).expect("owned focused fixture cleanup");
}
