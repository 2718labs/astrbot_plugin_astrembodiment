#![forbid(unsafe_code)]

use ae_contracts::r7::EvidenceVector;
use ae_fixed::Fixed;
use ae_neurofield::{NEURON_SLOTS, REGION_LAYOUT};
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
    pub node_loads: Vec<Fixed>,
    pub regional_loads: Vec<Fixed>,
    pub route_digest: [u8; 32],
}

const PRIMARY_COEFFICIENT: Fixed = Fixed::ONE;
const SECONDARY_COEFFICIENT: Fixed = Fixed::from_raw(500_000);

#[derive(Clone, Copy)]
struct RouteRule {
    primary: usize,
    secondary: Option<usize>,
}

const ROUTES: [RouteRule; 15] = [
    RouteRule {
        primary: 1,
        secondary: Some(8),
    },
    RouteRule {
        primary: 1,
        secondary: Some(8),
    },
    RouteRule {
        primary: 0,
        secondary: Some(5),
    },
    RouteRule {
        primary: 4,
        secondary: Some(5),
    },
    RouteRule {
        primary: 3,
        secondary: Some(8),
    },
    RouteRule {
        primary: 2,
        secondary: Some(7),
    },
    RouteRule {
        primary: 6,
        secondary: Some(2),
    },
    RouteRule {
        primary: 2,
        secondary: Some(3),
    },
    RouteRule {
        primary: 3,
        secondary: Some(7),
    },
    RouteRule {
        primary: 3,
        secondary: Some(7),
    },
    RouteRule {
        primary: 4,
        secondary: Some(7),
    },
    RouteRule {
        primary: 5,
        secondary: Some(4),
    },
    RouteRule {
        primary: 4,
        secondary: Some(7),
    },
    RouteRule {
        primary: 8,
        secondary: Some(7),
    },
    RouteRule {
        primary: 0,
        secondary: Some(4),
    },
];

fn evidence_values(evidence: &EvidenceVector) -> [Fixed; 15] {
    [
        evidence.positive,
        evidence.affiliation,
        evidence.harm,
        evidence.boundary,
        evidence.repair,
        evidence.repetition,
        evidence.new_information,
        evidence.constraint_instability,
        evidence.epistemic_conflict,
        evidence.self_responsibility,
        evidence.other_responsibility,
        evidence.hostility,
        evidence.publicness,
        evidence.engagement,
        evidence.rejection,
    ]
}

fn contribution(value: Fixed, coefficient: Fixed) -> Fixed {
    if !(Fixed::ZERO..=Fixed::ONE).contains(&value) {
        return Fixed::ZERO;
    }
    value.checked_mul(coefficient).unwrap_or(Fixed::ZERO)
}

fn route_digest() -> [u8; 32] {
    let mut route_bytes = Vec::with_capacity(ROUTES.len() * 2);
    for route in ROUTES {
        route_bytes.push(route.primary as u8);
        route_bytes.push(
            route
                .secondary
                .map(|region| region as u8)
                .unwrap_or(u8::MAX),
        );
    }
    ae_contracts::r7::wire::domain_hash(
        b"astr-embodiment/semantic-evidence-route-v1",
        &[&route_bytes],
    )
}

pub fn assemble_load(evidence: &EvidenceVector, node_limit: u32) -> LoadCandidate {
    let mut regional_loads = vec![Fixed::ZERO; REGION_LAYOUT.len()];
    for (value, route) in evidence_values(evidence).into_iter().zip(ROUTES) {
        regional_loads[route.primary] =
            regional_loads[route.primary].saturating_add(contribution(value, PRIMARY_COEFFICIENT));
        if let Some(secondary) = route.secondary {
            regional_loads[secondary] = regional_loads[secondary]
                .saturating_add(contribution(value, SECONDARY_COEFFICIENT));
        }
    }

    let node_limit = usize::try_from(node_limit)
        .unwrap_or(NEURON_SLOTS)
        .min(NEURON_SLOTS);
    let mut active_nodes = Vec::new();
    let mut node_loads = Vec::new();
    for (region, &(start, count)) in REGION_LAYOUT.iter().enumerate() {
        let load = regional_loads[region];
        if load == Fixed::ZERO || start >= node_limit {
            continue;
        }
        let end = start.saturating_add(count).min(node_limit);
        for node in start..end {
            active_nodes.push(node as u32);
            node_loads.push(load);
        }
    }

    LoadCandidate {
        active_nodes,
        node_loads,
        regional_loads,
        route_digest: route_digest(),
    }
}
