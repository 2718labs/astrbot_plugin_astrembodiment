#![forbid(unsafe_code)]

use ae_contracts::SourceAuthority;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResidualCoordinate {
    Bond,
    Reciprocity,
    FrictionRepetition,
    FrictionInstability,
    BoundaryViolation,
    Scar,
    Repair,
    Fallibility,
    FairCorrection,
    Humiliation,
    FalseAccusation,
    OutreachRejection,
}

pub struct AuthorityProjection;

impl AuthorityProjection {
    pub fn allows(source: SourceAuthority, coordinate: ResidualCoordinate) -> bool {
        use ResidualCoordinate::*;
        use SourceAuthority::*;
        match source {
            SelfAction | SelfCritique | PlatformObserved | TimeAdvance => false,
            UserObserved => matches!(
                coordinate,
                Bond | Reciprocity
                    | FrictionRepetition
                    | FrictionInstability
                    | BoundaryViolation
                    | Scar
                    | Humiliation
                    | OutreachRejection
            ),
            ExplicitFeedback => {
                !matches!(coordinate, Fallibility | FairCorrection | FalseAccusation)
            }
            VerifierResult => matches!(
                coordinate,
                BoundaryViolation
                    | Repair
                    | Fallibility
                    | FairCorrection
                    | Humiliation
                    | FalseAccusation
            ),
            AdminAction => true,
        }
    }
}
