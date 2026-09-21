// Immutable-source finite migration acceptance.
//
// The AESEM2 writers, predecessor replay, graph/context sidecars, and context
// projection below are independent fixtures derived from immutable source
// commit `710829ae5d3bef82ce818754354272517cb28056`:
// - `ae-store/src/lib.rs` blob `657c8f43f74b45a6ef6e623b5398b4ac5eed6153`
// - `ae-store/src/semantic_field_attestation.rs` blob
//   `9b7c272c10051f4406cabc448388111ae9993dad`
// - `ae-context-projector/src/store.rs` blob
//   `0aa1f3912e9f3d3f63a3dbfc75c90b73b41dd6cd`
// They intentionally do not call a current runtime/store snapshot encoder or
// migration replay helper.

use ae_attention::emotion_matrix::assemble_full_vector_load;
use ae_authority::authority_projection_digest;
use ae_continuum::{chain_link, CommitEnvelope};
use ae_contracts::{
    legacy_reserved_zero_digest_v1, phase0_canonical_formula_digest_v1, wire, AllostaticSetpoints,
    CanonicalEvent, CapacityTelemetryV1, CausalRef, CommitStatus, EnergyTelemetryV1,
    EpistemicPriors, EvidenceVector, ExpressionPhenotype, GenesisManifest, GenesisReceipt,
    GenesisStatus, InvariantResiduals, NativeTelemetryFormulaV1, NativeTelemetryPhaseV1,
    NativeTelemetryReceiptV1, PersonaScopeRef, PersonaSelectionKind, PersonaSourceRef,
    PersonalityVector, ScopeRef, SemanticEstimate, SocialPriors, TransitionReceipt, UserStimulus,
    NATIVE_TELEMETRY_RECEIPT_SCHEMA_V1,
};
use ae_fixed::Fixed;
use ae_neurofield::{
    graph_digest, initial_state_from_manifest, state_digest, NeuralField, SparseGraph,
    EDGE_CAPACITY, NEURON_SLOTS, REGION_LAYOUT,
};
#[cfg(feature = "migration-test-hooks")]
use ae_store::{
    set_semantic_migration_test_failpoint_v1, set_semantic_migration_test_hook_v1,
    SemanticMigrationTestFailpointV1, SemanticMigrationTestHookV1,
};
use ae_store::{
    ClaimOutcome, GenesisCommit, LegacySemanticFieldDomainUpgradeV1,
    LegacySemanticFormulaUpgradeReceiptV1, SemanticMigrationOutcomeV1, SemanticMigrationRequestV2,
    Store, StoreError, JOINT_MAX_LINEAR_FXP6_V1, LEGACY_FIELD_FXP6_SCALE,
};
use rusqlite::{params, Connection, TransactionBehavior};
use sha2::{Digest as ShaDigest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const SEMANTIC_LANE_NAMESPACE_DOMAIN_V1: &[u8] = b"astr-embodiment/semantic-lane-namespace-v1";
const CONTEXT_DIGEST_DOMAIN_V1: &[u8] = b"AE-CONTEXT-PROJECTION-STATE-V1";
const CONTEXT_MAGIC_V1: &[u8; 8] = b"AECPSTV1";
const CONTEXT_RELATION_HMAC_KEY_V1: [u8; 32] = [
    0x6d, 0x18, 0x4a, 0xf3, 0x82, 0x97, 0x51, 0x0c, 0x34, 0xbe, 0x76, 0x29, 0xd1, 0x45, 0xa8, 0x63,
    0x9b, 0x27, 0xce, 0x40, 0x75, 0x1f, 0xe2, 0x5a, 0xb4, 0x08, 0x9d, 0x36, 0xc1, 0x7e, 0x52, 0xfa,
];
const LEGACY_NEUTRAL_RELAXATION_MAX_RATE: Fixed = Fixed::from_raw(125_000);

static NEXT_DIR: AtomicU64 = AtomicU64::new(1);

struct Fixture {
    store: Store,
    database: PathBuf,
    source_bytes: Vec<Vec<u8>>,
    source_fields: Vec<NeuralField>,
    source_receipts: Vec<TransitionReceipt>,
    graph: SparseGraph,
    target_bytes: Vec<u8>,
    scope_digest: [u8; 32],
    envelope: CommitEnvelope,
    upgrade: LegacySemanticFormulaUpgradeReceiptV1,
    incarnation_id: [u8; 32],
    manifest_digest: [u8; 32],
    scope: ScopeRef,
}

impl Fixture {
    fn migrate(&mut self) -> Result<SemanticMigrationOutcomeV1, ae_store::StoreError> {
        self.store
            .migrate_legacy_semantic_snapshot_v2(SemanticMigrationRequestV2 {
                scope: &self.scope,
                expected_source_revision: self.upgrade.base_revision,
                expected_source_state_digest: self.upgrade.source_state_digest,
                expected_source_graph_digest: self.upgrade.source_graph_digest,
                expected_source_history_root: self.upgrade.prior_chain_digest,
                expected_incarnation_id: self.incarnation_id,
                expected_manifest_digest: self.manifest_digest,
            })
    }
}

#[derive(Debug, PartialEq, Eq)]
struct AuthorityState {
    journal: Vec<(
        i64,
        i64,
        String,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
    )>,
    snapshots: Vec<(i64, Vec<u8>, Vec<u8>)>,
    graphs: Vec<(i64, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>)>,
    contexts: Vec<(i64, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>)>,
    applied_events: i64,
    migrations: (i64, i64),
}

#[derive(Clone)]
struct ContextTurnFixtureV1 {
    event_id: [u8; 16],
    receipt_digest: [u8; 32],
    revision: u64,
    dimensions: [i64; 15],
    boundary: bool,
    repair: bool,
}

#[derive(Clone, Default)]
struct ContextFixtureV1 {
    turns: Vec<ContextTurnFixtureV1>,
    summary_revision: u32,
}

fn unique_database(label: &str) -> PathBuf {
    let serial = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "ae-store-task5-{label}-{}-{serial}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root.join("authority.sqlite")
}

fn evidence_all_one() -> EvidenceVector {
    EvidenceVector {
        positive: Fixed::ONE,
        affiliation: Fixed::ONE,
        harm: Fixed::ONE,
        boundary: Fixed::ONE,
        repair: Fixed::ONE,
        repetition: Fixed::ONE,
        new_information: Fixed::ONE,
        constraint_instability: Fixed::ONE,
        epistemic_conflict: Fixed::ONE,
        self_responsibility: Fixed::ONE,
        other_responsibility: Fixed::ONE,
        hostility: Fixed::ONE,
        publicness: Fixed::ONE,
        engagement: Fixed::ONE,
        rejection: Fixed::ONE,
    }
}

fn event(
    scope: ScopeRef,
    id: u8,
    base_revision: u64,
    dimensions: EvidenceVector,
) -> CanonicalEvent {
    CanonicalEvent::UserStimulus(UserStimulus {
        event_id: [id; 16],
        scope,
        causal: CausalRef {
            turn_id: [id.wrapping_add(1); 16],
            action_id: None,
            delivery_id: None,
            claim_id: None,
            base_revision,
        },
        observed_at_ms: 1_700_000_000_000 + u64::from(id),
        evidence: SemanticEstimate {
            schema_version: 1,
            dimensions,
            estimator_confidence: Fixed::ONE,
            estimator_digest: [id.wrapping_add(2); 32],
        },
    })
}

fn test_manifest() -> GenesisManifest {
    let mut manifest = GenesisManifest {
        schema_version: 1,
        traits: PersonalityVector::default(),
        expression: ExpressionPhenotype::default(),
        allostasis: AllostaticSetpoints::default(),
        epistemic: EpistemicPriors::default(),
        social: SocialPriors::default(),
        manifest_digest: [0; 32],
    };
    manifest.traits.baseline_warmth = Fixed::from_raw(600_000);
    manifest.traits.composure = Fixed::from_raw(700_000);
    manifest.manifest_digest = wire::manifest_body_digest(&manifest);
    manifest
}

fn fixture_field_bytes(field: &NeuralField) -> Vec<u8> {
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

fn fixture_graph_bytes(graph: &SparseGraph) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(graph.row_offsets.len() as u32).to_le_bytes());
    for offset in &graph.row_offsets {
        out.extend_from_slice(&offset.to_le_bytes());
    }
    out.extend_from_slice(&(graph.edges.len() as u32).to_le_bytes());
    for edge in &graph.edges {
        out.extend_from_slice(&edge.target.to_le_bytes());
        out.extend_from_slice(&edge.weight.to_le_bytes());
        out.extend_from_slice(&edge.eligibility.to_le_bytes());
        out.extend_from_slice(&edge.stability.to_le_bytes());
        out.extend_from_slice(&edge.last_used_epoch.to_le_bytes());
        out.push(edge.operator_id);
        out.push(edge.delay_class);
        out.extend_from_slice(&edge.flags.to_le_bytes());
    }
    out
}

fn fixture_aesem2_receipt_bytes(
    receipt: &TransitionReceipt,
    nonzero_dimensions: u8,
    state_changed: bool,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(302);
    out.extend_from_slice(&2_u16.to_le_bytes());
    for digest in [
        receipt.formula_digest,
        receipt.scope_digest,
        receipt.event_digest,
        receipt.authority_digest,
    ] {
        out.extend_from_slice(&digest);
    }
    out.extend_from_slice(&receipt.base_revision.to_le_bytes());
    out.extend_from_slice(&receipt.next_revision.to_le_bytes());
    for digest in [
        receipt.state_before,
        receipt.state_after,
        receipt.graph_after,
    ] {
        out.extend_from_slice(&digest);
    }
    assert!(receipt.action_contract.is_none());
    out.push(0);
    out.extend_from_slice(&receipt.active_nodes.to_le_bytes());
    out.extend_from_slice(&receipt.active_edges.to_le_bytes());
    for value in [
        receipt.residuals.authority,
        receipt.residuals.continuity,
        receipt.residuals.energy,
        receipt.residuals.renormalization,
        receipt.residuals.capacity,
    ] {
        out.extend_from_slice(&value.encode());
    }
    out.push(1);
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&[
        1,
        15,
        15,
        15,
        nonzero_dimensions,
        15 - nonzero_dimensions,
        0,
        u8::from(state_changed),
    ]);
    assert_eq!(out.len(), 302);
    out
}

fn fixture_aesem2(
    field: &NeuralField,
    graph: &SparseGraph,
    receipt: &TransitionReceipt,
    nonzero_dimensions: u8,
) -> Vec<u8> {
    let field = fixture_field_bytes(field);
    let graph = fixture_graph_bytes(graph);
    let receipt = fixture_aesem2_receipt_bytes(
        receipt,
        nonzero_dimensions,
        receipt.state_before != receipt.state_after,
    );
    let mut out = Vec::new();
    out.extend_from_slice(b"AESEM2\0");
    out.extend_from_slice(&2_u16.to_le_bytes());
    for block in [&field, &graph, &receipt] {
        out.extend_from_slice(&(block.len() as u32).to_le_bytes());
        out.extend_from_slice(block);
    }
    out
}

fn fixture_telemetry_bytes(receipt: &NativeTelemetryReceiptV1) -> Vec<u8> {
    let mut out = Vec::with_capacity(588);
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&[1, 1]);
    for digest in [
        receipt.formula_digest,
        receipt.scope_digest,
        receipt.event_digest,
        receipt.source_digest,
    ] {
        out.extend_from_slice(&digest);
    }
    out.extend_from_slice(&receipt.base_revision.to_le_bytes());
    out.extend_from_slice(&receipt.next_revision.to_le_bytes());
    for digest in [
        receipt.state_before,
        receipt.state_after,
        receipt.graph_before,
        receipt.graph_after,
        receipt.local_digest,
        receipt.compensation_digest,
        receipt.effective_digest,
    ] {
        out.extend_from_slice(&digest);
    }
    for value in [
        receipt.energy.reserve_before,
        receipt.energy.reserve_after,
        receipt.energy.recovered,
        receipt.energy.spent,
        receipt.energy.headroom,
        receipt.energy.residual,
    ] {
        out.extend_from_slice(&value.encode());
    }
    out.extend_from_slice(&receipt.capacity.upper_saturated_nodes.to_le_bytes());
    out.extend_from_slice(&receipt.capacity.node_limit.to_le_bytes());
    out.extend_from_slice(&receipt.capacity.node_headroom.encode());
    out.extend_from_slice(&receipt.capacity.edge_used.to_le_bytes());
    out.extend_from_slice(&receipt.capacity.edge_limit.to_le_bytes());
    for value in [
        receipt.capacity.edge_headroom,
        receipt.capacity.headroom,
        receipt.capacity.residual,
        receipt.residuals.authority,
        receipt.residuals.continuity,
        receipt.residuals.energy,
        receipt.residuals.renormalization,
        receipt.residuals.capacity,
        receipt.residual_health,
        receipt.native_gate,
    ] {
        out.extend_from_slice(&value.encode());
    }
    out.extend_from_slice(&receipt.checkpoint_digest);
    out.extend_from_slice(&receipt.telemetry_digest);
    assert_eq!(out.len(), 588);
    out
}

fn fixture_aesem3(
    field: &NeuralField,
    graph: &SparseGraph,
    telemetry: &NativeTelemetryReceiptV1,
) -> Vec<u8> {
    let field = fixture_field_bytes(field);
    let graph = fixture_graph_bytes(graph);
    let telemetry = fixture_telemetry_bytes(telemetry);
    let reserved = vec![0_u8; REGION_LAYOUT.len() * 8];
    let mut out = Vec::new();
    out.extend_from_slice(b"AESEM3\0");
    out.extend_from_slice(&3_u16.to_le_bytes());
    for block in [&field, &graph, &telemetry, &reserved] {
        out.extend_from_slice(&(block.len() as u32).to_le_bytes());
        out.extend_from_slice(block);
    }
    out
}

fn source_component_update(
    current: Fixed,
    baseline: Fixed,
    drive: Fixed,
    neutral_rate: Fixed,
) -> (Fixed, Fixed) {
    let displacement = current.saturating_sub(baseline);
    let recovery = displacement.checked_mul(neutral_rate).unwrap();
    (
        current.saturating_add(drive).saturating_sub(recovery),
        recovery,
    )
}

