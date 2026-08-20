#![forbid(unsafe_code)]

use ae_fixed::Fixed;
use serde::{Deserialize, Serialize};

pub const NEURON_SLOTS: usize = 16_384;
pub const EDGE_CAPACITY: usize = 524_288;
pub const NODE_DOF: usize = 8;

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
}
