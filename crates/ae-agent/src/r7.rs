#![forbid(unsafe_code)]

use ae_contracts::r7::{wire as r7_wire, ActionContract, ActionVector};
use ae_contracts::{wire, Digest, Id128};
use ae_fixed::Fixed;
use ae_renorm::Workspace;
use serde::{Deserialize, Serialize};

pub const EPISTEMIC_ACTION_POLICY_DOMAIN_V1: &[u8] =
    b"astr-embodiment/r7/epistemic-action-policy-v1";
pub const EPISTEMIC_ACTION_PROVENANCE_DOMAIN_V1: &[u8] =
    b"astr-embodiment/r7/epistemic-action-provenance-v1";
pub const EPISTEMIC_SOURCE_PROVENANCE_DOMAIN_V1: &[u8] =
    b"astr-embodiment/r7/epistemic-source-provenance-v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionPolicyErrorV1 {
    MissingEpistemicSource,
    ZeroSourceDigest,
    SourceDigestMismatch,
    InvalidPolicyVersion,
    PolicyDigestMismatch,
    ForeignIdentity,
    ForeignState,
    ForeignScope,
    ForeignRevision,
    ForeignTurn,
    TimeOverflow,
    Expired,
    CallerSelectedFieldRejected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PolicyArtifactV1 {
    pub version: u32,
    pub digest: Digest,
    pub ttl_ms: u64,
}