fn source_replay(
    field: &NeuralField,
    baseline: &NeuralField,
    dimensions: &EvidenceVector,
    confidence: Fixed,
) -> (NeuralField, u32) {
    let load = assemble_full_vector_load(dimensions).unwrap();
    assert_eq!(load.evaluated_dimension_count, 15);
    assert_eq!(load.injected_dimension_count, 15);
    let mut next = field.clone();
    let mut active_nodes = 0_u32;
    for (region, &(start, count)) in REGION_LAYOUT.iter().enumerate() {
        let drive = load.evidence_means[region].checked_mul(confidence).unwrap();
        let neutral_rate = load.neutral_means[region]
            .checked_mul(LEGACY_NEUTRAL_RELAXATION_MAX_RATE)
            .unwrap();
        for node in start..start + count {
            let (potential, potential_recovery) = source_component_update(
                field.potential[node],
                baseline.potential[node],
                drive,
                neutral_rate,
            );
            let (excitation, excitation_recovery) = source_component_update(
                field.excitation[node],
                baseline.excitation[node],
                drive,
                neutral_rate,
            );
            if drive == Fixed::ZERO
                && potential_recovery == Fixed::ZERO
                && excitation_recovery == Fixed::ZERO
            {
                continue;
            }
            active_nodes += 1;
            next.potential[node] = potential;
            next.excitation[node] = excitation;
        }
    }
    (next, active_nodes)
}

fn ties_even_scaled(value: i64, common_max: i64) -> i64 {
    let numerator = i128::from(value) * i128::from(LEGACY_FIELD_FXP6_SCALE);
    let denominator = i128::from(common_max);
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    let doubled = remainder * 2;
    let rounded = if doubled > denominator || (doubled == denominator && quotient & 1 == 1) {
        quotient + 1
    } else {
        quotient
    };
    i64::try_from(rounded).unwrap()
}

fn normalized_field(source: &NeuralField, common_max: i64) -> NeuralField {
    let mut target = source.clone();
    for (before, after) in source.potential.iter().zip(target.potential.iter_mut()) {
        *after = Fixed::from_raw(ties_even_scaled(before.raw(), common_max));
    }
    for (before, after) in source.excitation.iter().zip(target.excitation.iter_mut()) {
        *after = Fixed::from_raw(ties_even_scaled(before.raw(), common_max));
    }
    target
}

fn field_metadata(
    source: &NeuralField,
    target: &NeuralField,
    common_max: i64,
) -> LegacySemanticFieldDomainUpgradeV1 {
    let potential = source
        .potential
        .iter()
        .filter(|value| value.raw() > i64::from(LEGACY_FIELD_FXP6_SCALE))
        .count() as u32;
    let excitation = source
        .excitation
        .iter()
        .filter(|value| value.raw() > i64::from(LEGACY_FIELD_FXP6_SCALE))
        .count() as u32;
    let before = source
        .potential
        .iter()
        .chain(source.excitation.iter())
        .map(|value| i128::from(value.raw()))
        .sum();
    let after = target
        .potential
        .iter()
        .chain(target.excitation.iter())
        .map(|value| i128::from(value.raw()))
        .sum();
    LegacySemanticFieldDomainUpgradeV1 {
        algorithm: JOINT_MAX_LINEAR_FXP6_V1,
        fxp6_scale: LEGACY_FIELD_FXP6_SCALE,
        source_common_max: common_max,
        out_of_range_count: potential + excitation,
        potential_out_of_range_count: potential,
        excitation_out_of_range_count: excitation,
        signal_mass_before: before,
        signal_mass_after: after,
    }
}

fn relation_hmac(relation: [u8; 16]) -> [u8; 32] {
    let mut inner_pad = [0x36_u8; 64];
    let mut outer_pad = [0x5c_u8; 64];
    for (index, key) in CONTEXT_RELATION_HMAC_KEY_V1.iter().enumerate() {
        inner_pad[index] ^= key;
        outer_pad[index] ^= key;
    }
    let mut inner = Vec::with_capacity(80);
    inner.extend_from_slice(&inner_pad);
    inner.extend_from_slice(&relation);
    let inner_digest: [u8; 32] = Sha256::digest(&inner).into();
    let mut outer = Vec::with_capacity(96);
    outer.extend_from_slice(&outer_pad);
    outer.extend_from_slice(&inner_digest);
    Sha256::digest(&outer).into()
}

fn context_dimensions(event: &CanonicalEvent) -> [i64; 15] {
    let CanonicalEvent::UserStimulus(stimulus) = event else {
        panic!("fixture only supports UserStimulus");
    };
    let dimensions = &stimulus.evidence.dimensions;
    [
        dimensions.positive,
        dimensions.affiliation,
        dimensions.harm,
        dimensions.boundary,
        dimensions.repair,
        dimensions.repetition,
        dimensions.new_information,
        dimensions.constraint_instability,
        dimensions.epistemic_conflict,
        dimensions.self_responsibility,
        dimensions.other_responsibility,
        dimensions.hostility,
        dimensions.publicness,
        dimensions.engagement,
        dimensions.rejection,
    ]
    .map(|value| value.raw().clamp(0, 1_000_000))
}

impl ContextFixtureV1 {
    fn project(&mut self, event: &CanonicalEvent, relation: [u8; 16], revision: u64) {
        let CanonicalEvent::UserStimulus(stimulus) = event else {
            panic!("fixture only supports UserStimulus");
        };
        let dimensions = context_dimensions(event);
        let boundary = stimulus.evidence.dimensions.boundary.raw() > 0;
        let repair = stimulus.evidence.dimensions.repair.raw() > 0;
        let mut receipt = Vec::with_capacity(177);
        receipt.extend_from_slice(b"AECRPTV1");
        receipt.extend_from_slice(&stimulus.event_id);
        receipt.extend_from_slice(&relation);
        receipt.extend_from_slice(&revision.to_le_bytes());
        for dimension in dimensions {
            receipt.extend_from_slice(&dimension.to_le_bytes());
        }
        receipt.push(u8::from(boundary));
        receipt.push(u8::from(repair));
        receipt.extend_from_slice(&1_u64.to_le_bytes());
        receipt.push(0);
        self.turns.push(ContextTurnFixtureV1 {
            event_id: stimulus.event_id,
            receipt_digest: *blake3::hash(&receipt).as_bytes(),
            revision,
            dimensions,
            boundary,
            repair,
        });
        if self.turns.len() > 32 {
            self.turns.remove(0);
        }
        self.summary_revision += 1;
    }

    fn canonical_bytes(&self, relation: [u8; 16]) -> Vec<u8> {
        let mut out = Vec::with_capacity(52 + self.turns.len() * 187);
        out.extend_from_slice(CONTEXT_MAGIC_V1);
        out.extend_from_slice(&1_u32.to_le_bytes());
        out.extend_from_slice(&relation_hmac(relation));
        out.extend_from_slice(&self.summary_revision.to_le_bytes());
        out.extend_from_slice(&(self.turns.len() as u32).to_le_bytes());
        for turn in &self.turns {
            out.extend_from_slice(&turn.event_id);
            out.extend_from_slice(&turn.receipt_digest);
            out.extend_from_slice(&turn.revision.to_le_bytes());
            for dimension in turn.dimensions {
                out.extend_from_slice(&dimension.to_le_bytes());
            }
            out.push(u8::from(turn.boundary));
            out.push(u8::from(turn.repair));
            out.extend_from_slice(&1_u64.to_le_bytes());
            out.push(0);
        }
        out
    }
}

fn commit_identity(
    store: &mut Store,
    scope: &ScopeRef,
    formula_digest: [u8; 32],
    manifest: &GenesisManifest,
    baseline_field: &NeuralField,
    baseline_graph: &SparseGraph,
    development_seed_digest: [u8; 32],
    incarnation_id: [u8; 32],
) {
    let source = PersonaSourceRef {
        scope: PersonaScopeRef {
            bot_token: scope.bot_token,
            persona_token: scope.persona_token,
        },
        source_digest: [0x22; 32],
        capability_digest: [0x23; 32],
        selection: PersonaSelectionKind::Conversation,
        prompt_chars: 1,
        begin_dialog_count: 0,
        mood_dialog_count: 0,
    };
    let scope_key = ae_genesis::genesis_scope_key(
        &scope.bot_token,
        &scope.persona_token,
        &source.source_digest,
        &formula_digest,
    );
    let nonce = [0x25; 32];
    let ClaimOutcome::Claimed { lease_epoch, .. } =
        store.claim_lease(&scope_key, Some(nonce)).unwrap()
    else {
        panic!("fresh identity lease must be claimed");
    };
    let seed_code_digest = ae_genesis::derive_seed_code_digest(&manifest.manifest_digest);
    let initial_snapshot_digest = state_digest(baseline_field, &formula_digest);
    let initial_graph_digest = graph_digest(baseline_graph);
    let receipt = GenesisReceipt {
        schema_version: 1,
        seed_code_digest,
        manifest_digest: manifest.manifest_digest,
        incarnation_id,
        formula_digest,
        persona_source_digest: source.source_digest,
        compiler_protocol_digest: [0x28; 32],
        compiler_model_digest: [0x29; 32],
        development_seed_digest,
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
            manifest_body: wire::encode_manifest_body(manifest),
            seed_code_digest,
            incarnation_id,
            formula_digest,
            source,
            compiler_protocol_digest: [0x28; 32],
            compiler_model_digest: [0x29; 32],
            compiled_at_ms: 1,
            receipt,
            initial_snapshot_digest,
            state_bytes: fixture_field_bytes(baseline_field),
            graph_digest: initial_graph_digest,
            manifest: manifest.clone(),
        })
        .unwrap();
}

fn insert_graph_sidecar(
    database: &Path,
    scope_digest: [u8; 32],
    revision: u64,
    graph: &SparseGraph,
    formula: [u8; 32],
) {
    let connection = Connection::open(database).unwrap();
    let digest = graph_digest(graph);
    connection
        .execute(
            "INSERT INTO graph_commits (scope_digest, revision, base_graph_digest, graph_digest, formula_digest, delta_bytes, replay_state_bytes) VALUES (?1, ?2, ?3, ?4, ?5, X'', ?6)",
            params![
                &scope_digest[..],
                i64::try_from(revision).unwrap(),
                &digest[..],
                &digest[..],
                &formula[..],
                fixture_graph_bytes(graph),
            ],
        )
        .unwrap();
}

fn insert_context_sidecar(
    database: &Path,
    scope_digest: [u8; 32],
    relation: [u8; 16],
    revision: u64,
    state: &ContextFixtureV1,
) {
    let bytes = state.canonical_bytes(relation);
    let digest = wire::domain_hash(CONTEXT_DIGEST_DOMAIN_V1, &[&bytes]);
    let hmac = relation_hmac(relation);
    let connection = Connection::open(database).unwrap();
    connection
        .execute(
            "INSERT INTO context_commits (scope_digest, relation_scope_token, relation_hmac, revision, context_digest, canonical_state_bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                &scope_digest[..],
                &relation[..],
                &hmac[..],
                i64::try_from(revision).unwrap(),
                &digest[..],
                bytes,
            ],
        )
        .unwrap();
}

/// Materialize the exact journal/applied-event rows produced by the
/// pre-Task-6 journal-only `UserStimulus` lane.  Task 6 intentionally rejects
/// that lane in production; this local fixture preserves the historical
/// database shape that Task 5 must continue to migrate.
fn insert_pre_task6_user_stimulus(database: &Path, envelope: &CommitEnvelope) -> [u8; 32] {
    let event = wire::decode_event(&envelope.event_bytes).expect("canonical legacy event");
    assert_eq!(
        wire::encode_event(&event),
        envelope.event_bytes,
        "legacy event must use canonical wire bytes"
    );
    let CanonicalEvent::UserStimulus(stimulus) = &event else {
        panic!("legacy migration fixture only inserts UserStimulus history");
    };
    assert_eq!(wire::event_kind_name(&event), envelope.event_kind);
    assert!(envelope.delta_bytes.is_empty());
    assert_eq!(
        stimulus.causal.base_revision,
        envelope.receipt.base_revision
    );
    assert_eq!(
        envelope.receipt.next_revision,
        envelope.receipt.base_revision.checked_add(1).unwrap()
    );
    let scope_digest = wire::persona_scope_digest(
        &stimulus.scope.bot_token,
        &stimulus.scope.persona_token,
        stimulus.scope.relation_token.as_ref(),
    );
    assert_eq!(scope_digest, envelope.receipt.scope_digest);

    let event_digest = wire::event_digest(&event);
    assert_eq!(event_digest, envelope.receipt.event_digest);
    let receipt_bytes = wire::encode_transition_receipt(&envelope.receipt);
    let chain_digest = chain_link(&envelope.chain_seed, &envelope.event_bytes, &receipt_bytes);
    let committed_at_ms = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after unix epoch")
            .as_millis(),
    )
    .expect("fixture timestamp fits SQLite INTEGER");

    let mut connection = Connection::open(database).unwrap();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let (row_count, current_revision): (i64, i64) = transaction
        .query_row(
            "SELECT COUNT(*), COALESCE(MAX(logical_revision), 0)
             FROM journal WHERE scope_digest=?1",
            params![&scope_digest[..]],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        row_count,
        i64::try_from(envelope.receipt.base_revision).unwrap(),
        "legacy fixture history must be contiguous"
    );
    assert_eq!(current_revision, row_count);
    if current_revision > 0 {
        let previous_chain: Vec<u8> = transaction
            .query_row(
                "SELECT chain_digest FROM journal
                 WHERE scope_digest=?1 AND logical_revision=?2",
                params![&scope_digest[..], current_revision],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(previous_chain.as_slice(), &envelope.chain_seed);
    }
    let duplicate_count: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM applied_events
             WHERE scope_digest=?1 AND event_digest=?2",
            params![&scope_digest[..], &event_digest[..]],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(duplicate_count, 0);

    let revision = i64::try_from(envelope.receipt.next_revision).unwrap();
    let base_revision = i64::try_from(envelope.receipt.base_revision).unwrap();
    transaction
        .execute(
            "INSERT INTO journal
             (logical_revision,scope_digest,base_revision,event_kind,event_bytes,event_digest,
              receipt_bytes,delta_bytes,chain_digest,committed_at_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?7,X'',?8,?9)",
            params![
                revision,
                &scope_digest[..],
                base_revision,
                &envelope.event_kind,
                &envelope.event_bytes,
                &event_digest[..],
                receipt_bytes,
                &chain_digest[..],
                committed_at_ms,
            ],
        )
        .unwrap();
    transaction
        .execute(
            "INSERT INTO applied_events(scope_digest,event_digest,revision) VALUES (?1,?2,?3)",
            params![&scope_digest[..], &event_digest[..], revision],
        )
        .unwrap();
    transaction.commit().unwrap();
    chain_digest
}

