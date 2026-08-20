#![forbid(unsafe_code)]

use ae_authority::ResidualCoordinate;
use ae_contracts::{InvariantResiduals, SourceAuthority};
use ae_fixed::Fixed;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResidualIncrement {
    pub coordinate: ResidualCoordinate,
    pub amount: Fixed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlasticityCandidate {
    pub source: SourceAuthority,
    pub increments: Vec<ResidualIncrement>,
    pub invariant_residuals: InvariantResiduals,
}

pub fn positive_part(value: Fixed) -> Fixed {
    value.clamp(Fixed::ZERO, Fixed::from_raw(i64::MAX))
}
