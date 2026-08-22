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

/// Full-vector regional input for the semantic dynamics path.  Unlike the
/// legacy sparse `LoadCandidate`, this value proves that every fixed semantic
/// slot was consumed, including literal zeroes in the neutral channel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullVectorLoad {
    pub evidence_means: [Fixed; REGION_LAYOUT.len()],
    pub neutral_means: [Fixed; REGION_LAYOUT.len()],
    pub evaluated_dimension_count: u8,
    pub injected_dimension_count: u8,
    pub route_digest: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullVectorLoadError {
    InvalidDimension,
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

fn full_vector_route_digest() -> [u8; 32] {
    let mut route_bytes = Vec::with_capacity(ROUTES.len() * 18);
    for route in ROUTES {
        route_bytes.push(route.primary as u8);
        route_bytes.push(
            route
                .secondary
                .map(|region| region as u8)
                .unwrap_or(u8::MAX),
        );
        route_bytes.extend_from_slice(&PRIMARY_COEFFICIENT.raw().to_be_bytes());
        route_bytes.extend_from_slice(&SECONDARY_COEFFICIENT.raw().to_be_bytes());
    }
    ae_contracts::r7::wire::domain_hash(
        b"astr-embodiment/semantic-evidence-route-neutral-v1",
        &[&route_bytes],
    )
}

/// Assemble the exact fifteen semantic slots into independent evidence and
/// neutral regional means.  `0` remains a real input: it contributes no
/// evidence drive but a full neutral value to each of its fixed route edges.
pub fn assemble_full_vector_load(
    evidence: &EvidenceVector,
) -> Result<FullVectorLoad, FullVectorLoadError> {
    let mut evidence_sums = [Fixed::ZERO; REGION_LAYOUT.len()];
    let mut neutral_sums = [Fixed::ZERO; REGION_LAYOUT.len()];
    let mut weight_sums = [Fixed::ZERO; REGION_LAYOUT.len()];

    for (value, route) in evidence_values(evidence).into_iter().zip(ROUTES) {
        if !(Fixed::ZERO..=Fixed::ONE).contains(&value) {
            return Err(FullVectorLoadError::InvalidDimension);
        }
        let neutral = Fixed::ONE.saturating_sub(value);
        for (region, coefficient) in std::iter::once((route.primary, PRIMARY_COEFFICIENT)).chain(
            route
                .secondary
                .into_iter()
                .map(|region| (region, SECONDARY_COEFFICIENT)),
        ) {
            let evidence_contribution = value
                .checked_mul(coefficient)
                .ok_or(FullVectorLoadError::InvalidDimension)?;
            let neutral_contribution = neutral
                .checked_mul(coefficient)
                .ok_or(FullVectorLoadError::InvalidDimension)?;
            evidence_sums[region] = evidence_sums[region].saturating_add(evidence_contribution);
            neutral_sums[region] = neutral_sums[region].saturating_add(neutral_contribution);
            weight_sums[region] = weight_sums[region].saturating_add(coefficient);
        }
    }

    let evidence_means = std::array::from_fn(|region| {
        evidence_sums[region]
            .checked_div(weight_sums[region])
            .expect("the fixed route topology covers every region")
    });
    let neutral_means = std::array::from_fn(|region| {
        neutral_sums[region]
            .checked_div(weight_sums[region])
            .expect("the fixed route topology covers every region")
    });

    Ok(FullVectorLoad {
        evidence_means,
        neutral_means,
        evaluated_dimension_count: ROUTES.len() as u8,
        injected_dimension_count: ROUTES.len() as u8,
        route_digest: full_vector_route_digest(),
    })
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

#[cfg(test)]
mod full_vector_tests {
    use super::*;

    #[test]
    fn full_vector_assembler_consumes_all_fifteen_slots_including_zero() {
        let all_neutral = EvidenceVector::default();
        let neutral_load = assemble_full_vector_load(&all_neutral)
            .expect("the exact all-zero vector is a valid neutral input");

        assert_eq!(neutral_load.evaluated_dimension_count, 15);
        assert_eq!(neutral_load.injected_dimension_count, 15);
        assert_eq!(
            neutral_load.evidence_means,
            [Fixed::ZERO; REGION_LAYOUT.len()]
        );
        assert_eq!(
            neutral_load.neutral_means,
            [Fixed::ONE; REGION_LAYOUT.len()]
        );

        let all_evidence = EvidenceVector {
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
        };
        let evidence_load = assemble_full_vector_load(&all_evidence)
            .expect("the exact all-one vector is a valid evidence input");

        assert_eq!(evidence_load.evaluated_dimension_count, 15);
        assert_eq!(evidence_load.injected_dimension_count, 15);
        assert_eq!(
            evidence_load.evidence_means,
            [Fixed::ONE; REGION_LAYOUT.len()]
        );
        assert_eq!(
            evidence_load.neutral_means,
            [Fixed::ZERO; REGION_LAYOUT.len()]
        );
    }
}