fn fixture(label: &str) -> Fixture {
    let database = unique_database(label);
    let mut store = Store::open(&database).unwrap();
    let source_formula = [0x31; 32];
    let target_formula = phase0_canonical_formula_digest_v1(&source_formula);
    let incarnation_id = [0x26; 32];
    let bot_token = [0x11; 16];
    let persona_token = [0x12; 16];
    let root_scope = wire::persona_scope_digest(&bot_token, &persona_token, None);
    let binding = wire::domain_hash(
        SEMANTIC_LANE_NAMESPACE_DOMAIN_V1,
        &[&root_scope, &incarnation_id, &source_formula],
    );
    let mut relation = [0_u8; 16];
    relation.copy_from_slice(&binding[..16]);
    let mut session = [0_u8; 16];
    session.copy_from_slice(&binding[16..]);
    let scope = ScopeRef {
        bot_token,
        persona_token,
        relation_token: Some(relation),
        session_token: session,
    };
    let scope_digest = wire::persona_scope_digest(
        &scope.bot_token,
        &scope.persona_token,
        scope.relation_token.as_ref(),
    );
    let manifest = test_manifest();
    let development_seed = ae_genesis::derive_development_seed(
        &ae_genesis::derive_seed_code_digest(&manifest.manifest_digest),
        &incarnation_id,
        &source_formula,
    );
    let (baseline_field, baseline_graph) =
        initial_state_from_manifest(&manifest, &source_formula, &development_seed);
    commit_identity(
        &mut store,
        &scope,
        source_formula,
        &manifest,
        &baseline_field,
        &baseline_graph,
        development_seed,
        incarnation_id,
    );

    let initial_snapshot_digest = state_digest(&baseline_field, &source_formula);
    let graph_after = graph_digest(&baseline_graph);
    let mut replay_field = baseline_field.clone();
    let mut chain_seed = initial_snapshot_digest;
    let mut context = ContextFixtureV1::default();
    let mut source_bytes = Vec::new();
    let mut source_fields = Vec::new();
    let mut source_receipts = Vec::new();
    for revision in 1..=2_u64 {
        let source_event = event(
            scope.clone(),
            0x40 + u8::try_from(revision).unwrap(),
            revision - 1,
            evidence_all_one(),
        );
        let source_event_bytes = wire::encode_event(&source_event);
        let source_event_digest = wire::event_digest(&source_event);
        let (next_field, active_nodes) = source_replay(
            &replay_field,
            &baseline_field,
            &evidence_all_one(),
            Fixed::ONE,
        );
        let source_state = state_digest(&next_field, &source_formula);
        let source_receipt = TransitionReceipt {
            schema_version: 1,
            formula_digest: source_formula,
            scope_digest,
            event_digest: source_event_digest,
            authority_digest: authority_projection_digest(&source_event),
            base_revision: revision - 1,
            next_revision: revision,
            state_before: state_digest(&replay_field, &source_formula),
            state_after: source_state,
            graph_after,
            action_contract: None,
            active_nodes,
            active_edges: 0,
            residuals: InvariantResiduals::default(),
            status: CommitStatus::Committed,
        };
        let snapshot = fixture_aesem2(&next_field, &baseline_graph, &source_receipt, 15);
        let legacy_envelope = CommitEnvelope {
            event_kind: wire::event_kind_name(&source_event).to_owned(),
            event_bytes: source_event_bytes,
            receipt: source_receipt.clone(),
            chain_seed,
            delta_bytes: Vec::new(),
        };
        let next_chain = insert_pre_task6_user_stimulus(&database, &legacy_envelope);
        store
            .write_snapshot(&scope_digest, revision, &source_state, &snapshot)
            .unwrap();
        insert_graph_sidecar(
            &database,
            scope_digest,
            revision,
            &baseline_graph,
            source_formula,
        );
        context.project(&source_event, relation, revision);
        insert_context_sidecar(&database, scope_digest, relation, revision, &context);
        replay_field = next_field.clone();
        chain_seed = next_chain;
        source_bytes.push(snapshot);
        source_fields.push(next_field);
        source_receipts.push(source_receipt);
    }

    let common_max = replay_field
        .potential
        .iter()
        .chain(replay_field.excitation.iter())
        .map(|value| value.raw())
        .max()
        .unwrap();
    assert!(common_max > i64::from(LEGACY_FIELD_FXP6_SCALE));
    let target_field = normalized_field(&replay_field, common_max);
    let target_state = state_digest(&target_field, &target_formula);
    let target_event = event(scope.clone(), 0x51, 2, EvidenceVector::default());
    let target_event_bytes = wire::encode_event(&target_event);
    let target_event_digest = wire::event_digest(&target_event);
    let target_receipt = TransitionReceipt {
        schema_version: 1,
        formula_digest: target_formula,
        scope_digest,
        event_digest: target_event_digest,
        authority_digest: authority_projection_digest(&target_event),
        base_revision: 2,
        next_revision: 3,
        state_before: target_state,
        state_after: target_state,
        graph_after,
        action_contract: None,
        active_nodes: target_field.active_node_count(),
        active_edges: 0,
        residuals: InvariantResiduals::default(),
        status: CommitStatus::Committed,
    };
    let telemetry = NativeTelemetryReceiptV1 {
        schema: NATIVE_TELEMETRY_RECEIPT_SCHEMA_V1.to_owned(),
        formula: NativeTelemetryFormulaV1::Phase0NativePropagationFxp6V1,
        formula_digest: target_formula,
        scope_digest,
        event_digest: target_event_digest,
        source_digest: [0x53; 32],
        base_revision: 2,
        next_revision: 3,
        phase: NativeTelemetryPhaseV1::Prepare,
        state_before: target_state,
        state_after: target_state,
        graph_before: graph_after,
        graph_after,
        local_digest: [0x54; 32],
        compensation_digest: legacy_reserved_zero_digest_v1(),
        effective_digest: [0x55; 32],
        energy: EnergyTelemetryV1 {
            reserve_before: Fixed::ONE,
            reserve_after: Fixed::ONE,
            recovered: Fixed::ZERO,
            spent: Fixed::ZERO,
            headroom: Fixed::ONE,
            residual: Fixed::ZERO,
        },
        capacity: CapacityTelemetryV1 {
            upper_saturated_nodes: 0,
            node_limit: NEURON_SLOTS as u32,
            node_headroom: Fixed::ONE,
            edge_used: 0,
            edge_limit: EDGE_CAPACITY as u32,
            edge_headroom: Fixed::ONE,
            headroom: Fixed::ONE,
            residual: Fixed::ZERO,
        },
        residuals: InvariantResiduals::default(),
        residual_health: Fixed::ONE,
        native_gate: Fixed::ONE,
        checkpoint_digest: [0; 32],
        telemetry_digest: [0; 32],
    }
    .seal();
    assert!(telemetry.validate());
    let target_bytes = fixture_aesem3(&target_field, &baseline_graph, &telemetry);
    let upgrade = LegacySemanticFormulaUpgradeReceiptV1::from_transition_receipt_with_field_domain(
        &target_receipt,
        state_digest(&replay_field, &source_formula),
        graph_after,
        source_formula,
        chain_seed,
        field_metadata(&replay_field, &target_field, common_max),
    );
    let envelope = CommitEnvelope {
        event_kind: wire::event_kind_name(&target_event).to_owned(),
        event_bytes: target_event_bytes,
        receipt: target_receipt,
        chain_seed,
        delta_bytes: upgrade.canonical_bytes(),
    };
    Fixture {
        store,
        database,
        source_bytes,
        source_fields,
        source_receipts,
        graph: baseline_graph,
        target_bytes,
        scope_digest,
        envelope,
        upgrade,
        incarnation_id,
        manifest_digest: manifest.manifest_digest,
        scope,
    }
}