impl PolicyArtifactV1 {
    pub fn derive(
        version: u32,
        constitution_digest: Digest,
        scope_digest: Digest,
        ttl_ms: u64,
    ) -> Self {
        let version_bytes = version.to_be_bytes();
        let ttl_bytes = ttl_ms.to_be_bytes();
        let digest = wire::domain_hash(
            EPISTEMIC_ACTION_POLICY_DOMAIN_V1,
            &[
                &version_bytes,
                &constitution_digest,
                &scope_digest,
                &ttl_bytes,
            ],
        );
        Self {
            version,
            digest,
            ttl_ms,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EpistemicSourceRefV1 {
    pub source_digest: Digest,
    pub state_digest: Digest,
    pub identity_digest: Digest,
    pub scope_digest: Digest,
    pub turn_id: Id128,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionAuthorityContextV1 {
    pub state_digest: Digest,
    pub identity_digest: Digest,
    pub scope_digest: Digest,
    pub turn_id: Id128,
    pub revision: u64,
    pub now_ms: u64,
    pub policy: PolicyArtifactV1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EpistemicActionInputV1 {
    pub source: Option<EpistemicSourceRefV1>,
    pub context: ActionAuthorityContextV1,
    pub r7_available: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CallerSelectedFieldsV1 {
    pub classification: bool,
    pub action: bool,
    pub text: bool,
    pub provider: bool,
    pub control: bool,
}

impl CallerSelectedFieldsV1 {
    fn any(self) -> bool {
        self.classification || self.action || self.text || self.provider || self.control
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpistemicActionPolicyV1 {
    contract: Option<ActionContract>,
    contract_digest: Digest,
    provenance_digest: Digest,
    g0_only: bool,
}

impl EpistemicActionPolicyV1 {
    pub fn contract(&self) -> Option<&ActionContract> {
        self.contract.as_ref()
    }

    pub fn contract_digest(&self) -> &Digest {
        &self.contract_digest
    }

    pub fn provenance_digest(&self) -> &Digest {
        &self.provenance_digest
    }

    pub fn g0_only(&self) -> bool {
        self.g0_only
    }
}

pub fn compile_epistemic_action_policy_v1(
    input: &EpistemicActionInputV1,
    caller_fields: CallerSelectedFieldsV1,
) -> Result<EpistemicActionPolicyV1, ActionPolicyErrorV1> {
    if caller_fields.any() {
        return Err(ActionPolicyErrorV1::CallerSelectedFieldRejected);
    }
    let source = input
        .source
        .as_ref()
        .ok_or(ActionPolicyErrorV1::MissingEpistemicSource)?;
    let context = input.context;
    if context.policy.version == 0 {
        return Err(ActionPolicyErrorV1::InvalidPolicyVersion);
    }
    let expected_policy = PolicyArtifactV1::derive(
        context.policy.version,
        context.identity_digest,
        context.scope_digest,
        context.policy.ttl_ms,
    );
    if context.policy.digest != expected_policy.digest {
        return Err(ActionPolicyErrorV1::PolicyDigestMismatch);
    }
    if source.identity_digest != context.identity_digest {
        return Err(ActionPolicyErrorV1::ForeignIdentity);
    }
    if source.state_digest != context.state_digest {
        return Err(ActionPolicyErrorV1::ForeignState);
    }
    if source.scope_digest != context.scope_digest {
        return Err(ActionPolicyErrorV1::ForeignScope);
    }
    if source.revision != context.revision {
        return Err(ActionPolicyErrorV1::ForeignRevision);
    }
    if source.turn_id != context.turn_id {
        return Err(ActionPolicyErrorV1::ForeignTurn);
    }
    let revision = context.revision.to_be_bytes();
    if source.source_digest == [0; 32] {
        return Err(ActionPolicyErrorV1::ZeroSourceDigest);
    }
    let expected_source_digest = r7_wire::domain_hash(
        EPISTEMIC_SOURCE_PROVENANCE_DOMAIN_V1,
        &[
            &source.state_digest,
            &source.identity_digest,
            &source.scope_digest,
            &source.turn_id,
            &revision,
        ],
    );
    if source.source_digest != expected_source_digest {
        return Err(ActionPolicyErrorV1::SourceDigestMismatch);
    }
    let expires_at_ms = context
        .now_ms
        .checked_add(context.policy.ttl_ms)
        .ok_or(ActionPolicyErrorV1::TimeOverflow)?;
    if context.policy.ttl_ms == 0 || context.now_ms >= expires_at_ms {
        return Err(ActionPolicyErrorV1::Expired);
    }
    let provenance_digest = wire::domain_hash(
        EPISTEMIC_ACTION_PROVENANCE_DOMAIN_V1,
        &[
            &context.policy.digest,
            &context.state_digest,
            &context.identity_digest,
            &context.scope_digest,
            &context.turn_id,
            &revision,
            &source.source_digest,
        ],
    );
    if !input.r7_available {
        return Ok(EpistemicActionPolicyV1 {
            contract: None,
            contract_digest: [0; 32],
            provenance_digest,
            g0_only: true,
        });
    }

    let action_hash = wire::domain_hash(
        EPISTEMIC_ACTION_POLICY_DOMAIN_V1,
        &[&provenance_digest, &source.source_digest],
    );
    let mut action_id = [0; 16];
    action_id.copy_from_slice(&action_hash[..16]);
    let directness = Fixed::from_raw(400_000 + i64::from(source.source_digest[0] % 5) * 50_000);
    let confidence = Fixed::from_raw(500_000 + i64::from(source.source_digest[1] % 4) * 50_000);
    let contract = ActionContract {
        action_id,
        turn_id: context.turn_id,
        continuous: ActionVector {
            answer: Fixed::ONE,
            directness,
            confidence_ceiling: confidence,
            ..ActionVector::default()
        },
        must_verify: source.source_digest[2] & 1 == 1,
        must_acknowledge_error: source.source_digest[3] & 1 == 1,
        must_correct_claim: false,
        may_set_boundary: true,
        may_withdraw: true,
        must_not_seek_reassurance: true,
        expires_at_ms,
    };
    let contract_digest = r7_wire::domain_hash(
        b"astr-embodiment/r7/epistemic-action-contract-v1",
        &[
            &contract.action_id,
            &contract.turn_id,
            &contract.continuous.answer.raw().to_le_bytes(),
            &contract.continuous.directness.raw().to_le_bytes(),
            &contract.continuous.confidence_ceiling.raw().to_le_bytes(),
            &[contract.must_verify as u8],
            &[contract.must_acknowledge_error as u8],
            &[contract.must_correct_claim as u8],
            &[contract.may_set_boundary as u8],
            &[contract.may_withdraw as u8],
            &[contract.must_not_seek_reassurance as u8],
            &contract.expires_at_ms.to_le_bytes(),
        ],
    );
    Ok(EpistemicActionPolicyV1 {
        contract: Some(contract),
        contract_digest,
        provenance_digest,
        g0_only: false,
    })
}

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
