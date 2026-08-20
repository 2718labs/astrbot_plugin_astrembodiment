#![forbid(unsafe_code)]

use ae_contracts::EvidenceVector;
use ae_fixed::Fixed;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttentionHead {
    Salience,
    Interoceptive,
    Epistemic,
    SocialBoundary,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadCandidate {
    pub active_nodes: Vec<u32>,
    pub regional_loads: Vec<Fixed>,
    pub route_digest: [u8; 32],
}

pub fn assemble_load(evidence: &EvidenceVector, node_limit: u32) -> LoadCandidate {
    // MVP scaffold: deterministic placeholder. Replace in G2 with sparse masked heads.
    let intensity = evidence
        .positive
        .saturating_add(evidence.harm)
        .saturating_add(evidence.epistemic_conflict)
        .saturating_add(evidence.boundary);
    let active = if intensity > Fixed::ZERO {
        node_limit.min(2048)
    } else {
        0
    };
    LoadCandidate {
        active_nodes: (0..active).collect(),
        regional_loads: vec![intensity; 9],
        route_digest: [0; 32],
    }
}