fn migration_counts(database: &Path) -> (i64, i64) {
    let connection = Connection::open(database).unwrap();
    let upgrades = connection
        .query_row(
            "SELECT COUNT(*) FROM legacy_semantic_formula_upgrades",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let backups = connection
        .query_row(
            "SELECT COUNT(*) FROM field_migration_preimage_backups",
            [],
            |row| row.get(0),
        )
        .unwrap();
    (upgrades, backups)
}

fn authority_state(fixture: &Fixture) -> AuthorityState {
    let connection = Connection::open(&fixture.database).unwrap();
    let journal = connection
        .prepare("SELECT logical_revision, base_revision, event_kind, event_bytes, event_digest, receipt_bytes, delta_bytes, chain_digest FROM journal WHERE scope_digest = ?1 ORDER BY logical_revision")
        .unwrap()
        .query_map(params![&fixture.scope_digest[..]], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let snapshots = connection
        .prepare("SELECT revision, state_digest, state_bytes FROM snapshots WHERE scope_digest = ?1 ORDER BY revision")
        .unwrap()
        .query_map(params![&fixture.scope_digest[..]], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let graphs = connection
        .prepare("SELECT revision, base_graph_digest, graph_digest, formula_digest, delta_bytes, replay_state_bytes FROM graph_commits WHERE scope_digest = ?1 ORDER BY revision")
        .unwrap()
        .query_map(params![&fixture.scope_digest[..]], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let contexts = connection
        .prepare("SELECT revision, relation_scope_token, relation_hmac, context_digest, canonical_state_bytes FROM context_commits WHERE scope_digest = ?1 ORDER BY revision")
        .unwrap()
        .query_map(params![&fixture.scope_digest[..]], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let applied_events = connection
        .query_row(
            "SELECT COUNT(*) FROM applied_events WHERE scope_digest = ?1",
            params![&fixture.scope_digest[..]],
            |row| row.get(0),
        )
        .unwrap();
    AuthorityState {
        journal,
        snapshots,
        graphs,
        contexts,
        applied_events,
        migrations: migration_counts(&fixture.database),
    }
}

fn backup_root(database: &Path) -> PathBuf {
    database
        .parent()
        .unwrap()
        .join(".astr-embodiment-field-migration-preimages")
}

fn atomic_backup_paths(fixture: &Fixture) -> (PathBuf, PathBuf, PathBuf) {
    let root = backup_root(&fixture.database);
    let migration = ae_contracts::hex::encode32(&fixture.upgrade.migration_id);
    let final_dir = root.join(&migration);
    (
        final_dir.join("authority.sqlite"),
        final_dir.join("manifest"),
        final_dir,
    )
}

fn manifest_with_historical_creator(
    manifest: &[u8],
    package_identity: &str,
    build_identity: &str,
) -> Vec<u8> {
    const MAGIC: &[u8] = b"AE-FMP2\0";
    assert!(manifest.starts_with(MAGIC));
    let package_len_index = MAGIC.len();
    let old_package_len = usize::from(manifest[package_len_index]);
    let build_len_index = package_len_index + 1 + old_package_len;
    let old_build_len = usize::from(manifest[build_len_index]);
    let payload_index = build_len_index + 1 + old_build_len;
    let package_len = u8::try_from(package_identity.len()).expect("bounded package identity");
    let build_len = u8::try_from(build_identity.len()).expect("bounded build identity");

    let mut rewritten = Vec::with_capacity(
        manifest.len() + package_identity.len() + build_identity.len()
            - old_package_len
            - old_build_len,
    );
    rewritten.extend_from_slice(MAGIC);
    rewritten.push(package_len);
    rewritten.extend_from_slice(package_identity.as_bytes());
    rewritten.push(build_len);
    rewritten.extend_from_slice(build_identity.as_bytes());
    rewritten.extend_from_slice(&manifest[payload_index..]);
    rewritten
}

fn replace_backup_creator_and_restore_task5_schema(
    fixture: &Fixture,
    package_identity: &str,
    build_identity: &str,
) {
    let (_, manifest_path, _) = atomic_backup_paths(fixture);
    let historical_manifest = manifest_with_historical_creator(
        &std::fs::read(&manifest_path).expect("backup manifest"),
        package_identity,
        build_identity,
    );
    std::fs::write(&manifest_path, &historical_manifest).expect("historical manifest");

    let connection = Connection::open(&fixture.database).expect("historical database");
    connection
        .execute_batch(
            "ALTER TABLE field_migration_preimage_backups RENAME TO field_migration_preimage_backups_new;
             CREATE TABLE field_migration_preimage_backups (
                migration_id BLOB PRIMARY KEY,
                scope_digest BLOB NOT NULL,
                source_revision INTEGER NOT NULL,
                source_state_digest BLOB NOT NULL,
                source_formula_digest BLOB NOT NULL,
                source_graph_digest BLOB NOT NULL,
                incarnation_id BLOB NOT NULL,
                manifest_digest BLOB NOT NULL,
                byte_len INTEGER NOT NULL,
                sha256 BLOB NOT NULL,
                manifest_bytes BLOB NOT NULL
             );
             INSERT INTO field_migration_preimage_backups
             SELECT migration_id, scope_digest, source_revision, source_state_digest,
                    source_formula_digest, source_graph_digest, incarnation_id,
                    manifest_digest, byte_len, sha256, manifest_bytes
             FROM field_migration_preimage_backups_new;
             DROP TABLE field_migration_preimage_backups_new;",
        )
        .expect("restore Task 5 backup schema");
    connection
        .execute(
            "UPDATE field_migration_preimage_backups SET manifest_bytes=?1",
            params![historical_manifest],
        )
        .expect("store historical manifest");
}

fn close_store_and_restore_database(fixture: &mut Fixture, snapshot: &Path) {
    let placeholder = Store::open_in_memory().unwrap();
    let old = std::mem::replace(&mut fixture.store, placeholder);
    old.close().unwrap();
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{}", fixture.database.display(), suffix));
        if sidecar.exists() {
            std::fs::remove_file(sidecar).unwrap();
        }
    }
    std::fs::copy(snapshot, &fixture.database).unwrap();
    let reopened = Store::open(&fixture.database).unwrap();
    let placeholder = std::mem::replace(&mut fixture.store, reopened);
    placeholder.close().unwrap();
}

fn reopen_store(fixture: &mut Fixture) {
    let placeholder = Store::open_in_memory().unwrap();
    let old = std::mem::replace(&mut fixture.store, placeholder);
    old.close().unwrap();
    let reopened = Store::open(&fixture.database).unwrap();
    let placeholder = std::mem::replace(&mut fixture.store, reopened);
    placeholder.close().unwrap();
}

fn append_valid_post_migration_revision(fixture: &mut Fixture) -> u64 {
    let base_revision = fixture.upgrade.next_revision;
    let next_revision = base_revision + 1;
    let target_formula = fixture.upgrade.to_formula_digest;
    let source_field = fixture.source_fields.last().expect("source field");
    let common_max = fixture
        .upgrade
        .field_domain
        .as_ref()
        .expect("finite field migration")
        .source_common_max;
    let target_field = normalized_field(source_field, common_max);
    let target_state = state_digest(&target_field, &target_formula);
    let graph = fixture.graph.clone();
    let graph_after = graph_digest(&graph);
    assert_eq!(graph_after, fixture.upgrade.source_graph_digest);

    let future_event = event(
        fixture.scope.clone(),
        0x72,
        base_revision,
        EvidenceVector::default(),
    );
    let event_bytes = wire::encode_event(&future_event);
    let event_digest = wire::event_digest(&future_event);
    let receipt = TransitionReceipt {
        schema_version: 1,
        formula_digest: target_formula,
        scope_digest: fixture.scope_digest,
        event_digest,
        authority_digest: authority_projection_digest(&future_event),
        base_revision,
        next_revision,
        state_before: target_state,
        state_after: target_state,
        graph_after,
        action_contract: None,
        active_nodes: target_field.active_node_count(),
        active_edges: u32::try_from(graph.edges.len()).unwrap(),
        residuals: InvariantResiduals::default(),
        status: CommitStatus::Committed,
    };
    let telemetry = NativeTelemetryReceiptV1 {
        schema: NATIVE_TELEMETRY_RECEIPT_SCHEMA_V1.to_owned(),
        formula: NativeTelemetryFormulaV1::Phase0NativePropagationFxp6V1,
        formula_digest: target_formula,
        scope_digest: fixture.scope_digest,
        event_digest,
        source_digest: [0x73; 32],
        base_revision,
        next_revision,
        phase: NativeTelemetryPhaseV1::Prepare,
        state_before: target_state,
        state_after: target_state,
        graph_before: graph_after,
        graph_after,
        local_digest: [0x74; 32],
        compensation_digest: legacy_reserved_zero_digest_v1(),
        effective_digest: [0x75; 32],
        energy: EnergyTelemetryV1 {
            reserve_before: Fixed::ONE,
            reserve_after: Fixed::ONE,
            recovered: Fixed::ZERO,
            spent: Fixed::ZERO,
            headroom: Fixed::ONE,
            residual: Fixed::ZERO,
        },
        capacity: CapacityTelemetryV1 {
            upper_saturated_nodes: 0,
            node_limit: NEURON_SLOTS as u32,
            node_headroom: Fixed::ONE,
            edge_used: u32::try_from(graph.edges.len()).unwrap(),
            edge_limit: EDGE_CAPACITY as u32,
            edge_headroom: Fixed::ONE,
            headroom: Fixed::ONE,
            residual: Fixed::ZERO,
        },
        residuals: InvariantResiduals::default(),
        residual_health: Fixed::ONE,
        native_gate: Fixed::ONE,
        checkpoint_digest: [0; 32],
        telemetry_digest: [0; 32],
    }
    .seal();
    assert!(telemetry.validate());
    let snapshot = fixture_aesem3(&target_field, &graph, &telemetry);
    let chain_seed = fixture
        .store
        .last_chain_digest(&fixture.scope_digest)
        .unwrap()
        .expect("migration chain");
    let legacy_suffix = CommitEnvelope {
        event_kind: wire::event_kind_name(&future_event).to_owned(),
        event_bytes,
        receipt,
        chain_seed,
        delta_bytes: Vec::new(),
    };
    insert_pre_task6_user_stimulus(&fixture.database, &legacy_suffix);
    fixture
        .store
        .write_snapshot(
            &fixture.scope_digest,
            next_revision,
            &target_state,
            &snapshot,
        )
        .expect("post-migration snapshot");
    insert_graph_sidecar(
        &fixture.database,
        fixture.scope_digest,
        next_revision,
        &graph,
        target_formula,
    );

    let connection = Connection::open(&fixture.database).expect("context source");
    let mut statement = connection
        .prepare(
            "SELECT logical_revision,event_bytes FROM journal WHERE scope_digest=?1 ORDER BY logical_revision",
        )
        .expect("context journal query");
    let mut rows = statement
        .query(params![&fixture.scope_digest[..]])
        .expect("context rows");
    let relation = fixture.scope.relation_token.expect("relation scope");
    let mut context = ContextFixtureV1::default();
    while let Some(row) = rows.next().expect("context row") {
        let revision: u64 = row.get::<_, i64>(0).unwrap().try_into().unwrap();
        let bytes: Vec<u8> = row.get(1).unwrap();
        let event = wire::decode_event(&bytes).expect("canonical context event");
        context.project(&event, relation, revision);
    }
    drop(rows);
    drop(statement);
    drop(connection);
    insert_context_sidecar(
        &fixture.database,
        fixture.scope_digest,
        relation,
        next_revision,
        &context,
    );
    next_revision
}

fn close_store(fixture: &mut Fixture) {
    let placeholder = Store::open_in_memory().unwrap();
    let old = std::mem::replace(&mut fixture.store, placeholder);
    old.close().unwrap();
}

fn prime_backup_and_restore_source(
    fixture: &mut Fixture,
) -> (AuthorityState, SemanticMigrationOutcomeV1) {
    let before = authority_state(fixture);
    let snapshot = fixture.database.with_extension("precrash.sqlite");
    Connection::open(&fixture.database)
        .unwrap()
        .backup(rusqlite::DatabaseName::Main, &snapshot, None)
        .unwrap();
    let outcome = fixture.migrate().unwrap();
    close_store_and_restore_database(fixture, &snapshot);
    assert_eq!(authority_state(fixture), before);
    (before, outcome)
}

fn assert_zero_authority_writes(
    fixture: &Fixture,
    before: &AuthorityState,
    backup_must_be_absent: bool,
) {
    assert_eq!(&authority_state(fixture), before);
    assert_eq!(before.migrations, (0, 0));
    if backup_must_be_absent {
        assert!(!backup_root(&fixture.database).exists());
    }
}

fn mutate_revision_one_receipt(fixture: &Fixture, mutate: impl FnOnce(&mut TransitionReceipt)) {
    let connection = Connection::open(&fixture.database).unwrap();
    let bytes: Vec<u8> = connection
        .query_row(
            "SELECT receipt_bytes FROM journal WHERE scope_digest = ?1 AND logical_revision = 1",
            params![&fixture.scope_digest[..]],
            |row| row.get(0),
        )
        .unwrap();
    let mut receipt = wire::decode_transition_receipt(&bytes).unwrap();
    mutate(&mut receipt);
    connection
        .execute(
            "UPDATE journal SET receipt_bytes = ?1 WHERE scope_digest = ?2 AND logical_revision = 1",
            params![wire::encode_transition_receipt(&receipt), &fixture.scope_digest[..]],
        )
        .unwrap();
}

fn tamper_authority(fixture: &mut Fixture) {
    mutate_revision_one_receipt(fixture, |receipt| receipt.authority_digest[0] ^= 1);
}

fn tamper_chain_link(fixture: &mut Fixture) {
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE journal SET chain_digest = ?1 WHERE scope_digest = ?2 AND logical_revision = 1",
            params![&[0x91_u8; 32][..], &fixture.scope_digest[..]],
        )
        .unwrap();
}

fn tamper_state_side(fixture: &mut Fixture) {
    let mut bytes = fixture.source_bytes[0].clone();
    bytes.push(0xff);
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE snapshots SET state_bytes = ?1 WHERE scope_digest = ?2 AND revision = 1",
            params![bytes, &fixture.scope_digest[..]],
        )
        .unwrap();
}

fn tamper_graph_sidecar(fixture: &mut Fixture) {
    let connection = Connection::open(&fixture.database).unwrap();
    let mut bytes: Vec<u8> = connection
        .query_row(
            "SELECT replay_state_bytes FROM graph_commits WHERE scope_digest = ?1 AND revision = 1",
            params![&fixture.scope_digest[..]],
            |row| row.get(0),
        )
        .unwrap();
    bytes.push(0);
    connection
        .execute(
            "UPDATE graph_commits SET replay_state_bytes = ?1 WHERE scope_digest = ?2 AND revision = 1",
            params![bytes, &fixture.scope_digest[..]],
        )
        .unwrap();
}

fn tamper_context_sidecar(fixture: &mut Fixture) {
    let connection = Connection::open(&fixture.database).unwrap();
    let mut bytes: Vec<u8> = connection
        .query_row(
            "SELECT canonical_state_bytes FROM context_commits WHERE scope_digest = ?1 AND revision = 1",
            params![&fixture.scope_digest[..]],
            |row| row.get(0),
        )
        .unwrap();
    bytes.push(0);
    connection
        .execute(
            "UPDATE context_commits SET canonical_state_bytes = ?1 WHERE scope_digest = ?2 AND revision = 1",
            params![bytes, &fixture.scope_digest[..]],
        )
        .unwrap();
}

fn tamper_retired_state_changed(fixture: &mut Fixture) {
    let mut bytes = fixture.source_bytes[0].clone();
    assert_eq!(bytes.last(), Some(&1));
    *bytes.last_mut().unwrap() = 0;
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE snapshots SET state_bytes = ?1 WHERE scope_digest = ?2 AND revision = 1",
            params![bytes, &fixture.scope_digest[..]],
        )
        .unwrap();
}

fn tamper_arbitrary_overflow(fixture: &mut Fixture) {
    let mut field = fixture.source_fields[0].clone();
    field.potential[0] = Fixed::from_raw(4_000_000);
    let bytes = fixture_aesem2(
        &field,
        &SparseGraph::empty(),
        &fixture.source_receipts[0],
        15,
    );
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE snapshots SET state_bytes = ?1 WHERE scope_digest = ?2 AND revision = 1",
            params![bytes, &fixture.scope_digest[..]],
        )
        .unwrap();
}

fn tamper_non_pe_overflow(fixture: &mut Fixture) {
    let mut field = fixture.source_fields[0].clone();
    field.inhibition[0] = Fixed::from_raw(1_000_001);
    let bytes = fixture_aesem2(
        &field,
        &SparseGraph::empty(),
        &fixture.source_receipts[0],
        15,
    );
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE snapshots SET state_bytes = ?1 WHERE scope_digest = ?2 AND revision = 1",
            params![bytes, &fixture.scope_digest[..]],
        )
        .unwrap();
}

#[test]
fn finite_aesem2_migration_is_copy_verify_commit_and_idempotent() {
    let mut fixture = fixture("finite");
    let mut provenance = Vec::new();
    for source in &fixture.source_bytes {
        provenance.extend_from_slice(&(source.len() as u64).to_le_bytes());
        provenance.extend_from_slice(source);
    }
    let source_hash: [u8; 32] = Sha256::digest(&provenance).into();
    assert_eq!(
        source_hash,
        [
            65, 178, 130, 52, 224, 225, 160, 230, 247, 36, 213, 38, 34, 150, 207, 84, 217, 69, 147,
            84, 244, 64, 231, 163, 51, 89, 228, 192, 107, 213, 108, 8,
        ]
    );

    let formula_only = LegacySemanticFormulaUpgradeReceiptV1::from_transition_receipt(
        &fixture.envelope.receipt,
        fixture.upgrade.source_state_digest,
        fixture.upgrade.source_graph_digest,
        fixture.upgrade.from_formula_digest,
        fixture.upgrade.prior_chain_digest,
    )
    .canonical_bytes();
    assert!(formula_only.starts_with(b"AE-LSU1\0"));
    assert_eq!(&formula_only[8..10], &1_u16.to_le_bytes());
    assert_eq!(formula_only[10], 1);

    let first = fixture.migrate().unwrap();
    let SemanticMigrationOutcomeV1::Migrated {
        from_revision,
        to_revision,
        backup_digest,
    } = first
    else {
        panic!("finite predecessor must migrate");
    };
    assert_eq!((from_revision, to_revision), (2, 3));
    assert_ne!(backup_digest, [0; 32]);
    let after = authority_state(&fixture);
    assert_eq!(after.journal.len(), 3);
    assert_eq!(after.snapshots.len(), 3);
    assert_eq!(after.graphs.len(), 3);
    assert_eq!(after.contexts.len(), 3);
    assert_eq!(after.migrations, (1, 1));
    for (index, source) in fixture.source_bytes.iter().enumerate() {
        assert_eq!(after.snapshots[index].2, *source);
        assert!(source.starts_with(b"AESEM2\0"));
    }
    assert_ne!(after.snapshots[2].2, fixture.target_bytes);
    assert!(after.snapshots[2].2.starts_with(b"AESEM3\0"));

    let (backup_database, backup_manifest, _) = atomic_backup_paths(&fixture);
    assert!(backup_database.is_file());
    assert!(backup_manifest.is_file());
    let backup_hash: [u8; 32] = Sha256::digest(std::fs::read(&backup_database).unwrap()).into();
    assert_eq!(backup_hash, backup_digest);
    assert!(std::fs::read(&backup_manifest)
        .unwrap()
        .starts_with(b"AE-FMP2\0"));
    // SQLite opens a WAL-mode database by creating `-wal`/`-shm` siblings,
    // even with READ_ONLY flags. Inspect a disposable copy so this acceptance
    // test does not mutate the exact two-member authority package it verifies.
    let inspection_database = fixture
        .database
        .with_extension("preimage-inspection.sqlite");
    std::fs::copy(&backup_database, &inspection_database).unwrap();
    let preimage = Connection::open(&inspection_database).unwrap();
    let preimage_journal: i64 = preimage
        .query_row("SELECT COUNT(*) FROM journal", [], |row| row.get(0))
        .unwrap();
    let preimage_graphs: i64 = preimage
        .query_row("SELECT COUNT(*) FROM graph_commits", [], |row| row.get(0))
        .unwrap();
    let preimage_contexts: i64 = preimage
        .query_row("SELECT COUNT(*) FROM context_commits", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        (preimage_journal, preimage_graphs, preimage_contexts),
        (2, 2, 2)
    );
    drop(preimage);
    for suffix in ["", "-wal", "-shm"] {
        let path = PathBuf::from(format!("{}{}", inspection_database.display(), suffix));
        if path.exists() {
            std::fs::remove_file(path).unwrap();
        }
    }

    let second = fixture.migrate().unwrap();
    assert_eq!(second, first);
    assert_eq!(authority_state(&fixture), after);
}

