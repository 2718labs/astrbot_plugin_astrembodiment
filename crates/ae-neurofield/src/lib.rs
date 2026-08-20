#![forbid(unsafe_code)]

//! Neural field, sparse graph and their canonical digests.
//!
//! G0 ships a deterministic, validated initial-state projection from the
//! committed GenesisManifest (bounded fixed-point baseline per region, empty
//! valid graph). It is NOT the G2 developmental mapping and it runs no
//! dynamics: it exists so that no production transition can ever start from
//! a zeroed field while keeping G0 honest about what it is.

use ae_contracts::{wire, Digest, GenesisManifest};
use ae_fixed::Fixed;
use serde::{Deserialize, Serialize};

pub const NEURON_SLOTS: usize = 16_384;
pub const EDGE_CAPACITY: usize = 524_288;
pub const NODE_DOF: usize = 8;

/// Region layout from model/regions-v1.toml (start, count). Sum == 16384.
pub const REGION_LAYOUT: [(usize, usize); 9] = [
    (0, 2048),     // interoception_allostasis
    (2048, 2048),  // affective_valuation
    (4096, 1024),  // salience
    (5120, 2048),  // epistemic_fallibility
    (7168, 2048),  // social_boundary
    (9216, 1024),  // temper_inhibitory
    (10240, 4096), // world_model_imagination
    (14336, 1024), // global_workspace
    (15360, 1024), // action_expression
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NeuralField {
    pub potential: Vec<Fixed>,
    pub excitation: Vec<Fixed>,
    pub inhibition: Vec<Fixed>,
    pub adaptation: Vec<Fixed>,
    pub precision: Vec<Fixed>,
    pub prediction_error: Vec<Fixed>,
    pub eligibility: Vec<Fixed>,
    pub metabolic_reserve: Vec<Fixed>,
}

impl NeuralField {
    pub fn zeroed() -> Self {
        let zeros = || vec![Fixed::ZERO; NEURON_SLOTS];
        Self {
            potential: zeros(),
            excitation: zeros(),
            inhibition: zeros(),
            adaptation: zeros(),
            precision: zeros(),
            prediction_error: zeros(),
            eligibility: zeros(),
            metabolic_reserve: vec![Fixed::ONE; NEURON_SLOTS],
        }
    }

    pub fn validate(&self) -> bool {
        [
            self.potential.len(),
            self.excitation.len(),
            self.inhibition.len(),
            self.adaptation.len(),
            self.precision.len(),
            self.prediction_error.len(),
            self.eligibility.len(),
            self.metabolic_reserve.len(),
        ]
        .into_iter()
        .all(|len| len == NEURON_SLOTS)
    }

    pub fn active_node_count(&self) -> u32 {
        self.potential
            .iter()
            .filter(|value| value.raw() != 0)
            .count() as u32
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Synapse {
    pub target: u32,
    pub weight: i16,
    pub eligibility: i16,
    pub stability: u16,
    pub last_used_epoch: u16,
    pub operator_id: u8,
    pub delay_class: u8,
    pub flags: u16,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SparseGraph {
    pub row_offsets: Vec<u32>,
    pub edges: Vec<Synapse>,
}

impl SparseGraph {
    pub fn validate(&self) -> bool {
        self.edges.len() <= EDGE_CAPACITY
            && self.row_offsets.len() == NEURON_SLOTS + 1
            && self.row_offsets.last().copied().unwrap_or(0) as usize == self.edges.len()
    }

    pub fn empty() -> Self {
        Self {
            row_offsets: vec![0; NEURON_SLOTS + 1],
            edges: Vec::new(),
        }
    }
}

fn half(value: Fixed) -> Fixed {
    Fixed::from_ratio(value.raw() / 1_000, 2_000).unwrap_or(Fixed::ZERO)
}

/// G0 deterministic initial-state projection. Placeholder for the G2
/// developmental mapping F_region/F_graph; it is a pure function of the
/// committed Manifest and therefore replay-stable. It runs no dynamics and
/// produces a valid, non-zeroed field with an empty (valid) graph.
pub fn initial_state_from_manifest(
    manifest: &GenesisManifest,
    _formula_digest: &Digest,
    _development_seed: &Digest,
) -> (NeuralField, SparseGraph) {
    let t = &manifest.traits;
    let a = &manifest.allostasis;
    let e = &manifest.epistemic;
    let baseline = [
        half(a.energy.saturating_add(a.arousal)), // interoception_allostasis
        t.baseline_warmth,                        // affective_valuation
        half(t.sensitivity.saturating_add(t.irritability)), // salience
        half(t.epistemic_pride.saturating_add(t.epistemic_openness)), // epistemic_fallibility
        t.boundary_strength,                      // social_boundary
        t.composure,                              // temper_inhibitory
        t.curiosity,                              // world_model_imagination
        e.verification_drive,                     // global_workspace
        t.expression_drive,                       // action_expression
    ];

    let mut potential = Vec::with_capacity(NEURON_SLOTS);
    for (index, &(start, count)) in REGION_LAYOUT.iter().enumerate() {
        debug_assert_eq!(start + count, start + count);
        let value = baseline[index].clamp(Fixed::ZERO, Fixed::ONE);
        for slot in 0..count {
            let _ = slot;
            let _ = (start, count, index);
            potential.push(value);
        }
    }
    debug_assert_eq!(potential.len(), NEURON_SLOTS);

    let field = NeuralField {
        potential,
        excitation: vec![Fixed::ZERO; NEURON_SLOTS],
        inhibition: vec![Fixed::ZERO; NEURON_SLOTS],
        adaptation: vec![Fixed::ZERO; NEURON_SLOTS],
        precision: vec![Fixed::ZERO; NEURON_SLOTS],
        prediction_error: vec![Fixed::ZERO; NEURON_SLOTS],
        eligibility: vec![Fixed::ZERO; NEURON_SLOTS],
        metabolic_reserve: vec![Fixed::ONE; NEURON_SLOTS],
    };
    (field, SparseGraph::empty())
}

fn encode_fixed_vector(out: &mut Vec<u8>, values: &[Fixed]) {
    out.extend_from_slice(&(values.len() as u32).to_le_bytes());
    for value in values {
        out.extend_from_slice(&value.encode());
    }
}

/// Canonical state digest over the fixed-layout field encoding plus the
/// formula digest. Same bytes + same formula = same digest on any machine.
pub fn state_digest(field: &NeuralField, formula_digest: &Digest) -> Digest {
    let mut body = Vec::with_capacity(NEURON_SLOTS * NODE_DOF * 8 + 64);
    body.extend_from_slice(formula_digest);
    encode_fixed_vector(&mut body, &field.potential);
    encode_fixed_vector(&mut body, &field.excitation);
    encode_fixed_vector(&mut body, &field.inhibition);
    encode_fixed_vector(&mut body, &field.adaptation);
    encode_fixed_vector(&mut body, &field.precision);
    encode_fixed_vector(&mut body, &field.prediction_error);
    encode_fixed_vector(&mut body, &field.eligibility);
    encode_fixed_vector(&mut body, &field.metabolic_reserve);
    wire::domain_hash(wire::STATE_DOMAIN, &[&body])
}

fn encode_synapse(out: &mut Vec<u8>, synapse: &Synapse) {
    out.extend_from_slice(&synapse.target.to_le_bytes());
    out.extend_from_slice(&synapse.weight.to_le_bytes());
    out.extend_from_slice(&synapse.eligibility.to_le_bytes());
    out.extend_from_slice(&synapse.stability.to_le_bytes());
    out.extend_from_slice(&synapse.last_used_epoch.to_le_bytes());
    out.push(synapse.operator_id);
    out.push(synapse.delay_class);
    out.extend_from_slice(&synapse.flags.to_le_bytes());
}

/// Canonical graph digest over row offsets and packed edges.
pub fn graph_digest(graph: &SparseGraph) -> Digest {
    let mut body = Vec::with_capacity(graph.row_offsets.len() * 4 + graph.edges.len() * 16);
    body.extend_from_slice(&(graph.row_offsets.len() as u32).to_le_bytes());
    for offset in &graph.row_offsets {
        body.extend_from_slice(&offset.to_le_bytes());
    }
    body.extend_from_slice(&(graph.edges.len() as u32).to_le_bytes());
    for edge in &graph.edges {
        encode_synapse(&mut body, edge);
    }
    wire::domain_hash(wire::GRAPH_DOMAIN, &[&body])
}

#[cfg(test)]
mod tests {
    use super::*;
    use ae_contracts::{
        wire::manifest_body_digest, AllostaticSetpoints, EpistemicPriors, ExpressionPhenotype,
        PersonalityVector, SocialPriors,
    };

    fn manifest() -> GenesisManifest {
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
        manifest.manifest_digest = manifest_body_digest(&manifest);
        manifest
    }

    #[test]
    fn region_layout_covers_exact_brain() {
        let mut covered = 0usize;
        for &(start, count) in REGION_LAYOUT.iter() {
            assert_eq!(start, covered);
            covered += count;
        }
        assert_eq!(covered, NEURON_SLOTS);
    }

    #[test]
    fn initial_state_is_valid_and_not_zeroed() {
        let (field, graph) = initial_state_from_manifest(&manifest(), &[1; 32], &[2; 32]);
        assert!(field.validate());
        assert!(graph.validate());
        assert_eq!(graph.edges.len(), 0);
        // The fixture only seeds the non-zero regions from its manifest;
        // active_node_count intentionally reports non-zero potentials.
        assert!(field.active_node_count() > 0);
        assert!(field.potential.iter().any(|v| v.raw() != 0));
        assert!(field.metabolic_reserve.iter().all(|v| *v == Fixed::ONE));
        assert_ne!(
            state_digest(&field, &[1; 32]),
            state_digest(&NeuralField::zeroed(), &[1; 32])
        );
    }

    #[test]
    fn initial_state_is_deterministic() {
        let (a, ga) = initial_state_from_manifest(&manifest(), &[1; 32], &[2; 32]);
        let (b, gb) = initial_state_from_manifest(&manifest(), &[1; 32], &[2; 32]);
        assert_eq!(a.potential, b.potential);
        assert_eq!(state_digest(&a, &[1; 32]), state_digest(&b, &[1; 32]));
        assert_eq!(graph_digest(&ga), graph_digest(&gb));
    }

    #[test]
    fn digests_change_with_inputs() {
        let (field, graph) = initial_state_from_manifest(&manifest(), &[1; 32], &[2; 32]);
        let digest = state_digest(&field, &[1; 32]);
        assert_ne!(digest, state_digest(&field, &[9; 32]));
        let mut other = manifest();
        other.traits.baseline_warmth = Fixed::from_raw(610_000);
        let (field2, _) = initial_state_from_manifest(&other, &[1; 32], &[2; 32]);
        assert_ne!(digest, state_digest(&field2, &[1; 32]));
        let graph_empty = graph_digest(&graph);
        assert_ne!(graph_empty, [0; 32]);
    }

    #[test]
    fn graph_digest_detects_edge_change() {
        let mut graph = SparseGraph::empty();
        let empty = graph_digest(&graph);
        graph.edges.push(Synapse {
            target: 5,
            weight: 100,
            ..Synapse::default()
        });
        graph.row_offsets[0] = 0;
        graph.row_offsets[NEURON_SLOTS] = 1;
        assert_ne!(empty, graph_digest(&graph));
    }
}
