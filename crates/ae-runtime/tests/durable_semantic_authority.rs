use ae_contracts::{
    wire, AllostaticSetpoints, EpistemicPriors, ExpressionPhenotype, GenesisManifestProposal,
    GenesisReceipt, GenesisStatus, PersonaGenesisRequest, PersonaScopeRef, PersonaSelectionKind,
    PersonaSourceRef, PersonalityVector, ScopeRef, SocialPriors,
};
use ae_fixed::Fixed;
use ae_genesis::{derive_identity, genesis_scope_key, GenesisPrior};
use ae_neurofield::{
    graph_digest, initial_state_from_manifest, state_digest, NeuralField, SparseGraph, Synapse,
    EDGE_CAPACITY, NEURON_SLOTS,
};
use ae_runtime::AstrRuntime;
use ae_store::{ClaimOutcome, GenesisCommit, Store};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

const CANONICAL_HOT_STATE_MAGIC_V1: [u8; 8] = *b"AEHOTST\0";
const CANONICAL_HOT_STATE_SCHEMA_V1: u16 = 1;

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
    let (state_bytes, layout) = encode_test_canonical_hot_state(&formula_digest, &field, &graph);
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

    let (formula_digest, field, graph) = decode_test_canonical_hot_state(&stored.state_bytes);
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
    let mut store = Store::open(&fixture.database).expect("open fixture store for corruption");
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

#[test]
fn canonical_hot_state_rejects_truncation_counts_invalid_graph_formula_and_trailing_bytes() {
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
    oversized_vector
        [fixture.layout.vector_count_offsets[0]..fixture.layout.vector_count_offsets[0] + 4]
        .copy_from_slice(&u32::MAX.to_le_bytes());
    assert_rejected("oversized-vector-count", oversized_vector);

    let mut oversized_edge = fixture.state_bytes.clone();
    oversized_edge[fixture.layout.edge_count_offset..fixture.layout.edge_count_offset + 4]
        .copy_from_slice(&u32::MAX.to_le_bytes());
    assert_rejected("oversized-edge-count", oversized_edge);

    let mut capacity_overflow = fixture.state_bytes.clone();
    capacity_overflow[fixture.layout.edge_count_offset..fixture.layout.edge_count_offset + 4]
        .copy_from_slice(&((EDGE_CAPACITY + 1) as u32).to_le_bytes());
    assert_rejected("edge-capacity-overflow", capacity_overflow);

    let mut non_monotonic_offsets = fixture.state_bytes.clone();
    non_monotonic_offsets[fixture.layout.row_value_offset + 4..fixture.layout.row_value_offset + 8]
        .copy_from_slice(&2u32.to_le_bytes());
    non_monotonic_offsets
        [fixture.layout.row_value_offset + 8..fixture.layout.row_value_offset + 12]
        .copy_from_slice(&1u32.to_le_bytes());
    assert_rejected("non-monotonic-row-offsets", non_monotonic_offsets);

    let malformed_first_row_offset = fixture_with_first_row_offset("nonzero-first-row-offset", 1);
    let mut malformed_runtime =
        AstrRuntime::open(&malformed_first_row_offset.database).expect("open malformed runtime");
    assert!(
        malformed_runtime
            .current_revision(&malformed_first_row_offset.scope)
            .is_err(),
        "nonzero first row offset bytes must not bind HotBrain"
    );

    let mut target_out_of_bounds = fixture.state_bytes.clone();
    target_out_of_bounds[fixture.layout.edge_value_offset..fixture.layout.edge_value_offset + 4]
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