#[test]
fn backup_recovers_every_failpoint_without_source_mutation() {
    let mut primary = fixture("atomic-backup-layout-red");
    let before = authority_state(&primary);
    let outcome = primary.migrate().expect("migration");
    let SemanticMigrationOutcomeV1::Migrated { backup_digest, .. } = outcome else {
        panic!("finite predecessor must migrate");
    };

    let (database, manifest, final_dir) = atomic_backup_paths(&primary);
    assert!(final_dir.is_dir(), "backup publishes one final directory");
    assert!(database.is_file());
    assert!(manifest.is_file());
    let entries = std::fs::read_dir(backup_root(&primary.database))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(entries, vec![final_dir.file_name().unwrap()]);
    let hash: [u8; 32] = Sha256::digest(std::fs::read(database).unwrap()).into();
    assert_eq!(hash, backup_digest);
    assert_ne!(authority_state(&primary), before);

    let mut unowned = fixture("unowned-stage-is-preserved");
    let root = backup_root(&unowned.database);
    let foreign = root.join(format!(
        ".stage-{}-not-an-owned-nonce",
        ae_contracts::hex::encode32(&unowned.upgrade.migration_id)
    ));
    std::fs::create_dir_all(&foreign).unwrap();
    std::fs::write(foreign.join("owner-data"), b"foreign").unwrap();
    unowned.migrate().unwrap();
    assert_eq!(
        std::fs::read(foreign.join("owner-data")).unwrap(),
        b"foreign"
    );

    for (label, retain_manifest) in [
        ("after-database-backup", false),
        ("after-manifest-sync", true),
        ("after-staging-directory-sync", true),
    ] {
        let mut fixture = fixture(label);
        let (before, first) = prime_backup_and_restore_source(&mut fixture);
        let (_, manifest, final_dir) = atomic_backup_paths(&fixture);
        let stage = backup_root(&fixture.database).join(format!(
            ".stage-{}-{}",
            ae_contracts::hex::encode32(&fixture.upgrade.migration_id),
            "a".repeat(64)
        ));
        std::fs::rename(&final_dir, &stage).unwrap();
        if !retain_manifest {
            std::fs::remove_file(stage.join(manifest.file_name().unwrap())).unwrap();
        }
        let retried = fixture.migrate().unwrap();
        assert_eq!(retried, first, "{label}");
        assert_ne!(authority_state(&fixture), before, "{label}");
        assert!(final_dir.is_dir(), "{label}");
        assert!(!stage.exists(), "{label}");
    }

    let mut precommit = fixture("after-directory-rename-pre-sql-commit");
    let (before, first) = prime_backup_and_restore_source(&mut precommit);
    let (_, _, final_dir) = atomic_backup_paths(&precommit);
    assert!(final_dir.is_dir());
    assert_eq!(precommit.migrate().unwrap(), first);
    assert_ne!(authority_state(&precommit), before);

    let mut postcommit = fixture("after-sql-commit");
    let first = postcommit.migrate().unwrap();
    let database = postcommit.database.clone();
    let placeholder = Store::open_in_memory().unwrap();
    let old = std::mem::replace(&mut postcommit.store, placeholder);
    old.close().unwrap();
    let reopened = Store::open(&database).unwrap();
    let placeholder = std::mem::replace(&mut postcommit.store, reopened);
    placeholder.close().unwrap();
    assert_eq!(postcommit.migrate().unwrap(), first);
    assert_eq!(migration_counts(&database), (1, 1));

    #[cfg(feature = "migration-test-hooks")]
    {
        let failpoints = [
            (
                "before-snapshot",
                SemanticMigrationTestFailpointV1::BeforeSnapshot,
            ),
            (
                "after-snapshot",
                SemanticMigrationTestFailpointV1::AfterSnapshot,
            ),
            (
                "before-database-sync",
                SemanticMigrationTestFailpointV1::BeforeDatabaseSync,
            ),
            (
                "after-database-sync",
                SemanticMigrationTestFailpointV1::AfterDatabaseSync,
            ),
            (
                "before-manifest-sync",
                SemanticMigrationTestFailpointV1::BeforeManifestSync,
            ),
            (
                "after-manifest-sync",
                SemanticMigrationTestFailpointV1::AfterManifestSync,
            ),
            (
                "before-directory-sync",
                SemanticMigrationTestFailpointV1::BeforeDirectorySync,
            ),
            (
                "after-directory-sync",
                SemanticMigrationTestFailpointV1::AfterDirectorySync,
            ),
            (
                "before-atomic-publish",
                SemanticMigrationTestFailpointV1::BeforeAtomicPublish,
            ),
            (
                "after-atomic-publish",
                SemanticMigrationTestFailpointV1::AfterAtomicPublish,
            ),
            (
                "before-parent-sync",
                SemanticMigrationTestFailpointV1::BeforeParentSync,
            ),
            (
                "after-parent-sync",
                SemanticMigrationTestFailpointV1::AfterParentSync,
            ),
            (
                "before-transaction-begin",
                SemanticMigrationTestFailpointV1::BeforeTransactionBegin,
            ),
            (
                "after-transaction-begin",
                SemanticMigrationTestFailpointV1::AfterTransactionBegin,
            ),
            (
                "before-row-insertion",
                SemanticMigrationTestFailpointV1::BeforeRowInsertion,
            ),
            (
                "after-row-insertion",
                SemanticMigrationTestFailpointV1::AfterRowInsertion,
            ),
            (
                "before-commit",
                SemanticMigrationTestFailpointV1::BeforeCommit,
            ),
            (
                "after-commit",
                SemanticMigrationTestFailpointV1::AfterCommit,
            ),
        ];

        for (label, failpoint) in failpoints {
            println!("semantic-migration-failpoint={label}");
            let mut fixture = fixture(label);
            let before = authority_state(&fixture);
            set_semantic_migration_test_failpoint_v1(failpoint);
            assert!(fixture.migrate().is_err(), "{label} must inject failure");
            let interrupted = authority_state(&fixture);

            if failpoint == SemanticMigrationTestFailpointV1::AfterCommit {
                assert_ne!(interrupted, before, "after commit must persist atomically");
                assert_eq!(interrupted.migrations, (1, 1));
                let persisted = fixture.migrate().expect("re-attest committed outcome");
                assert_eq!(fixture.migrate().expect("exact committed retry"), persisted);
                assert_eq!(authority_state(&fixture), interrupted);
            } else {
                assert_eq!(interrupted, before, "{label} leaked a partial row set");
                assert_eq!(interrupted.migrations, (0, 0));
                let persisted = fixture.migrate().expect("clean retry after interruption");
                let committed = authority_state(&fixture);
                assert_eq!(committed.migrations, (1, 1));
                assert_eq!(fixture.migrate().expect("exact retry"), persisted);
                assert_eq!(authority_state(&fixture), committed);
            }

            let (backup_database, backup_manifest, final_dir) = atomic_backup_paths(&fixture);
            assert!(
                final_dir.is_dir(),
                "{label} did not recover final directory"
            );
            assert!(backup_database.is_file(), "{label} lost backup database");
            assert!(backup_manifest.is_file(), "{label} lost backup manifest");
            let entries = std::fs::read_dir(backup_root(&fixture.database))
                .expect("backup root")
                .map(|entry| entry.expect("backup entry").path())
                .collect::<Vec<_>>();
            assert_eq!(entries, vec![final_dir], "{label} left stage debris");
        }
    }

    let committed = authority_state(&postcommit);
    let (_, manifest, _) = atomic_backup_paths(&postcommit);
    std::fs::remove_file(manifest).unwrap();
    assert!(postcommit.migrate().is_err());
    assert_eq!(authority_state(&postcommit), committed);
}

#[cfg(feature = "migration-test-hooks")]
#[test]
fn mutation_after_backup_before_immediate_cas_fails_closed_with_zero_migration_writes() {
    let mut mutated = fixture("mutation-after-backup-before-immediate");
    let scope_digest = mutated.scope_digest;
    set_semantic_migration_test_hook_v1(
        SemanticMigrationTestHookV1::AfterBackupBeforeImmediateCas(Box::new(move |database| {
            let connection = Connection::open(database).expect("open competing writer");
            assert_eq!(
                connection
                    .execute(
                        "UPDATE journal SET event_kind='UserStimulus-mutated' WHERE scope_digest=?1 AND logical_revision=2",
                        params![&scope_digest[..]],
                    )
                    .expect("commit competing mutation"),
                1
            );
        })),
    );

    assert!(mutated.migrate().is_err());
    let after = authority_state(&mutated);
    assert_eq!(after.journal.len(), 2);
    assert_eq!(after.snapshots.len(), 2);
    assert_eq!(after.graphs.len(), 2);
    assert_eq!(after.contexts.len(), 2);
    assert_eq!(after.applied_events, 2);
    assert_eq!(after.migrations, (0, 0));
    assert_eq!(after.journal[1].2, "UserStimulus-mutated");
    let (database, manifest, final_dir) = atomic_backup_paths(&mutated);
    assert!(final_dir.is_dir());
    assert!(database.is_file());
    assert!(manifest.is_file());

    let mut stale = fixture("revision-after-backup-before-immediate");
    let scope_digest = stale.scope_digest;
    set_semantic_migration_test_hook_v1(
        SemanticMigrationTestHookV1::AfterBackupBeforeImmediateCas(Box::new(move |database| {
            let connection = Connection::open(database).expect("open competing revision writer");
            connection
                .execute(
                    "INSERT INTO journal (logical_revision, scope_digest, base_revision, event_kind, event_bytes, event_digest, receipt_bytes, delta_bytes, chain_digest, committed_at_ms) VALUES (3, ?1, 2, 'concurrent', X'', zeroblob(32), X'', X'', zeroblob(32), 0)",
                    params![&scope_digest[..]],
                )
                .expect("commit competing revision");
        })),
    );

    assert!(matches!(
        stale.migrate(),
        Err(ae_store::StoreError::StaleRevision {
            expected: 2,
            actual: 3
        })
    ));
    let stale_after = authority_state(&stale);
    assert_eq!(stale_after.journal.len(), 3);
    assert_eq!(stale_after.snapshots.len(), 2);
    assert_eq!(stale_after.graphs.len(), 2);
    assert_eq!(stale_after.contexts.len(), 2);
    assert_eq!(stale_after.applied_events, 2);
    assert_eq!(stale_after.migrations, (0, 0));
}

