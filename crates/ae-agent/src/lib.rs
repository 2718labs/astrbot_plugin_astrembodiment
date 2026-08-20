#![forbid(unsafe_code)]

use ae_contracts::wire;
use ae_contracts::{ActionContract, ActionVector, Digest, Id128};
use ae_fixed::Fixed;
use ae_renorm::Workspace;
use serde::{Deserialize, Serialize};

/// R7-only contract builder; the root API remains the alpha/G0 ABI.
pub mod r7;

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

/// The G0 deterministic no-op action contract.
///
/// The transition itself changes no neural state, but the contract is still a
/// pure function of (committed Manifest, canonical event): the same genesis
/// and the same stimulus yield byte-identical contracts on any machine, in
/// 1C1G and 2C2G alike. The action id is derived, never invented.
pub fn noop_action_contract(
    manifest_digest: &Digest,
    event_digest: &Digest,
    turn_id: Id128,
) -> ActionContract {
    let action_id = wire::domain_hash(b"ae.action-id.v1", &[manifest_digest, event_digest]);
    let mut action_id_bytes = [0u8; 16];
    action_id_bytes.copy_from_slice(&action_id[..16]);
    ActionContract {
        action_id: action_id_bytes,
        turn_id,
        continuous: ActionVector {
            answer: Fixed::ONE,
            directness: Fixed::from_raw(500_000),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_contract_is_deterministic() {
        let manifest = [7; 32];
        let event = [9; 32];
        let turn = [3; 16];
        let a = noop_action_contract(&manifest, &event, turn);
        let b = noop_action_contract(&manifest, &event, turn);
        assert_eq!(a, b);
        assert_eq!(
            wire::action_contract_digest(&a),
            wire::action_contract_digest(&b)
        );
    }

    #[test]
    fn noop_contract_depends_on_manifest_and_event() {
        let a = noop_action_contract(&[7; 32], &[9; 32], [3; 16]);
        let b = noop_action_contract(&[8; 32], &[9; 32], [3; 16]);
        let c = noop_action_contract(&[7; 32], &[10; 32], [3; 16]);
        assert_ne!(a.action_id, b.action_id);
        assert_ne!(a.action_id, c.action_id);
        assert_eq!(a.turn_id, [3; 16]);
        assert!(a.may_set_boundary && a.may_withdraw && a.must_not_seek_reassurance);
        assert!(!a.must_verify && !a.must_acknowledge_error && !a.must_correct_claim);
        assert_eq!(a.continuous.answer, Fixed::ONE);
    }
}
