#![forbid(unsafe_code)]

//! Closed fifteen-slot routing for the semantic preview lane.

use ae_contracts::{
    perception_dimension_values, phase0_semantic_route_digest_v1, EvidenceVector,
    PHASE0_SEMANTIC_ROUTE_PRIMARY_COEFFICIENT_FXP6, PHASE0_SEMANTIC_ROUTE_RULES_V1,
    PHASE0_SEMANTIC_ROUTE_SECONDARY_COEFFICIENT_FXP6,
};
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

pub(crate) fn evidence_values(evidence: &EvidenceVector) -> [Fixed; 15] {
    perception_dimension_values(evidence)
}

/// Frozen route commitment shared by Phase-0 formula derivation and Store
/// continuity validation.
pub(crate) fn full_vector_route_digest() -> [u8; 32] {
    phase0_semantic_route_digest_v1()
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

    for (value, route) in evidence_values(evidence)
        .into_iter()
        .zip(PHASE0_SEMANTIC_ROUTE_RULES_V1)
    {
        if !(Fixed::ZERO..=Fixed::ONE).contains(&value) {
            return Err(FullVectorLoadError::InvalidDimension);
        }
        let neutral = Fixed::ONE.saturating_sub(value);
        for (region, coefficient) in std::iter::once((
            usize::from(route.primary),
            PHASE0_SEMANTIC_ROUTE_PRIMARY_COEFFICIENT_FXP6,
        ))
        .chain(route.secondary.into_iter().map(|region| {
            (
                usize::from(region),
                PHASE0_SEMANTIC_ROUTE_SECONDARY_COEFFICIENT_FXP6,
            )
        })) {
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
        evaluated_dimension_count: PHASE0_SEMANTIC_ROUTE_RULES_V1.len() as u8,
        injected_dimension_count: PHASE0_SEMANTIC_ROUTE_RULES_V1.len() as u8,
        route_digest: full_vector_route_digest(),
    })
}