#[cfg(feature = "migration-test-hooks")]
#[test]
fn retry_recaptures_whole_database_after_unrelated_scope_commit() {
    let mut fixture = fixture("unrelated-scope-after-backup");
    let unrelated_scope = ScopeRef {
        bot_token: [0x91; 16],
        persona_token: [0x92; 16],
        relation_token: Some([0x93; 16]),
        session_token: [0x94; 16],
    };
    let unrelated_event = event(unrelated_scope.clone(), 0x95, 0, EvidenceVector::default());
    let unrelated_scope_digest = wire::persona_scope_digest(
        &unrelated_scope.bot_token,
        &unrelated_scope.persona_token,
        unrelated_scope.relation_token.as_ref(),
    );
    let unrelated_event_digest = wire::event_digest(&unrelated_event);
    let unrelated_envelope = CommitEnvelope {
        event_kind: wire::event_kind_name(&unrelated_event).to_owned(),
        event_bytes: wire::encode_event(&unrelated_event),
        receipt: TransitionReceipt {
            schema_version: 1,
            formula_digest: [0x96; 32],
            scope_digest: unrelated_scope_digest,
            event_digest: unrelated_event_digest,
            authority_digest: authority_projection_digest(&unrelated_event),
            base_revision: 0,
            next_revision: 1,
            state_before: [0x97; 32],
            state_after: [0x98; 32],
            graph_after: [0x99; 32],
            action_contract: None,
            active_nodes: 0,
            active_edges: 0,
            residuals: InvariantResiduals::default(),
            status: CommitStatus::Committed,
        },
        chain_seed: [0x9a; 32],
        delta_bytes: Vec::new(),
    };
    set_semantic_migration_test_hook_v1(
        SemanticMigrationTestHookV1::AfterBackupBeforeImmediateCas(Box::new(move |database| {
            let mut competing = Store::open(database).expect("open unrelated-scope writer");
            competing
                .commit_journal(&unrelated_envelope)
                .expect("commit unrelated scope through Store authority");
            competing.close().expect("close unrelated-scope writer");
        })),
    );

    let first_error = fixture
        .migrate()
        .expect_err("cross-scope mutation must fail CAS");
    assert!(
        matches!(
            first_error,
            ae_store::StoreError::ContinuityFence("field_backup_source_changed")
        ),
        "unexpected first-attempt error: {first_error:?}"
    );
    assert_eq!(migration_counts(&fixture.database), (0, 0));
    assert_eq!(
        Connection::open(&fixture.database)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM journal WHERE scope_digest=?1",
                params![&unrelated_scope_digest[..]],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );

    // Fail while the stale final is atomically quarantined and the fresh
    // whole-database stage is about to publish. The following retry must still
    // recover without reverting or losing the unrelated legal commit.
    set_semantic_migration_test_failpoint_v1(SemanticMigrationTestFailpointV1::BeforeAtomicPublish);
    assert!(
        fixture.migrate().is_err(),
        "replacement publication failure must interrupt the retry"
    );
    assert_eq!(migration_counts(&fixture.database), (0, 0));

    fixture
        .migrate()
        .expect("retry from recaptured whole database");
    let (backup_database, _, _) = atomic_backup_paths(&fixture);
    let restored = Connection::open(backup_database).expect("open recorded preimage");
    assert_eq!(
        restored
            .query_row(
                "SELECT COUNT(*) FROM journal WHERE scope_digest=?1",
                params![&unrelated_scope_digest[..]],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1,
        "recorded whole-database preimage must retain the unrelated legal commit"
    );
}

#[test]
fn forged_upgrade_row_cannot_exempt_a_journal_delta_from_open_attestation() {
    let database = unique_database("forged-upgrade-journal-exemption");
    Store::open(&database).unwrap().close().unwrap();
    let connection = Connection::open(&database).unwrap();
    let scope = [0xa1_u8; 32];
    let event_digest = [0xa2_u8; 32];
    connection
        .execute(
            "INSERT INTO journal (logical_revision, scope_digest, base_revision, event_kind, event_bytes, event_digest, receipt_bytes, delta_bytes, chain_digest, committed_at_ms) VALUES (1, ?1, 0, 'forged', X'', ?2, X'', X'010203', zeroblob(32), 0)",
            params![&scope[..], &event_digest[..]],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO legacy_semantic_formula_upgrades (migration_id, scope_digest, from_formula_digest, to_formula_digest, base_revision, next_revision, event_digest, receipt_digest, source_state_digest, target_state_before, source_graph_digest, prior_chain_digest, upgrade_bytes, backup_digest) VALUES (?1, ?2, ?3, ?4, 0, 1, ?5, ?6, ?7, ?8, ?9, ?10, X'010203', ?11)",
            params![
                &[0xa0_u8; 32][..],
                &scope[..],
                &[0xa3_u8; 32][..],
                &[0xa4_u8; 32][..],
                &event_digest[..],
                &[0xa5_u8; 32][..],
                &[0xa6_u8; 32][..],
                &[0xa7_u8; 32][..],
                &[0xa8_u8; 32][..],
                &[0xa9_u8; 32][..],
                &[0xaa_u8; 32][..],
            ],
        )
        .unwrap();
    drop(connection);

    assert!(
        Store::open(&database).is_err(),
        "an unauthenticated upgrade row must not bypass journal-delta attestation"
    );
}

#[cfg(feature = "migration-test-hooks")]
#[test]
fn sql_failpoints_roll_back_all_migration_rows() {
    for (label, failpoint) in [
        (
            "after-backup-row-insert",
            SemanticMigrationTestFailpointV1::AfterBackupRowInsert,
        ),
        (
            "after-journal-insert",
            SemanticMigrationTestFailpointV1::AfterJournalInsert,
        ),
        (
            "after-applied-event-insert",
            SemanticMigrationTestFailpointV1::AfterAppliedEventInsert,
        ),
        (
            "after-snapshot-insert",
            SemanticMigrationTestFailpointV1::AfterSnapshotInsert,
        ),
        (
            "after-graph-insert",
            SemanticMigrationTestFailpointV1::AfterGraphInsert,
        ),
        (
            "after-context-insert",
            SemanticMigrationTestFailpointV1::AfterContextInsert,
        ),
        (
            "after-upgrade-insert",
            SemanticMigrationTestFailpointV1::AfterUpgradeInsert,
        ),
        (
            "after-authority-insert",
            SemanticMigrationTestFailpointV1::AfterAuthorityInsert,
        ),
        (
            "before-commit",
            SemanticMigrationTestFailpointV1::BeforeCommit,
        ),
    ] {
        let mut fixture = fixture(label);
        let before = authority_state(&fixture);
        set_semantic_migration_test_failpoint_v1(failpoint);

        assert!(fixture.migrate().is_err(), "{label} must inject failure");
        assert_eq!(authority_state(&fixture), before, "{label} leaked SQL rows");
        let (database, manifest, final_dir) = atomic_backup_paths(&fixture);
        assert!(final_dir.is_dir(), "{label} lost durable preimage");
        assert!(database.is_file(), "{label} lost backup database");
        assert!(manifest.is_file(), "{label} lost backup manifest");

        let first = fixture.migrate().expect("exact retry after rollback");
        let committed = authority_state(&fixture);
        assert_eq!(committed.migrations, (1, 1));
        assert_eq!(
            fixture.migrate().expect("idempotent committed retry"),
            first
        );
        assert_eq!(authority_state(&fixture), committed);
    }
}

#[test]
fn concurrent_cas_and_exact_retry_are_zero_write_or_identical() {
    let mut fixture = fixture("concurrent-cas");
    let before = authority_state(&fixture);
    let database = fixture.database.clone();
    let scope = fixture.scope.clone();
    let source_revision = fixture.upgrade.base_revision;
    let source_state_digest = fixture.upgrade.source_state_digest;
    let source_graph_digest = fixture.upgrade.source_graph_digest;
    let source_history_root = fixture.upgrade.prior_chain_digest;
    let incarnation_id = fixture.incarnation_id;
    let manifest_digest = fixture.manifest_digest;

    let placeholder = Store::open_in_memory().expect("placeholder store");
    let source_store = std::mem::replace(&mut fixture.store, placeholder);
    source_store.close().expect("close fixture writer");

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let database = database.clone();
        let scope = scope.clone();
        let barrier = std::sync::Arc::clone(&barrier);
        workers.push(std::thread::spawn(move || {
            let mut store = Store::open(&database).map_err(|error| error.to_string())?;
            barrier.wait();
            let outcome = store
                .migrate_legacy_semantic_snapshot_v2(SemanticMigrationRequestV2 {
                    scope: &scope,
                    expected_source_revision: source_revision,
                    expected_source_state_digest: source_state_digest,
                    expected_source_graph_digest: source_graph_digest,
                    expected_source_history_root: source_history_root,
                    expected_incarnation_id: incarnation_id,
                    expected_manifest_digest: manifest_digest,
                })
                .map_err(|error| error.to_string());
            store.close().map_err(|error| error.to_string())?;
            outcome
        }));
    }
    let concurrent = workers
        .into_iter()
        .map(|worker| worker.join().expect("concurrent migration worker"))
        .collect::<Vec<_>>();

    let reopened = Store::open(&database).expect("reopen authoritative store");
    let placeholder = std::mem::replace(&mut fixture.store, reopened);
    placeholder.close().expect("close placeholder");
    let persisted = fixture.migrate().expect("complete or re-attest winner");
    assert_eq!(fixture.migrate().expect("exact retry"), persisted);
    for outcome in concurrent.into_iter().flatten() {
        assert_eq!(outcome, persisted, "successful contender diverged");
    }

    let after = authority_state(&fixture);
    assert_eq!(after.migrations, (1, 1));
    assert_eq!(after.journal.len(), 3);
    assert_eq!(after.snapshots.len(), 3);
    assert_eq!(after.graphs.len(), 3);
    assert_eq!(after.contexts.len(), 3);
    assert_eq!(after.applied_events, 3);
    assert_eq!(&after.journal[..2], before.journal.as_slice());
    assert_eq!(&after.snapshots[..2], before.snapshots.as_slice());
    assert_eq!(&after.graphs[..2], before.graphs.as_slice());
    assert_eq!(&after.contexts[..2], before.contexts.as_slice());
}

#[test]
fn exact_retry_and_reopen_reattest_complete_committed_closure() {
    let clean_cases: [(&str, &str); 9] = [
        (
            "predecessor-journal",
            "UPDATE journal SET receipt_bytes=X'' WHERE logical_revision=1",
        ),
        (
            "final-journal",
            "UPDATE journal SET receipt_bytes=X'' WHERE logical_revision=3",
        ),
        (
            "final-applied-event",
            "DELETE FROM applied_events WHERE revision=3",
        ),
        (
            "final-snapshot",
            "UPDATE snapshots SET state_digest=zeroblob(32) WHERE revision=3",
        ),
        (
            "final-graph",
            "UPDATE graph_commits SET formula_digest=zeroblob(32) WHERE revision=3",
        ),
        (
            "final-context",
            "UPDATE context_commits SET context_digest=zeroblob(32) WHERE revision=3",
        ),
        (
            "upgrade-row",
            "UPDATE legacy_semantic_formula_upgrades SET source_state_digest=zeroblob(32)",
        ),
        (
            "migration-authority",
            "UPDATE semantic_migration_authority_v1 SET commitment_digest=zeroblob(32)",
        ),
        (
            "backup-row",
            "UPDATE field_migration_preimage_backups SET sha256=zeroblob(32)",
        ),
    ];

    let mut exact = fixture("reopen-exact-retry");
    let first = exact.migrate().expect("first migration");
    let committed = authority_state(&exact);
    reopen_store(&mut exact);
    assert_eq!(exact.migrate().expect("reopened exact retry"), first);
    assert_eq!(authority_state(&exact), committed);

    for (label, sql) in clean_cases {
        let mut fixture = fixture(label);
        fixture.migrate().expect("first migration");
        reopen_store(&mut fixture);
        Connection::open(&fixture.database)
            .unwrap()
            .execute(sql, [])
            .unwrap();
        let corrupted = authority_state(&fixture);
        assert!(
            {
                close_store(&mut fixture);
                Store::open(&fixture.database).is_err()
            },
            "{label} must fail Store::open full re-attestation"
        );
        assert_eq!(authority_state(&fixture), corrupted, "{label} wrote state");
    }
}

#[test]
fn committed_migration_fails_closed_for_suffix_without_dynamics_authority() {
    let mut fixture = fixture("future-semantic-suffix");
    fixture.migrate().expect("finite migration");
    let future_revision = append_valid_post_migration_revision(&mut fixture);
    let committed = authority_state(&fixture);

    close_store(&mut fixture);
    assert!(matches!(
        Store::open(&fixture.database),
        Err(StoreError::SemanticSuffixAuthorityUnavailable {
            boundary_revision,
            current_revision,
        }) if boundary_revision == fixture.upgrade.next_revision
            && current_revision == future_revision
    ));
    assert_eq!(authority_state(&fixture), committed);
}

#[test]
#[cfg(feature = "legacy-semantic-test-api")]
fn committed_migration_does_not_mint_unobserved_perception_authority() {
    let mut fixture = fixture("store-owned-semantic-origin");
    fixture.migrate().expect("finite migration");
    let persona_scope =
        wire::persona_scope_digest(&fixture.scope.bot_token, &fixture.scope.persona_token, None);
    let unobserved = event(
        fixture.scope.clone(),
        0x76,
        0,
        EvidenceVector {
            positive: Fixed::from_raw(700_000),
            engagement: Fixed::from_raw(500_000),
            ..EvidenceVector::default()
        },
    );
    let unobserved_digest = wire::event_digest(&unobserved);
    assert!(fixture
        .store
        .mint_perception_challenge_from_committed_inbound_v1(unobserved_digest)
        .is_err());
    assert_eq!(
        fixture.store.latest_semantic_v1(&persona_scope).unwrap(),
        None
    );
    let committed = authority_state(&fixture);

    close_store(&mut fixture);
    let reopened = Store::open(&fixture.database).expect("reopen must replay migration origin");
    assert_eq!(reopened.latest_semantic_v1(&persona_scope).unwrap(), None);
    reopened.close().unwrap();
    assert_eq!(authority_state(&fixture), committed);
}

