#![forbid(unsafe_code)]

use ae_contracts::{ActionContract, ActionVector};
use ae_fixed::Fixed;
use ae_renorm::Workspace;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionScore {
    pub task: Fixed,
    pub epistemic: Fixed,
    pub boundary: Fixed,
    pub repair: Fixed,
    pub continuity: Fixed,
    pub uncertainty_cost: Fixed,
    pub load_cost: Fixed,
    pub total: Fixed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionCandidate {
    pub id: u16,
    pub vector: ActionVector,
    pub score: ActionScore,
    pub rollout_digest: [u8; 32],
}

pub fn scaffold_contract(workspace: &Workspace, turn_id: [u8; 16]) -> ActionContract {
    let directness = if workspace.consistency_residual > Fixed::ZERO {
        Fixed::from_raw(650_000)
    } else {
        Fixed::from_raw(450_000)
    };
    ActionContract {
        action_id: [0; 16],
        turn_id,
        continuous: ActionVector {
            answer: Fixed::ONE,
            directness,
            verbosity: Fixed::from_raw(500_000),
            confidence_ceiling: Fixed::from_raw(700_000),
            ..ActionVector::default()
        },
        must_verify: false,
        must_acknowledge_error: false,
        must_correct_claim: false,
        may_set_boundary: true,
        may_withdraw: true,
        must_not_seek_reassurance: true,
        expires_at_ms: 0,
    }
}
