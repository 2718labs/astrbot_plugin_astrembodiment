#![forbid(unsafe_code)]

//! Closed fifteen-slot routing for the semantic preview lane.

use ae_contracts::{perception_dimension_values, wire, EvidenceVector};
use ae_fixed::Fixed;
use ae_neurofield::REGION_LAYOUT;
use serde::{Deserialize, Serialize};

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

pub(crate) fn evidence_values(evidence: &EvidenceVector) -> [Fixed; 15] {
    perception_dimension_values(evidence)
}

pub(crate) fn full_vector_route_digest() -> [u8; 32] {
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
    wire::domain_hash(
        b"astr-embodiment/semantic-evidence-route-neutral-v1",
        &[&route_bytes],
    )
}

/// Consume every fixed proposal slot. Literal zero values remain inputs through
/// their neutral contribution, so a valid neutral estimate never becomes a
/// fallback path.
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
            .expect("fixed semantic routes cover every region")
    });
    let neutral_means = std::array::from_fn(|region| {
        neutral_sums[region]
            .checked_div(weight_sums[region])
            .expect("fixed semantic routes cover every region")
    });

    Ok(FullVectorLoad {
        evidence_means,
        neutral_means,
        evaluated_dimension_count: ROUTES.len() as u8,
        injected_dimension_count: ROUTES.len() as u8,
        route_digest: full_vector_route_digest(),
    })
}