#[test]
fn backup_creator_provenance_survives_binary_version_change_and_tampering_fails() {
    let mut fixture = fixture("historical-backup-creator");
    fixture.migrate().expect("finite migration");
    close_store(&mut fixture);

    let historical_package = "ae-store@1.1.0-alpha2";
    let historical_build =
        "ae-store@1.1.0-alpha2;target=historical-os-historical-arch;manifest=AE-FMP2";
    replace_backup_creator_and_restore_task5_schema(&fixture, historical_package, historical_build);

    let reopened = Store::open(&fixture.database).expect("historical creator must reopen");
    reopened.close().expect("close historical creator store");
    let connection = Connection::open(&fixture.database).expect("creator provenance row");
    let stored: (String, String) = connection
        .query_row(
            "SELECT creator_package_identity, creator_build_identity
             FROM field_migration_preimage_backups",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("persisted creator provenance");
    assert_eq!(
        stored,
        (historical_package.to_owned(), historical_build.to_owned())
    );
    connection
        .execute(
            "UPDATE field_migration_preimage_backups
             SET creator_build_identity=creator_build_identity || '-tampered'",
            [],
        )
        .expect("tamper creator provenance");
    drop(connection);
    assert!(Store::open(&fixture.database).is_err());
}

#[cfg(feature = "migration-test-hooks")]
#[test]
fn rowless_published_backup_accepts_its_historical_creator_but_not_tampering() {
    let historical_package = "ae-store@1.1.0-alpha2";
    let historical_build =
        "ae-store@1.1.0-alpha2;target=historical-os-historical-arch;manifest=AE-FMP2";

    let mut recoverable = fixture("rowless-historical-creator");
    let before = authority_state(&recoverable);
    set_semantic_migration_test_failpoint_v1(SemanticMigrationTestFailpointV1::AfterAtomicPublish);
    assert!(recoverable.migrate().is_err());
    assert_eq!(authority_state(&recoverable), before);
    assert_eq!(migration_counts(&recoverable.database), (0, 0));
    let (_, manifest_path, final_dir) = atomic_backup_paths(&recoverable);
    assert!(final_dir.is_dir());
    let historical_manifest = manifest_with_historical_creator(
        &std::fs::read(&manifest_path).expect("published orphan manifest"),
        historical_package,
        historical_build,
    );
    std::fs::write(&manifest_path, &historical_manifest)
        .expect("simulate old-binary published orphan");

    recoverable
        .migrate()
        .expect("new binary must authenticate and reuse an exact historical orphan");
    let connection = Connection::open(&recoverable.database).expect("creator provenance row");
    let stored: (String, String, Vec<u8>) = connection
        .query_row(
            "SELECT creator_package_identity, creator_build_identity, manifest_bytes
             FROM field_migration_preimage_backups",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("historical orphan provenance");
    assert_eq!(stored.0, historical_package);
    assert_eq!(stored.1, historical_build);
    assert_eq!(stored.2, historical_manifest);
    drop(connection);

    let mut tampered = fixture("rowless-historical-tamper");
    let before = authority_state(&tampered);
    set_semantic_migration_test_failpoint_v1(SemanticMigrationTestFailpointV1::AfterAtomicPublish);
    assert!(tampered.migrate().is_err());
    let (_, tampered_manifest_path, _) = atomic_backup_paths(&tampered);
    let mut tampered_manifest = manifest_with_historical_creator(
        &std::fs::read(&tampered_manifest_path).expect("published orphan manifest"),
        historical_package,
        historical_build,
    );
    let source_fingerprint_index = tampered_manifest.len() - (8 + 32 + 1) - 32;
    tampered_manifest[source_fingerprint_index] ^= 0x01;
    std::fs::write(&tampered_manifest_path, tampered_manifest)
        .expect("tamper orphan source authority fingerprint");

    assert!(tampered.migrate().is_err());
    assert_eq!(authority_state(&tampered), before);
    assert_eq!(migration_counts(&tampered.database), (0, 0));
}

#[test]
fn backup_package_requires_exact_real_regular_file_members() {
    let mut fixture = fixture("backup-exact-members");
    fixture.migrate().expect("finite migration");
    close_store(&mut fixture);
    let (_, manifest, final_dir) = atomic_backup_paths(&fixture);

    let extra = final_dir.join("unattested-extra");
    std::fs::write(&extra, b"extra").expect("extra member");
    assert!(Store::open(&fixture.database).is_err());
    std::fs::remove_file(extra).expect("remove extra member");

    let outside_manifest = final_dir.parent().unwrap().join("outside-manifest");
    std::fs::copy(&manifest, &outside_manifest).expect("outside manifest");
    std::fs::remove_file(&manifest).expect("remove packaged manifest");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside_manifest, &manifest).expect("manifest symlink");
    #[cfg(windows)]
    {
        if let Err(error) = std::os::windows::fs::symlink_file(&outside_manifest, &manifest) {
            if error.kind() == std::io::ErrorKind::PermissionDenied
                || error.raw_os_error() == Some(1314)
            {
                return;
            }
            panic!("manifest symlink: {error}");
        }
    }
    assert!(Store::open(&fixture.database).is_err());
}

#[test]
fn committed_retry_fails_closed_for_missing_or_corrupt_backup_package() {
    for label in [
        "missing-final-directory",
        "missing-backup-database",
        "corrupt-backup-database",
        "missing-backup-manifest",
        "corrupt-backup-manifest",
    ] {
        let mut fixture = fixture(label);
        fixture.migrate().expect("first migration");
        reopen_store(&mut fixture);
        let (database, manifest, final_dir) = atomic_backup_paths(&fixture);
        match label {
            "missing-final-directory" => std::fs::remove_dir_all(&final_dir).unwrap(),
            "missing-backup-database" => std::fs::remove_file(&database).unwrap(),
            "corrupt-backup-database" => std::fs::write(&database, b"corrupt").unwrap(),
            "missing-backup-manifest" => std::fs::remove_file(&manifest).unwrap(),
            "corrupt-backup-manifest" => std::fs::write(&manifest, b"corrupt").unwrap(),
            _ => unreachable!(),
        }
        let committed = authority_state(&fixture);

        assert!(fixture.migrate().is_err(), "{label} must fail closed");
        assert_eq!(authority_state(&fixture), committed, "{label} wrote state");
    }
}

#[test]
fn source_path_replacement_is_rejected_before_backup() {
    let mut fixture = fixture("source-path-replacement");
    let replacement = fixture.database.with_extension("replacement.sqlite");
    Connection::open(&fixture.database)
        .unwrap()
        .backup(rusqlite::DatabaseName::Main, &replacement, None)
        .unwrap();
    let displaced = fixture.database.with_extension("displaced.sqlite");
    if let Err(error) = std::fs::rename(&fixture.database, &displaced) {
        #[cfg(windows)]
        {
            assert_eq!(error.raw_os_error(), Some(32));
            assert!(!backup_root(&fixture.database).exists());
            return;
        }
        #[cfg(not(windows))]
        panic!("source replacement setup failed: {error}");
    }
    std::fs::rename(&replacement, &fixture.database).unwrap();
    let before = authority_state(&fixture);

    assert!(fixture.migrate().is_err());
    assert_eq!(authority_state(&fixture), before);
    assert!(!backup_root(&fixture.database).exists());
}

#[test]
fn caller_aesem3_missing_mandatory_fourth_block_cannot_influence_store_output() {
    let mut fixture = fixture("missing-aesem3-fourth-block");
    fixture
        .target_bytes
        .truncate(fixture.target_bytes.len() - (4 + REGION_LAYOUT.len() * 8));
    let before = authority_state(&fixture);

    assert!(fixture.migrate().is_ok());
    assert_ne!(authority_state(&fixture), before);
}

#[test]
fn corrupt_history_and_budget_violations_cause_zero_writes() {
    let cases: [(&str, fn(&mut Fixture)); 7] = [
        ("authority", tamper_authority),
        ("chain-link", tamper_chain_link),
        ("state", tamper_state_side),
        ("graph-sidecar", tamper_graph_sidecar),
        ("context-sidecar", tamper_context_sidecar),
        ("arbitrary-overflow", tamper_arbitrary_overflow),
        ("non-pe-overflow", tamper_non_pe_overflow),
    ];
    for (label, tamper) in cases {
        println!("authenticated-history-tamper-case={label}");
        let mut fixture = fixture(label);
        tamper(&mut fixture);
        let before = authority_state(&fixture);
        assert!(fixture.migrate().is_err(), "{label} must fail closed");
        assert_zero_authority_writes(&fixture, &before, true);
    }

    let mut verify_failure = fixture("verify-failure");
    let (backup_database, backup_manifest, final_dir) = atomic_backup_paths(&verify_failure);
    std::fs::create_dir_all(final_dir).unwrap();
    std::fs::write(&backup_database, b"not-a-sqlite-preimage").unwrap();
    std::fs::write(&backup_manifest, b"not-a-manifest").unwrap();
    let before = authority_state(&verify_failure);
    assert!(verify_failure.migrate().is_err());
    assert_zero_authority_writes(&verify_failure, &before, false);

    for (label, violate_budget) in [
        (
            "manifest-wire-byte-budget",
            oversize_manifest_wire as fn(&Fixture),
        ),
        (
            "source-json-byte-budget",
            oversize_source_json as fn(&Fixture),
        ),
    ] {
        let mut fixture = fixture(label);
        violate_budget(&fixture);
        let before = authority_state(&fixture);
        assert!(fixture.migrate().is_err(), "{label} must fail closed");
        assert_zero_authority_writes(&fixture, &before, true);
    }
}

#[test]
fn corrupt_or_unrelated_legacy_state_causes_zero_writes() {
    for (label, tamper) in [
        ("legacy-authority", tamper_authority as fn(&mut Fixture)),
        (
            "legacy-overflow",
            tamper_arbitrary_overflow as fn(&mut Fixture),
        ),
    ] {
        let mut fixture = fixture(label);
        tamper(&mut fixture);
        let before = authority_state(&fixture);
        assert!(fixture.migrate().is_err(), "{label} must fail closed");
        assert_zero_authority_writes(&fixture, &before, true);
    }
}

#[test]
fn retired_receipt_state_changed_mismatch_is_rejected_before_writes() {
    let mut fixture = fixture("retired-state-changed");
    tamper_retired_state_changed(&mut fixture);
    let before = authority_state(&fixture);
    assert!(fixture.migrate().is_err());
    assert_zero_authority_writes(&fixture, &before, true);
}

#[test]
fn public_selector_cannot_supply_migration_authority() {
    let mut fixture = fixture("selector-no-authority");
    fixture.envelope.event_bytes =
        wire::encode_event(&event(fixture.scope.clone(), 0xee, 2, evidence_all_one()));
    // This is a fully self-sealed public NativeTelemetryReceiptV1 wrapped in
    // caller-built AESEM3 bytes. It remains valid as a DTO but is not authority.
    assert!(fixture.target_bytes.starts_with(b"AESEM3\0"));

    fixture
        .migrate()
        .expect("Store derives migration artifacts");
    let state = authority_state(&fixture);
    let stored_event = wire::decode_event(&state.journal[2].3).expect("canonical event");
    let CanonicalEvent::UserStimulus(stimulus) = stored_event else {
        panic!("wire-compatible migration event");
    };
    assert_eq!(stimulus.evidence.dimensions, EvidenceVector::default());
    assert_eq!(stimulus.evidence.estimator_confidence, Fixed::ZERO);
    assert_eq!(stimulus.evidence.estimator_digest, [0; 32]);
    assert_ne!(state.snapshots[2].2, fixture.target_bytes);
}

#[test]
fn authority_binds_formula_upgrade_and_rejects_public_raw_envelopes() {
    let mut fixture = fixture("authority-sidecar");
    let first = fixture.migrate().expect("migration");
    let conn = Connection::open(&fixture.database).unwrap();
    let row: (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) = conn
        .query_row(
            "SELECT commitment_digest, telemetry_digest, snapshot_wire_digest, authority_receipt_digest FROM semantic_migration_authority_v1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert!([&row.0, &row.1, &row.2, &row.3]
        .into_iter()
        .all(|digest| digest.len() == 32 && digest.iter().any(|byte| *byte != 0)));
    let before = authority_state(&fixture);
    assert_eq!(fixture.migrate().expect("exact retry"), first);
    assert_eq!(authority_state(&fixture), before);

    conn.execute(
        "UPDATE semantic_migration_authority_v1 SET commitment_digest=zeroblob(32)",
        [],
    )
    .unwrap();
    drop(conn);
    let corrupted = authority_state(&fixture);
    assert!(fixture.migrate().is_err());
    assert_eq!(authority_state(&fixture), corrupted);
}

#[test]
fn task5_sqlite_type_gates_reject_dynamic_types_without_raw_driver_errors() {
    for (label, sql, expected_fence) in [
        (
            "incarnation-status-blob",
            "UPDATE incarnations SET status=zeroblob(4096)",
            "semantic_identity",
        ),
        (
            "incarnation-parent-text",
            "UPDATE incarnations SET parent_incarnation_id=CAST('parent' AS TEXT)",
            "semantic_identity",
        ),
        (
            "manifest-source-blob",
            "UPDATE genesis_manifests SET source_json=zeroblob(4096)",
            "semantic_source",
        ),
        (
            "journal-event-text",
            "UPDATE journal SET event_bytes=CAST('event' AS TEXT) WHERE logical_revision=1",
            "journal_event_bytes_type",
        ),
    ] {
        let mut fixture = fixture(label);
        Connection::open(&fixture.database)
            .unwrap()
            .execute(sql, [])
            .unwrap();
        let error = fixture
            .migrate()
            .expect_err("dynamic SQLite type must fail closed");
        assert!(
            matches!(error, StoreError::ContinuityFence(fence) if fence == expected_fence),
            "{label} returned an untyped or late error: {error:?}"
        );
    }
}

#[test]
fn task5_oversized_authority_digest_is_rejected_by_length_before_blob_read() {
    let mut fixture = fixture("oversized-authority-digest");
    fixture.migrate().expect("commit migration");
    close_store(&mut fixture);
    let connection = Connection::open(&fixture.database).unwrap();
    connection
        .execute_batch(
            "PRAGMA ignore_check_constraints=ON;
             UPDATE semantic_migration_authority_v1 SET commitment_digest=zeroblob(4096);",
        )
        .unwrap();
    drop(connection);

    let error = match Store::open(&fixture.database) {
        Ok(store) => {
            store.close().unwrap();
            panic!("oversized digest must fail closed");
        }
        Err(error) => error,
    };
    assert!(
        matches!(
            error,
            StoreError::InvalidStoredDigest {
                field: "semantic_migration_authority.commitment_digest",
                actual: 4096,
            }
        ),
        "oversized authority digest was materialized or rejected late: {error:?}"
    );
}

#[test]
fn task5_creator_identity_type_is_rejected_before_cast_materialization() {
    let mut fixture = fixture("creator-identity-type");
    fixture.migrate().expect("commit migration");
    close_store(&mut fixture);
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE field_migration_preimage_backups
             SET creator_package_identity=zeroblob(4096)",
            [],
        )
        .unwrap();

    let error = match Store::open(&fixture.database) {
        Ok(store) => {
            store.close().unwrap();
            panic!("creator type must fail closed");
        }
        Err(error) => error,
    };
    assert!(
        matches!(
            error,
            StoreError::ContinuityFence("field_backup_creator_identity")
        ),
        "creator identity was cast/materialized before type rejection: {error:?}"
    );
}

fn mutate_sql(fixture: &Fixture, sql: &str) {
    Connection::open(&fixture.database)
        .unwrap()
        .execute(sql, params![&fixture.scope_digest[..]])
        .unwrap();
}

fn gap_journal(fixture: &Fixture) {
    mutate_sql(
        fixture,
        "DELETE FROM journal WHERE scope_digest=?1 AND logical_revision=1",
    );
}

fn extra_journal(fixture: &Fixture) {
    mutate_sql(
        fixture,
        "INSERT INTO journal(revision,scope_digest,logical_revision,base_revision,event_kind,event_bytes,event_digest,receipt_bytes,delta_bytes,chain_digest,committed_at_ms) SELECT 99,scope_digest,99,base_revision,event_kind,event_bytes,zeroblob(32),receipt_bytes,delta_bytes,chain_digest,committed_at_ms FROM journal WHERE scope_digest=?1 AND logical_revision=2",
    );
}

fn delete_applied_event(fixture: &Fixture) {
    mutate_sql(
        fixture,
        "DELETE FROM applied_events WHERE scope_digest=?1 AND revision=1",
    );
}

fn add_orphan_applied_event(fixture: &Fixture) {
    mutate_sql(
        fixture,
        "INSERT INTO applied_events(scope_digest,event_digest,revision) VALUES(?1,zeroblob(32),1)",
    );
}

fn misbind_applied_event(fixture: &Fixture) {
    mutate_sql(
        fixture,
        "UPDATE applied_events SET revision=2 WHERE scope_digest=?1 AND revision=1",
    );
}

fn gap_snapshot(fixture: &Fixture) {
    mutate_sql(
        fixture,
        "DELETE FROM snapshots WHERE scope_digest=?1 AND revision=1",
    );
}

fn extra_snapshot(fixture: &Fixture) {
    mutate_sql(
        fixture,
        "INSERT INTO snapshots(revision,scope_digest,state_digest,state_bytes) SELECT 99,scope_digest,state_digest,state_bytes FROM snapshots WHERE scope_digest=?1 AND revision=2",
    );
}

fn gap_graph(fixture: &Fixture) {
    mutate_sql(
        fixture,
        "DELETE FROM graph_commits WHERE scope_digest=?1 AND revision=1",
    );
}

fn extra_graph(fixture: &Fixture) {
    mutate_sql(
        fixture,
        "INSERT INTO graph_commits(scope_digest,revision,base_graph_digest,graph_digest,formula_digest,delta_bytes,replay_state_bytes) SELECT scope_digest,99,base_graph_digest,graph_digest,formula_digest,delta_bytes,replay_state_bytes FROM graph_commits WHERE scope_digest=?1 AND revision=2",
    );
}

fn gap_context(fixture: &Fixture) {
    mutate_sql(
        fixture,
        "DELETE FROM context_commits WHERE scope_digest=?1 AND revision=1",
    );
}

fn extra_context(fixture: &Fixture) {
    mutate_sql(
        fixture,
        "INSERT INTO context_commits(scope_digest,relation_scope_token,relation_hmac,revision,context_digest,canonical_state_bytes) SELECT scope_digest,relation_scope_token,relation_hmac,99,context_digest,canonical_state_bytes FROM context_commits WHERE scope_digest=?1 AND revision=2",
    );
}

fn corrupt_binding_revision(fixture: &Fixture) {
    let connection = Connection::open(&fixture.database).unwrap();
    connection
        .execute(
            "UPDATE active_bindings SET revision=2 WHERE bot_token=?1 AND persona_token=?2",
            params![
                &fixture.scope.bot_token[..],
                &fixture.scope.persona_token[..]
            ],
        )
        .unwrap();
}

fn corrupt_binding_incarnation(fixture: &Fixture) {
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE active_bindings SET incarnation_id=zeroblob(32) WHERE bot_token=?1 AND persona_token=?2",
            params![
                &fixture.scope.bot_token[..],
                &fixture.scope.persona_token[..]
            ],
        )
        .unwrap();
}

fn corrupt_seed_link(fixture: &Fixture) {
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE genesis_manifests SET seed_code_digest=zeroblob(32) WHERE manifest_digest=?1",
            params![&fixture.manifest_digest[..]],
        )
        .unwrap();
}

fn corrupt_incarnation_seed_link(fixture: &Fixture) {
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE incarnations SET seed_code_digest=zeroblob(32) WHERE incarnation_id=?1",
            params![&fixture.incarnation_id[..]],
        )
        .unwrap();
}

fn corrupt_compiler_link(fixture: &Fixture) {
    let connection = Connection::open(&fixture.database).unwrap();
    connection
        .execute(
            "UPDATE incarnations SET compiler_protocol_digest=zeroblob(32) WHERE incarnation_id=?1",
            params![&fixture.incarnation_id[..]],
        )
        .unwrap();
}

fn corrupt_manifest_compiler_link(fixture: &Fixture) {
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE genesis_manifests SET compiler_model_digest=zeroblob(32) WHERE manifest_digest=?1",
            params![&fixture.manifest_digest[..]],
        )
        .unwrap();
}

fn corrupt_incarnation_manifest_link(fixture: &Fixture) {
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE incarnations SET manifest_digest=zeroblob(32) WHERE incarnation_id=?1",
            params![&fixture.incarnation_id[..]],
        )
        .unwrap();
}

fn corrupt_incarnation_parent(fixture: &Fixture) {
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE incarnations SET parent_incarnation_id=zeroblob(32) WHERE incarnation_id=?1",
            params![&fixture.incarnation_id[..]],
        )
        .unwrap();
}

fn corrupt_manifest_wire(fixture: &Fixture) {
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE genesis_manifests SET canonical_bytes=X'00' WHERE manifest_digest=?1",
            params![&fixture.manifest_digest[..]],
        )
        .unwrap();
}

fn oversize_manifest_wire(fixture: &Fixture) {
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE genesis_manifests SET canonical_bytes=zeroblob(262145) WHERE manifest_digest=?1",
            params![&fixture.manifest_digest[..]],
        )
        .unwrap();
}

fn corrupt_source_link(fixture: &Fixture) {
    let connection = Connection::open(&fixture.database).unwrap();
    let source_json: String = connection
        .query_row(
            "SELECT source_json FROM genesis_manifests WHERE manifest_digest=?1",
            params![&fixture.manifest_digest[..]],
            |row| row.get(0),
        )
        .unwrap();
    let mut source: PersonaSourceRef = serde_json::from_str(&source_json).unwrap();
    source.source_digest[0] ^= 1;
    connection
        .execute(
            "UPDATE genesis_manifests SET source_json=?1 WHERE manifest_digest=?2",
            params![
                serde_json::to_string(&source).unwrap(),
                &fixture.manifest_digest[..]
            ],
        )
        .unwrap();
}

fn corrupt_source_token(fixture: &Fixture) {
    let connection = Connection::open(&fixture.database).unwrap();
    let source_json: String = connection
        .query_row(
            "SELECT source_json FROM genesis_manifests WHERE manifest_digest=?1",
            params![&fixture.manifest_digest[..]],
            |row| row.get(0),
        )
        .unwrap();
    let mut source: PersonaSourceRef = serde_json::from_str(&source_json).unwrap();
    source.scope.bot_token[0] ^= 1;
    connection
        .execute(
            "UPDATE genesis_manifests SET source_json=?1 WHERE manifest_digest=?2",
            params![
                serde_json::to_string(&source).unwrap(),
                &fixture.manifest_digest[..]
            ],
        )
        .unwrap();
}

fn oversize_source_json(fixture: &Fixture) {
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE genesis_manifests SET source_json=zeroblob(65537) WHERE manifest_digest=?1",
            params![&fixture.manifest_digest[..]],
        )
        .unwrap();
}

fn corrupt_lease_key(fixture: &Fixture) {
    Connection::open(&fixture.database)
        .unwrap()
        .execute("UPDATE genesis_leases SET scope_key=zeroblob(32)", [])
        .unwrap();
}

fn corrupt_lease_status(fixture: &Fixture) {
    Connection::open(&fixture.database)
        .unwrap()
        .execute("UPDATE genesis_leases SET status='claimed'", [])
        .unwrap();
}

fn corrupt_lease_nonce(fixture: &Fixture) {
    Connection::open(&fixture.database)
        .unwrap()
        .execute("UPDATE genesis_leases SET nonce_digest=zeroblob(32)", [])
        .unwrap();
}

fn corrupt_lease_manifest(fixture: &Fixture) {
    Connection::open(&fixture.database)
        .unwrap()
        .execute("UPDATE genesis_leases SET manifest_digest=zeroblob(32)", [])
        .unwrap();
}

fn corrupt_lease_incarnation(fixture: &Fixture) {
    Connection::open(&fixture.database)
        .unwrap()
        .execute("UPDATE genesis_leases SET incarnation_id=zeroblob(32)", [])
        .unwrap();
}

fn corrupt_incarnation_nonce_link(fixture: &Fixture) {
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE incarnations SET nonce_digest=zeroblob(32) WHERE incarnation_id=?1",
            params![&fixture.incarnation_id[..]],
        )
        .unwrap();
}

fn corrupt_root_snapshot_bytes(fixture: &Fixture) {
    let root_scope =
        wire::persona_scope_digest(&fixture.scope.bot_token, &fixture.scope.persona_token, None);
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE snapshots SET state_bytes=X'00' WHERE scope_digest=?1 AND revision=0",
            params![&root_scope[..]],
        )
        .unwrap();
}

fn corrupt_root_snapshot_digest(fixture: &Fixture) {
    let root_scope =
        wire::persona_scope_digest(&fixture.scope.bot_token, &fixture.scope.persona_token, None);
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE snapshots SET state_digest=zeroblob(32) WHERE scope_digest=?1 AND revision=0",
            params![&root_scope[..]],
        )
        .unwrap();
}

fn corrupt_baseline_graph_link(fixture: &Fixture) {
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE incarnations SET graph_digest=zeroblob(32) WHERE incarnation_id=?1",
            params![&fixture.incarnation_id[..]],
        )
        .unwrap();
}

fn corrupt_initial_snapshot_link(fixture: &Fixture) {
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE incarnations SET initial_snapshot_digest=zeroblob(32) WHERE incarnation_id=?1",
            params![&fixture.incarnation_id[..]],
        )
        .unwrap();
}

fn corrupt_genesis_residual(fixture: &Fixture) {
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE incarnations SET equilibrium_residual=1 WHERE incarnation_id=?1",
            params![&fixture.incarnation_id[..]],
        )
        .unwrap();
}

#[test]
fn authority_constructs_finite_migration_and_closes_all_history_sets() {
    let mut closed = fixture("authority-complete-history-closure");
    let outcome = closed.migrate().expect("finite authority-owned migration");
    let committed = authority_state(&closed);
    assert_eq!(committed.journal.len(), 3);
    assert_eq!(committed.applied_events, 3);
    assert_eq!(committed.snapshots.len(), 3);
    assert_eq!(committed.graphs.len(), 3);
    assert_eq!(committed.contexts.len(), 3);
    assert_eq!(committed.migrations, (1, 1));
    reopen_store(&mut closed);
    assert_eq!(
        closed.migrate().expect("fresh full-closure re-attestation"),
        outcome
    );
    assert_eq!(authority_state(&closed), committed);

    let cases: [(&str, fn(&Fixture)); 34] = [
        ("journal-gap", gap_journal),
        ("journal-extra", extra_journal),
        ("applied-delete", delete_applied_event),
        ("applied-orphan", add_orphan_applied_event),
        ("applied-misbind", misbind_applied_event),
        ("snapshot-gap", gap_snapshot),
        ("snapshot-extra", extra_snapshot),
        ("graph-gap", gap_graph),
        ("graph-extra", extra_graph),
        ("context-gap", gap_context),
        ("context-extra", extra_context),
        ("binding-revision", corrupt_binding_revision),
        ("binding-incarnation", corrupt_binding_incarnation),
        ("seed-link", corrupt_seed_link),
        ("incarnation-seed-link", corrupt_incarnation_seed_link),
        ("compiler-link", corrupt_compiler_link),
        ("manifest-compiler-link", corrupt_manifest_compiler_link),
        (
            "incarnation-manifest-link",
            corrupt_incarnation_manifest_link,
        ),
        ("incarnation-parent", corrupt_incarnation_parent),
        ("manifest-wire", corrupt_manifest_wire),
        ("manifest-wire-oversize", oversize_manifest_wire),
        ("source-link", corrupt_source_link),
        ("source-token", corrupt_source_token),
        ("source-json-oversize", oversize_source_json),
        ("lease-key", corrupt_lease_key),
        ("lease-status", corrupt_lease_status),
        ("lease-nonce", corrupt_lease_nonce),
        ("lease-manifest", corrupt_lease_manifest),
        ("lease-incarnation", corrupt_lease_incarnation),
        ("incarnation-nonce", corrupt_incarnation_nonce_link),
        ("root-snapshot-bytes", corrupt_root_snapshot_bytes),
        ("root-snapshot-digest", corrupt_root_snapshot_digest),
        ("baseline-graph-link", corrupt_baseline_graph_link),
        ("initial-snapshot-link", corrupt_initial_snapshot_link),
    ];
    for (label, corrupt) in cases {
        println!("phase-d-predecessor-corruption={label}");
        let mut fixture = fixture(label);
        corrupt(&fixture);
        let before = authority_state(&fixture);
        assert!(fixture.migrate().is_err(), "{label} must fail closed");
        assert_zero_authority_writes(&fixture, &before, true);
    }

    let mut fixture = fixture("genesis-residual");
    corrupt_genesis_residual(&fixture);
    let before = authority_state(&fixture);
    assert!(
        fixture.migrate().is_err(),
        "genesis residual must fail closed"
    );
    assert_zero_authority_writes(&fixture, &before, true);
}

#[test]
fn history_identity_requires_exact_set_and_root_closure() {
    for (label, corrupt) in [
        ("history-journal-gap", gap_journal as fn(&Fixture)),
        (
            "history-root-snapshot",
            corrupt_root_snapshot_digest as fn(&Fixture),
        ),
    ] {
        let mut fixture = fixture(label);
        corrupt(&fixture);
        let before = authority_state(&fixture);
        assert!(fixture.migrate().is_err(), "{label} must fail closed");
        assert_zero_authority_writes(&fixture, &before, true);
    }
}

#[test]
fn phase_d_idempotent_retry_revalidates_complete_authority_and_metadata() {
    let retry_cases: [(&str, &str); 5] = [
        (
            "migration-base-revision",
            "UPDATE legacy_semantic_formula_upgrades SET base_revision=1",
        ),
        (
            "migration-event-kind",
            "UPDATE journal SET event_kind='DeliveryOutcome' WHERE logical_revision=3",
        ),
        (
            "migration-applied-event",
            "DELETE FROM applied_events WHERE revision=3",
        ),
        (
            "migration-redundant-source-state",
            "UPDATE legacy_semantic_formula_upgrades SET source_state_digest=zeroblob(32)",
        ),
        (
            "migration-authority-sidecar",
            "UPDATE semantic_migration_authority_v1 SET commitment_digest=zeroblob(32)",
        ),
    ];
    for (label, sql) in retry_cases {
        println!("phase-d-retry-corruption={label}");
        let mut fixture = fixture(label);
        fixture.migrate().expect("first migration");
        Connection::open(&fixture.database)
            .unwrap()
            .execute(sql, [])
            .unwrap();
        let corrupted = authority_state(&fixture);
        assert!(fixture.migrate().is_err(), "{label} must fail closed");
        assert_eq!(authority_state(&fixture), corrupted, "{label} wrote state");
    }
}
