#![forbid(unsafe_code)]

//! AstrRuntime: the G0 vertical slice orchestrator.
//!
//! ensure_genesis -> deterministic no-op apply_event -> SQLite commit ->
//! replay verification. Python cannot reach any of this state directly; the
//! PyO3 surface exposes only coarse calls.
//!
//! The R7 compatibility implementation is deliberately crate-private; it is
//! not a second production runtime authority.
//!
//! ```compile_fail
//! use ae_runtime::r7::{AstrRuntime, R7PreOutputProjectionInputV1};
//!
//! let _ = std::any::TypeId::of::<AstrRuntime>();
//! let _ = std::any::TypeId::of::<R7PreOutputProjectionInputV1>();
//! ```

use ae_agent::noop_action_contract;
use ae_authority::authority_projection_digest;
use ae_continuum::{CommitEnvelope, ReplayReport};
use ae_contracts::r7::{PerceptionProposalErrorV1, PerceptionProposalV1};
use ae_contracts::{
    wire, ActionContract, CanonicalEvent, CommitStatus, Digest, GenesisReceipt, GenesisStatus,
    Id128, InvariantResiduals, PersonaGenesisRequest, ScopeRef, TransitionReceipt,
};
use ae_fixed::Fixed;
use ae_neurofield::{
    graph_digest, initial_state_from_manifest, state_digest, NeuralField, SparseGraph, Synapse,
    EDGE_CAPACITY, NEURON_SLOTS,
};
use ae_store::{
    ClaimOutcome, GenesisCommit, R7PolicyBindingKeyV1, R7PolicyCommitOutcomeV1,
    R7PolicyValidationContextV1, R7PublicPolicyBundleV1, StatefulCommit, Store, StoreError,
};
use std::path::Path;
use thiserror::Error;

mod n2_native_assembly;
/// Native R7 compatibility projection is an implementation detail of the
/// durable root runtime, never an alternate production authority.
#[cfg_attr(not(test), allow(dead_code, unused_imports))]
mod r7;

const CANONICAL_HOT_STATE_MAGIC_V1: [u8; 8] = *b"AEHOTST\0";
const CANONICAL_HOT_STATE_SCHEMA_V1: u16 = 1;
const CANONICAL_HOT_STATE_VECTOR_COUNT: usize = 8;
const SYNAPSE_WIRE_BYTES: usize = 16;
const R7_SEMANTIC_PERSONA_SCOPE_DOMAIN_V1: &[u8] =
    b"astr-embodiment/runtime/r7-semantic-persona-scope-v1";

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("storage error: {0}")]
    Store(#[from] StoreError),
    #[error("genesis error: {0}")]
    Genesis(#[from] ae_genesis::GenesisError),
    #[error("persona genesis is required before production events")]
    PersonaGenesisRequired,
    #[error("event persona does not match the bound incarnation")]
    GenesisManifestMismatch,
    #[error("event causal base revision {actual} does not match committed revision {expected}")]
    StaleCausalBase { expected: u64, actual: u64 },
    #[error("event kind {0} is not supported by the G0 no-op lane")]
    UnsupportedEvent(&'static str),
    #[error("genesis lease is in flight; retry after backoff")]
    RetryWait,
    #[error("runtime is closed")]
    Closed,
    #[error("invalid neural state")]
    InvalidNeuralState,
    #[error("private projection unavailable")]
    PrivateProjectionUnavailable,
    #[error("invalid perception proposal")]
    InvalidPerceptionProposal,
    #[error("invalid perception scope")]
    InvalidPerceptionScope,
    #[error("semantic proposal identity conflicts with a committed event")]
    SemanticIdentityConflict,
    #[error("semantic revision overflow")]
    SemanticRevisionOverflow,
    #[error("semantic transition did not change state")]
    SemanticStateUnchanged,
}

impl From<r7::RuntimeError> for RuntimeError {
    fn from(_error: r7::RuntimeError) -> Self {
        Self::PrivateProjectionUnavailable
    }
}

#[derive(Debug)]
pub struct ApplyDecision {
    pub contract: ActionContract,
    pub receipt: TransitionReceipt,
    pub revision: u64,
    /// True when this exact event had already been applied; the state was not
    /// changed and the returned receipt is the originally committed one.
    pub deduplicated: bool,
}

/// Closed result of the SPC1 native semantic ingress.  It intentionally has
/// no action contract, private projection, wire bytes, callback, or payload.
#[derive(Clone, Debug)]
pub struct PerceptionProposalDecisionV1 {
    pub receipt: TransitionReceipt,
    pub revision: u64,
    pub deduplicated: bool,
}

/// Result of the one supported production R7 semantic transition. Its receipt
/// is always the root canonical receipt that was committed to SQLite. An exact
/// retry returns no second one-shot wire.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct UserStimulusDecisionV1 {
    pub(crate) receipt: TransitionReceipt,
    pub(crate) revision: u64,
    pub(crate) deduplicated: bool,
    private_projection_wire: Option<r7::PrivateProjectionPayloadWireV1>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl UserStimulusDecisionV1 {
    pub(crate) fn discard_private_projection_v1(
        mut self,
    ) -> Result<Option<r7::PrivateProjectionTransferReceiptV1>, RuntimeError> {
        let Some(mut wire) = self.private_projection_wire.take() else {
            return Ok(None);
        };
        let transfer = wire
            .begin_transfer_once_v1()
            .map_err(|_| RuntimeError::PrivateProjectionUnavailable)?;
        Ok(Some(r7::discard_private_projection_transfer_v1(transfer)))
    }
}

#[derive(Clone, Debug)]
pub struct InspectReport {
    pub bound: bool,
    pub bot_token: Id128,
    pub persona_token: Id128,
    pub seed_code: String,
    pub seed_code_short: String,
    pub incarnation_id: String,
    pub revision: u64,
    pub initial_snapshot_digest: Digest,
    pub last_chain_digest: Option<Digest>,
    pub journal_count: u64,
    pub observatory_genesis_unavailable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum R7HydrationOutcomeV1 {
    /// No R7 state was changed; callers continue on the committed G0 lane.
    G0Only,
    /// The public chain was accepted and durably CAS-recorded.  A later
    /// producer may construct typed R7 state from this validated boundary.
    Validated { sequence: u64 },
}

#[cfg_attr(not(test), allow(dead_code))]
struct HotBrain {
    bot_token: Id128,
    persona_token: Id128,
    /// Legacy G0 events retain the root persona revision lane for compatibility.
    legacy_persona_scope: Digest,
    legacy_revision: u64,
    /// The R7 semantic transition is deliberately separate from the G0 no-op
    /// lane, while its event bytes and event digest stay root canonical.
    persona_scope: Digest,
    identity: ae_genesis::GenesisIdentity,
    formula_digest: Digest,
    field: NeuralField,
    graph: SparseGraph,
    initial_snapshot_digest: Digest,
    semantic_revision: u64,
}

pub struct AstrRuntime {
    store: Store,
    hot: Option<HotBrain>,
}

fn fixed_zero_vector() -> InvariantResiduals {
    InvariantResiduals::default()
}

fn r7_semantic_persona_scope(bot_token: &Id128, persona_token: &Id128) -> Digest {
    let root_persona_scope = wire::persona_scope_digest(bot_token, persona_token, None);
    wire::domain_hash(R7_SEMANTIC_PERSONA_SCOPE_DOMAIN_V1, &[&root_persona_scope])
}

struct HotStateCursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> HotStateCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], RuntimeError> {
        let end = self
            .position
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(RuntimeError::InvalidNeuralState)?;
        let value = &self.bytes[self.position..end];
        self.position = end;
        Ok(value)
    }

    fn read_u16(&mut self) -> Result<u16, RuntimeError> {
        let mut bytes = [0; 2];
        bytes.copy_from_slice(self.take(2)?);
        Ok(u16::from_le_bytes(bytes))
    }

    fn read_u32(&mut self) -> Result<u32, RuntimeError> {
        let mut bytes = [0; 4];
        bytes.copy_from_slice(self.take(4)?);
        Ok(u32::from_le_bytes(bytes))
    }

    fn read_fixed(&mut self) -> Result<Fixed, RuntimeError> {
        let mut bytes = [0; 8];
        bytes.copy_from_slice(self.take(8)?);
        Ok(Fixed::decode(bytes))
    }

    fn ensure_available(&self, count: usize) -> Result<(), RuntimeError> {
        self.position
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(RuntimeError::InvalidNeuralState)
            .map(|_| ())
    }

    fn is_at_eof(&self) -> bool {
        self.position == self.bytes.len()
    }
}

fn encode_hot_state_v1(
    formula_digest: &Digest,
    field: &NeuralField,
    graph: &SparseGraph,
) -> Vec<u8> {
    let field_bytes = [
        &field.potential,
        &field.excitation,
        &field.inhibition,
        &field.adaptation,
        &field.precision,
        &field.prediction_error,
        &field.eligibility,
        &field.metabolic_reserve,
    ]
    .iter()
    .try_fold(0usize, |total, values| {
        values
            .len()
            .checked_mul(8)
            .and_then(|value_bytes| value_bytes.checked_add(4))
            .and_then(|section_bytes| total.checked_add(section_bytes))
    })
    .unwrap_or(0);
    let graph_bytes = graph
        .row_offsets
        .len()
        .checked_mul(4)
        .and_then(|row_bytes| row_bytes.checked_add(4))
        .and_then(|row_section| {
            graph
                .edges
                .len()
                .checked_mul(SYNAPSE_WIRE_BYTES)
                .and_then(|edge_bytes| edge_bytes.checked_add(4))
                .and_then(|edge_section| row_section.checked_add(edge_section))
        })
        .unwrap_or(0);
    let capacity = CANONICAL_HOT_STATE_MAGIC_V1
        .len()
        .checked_add(2)
        .and_then(|header| header.checked_add(formula_digest.len()))
        .and_then(|header| header.checked_add(field_bytes))
        .and_then(|header| header.checked_add(graph_bytes))
        .unwrap_or(0);
    let mut body = Vec::with_capacity(capacity);
    body.extend_from_slice(&CANONICAL_HOT_STATE_MAGIC_V1);
    body.extend_from_slice(&CANONICAL_HOT_STATE_SCHEMA_V1.to_le_bytes());
    body.extend_from_slice(formula_digest);
    for values in [
        &field.potential,
        &field.excitation,
        &field.inhibition,
        &field.adaptation,
        &field.precision,
        &field.prediction_error,
        &field.eligibility,
        &field.metabolic_reserve,
    ] {
        body.extend_from_slice(&(values.len() as u32).to_le_bytes());
        for value in values {
            body.extend_from_slice(&value.encode());
        }
    }
    body.extend_from_slice(&(graph.row_offsets.len() as u32).to_le_bytes());
    for offset in &graph.row_offsets {
        body.extend_from_slice(&offset.to_le_bytes());
    }
    body.extend_from_slice(&(graph.edges.len() as u32).to_le_bytes());
    for edge in &graph.edges {
        body.extend_from_slice(&edge.target.to_le_bytes());
        body.extend_from_slice(&edge.weight.to_le_bytes());
        body.extend_from_slice(&edge.eligibility.to_le_bytes());
        body.extend_from_slice(&edge.stability.to_le_bytes());
        body.extend_from_slice(&edge.last_used_epoch.to_le_bytes());
        body.push(edge.operator_id);
        body.push(edge.delay_class);
        body.extend_from_slice(&edge.flags.to_le_bytes());
    }
    body
}

fn decode_hot_state_v1(
    bytes: &[u8],
    expected_formula_digest: &Digest,
    expected_state_digest: &Digest,
    expected_graph_digest: &Digest,
) -> Result<(NeuralField, SparseGraph), RuntimeError> {
    let mut cursor = HotStateCursor::new(bytes);
    if cursor.take(CANONICAL_HOT_STATE_MAGIC_V1.len())? != CANONICAL_HOT_STATE_MAGIC_V1 {
        return Err(RuntimeError::InvalidNeuralState);
    }
    if cursor.read_u16()? != CANONICAL_HOT_STATE_SCHEMA_V1 {
        return Err(RuntimeError::InvalidNeuralState);
    }
    let mut formula_digest = [0; 32];
    formula_digest.copy_from_slice(cursor.take(32)?);
    if &formula_digest != expected_formula_digest {
        return Err(RuntimeError::InvalidNeuralState);
    }

    let mut vectors = Vec::with_capacity(CANONICAL_HOT_STATE_VECTOR_COUNT);
    for _ in 0..CANONICAL_HOT_STATE_VECTOR_COUNT {
        let count =
            usize::try_from(cursor.read_u32()?).map_err(|_| RuntimeError::InvalidNeuralState)?;
        if count != NEURON_SLOTS {
            return Err(RuntimeError::InvalidNeuralState);
        }
        cursor.ensure_available(
            count
                .checked_mul(8)
                .ok_or(RuntimeError::InvalidNeuralState)?,
        )?;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(cursor.read_fixed()?);
        }
        vectors.push(values);
    }

    let row_count =
        usize::try_from(cursor.read_u32()?).map_err(|_| RuntimeError::InvalidNeuralState)?;
    if row_count != NEURON_SLOTS + 1 {
        return Err(RuntimeError::InvalidNeuralState);
    }
    cursor.ensure_available(
        row_count
            .checked_mul(4)
            .ok_or(RuntimeError::InvalidNeuralState)?,
    )?;
    let mut row_offsets = Vec::with_capacity(row_count);
    for _ in 0..row_count {
        row_offsets.push(cursor.read_u32()?);
    }

    let edge_count =
        usize::try_from(cursor.read_u32()?).map_err(|_| RuntimeError::InvalidNeuralState)?;
    if edge_count > EDGE_CAPACITY {
        return Err(RuntimeError::InvalidNeuralState);
    }
    cursor.ensure_available(
        edge_count
            .checked_mul(SYNAPSE_WIRE_BYTES)
            .ok_or(RuntimeError::InvalidNeuralState)?,
    )?;
    let mut edges = Vec::with_capacity(edge_count);
    for _ in 0..edge_count {
        let target = cursor.read_u32()?;
        if usize::try_from(target).map_err(|_| RuntimeError::InvalidNeuralState)? >= NEURON_SLOTS {
            return Err(RuntimeError::InvalidNeuralState);
        }
        let mut i16_bytes = [0; 2];
        i16_bytes.copy_from_slice(cursor.take(2)?);
        let weight = i16::from_le_bytes(i16_bytes);
        i16_bytes.copy_from_slice(cursor.take(2)?);
        let eligibility = i16::from_le_bytes(i16_bytes);
        let mut u16_bytes = [0; 2];
        u16_bytes.copy_from_slice(cursor.take(2)?);
        let stability = u16::from_le_bytes(u16_bytes);
        u16_bytes.copy_from_slice(cursor.take(2)?);
        let last_used_epoch = u16::from_le_bytes(u16_bytes);
        let operator_id = cursor.take(1)?[0];
        let delay_class = cursor.take(1)?[0];
        u16_bytes.copy_from_slice(cursor.take(2)?);
        let flags = u16::from_le_bytes(u16_bytes);
        edges.push(Synapse {
            target,
            weight,
            eligibility,
            stability,
            last_used_epoch,
            operator_id,
            delay_class,
            flags,
        });
    }
    if !cursor.is_at_eof()
        || row_offsets.first().copied() != Some(0)
        || !row_offsets.windows(2).all(|pair| pair[0] <= pair[1])
        || row_offsets
            .iter()
            .any(|offset| usize::try_from(*offset).map_or(true, |value| value > edge_count))
    {
        return Err(RuntimeError::InvalidNeuralState);
    }

    let mut vectors = vectors.into_iter();
    let field = NeuralField {
        potential: vectors.next().ok_or(RuntimeError::InvalidNeuralState)?,
        excitation: vectors.next().ok_or(RuntimeError::InvalidNeuralState)?,
        inhibition: vectors.next().ok_or(RuntimeError::InvalidNeuralState)?,
        adaptation: vectors.next().ok_or(RuntimeError::InvalidNeuralState)?,
        precision: vectors.next().ok_or(RuntimeError::InvalidNeuralState)?,
        prediction_error: vectors.next().ok_or(RuntimeError::InvalidNeuralState)?,
        eligibility: vectors.next().ok_or(RuntimeError::InvalidNeuralState)?,
        metabolic_reserve: vectors.next().ok_or(RuntimeError::InvalidNeuralState)?,
    };
    let graph = SparseGraph { row_offsets, edges };
    if !field.validate()
        || !graph.validate()
        || state_digest(&field, expected_formula_digest) != *expected_state_digest
        || graph_digest(&graph) != *expected_graph_digest
    {
        return Err(RuntimeError::InvalidNeuralState);
    }
    Ok((field, graph))
}

#[cfg_attr(not(test), allow(dead_code))]
fn canonical_event_from_r7(
    event: &ae_contracts::r7::CanonicalEvent,
) -> Result<CanonicalEvent, r7::RuntimeError> {
    let ae_contracts::r7::CanonicalEvent::UserStimulus(stimulus) = event else {
        return Err(r7::RuntimeError::UnsupportedEvent);
    };
    Ok(CanonicalEvent::UserStimulus(ae_contracts::UserStimulus {
        event_id: stimulus.event_id,
        scope: ScopeRef {
            bot_token: stimulus.scope.bot_token,
            persona_token: stimulus.scope.persona_token,
            relation_token: stimulus.scope.relation_token,
            session_token: stimulus.scope.session_token,
        },
        causal: ae_contracts::CausalRef {
            turn_id: stimulus.causal.turn_id,
            action_id: stimulus.causal.action_id,
            delivery_id: stimulus.causal.delivery_id,
            claim_id: stimulus.causal.claim_id,
            base_revision: stimulus.causal.base_revision,
        },
        observed_at_ms: stimulus.observed_at_ms,
        evidence: ae_contracts::SemanticEstimate {
            schema_version: stimulus.evidence.schema_version,
            dimensions: ae_contracts::EvidenceVector {
                positive: stimulus.evidence.dimensions.positive,
                affiliation: stimulus.evidence.dimensions.affiliation,
                harm: stimulus.evidence.dimensions.harm,
                boundary: stimulus.evidence.dimensions.boundary,
                repair: stimulus.evidence.dimensions.repair,
                repetition: stimulus.evidence.dimensions.repetition,
                new_information: stimulus.evidence.dimensions.new_information,
                constraint_instability: stimulus.evidence.dimensions.constraint_instability,
                epistemic_conflict: stimulus.evidence.dimensions.epistemic_conflict,
                self_responsibility: stimulus.evidence.dimensions.self_responsibility,
                other_responsibility: stimulus.evidence.dimensions.other_responsibility,
                hostility: stimulus.evidence.dimensions.hostility,
                publicness: stimulus.evidence.dimensions.publicness,
                engagement: stimulus.evidence.dimensions.engagement,
                rejection: stimulus.evidence.dimensions.rejection,
            },
            estimator_confidence: stimulus.evidence.estimator_confidence,
            estimator_digest: stimulus.evidence.estimator_digest,
        },
    }))
}

fn r7_scope_from_root(scope: &ScopeRef) -> ae_contracts::r7::ScopeRef {
    ae_contracts::r7::ScopeRef {
        bot_token: scope.bot_token,
        persona_token: scope.persona_token,
        relation_token: scope.relation_token,
        session_token: scope.session_token,
    }
}

fn r7_perception_event(
    scope: &ScopeRef,
    proposal: &PerceptionProposalV1,
    estimator_digest: Digest,
) -> ae_contracts::r7::CanonicalEvent {
    ae_contracts::r7::CanonicalEvent::UserStimulus(ae_contracts::r7::UserStimulus {
        event_id: proposal.event_id,
        scope: r7_scope_from_root(scope),
        causal: ae_contracts::r7::CausalRef {
            turn_id: proposal.turn_id,
            action_id: None,
            delivery_id: None,
            claim_id: None,
            base_revision: proposal.base_revision,
        },
        observed_at_ms: proposal.observed_at_ms,
        evidence: ae_contracts::r7::SemanticEstimate {
            schema_version: proposal.schema_version,
            dimensions: ae_contracts::r7::EvidenceVector {
                positive: proposal.dimensions.positive,
                affiliation: proposal.dimensions.affiliation,
                harm: proposal.dimensions.harm,
                boundary: proposal.dimensions.boundary,
                repair: proposal.dimensions.repair,
                repetition: proposal.dimensions.repetition,
                new_information: proposal.dimensions.new_information,
                constraint_instability: proposal.dimensions.constraint_instability,
                epistemic_conflict: proposal.dimensions.epistemic_conflict,
                self_responsibility: proposal.dimensions.self_responsibility,
                other_responsibility: proposal.dimensions.other_responsibility,
                hostility: proposal.dimensions.hostility,
                publicness: proposal.dimensions.publicness,
                engagement: proposal.dimensions.engagement,
                rejection: proposal.dimensions.rejection,
            },
            estimator_confidence: proposal.estimator_confidence,
            estimator_digest,
        },
    })
}

fn validate_perception_scope(scope: &ScopeRef) -> Result<(), RuntimeError> {
    let nonzero = |value: &[u8]| value.iter().any(|byte| *byte != 0);
    if !nonzero(&scope.bot_token)
        || !nonzero(&scope.persona_token)
        || !nonzero(&scope.session_token)
        || scope
            .relation_token
            .as_ref()
            .is_some_and(|relation| !nonzero(relation))
    {
        return Err(RuntimeError::InvalidPerceptionScope);
    }
    Ok(())
}

fn map_perception_proposal_error(_error: PerceptionProposalErrorV1) -> RuntimeError {
    RuntimeError::InvalidPerceptionProposal
}

fn map_semantic_prepare_error(error: r7::RuntimeError) -> RuntimeError {
    match error {
        r7::RuntimeError::InvalidNeuralField
        | r7::RuntimeError::InvalidSparseGraph
        | r7::RuntimeError::NativeFormulaDigestMismatch => RuntimeError::InvalidNeuralState,
        r7::RuntimeError::InvalidUserStimulus | r7::RuntimeError::InvalidSemanticEstimate => {
            RuntimeError::InvalidPerceptionProposal
        }
        r7::RuntimeError::NativeStateUnchanged => RuntimeError::SemanticStateUnchanged,
        r7::RuntimeError::RevisionOverflow => RuntimeError::SemanticRevisionOverflow,
        r7::RuntimeError::UnsupportedEvent => RuntimeError::UnsupportedEvent("user_stimulus"),
        _ => RuntimeError::InvalidPerceptionProposal,
    }
}

impl AstrRuntime {
    pub fn open(path: &Path) -> Result<Self, RuntimeError> {
        Ok(Self {
            store: Store::open(path)?,
            hot: None,
        })
    }

    /// Explicit post-G0, pre-R7 public-material boundary.  Missing, malformed,
    /// stale, revoked, conflicting, or persistence-failed material collapses
    /// to deterministic G0-only behavior; `hot` and committed G0 rows are not
    /// touched on those paths.
    pub fn hydrate_r7_public_policy(
        &mut self,
        key: &R7PolicyBindingKeyV1,
        bundle: Option<&R7PublicPolicyBundleV1>,
    ) -> Result<R7HydrationOutcomeV1, RuntimeError> {
        self.hydrate_r7_public_policy_with_context(key, bundle, None)
    }

    pub fn hydrate_r7_public_policy_with_context(
        &mut self,
        key: &R7PolicyBindingKeyV1,
        bundle: Option<&R7PublicPolicyBundleV1>,
        context: Option<&R7PolicyValidationContextV1>,
    ) -> Result<R7HydrationOutcomeV1, RuntimeError> {
        let Some(bundle) = bundle else {
            return Ok(R7HydrationOutcomeV1::G0Only);
        };
        let Some(context) = context else {
            return Ok(R7HydrationOutcomeV1::G0Only);
        };
        let committed = match self
            .store
            .lookup_bound_genesis(&key.bot_token, &key.persona_token)
        {
            Ok(Some(committed)) => committed,
            Ok(None) | Err(_) => return Ok(R7HydrationOutcomeV1::G0Only),
        };
        if committed.receipt.incarnation_id != key.committed_g0_incarnation_id
            || context.committed_g0_incarnation_id != key.committed_g0_incarnation_id
            || context.committed_g0_manifest_digest != committed.receipt.manifest_digest
            || context.committed_g0_seed_code_digest != committed.receipt.seed_code_digest
            || context.committed_g0_persona_source_digest != committed.receipt.persona_source_digest
            || context.committed_g0_genesis_receipt_digest
                != wire::genesis_receipt_digest(&committed.receipt)
            || bundle.policy.g0_manifest_digest != committed.receipt.manifest_digest
            || bundle.policy.g0_seed_code_digest != committed.receipt.seed_code_digest
            || bundle.policy.g0_persona_source_digest != committed.receipt.persona_source_digest
            || bundle.policy.g0_genesis_receipt_digest
                != wire::genesis_receipt_digest(&committed.receipt)
        {
            return Ok(R7HydrationOutcomeV1::G0Only);
        }
        match self
            .store
            .compare_and_commit_r7_policy_with_context(key, bundle, context)
        {
            Ok(R7PolicyCommitOutcomeV1::Inserted)
            | Ok(R7PolicyCommitOutcomeV1::Replay)
            | Ok(R7PolicyCommitOutcomeV1::Successor) => Ok(R7HydrationOutcomeV1::Validated {
                sequence: bundle.policy.incarnation_sequence,
            }),
            Err(_) => Ok(R7HydrationOutcomeV1::G0Only),
        }
    }

    /// Rust-only additive R7 ingress.  It takes the authority's closed typed
    /// source and returns only its opaque, one-shot decision capability.
    /// Python keeps its unchanged G0 compatibility surface and has no route
    /// to this method, a raw wire, or a source-state mutation API.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn apply_user_stimulus_with_private_projection_wire_v1(
        &mut self,
        event: &ae_contracts::r7::CanonicalEvent,
        input: &r7::R7PreOutputProjectionInputV1,
    ) -> Result<UserStimulusDecisionV1, RuntimeError> {
        let root_event = canonical_event_from_r7(event)?;
        let CanonicalEvent::UserStimulus(stimulus) = &root_event else {
            unreachable!("the R7 conversion admits only user stimuli");
        };
        let scope = stimulus.scope.clone();
        let (
            hot_bot_token,
            hot_persona_token,
            semantic_persona_scope,
            semantic_revision,
            formula_digest,
            manifest_digest,
            initial_snapshot_digest,
            field,
            graph,
        ) = {
            let hot = self.hot_for(&scope)?;
            (
                hot.bot_token,
                hot.persona_token,
                hot.persona_scope,
                hot.semantic_revision,
                hot.formula_digest,
                hot.identity.manifest_digest,
                hot.initial_snapshot_digest,
                hot.field.clone(),
                hot.graph.clone(),
            )
        };
        if scope.bot_token != hot_bot_token || scope.persona_token != hot_persona_token {
            return Err(RuntimeError::GenesisManifestMismatch);
        }

        let event_bytes = wire::encode_event(&root_event);
        let event_digest = wire::event_digest(&root_event);
        let contract =
            noop_action_contract(&manifest_digest, &event_digest, stimulus.causal.turn_id);
        if let Some(row) = self
            .store
            .lookup_event(&semantic_persona_scope, &event_digest)?
        {
            let receipt = row
                .decode_receipt()
                .map_err(|error| RuntimeError::Store(StoreError::Sqlite(error.to_string())))?;
            return Ok(UserStimulusDecisionV1 {
                receipt,
                revision: row.revision,
                deduplicated: true,
                private_projection_wire: None,
            });
        }
        if stimulus.causal.base_revision != semantic_revision {
            return Err(RuntimeError::StaleCausalBase {
                expected: semantic_revision,
                actual: stimulus.causal.base_revision,
            });
        }

        let prepared = r7::prepare_production_user_stimulus_transition_v1(
            event,
            &field,
            &graph,
            semantic_revision,
        )?;
        let next_revision = semantic_revision
            .checked_add(1)
            .ok_or(r7::RuntimeError::RevisionOverflow)?;
        let state_before = state_digest(&field, &formula_digest);
        let state_after = state_digest(&prepared.next_field, &formula_digest);
        if state_before == state_after {
            return Err(r7::RuntimeError::NativeStateUnchanged.into());
        }
        let graph_after = graph_digest(&graph);
        let authority_digest = authority_projection_digest(&root_event);
        let projection_scope = wire::scope_digest(&scope);
        let wire = r7::compile_and_validate_production_private_projection_wire_v1(
            event,
            next_revision,
            state_after,
            projection_scope,
            event_digest,
            authority_digest,
            input,
        )?;
        let state_bytes = encode_hot_state_v1(&formula_digest, &prepared.next_field, &graph);
        let _ = decode_hot_state_v1(&state_bytes, &formula_digest, &state_after, &graph_after)?;
        let receipt = TransitionReceipt {
            schema_version: 1,
            formula_digest,
            scope_digest: semantic_persona_scope,
            event_digest,
            authority_digest,
            base_revision: semantic_revision,
            next_revision,
            state_before,
            state_after,
            graph_after,
            action_contract: Some(wire::action_contract_digest(&contract)),
            active_nodes: prepared.active_nodes,
            active_edges: graph.edges.len() as u32,
            residuals: fixed_zero_vector(),
            status: CommitStatus::Committed,
        };
        let chain_seed = self
            .store
            .last_chain_digest(&semantic_persona_scope)?
            .unwrap_or(initial_snapshot_digest);
        let commit = StatefulCommit {
            journal: CommitEnvelope {
                event_kind: wire::event_kind_name(&root_event).to_owned(),
                event_bytes,
                receipt: receipt.clone(),
                chain_seed,
                delta_bytes: vec![],
            },
            state_bytes,
        };

        match self.store.commit_stateful_journal(&commit) {
            Ok((revision, _row)) => {
                if let Some(hot) = self.hot.as_mut() {
                    hot.field = prepared.next_field;
                    hot.graph = graph;
                    hot.semantic_revision = revision;
                }
                Ok(UserStimulusDecisionV1 {
                    receipt,
                    revision,
                    deduplicated: false,
                    private_projection_wire: Some(wire),
                })
            }
            Err(StoreError::DuplicateEvent(revision)) => {
                let row = self
                    .store
                    .lookup_event(&semantic_persona_scope, &event_digest)?
                    .ok_or(RuntimeError::RetryWait)?;
                let receipt = row
                    .decode_receipt()
                    .map_err(|error| RuntimeError::Store(StoreError::Sqlite(error.to_string())))?;
                Ok(UserStimulusDecisionV1 {
                    receipt,
                    revision,
                    deduplicated: true,
                    private_projection_wire: None,
                })
            }
            Err(other) => Err(RuntimeError::Store(other)),
        }
    }

    // ------------------------------------------------------------- genesis

    /// Claim (or join) the durable Genesis lease, project the Manifest, build
    /// the deterministic initial state and atomically commit the birth.
    /// Concurrent callers converge on one committed receipt; a failure never
    /// creates a default brain.
    pub fn ensure_genesis(
        &mut self,
        request: &PersonaGenesisRequest,
    ) -> Result<GenesisReceipt, RuntimeError> {
        let scope_key = ae_genesis::genesis_scope_key(
            &request.source.scope.bot_token,
            &request.source.scope.persona_token,
            &request.source.source_digest,
            &request.formula_digest,
        );

        match self
            .store
            .claim_lease(&scope_key, Some(request.incarnation_nonce))?
        {
            ClaimOutcome::Committed => {
                let committed = self
                    .store
                    .lookup_committed_genesis(&scope_key)?
                    .ok_or(RuntimeError::RetryWait)?;
                self.bind_hot(
                    committed.source.scope.bot_token,
                    committed.source.scope.persona_token,
                )?;
                Ok(committed.receipt)
            }
            ClaimOutcome::InFlight => Err(RuntimeError::RetryWait),
            ClaimOutcome::Claimed { lease_epoch, nonce } => {
                // The persisted birth nonce wins: retries replay the original
                // birth transaction instead of starting a second one.
                let mut effective = request.clone();
                effective.incarnation_nonce = nonce;

                let identity =
                    ae_genesis::derive_identity(&effective, &ae_genesis::GenesisPrior::default())?;
                let (field, graph) = initial_state_from_manifest(
                    &identity.manifest,
                    &effective.formula_digest,
                    &identity.development_seed_digest,
                );
                if !field.validate() || !graph.validate() {
                    return Err(RuntimeError::InvalidNeuralState);
                }
                let initial_snapshot_digest = state_digest(&field, &effective.formula_digest);
                let graph_digest = graph_digest(&graph);

                let receipt = GenesisReceipt {
                    schema_version: 1,
                    seed_code_digest: identity.seed_code_digest,
                    manifest_digest: identity.manifest_digest,
                    incarnation_id: identity.incarnation_id,
                    formula_digest: effective.formula_digest,
                    persona_source_digest: effective.source.source_digest,
                    compiler_protocol_digest: effective.proposal.compiler_protocol_digest,
                    compiler_model_digest: effective.proposal.compiler_model_digest,
                    development_seed_digest: identity.development_seed_digest,
                    initial_snapshot_digest,
                    graph_digest,
                    equilibrium_residual: ae_fixed::Fixed::ZERO,
                    energy_residual: ae_fixed::Fixed::ZERO,
                    capacity_residual: ae_fixed::Fixed::ZERO,
                    sample_fit_residual: ae_fixed::Fixed::ZERO,
                    status: GenesisStatus::Committed,
                };

                let commit = GenesisCommit {
                    scope_key,
                    lease_epoch,
                    nonce_digest: nonce,
                    manifest: identity.manifest.clone(),
                    manifest_body: wire::encode_manifest_body(&identity.manifest),
                    seed_code_digest: identity.seed_code_digest,
                    incarnation_id: identity.incarnation_id,
                    formula_digest: effective.formula_digest,
                    source: effective.source.clone(),
                    compiler_protocol_digest: effective.proposal.compiler_protocol_digest,
                    compiler_model_digest: effective.proposal.compiler_model_digest,
                    compiled_at_ms: effective.observed_at_ms,
                    receipt: receipt.clone(),
                    initial_snapshot_digest,
                    state_bytes: encode_hot_state_v1(&effective.formula_digest, &field, &graph),
                    graph_digest,
                };

                match self.store.commit_genesis(&commit) {
                    Ok(()) => {
                        self.hot = Some(HotBrain {
                            bot_token: effective.source.scope.bot_token,
                            persona_token: effective.source.scope.persona_token,
                            legacy_persona_scope: wire::persona_scope_digest(
                                &effective.source.scope.bot_token,
                                &effective.source.scope.persona_token,
                                None,
                            ),
                            legacy_revision: 0,
                            persona_scope: r7_semantic_persona_scope(
                                &effective.source.scope.bot_token,
                                &effective.source.scope.persona_token,
                            ),
                            identity,
                            formula_digest: effective.formula_digest,
                            field,
                            graph,
                            initial_snapshot_digest,
                            semantic_revision: 0,
                        });
                        Ok(receipt)
                    }
                    Err(StoreError::LeaseConflict) => {
                        // A concurrent writer closed the lease first: join it.
                        let committed = self
                            .store
                            .lookup_committed_genesis(&scope_key)?
                            .ok_or(RuntimeError::RetryWait)?;
                        self.bind_hot(
                            committed.source.scope.bot_token,
                            committed.source.scope.persona_token,
                        )?;
                        Ok(committed.receipt)
                    }
                    Err(other) => Err(RuntimeError::Store(other)),
                }
            }
        }
    }

    fn bind_hot(&mut self, bot_token: Id128, persona_token: Id128) -> Result<(), RuntimeError> {
        let committed = self
            .store
            .lookup_bound_genesis(&bot_token, &persona_token)?
            .ok_or(RuntimeError::PersonaGenesisRequired)?;
        let identity = ae_genesis::GenesisIdentity {
            manifest: committed.manifest,
            manifest_digest: committed.receipt.manifest_digest,
            seed_code_digest: committed.receipt.seed_code_digest,
            incarnation_id: committed.receipt.incarnation_id,
            development_seed_digest: committed.receipt.development_seed_digest,
        };
        let legacy_persona_scope = wire::persona_scope_digest(&bot_token, &persona_token, None);
        let persona_scope = r7_semantic_persona_scope(&bot_token, &persona_token);
        let legacy_revision = self.store.current_revision(&legacy_persona_scope)?;
        let semantic_revision = self.store.current_revision(&persona_scope)?;
        self.verify_durable_history_v1(
            committed.receipt.formula_digest,
            committed.receipt.initial_snapshot_digest,
            committed.receipt.graph_digest,
            persona_scope,
            true,
        )?;
        let snapshot = if semantic_revision == 0 {
            self.store
                .read_snapshot(&legacy_persona_scope, 0)?
                .ok_or(RuntimeError::InvalidNeuralState)?
        } else {
            self.store
                .read_latest_snapshot(&persona_scope, semantic_revision)?
                .ok_or(RuntimeError::InvalidNeuralState)?
        };
        if snapshot.revision > semantic_revision {
            return Err(RuntimeError::InvalidNeuralState);
        }
        let rows = self.store.read_journal(&persona_scope)?;
        let (expected_state_digest, expected_graph_digest) = if snapshot.revision == 0 {
            (
                committed.receipt.initial_snapshot_digest,
                committed.receipt.graph_digest,
            )
        } else {
            let row = rows
                .iter()
                .find(|row| row.revision == snapshot.revision)
                .ok_or(RuntimeError::InvalidNeuralState)?;
            let receipt = row
                .decode_receipt()
                .map_err(|error| RuntimeError::Store(StoreError::Sqlite(error.to_string())))?;
            if row.revision != snapshot.revision
                || row.base_revision != receipt.base_revision
                || receipt.formula_digest != committed.receipt.formula_digest
                || receipt.scope_digest != persona_scope
                || receipt.next_revision != snapshot.revision
            {
                return Err(RuntimeError::InvalidNeuralState);
            }
            (receipt.state_after, receipt.graph_after)
        };
        if snapshot.state_digest != expected_state_digest {
            return Err(RuntimeError::InvalidNeuralState);
        }
        let mut expected_state_before = expected_state_digest;
        for row in rows.iter().filter(|row| row.revision > snapshot.revision) {
            let receipt = row
                .decode_receipt()
                .map_err(|error| RuntimeError::Store(StoreError::Sqlite(error.to_string())))?;
            if row.revision != receipt.next_revision
                || row.base_revision != receipt.base_revision
                || receipt.scope_digest != persona_scope
                || receipt.formula_digest != committed.receipt.formula_digest
                || receipt.state_before != expected_state_before
            {
                return Err(RuntimeError::InvalidNeuralState);
            }
            expected_state_before = receipt.state_after;
            let snapshot = self
                .store
                .read_snapshot(&persona_scope, row.revision)?
                .ok_or(RuntimeError::InvalidNeuralState)?;
            if snapshot.state_digest != receipt.state_after {
                return Err(RuntimeError::InvalidNeuralState);
            }
            let _ = decode_hot_state_v1(
                &snapshot.state_bytes,
                &committed.receipt.formula_digest,
                &receipt.state_after,
                &receipt.graph_after,
            )?;
        }
        let (field, graph) = decode_hot_state_v1(
            &snapshot.state_bytes,
            &committed.receipt.formula_digest,
            &expected_state_digest,
            &expected_graph_digest,
        )?;
        self.hot = Some(HotBrain {
            bot_token,
            persona_token,
            legacy_persona_scope,
            legacy_revision,
            persona_scope,
            identity,
            formula_digest: committed.receipt.formula_digest,
            field,
            graph,
            initial_snapshot_digest: committed.receipt.initial_snapshot_digest,
            semantic_revision,
        });
        Ok(())
    }

    fn hot_for(&mut self, scope: &ScopeRef) -> Result<&mut HotBrain, RuntimeError> {
        let matches = self
            .hot
            .as_ref()
            .map(|hot| hot.bot_token == scope.bot_token && hot.persona_token == scope.persona_token)
            .unwrap_or(false);
        if !matches {
            self.bind_hot(scope.bot_token, scope.persona_token)?;
        } else {
            let (
                legacy_persona_scope,
                persona_scope,
                hot_legacy_revision,
                hot_semantic_revision,
                formula_digest,
                field,
                graph,
            ) = {
                let hot = self
                    .hot
                    .as_ref()
                    .ok_or(RuntimeError::PersonaGenesisRequired)?;
                (
                    hot.legacy_persona_scope,
                    hot.persona_scope,
                    hot.legacy_revision,
                    hot.semantic_revision,
                    hot.formula_digest,
                    hot.field.clone(),
                    hot.graph.clone(),
                )
            };
            let store_legacy_revision = self.store.current_revision(&legacy_persona_scope)?;
            let store_semantic_revision = self.store.current_revision(&persona_scope)?;
            let mut needs_hydration = hot_legacy_revision != store_legacy_revision
                || hot_semantic_revision != store_semantic_revision;
            if !needs_hydration {
                let snapshot = if store_semantic_revision == 0 {
                    self.store.read_snapshot(&legacy_persona_scope, 0)?
                } else {
                    self.store
                        .read_latest_snapshot(&persona_scope, store_semantic_revision)?
                };
                needs_hydration = match snapshot {
                    Some(snapshot) => {
                        snapshot.revision != store_semantic_revision
                            || snapshot.state_digest != state_digest(&field, &formula_digest)
                            || decode_hot_state_v1(
                                &snapshot.state_bytes,
                                &formula_digest,
                                &snapshot.state_digest,
                                &graph_digest(&graph),
                            )
                            .is_err()
                    }
                    None => true,
                };
            }
            if needs_hydration {
                self.bind_hot(scope.bot_token, scope.persona_token)?;
            }
        }
        self.hot
            .as_mut()
            .ok_or(RuntimeError::PersonaGenesisRequired)
    }

    fn durable_g0_metadata_v1(
        &self,
        legacy_scope: &Digest,
        legacy_revision: u64,
        formula_digest: &Digest,
        initial_snapshot_digest: &Digest,
        genesis_graph_digest: &Digest,
    ) -> Result<(Digest, Digest, u32, u32), RuntimeError> {
        let snapshot = self
            .store
            .read_latest_snapshot(legacy_scope, legacy_revision)?
            .ok_or(RuntimeError::InvalidNeuralState)?;
        if snapshot.revision > legacy_revision {
            return Err(RuntimeError::InvalidNeuralState);
        }

        let rows = self.store.read_journal(legacy_scope)?;
        let mut expected_revision = 0_u64;
        let mut expected_state = *initial_snapshot_digest;
        let mut expected_graph = *genesis_graph_digest;
        for row in rows.iter().filter(|row| row.revision <= legacy_revision) {
            let next_revision = expected_revision
                .checked_add(1)
                .ok_or(RuntimeError::InvalidNeuralState)?;
            let receipt = row
                .decode_receipt()
                .map_err(|error| RuntimeError::Store(StoreError::Sqlite(error.to_string())))?;
            if row.revision != next_revision
                || receipt.next_revision != row.revision
                || row.base_revision != receipt.base_revision
                || receipt.base_revision != expected_revision
                || row.scope_digest != *legacy_scope
                || receipt.scope_digest != *legacy_scope
                || receipt.formula_digest != *formula_digest
                || receipt.state_before != expected_state
            {
                return Err(RuntimeError::InvalidNeuralState);
            }
            expected_revision = row.revision;
            expected_state = receipt.state_after;
            expected_graph = receipt.graph_after;
        }
        if expected_revision != legacy_revision || snapshot.state_digest != expected_state {
            return Err(RuntimeError::InvalidNeuralState);
        }

        // G0 snapshots are rooted at Genesis.  The latest durable G0 receipt
        // supplies the graph digest to verify when the cursor is non-zero;
        // this keeps semantic snapshots and hot state out of the metadata path.
        let (field, graph) = decode_hot_state_v1(
            &snapshot.state_bytes,
            formula_digest,
            &snapshot.state_digest,
            &expected_graph,
        )?;
        let decoded_graph_digest = graph_digest(&graph);
        if decoded_graph_digest != expected_graph {
            return Err(RuntimeError::InvalidNeuralState);
        }
        Ok((
            snapshot.state_digest,
            decoded_graph_digest,
            field.active_node_count(),
            graph.edges.len() as u32,
        ))
    }

    // --------------------------------------------------------------- events

    /// Apply one canonical event through the G0 no-op lane and commit it.
    /// The same committed genesis + the same stimulus always produce the same
    /// contract and receipt digest, in 1C1G and 2C2G alike.
    pub fn apply_event(
        &mut self,
        scope: &ScopeRef,
        event: &CanonicalEvent,
    ) -> Result<ApplyDecision, RuntimeError> {
        let supported = matches!(
            event,
            CanonicalEvent::UserStimulus(_)
                | CanonicalEvent::DeliveryOutcome(_)
                | CanonicalEvent::TimeAdvance(_)
        );
        if !supported {
            return Err(RuntimeError::UnsupportedEvent(wire::event_kind_name(event)));
        }

        let turn_id = match event {
            CanonicalEvent::UserStimulus(e) => e.causal.turn_id,
            CanonicalEvent::DeliveryOutcome(e) => e.causal.turn_id,
            CanonicalEvent::TimeAdvance(e) => e.event_id,
            _ => unreachable!(),
        };

        // Copy the hot-brain facts before touching SQLite. Keeping a mutable
        // reference across store calls violates the single-writer borrow
        // boundary and is unnecessary: the only in-memory mutation is the
        // revision update after a successful commit.
        let (
            hot_bot_token,
            hot_persona_token,
            legacy_persona_scope,
            legacy_revision,
            formula_digest,
            manifest_digest,
            initial_snapshot_digest,
        ) = {
            let hot = self.hot_for(scope)?;
            (
                hot.bot_token,
                hot.persona_token,
                hot.legacy_persona_scope,
                hot.legacy_revision,
                hot.formula_digest,
                hot.identity.manifest_digest,
                hot.initial_snapshot_digest,
            )
        };
        let committed = self
            .store
            .lookup_bound_genesis(&hot_bot_token, &hot_persona_token)?
            .ok_or(RuntimeError::PersonaGenesisRequired)?;
        if committed.receipt.formula_digest != formula_digest
            || committed.receipt.initial_snapshot_digest != initial_snapshot_digest
        {
            return Err(RuntimeError::InvalidNeuralState);
        }
        let (state_before, graph_after, active_nodes, active_edges) = self.durable_g0_metadata_v1(
            &legacy_persona_scope,
            legacy_revision,
            &formula_digest,
            &initial_snapshot_digest,
            &committed.receipt.graph_digest,
        )?;
        let event_scope = match event {
            CanonicalEvent::UserStimulus(e) => &e.scope,
            CanonicalEvent::DeliveryOutcome(e) => &e.scope,
            CanonicalEvent::TimeAdvance(e) => &e.scope,
            _ => unreachable!(),
        };
        if event_scope.bot_token != hot_bot_token || event_scope.persona_token != hot_persona_token
        {
            return Err(RuntimeError::GenesisManifestMismatch);
        }

        let event_bytes = wire::encode_event(event);
        let event_digest = wire::event_digest(event);
        let contract = noop_action_contract(&manifest_digest, &event_digest, turn_id);
        let contract_digest = wire::action_contract_digest(&contract);

        // Idempotency: an event that was already applied is never applied
        // twice; the original receipt is returned unchanged.
        if let Some(row) = self
            .store
            .lookup_event(&legacy_persona_scope, &event_digest)?
        {
            let receipt = row
                .decode_receipt()
                .map_err(|error| RuntimeError::Store(StoreError::Sqlite(error.to_string())))?;
            return Ok(ApplyDecision {
                contract,
                receipt,
                revision: row.revision,
                deduplicated: true,
            });
        }

        let causal_base = match event {
            CanonicalEvent::UserStimulus(e) => e.causal.base_revision,
            CanonicalEvent::DeliveryOutcome(e) => e.causal.base_revision,
            CanonicalEvent::TimeAdvance(_) => legacy_revision,
            _ => unreachable!(),
        };
        if causal_base != legacy_revision {
            return Err(RuntimeError::StaleCausalBase {
                expected: legacy_revision,
                actual: causal_base,
            });
        }

        let authority_digest = authority_projection_digest(event);
        let receipt = TransitionReceipt {
            schema_version: 1,
            formula_digest,
            scope_digest: legacy_persona_scope,
            event_digest,
            authority_digest,
            base_revision: legacy_revision,
            next_revision: legacy_revision + 1,
            state_before,
            state_after: state_before,
            graph_after,
            action_contract: Some(contract_digest),
            active_nodes,
            active_edges,
            residuals: fixed_zero_vector(),
            status: CommitStatus::Committed,
        };

        // An empty journal is the normal first-turn case: start the chain at
        // the committed Genesis snapshot. Only a store error should fail the
        // event; ``Ok(None)`` is not evidence that Genesis is missing.
        let chain_seed = self
            .store
            .last_chain_digest(&legacy_persona_scope)?
            .unwrap_or(initial_snapshot_digest);
        let envelope = CommitEnvelope {
            event_kind: wire::event_kind_name(event).to_string(),
            event_bytes,
            receipt,
            chain_seed,
            delta_bytes: vec![],
        };

        match self.store.commit_journal(&envelope) {
            Ok((revision, _row)) => {
                if let Some(hot) = self.hot.as_mut() {
                    hot.legacy_revision = revision;
                }
                Ok(ApplyDecision {
                    contract,
                    receipt: envelope.receipt,
                    revision,
                    deduplicated: false,
                })
            }
            Err(StoreError::DuplicateEvent(revision)) => {
                let row = self
                    .store
                    .lookup_event(&legacy_persona_scope, &event_digest)?
                    .ok_or(RuntimeError::RetryWait)?;
                let receipt = row
                    .decode_receipt()
                    .map_err(|error| RuntimeError::Store(StoreError::Sqlite(error.to_string())))?;
                Ok(ApplyDecision {
                    contract,
                    receipt,
                    revision,
                    deduplicated: true,
                })
            }
            Err(other) => Err(RuntimeError::Store(other)),
        }
    }

    // ------------------------------------------------------------ observatory

    /// Inspect only the public legacy G0 authority lane. The private semantic
    /// history is never a public observatory selector.
    pub fn inspect(
        &mut self,
        bot_token: &Id128,
        persona_token: &Id128,
    ) -> Result<InspectReport, RuntimeError> {
        let Some(committed) = self.store.lookup_bound_genesis(bot_token, persona_token)? else {
            return Ok(InspectReport {
                bound: false,
                bot_token: *bot_token,
                persona_token: *persona_token,
                seed_code: String::new(),
                seed_code_short: String::new(),
                incarnation_id: String::new(),
                revision: 0,
                initial_snapshot_digest: [0; 32],
                last_chain_digest: None,
                journal_count: 0,
                observatory_genesis_unavailable: true,
            });
        };
        let legacy_persona_scope = wire::persona_scope_digest(bot_token, persona_token, None);
        let legacy_rows = self.store.read_journal(&legacy_persona_scope)?;
        let journal_count =
            u64::try_from(legacy_rows.len()).map_err(|_| RuntimeError::InvalidNeuralState)?;
        Ok(InspectReport {
            bound: true,
            bot_token: *bot_token,
            persona_token: *persona_token,
            seed_code: ae_genesis::format_seed_code(&committed.receipt.seed_code_digest),
            seed_code_short: ae_genesis::format_short_seed_code(
                &committed.receipt.seed_code_digest,
            ),
            incarnation_id: ae_genesis::format_incarnation_id(&committed.receipt.incarnation_id),
            revision: self.store.current_revision(&legacy_persona_scope)?,
            initial_snapshot_digest: committed.receipt.initial_snapshot_digest,
            last_chain_digest: self.store.last_chain_digest(&legacy_persona_scope)?,
            journal_count,
            observatory_genesis_unavailable: false,
        })
    }

    fn verify_durable_history_v1(
        &self,
        formula_digest: Digest,
        initial_snapshot_digest: Digest,
        initial_graph_digest: Digest,
        persona_scope: Digest,
        requires_semantic_snapshots: bool,
    ) -> Result<ReplayReport, RuntimeError> {
        let rows = self.store.read_journal(&persona_scope)?;
        let current_revision = self.store.current_revision(&persona_scope)?;
        let report = ae_continuum::verify_replay(initial_snapshot_digest, &rows);
        if !report.ok || report.final_revision != current_revision {
            return Err(RuntimeError::InvalidNeuralState);
        }
        let mut expected_state_digest = initial_snapshot_digest;
        let mut expected_graph_digest = initial_graph_digest;
        for row in &rows {
            let receipt = row
                .decode_receipt()
                .map_err(|error| RuntimeError::Store(StoreError::Sqlite(error.to_string())))?;
            if row.revision != receipt.next_revision
                || row.base_revision != receipt.base_revision
                || receipt.formula_digest != formula_digest
                || receipt.scope_digest != persona_scope
                || receipt.state_before != expected_state_digest
            {
                return Err(RuntimeError::InvalidNeuralState);
            }
            expected_state_digest = receipt.state_after;
            expected_graph_digest = receipt.graph_after;
            let snapshot = self.store.read_snapshot(&persona_scope, row.revision)?;
            if requires_semantic_snapshots {
                let snapshot = snapshot.ok_or(RuntimeError::InvalidNeuralState)?;
                if snapshot.state_digest != receipt.state_after {
                    return Err(RuntimeError::InvalidNeuralState);
                }
                let _ = decode_hot_state_v1(
                    &snapshot.state_bytes,
                    &formula_digest,
                    &receipt.state_after,
                    &receipt.graph_after,
                )?;
            } else if let Some(snapshot) = snapshot {
                if snapshot.state_digest != receipt.state_after {
                    return Err(RuntimeError::InvalidNeuralState);
                }
                let _ = decode_hot_state_v1(
                    &snapshot.state_bytes,
                    &formula_digest,
                    &receipt.state_after,
                    &receipt.graph_after,
                )?;
            }
        }
        if current_revision > 0 {
            let latest_snapshot = self
                .store
                .read_latest_snapshot(&persona_scope, current_revision)?;
            if let Some(snapshot) = latest_snapshot {
                if snapshot.state_digest != expected_state_digest {
                    return Err(RuntimeError::InvalidNeuralState);
                }
                let _ = decode_hot_state_v1(
                    &snapshot.state_bytes,
                    &formula_digest,
                    &expected_state_digest,
                    &expected_graph_digest,
                )?;
            } else if requires_semantic_snapshots {
                return Err(RuntimeError::InvalidNeuralState);
            }
        }
        Ok(report)
    }

    /// Verify only the public legacy G0 chain.
    pub fn verify_replay(
        &mut self,
        bot_token: &Id128,
        persona_token: &Id128,
    ) -> Result<ReplayReport, RuntimeError> {
        let committed = self
            .store
            .lookup_bound_genesis(bot_token, persona_token)?
            .ok_or(RuntimeError::PersonaGenesisRequired)?;
        let legacy_persona_scope = wire::persona_scope_digest(bot_token, persona_token, None);
        self.verify_durable_history_v1(
            committed.receipt.formula_digest,
            committed.receipt.initial_snapshot_digest,
            committed.receipt.graph_digest,
            legacy_persona_scope,
            false,
        )
    }

    /// Internal integrity audit for both independent durable histories. It
    /// returns neither a lane selector nor any projection material.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn audit_durable_histories_v1(
        &mut self,
        bot_token: &Id128,
        persona_token: &Id128,
    ) -> Result<(), RuntimeError> {
        let committed = self
            .store
            .lookup_bound_genesis(bot_token, persona_token)?
            .ok_or(RuntimeError::PersonaGenesisRequired)?;
        let legacy_persona_scope = wire::persona_scope_digest(bot_token, persona_token, None);
        let semantic_persona_scope = r7_semantic_persona_scope(bot_token, persona_token);
        for persona_scope in [legacy_persona_scope, semantic_persona_scope] {
            self.verify_durable_history_v1(
                committed.receipt.formula_digest,
                committed.receipt.initial_snapshot_digest,
                committed.receipt.graph_digest,
                persona_scope,
                persona_scope == semantic_persona_scope,
            )?;
        }
        Ok(())
    }

    /// Drain the writer: drop the hot cache, checkpoint WAL and close the
    /// store. Semantic state was already committed atomically with its journal
    /// row, so close never writes an independent state truth.
    pub fn flush_and_close(&mut self) -> Result<(), RuntimeError> {
        self.hot = None;
        self.store.flush()?;
        Ok(())
    }

    pub fn closed(&self) -> bool {
        matches!(self.store.count_leases(), Err(StoreError::Closed))
    }

    /// Return the public ordinary-G0 causal revision. The production R7
    /// ingress intentionally uses `HotBrain::semantic_revision` internally,
    /// so its private semantic lane never leaks into a G0 causal base.
    pub fn current_revision(&mut self, scope: &ScopeRef) -> Result<u64, RuntimeError> {
        let hot = self.hot_for(scope)?;
        Ok(hot.legacy_revision)
    }

    /// Return the content-free cursor for the durable semantic lane.  This is
    /// deliberately separate from the public G0 `current_revision` cursor.
    pub fn semantic_revision_v1(&mut self, scope: &ScopeRef) -> Result<u64, RuntimeError> {
        validate_perception_scope(scope)?;
        let hot = self.hot_for(scope)?;
        Ok(hot.semantic_revision)
    }

    fn semantic_event_identity_conflict(
        &self,
        semantic_scope: &Digest,
        event_id: &Id128,
        event_digest: &Digest,
    ) -> Result<bool, RuntimeError> {
        for row in self.store.read_journal(semantic_scope)? {
            let event = wire::decode_event(&row.event_bytes)
                .map_err(|_| RuntimeError::InvalidNeuralState)?;
            if let CanonicalEvent::UserStimulus(stimulus) = event {
                if stimulus.event_id == *event_id && row.event_digest != *event_digest {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Validate and durably apply one closed SPC1 proposal.  The method owns
    /// the estimator commitment, reuses the existing semantic field
    /// preparation seam, and commits only the semantic journal/snapshot.  No
    /// ActionContract or private projection material is produced.
    pub fn apply_perception_proposal_v1(
        &mut self,
        scope: &ScopeRef,
        proposal: &PerceptionProposalV1,
    ) -> Result<PerceptionProposalDecisionV1, RuntimeError> {
        self.apply_perception_proposal_v1_inner(scope, proposal, || {})
    }

    #[cfg(test)]
    fn apply_perception_proposal_v1_with_pre_commit_hook(
        &mut self,
        scope: &ScopeRef,
        proposal: &PerceptionProposalV1,
        before_commit: &mut dyn FnMut(),
    ) -> Result<PerceptionProposalDecisionV1, RuntimeError> {
        self.apply_perception_proposal_v1_inner(scope, proposal, before_commit)
    }

    fn apply_perception_proposal_v1_inner<F>(
        &mut self,
        scope: &ScopeRef,
        proposal: &PerceptionProposalV1,
        mut before_commit: F,
    ) -> Result<PerceptionProposalDecisionV1, RuntimeError>
    where
        F: FnMut(),
    {
        validate_perception_scope(scope)?;
        proposal
            .validate_v1()
            .map_err(map_perception_proposal_error)?;

        let r7_scope = r7_scope_from_root(scope);
        let estimator_digest = proposal.estimator_digest_v1(&r7_scope);
        let r7_event = r7_perception_event(scope, proposal, estimator_digest);
        let root_event = canonical_event_from_r7(&r7_event).map_err(map_semantic_prepare_error)?;

        let (
            hot_bot_token,
            hot_persona_token,
            semantic_persona_scope,
            semantic_revision,
            formula_digest,
            initial_snapshot_digest,
            field,
            graph,
        ) = {
            let hot = self.hot_for(scope)?;
            (
                hot.bot_token,
                hot.persona_token,
                hot.persona_scope,
                hot.semantic_revision,
                hot.formula_digest,
                hot.initial_snapshot_digest,
                hot.field.clone(),
                hot.graph.clone(),
            )
        };
        if scope.bot_token != hot_bot_token || scope.persona_token != hot_persona_token {
            return Err(RuntimeError::GenesisManifestMismatch);
        }

        let event_digest = wire::event_digest(&root_event);
        if let Some(row) = self
            .store
            .lookup_event(&semantic_persona_scope, &event_digest)?
        {
            let receipt = row
                .decode_receipt()
                .map_err(|error| RuntimeError::Store(StoreError::Sqlite(error.to_string())))?;
            if receipt.action_contract.is_some() {
                return Err(RuntimeError::SemanticIdentityConflict);
            }
            self.bind_hot(scope.bot_token, scope.persona_token)?;
            return Ok(PerceptionProposalDecisionV1 {
                receipt,
                revision: row.revision,
                deduplicated: true,
            });
        }
        if self.semantic_event_identity_conflict(
            &semantic_persona_scope,
            &proposal.event_id,
            &event_digest,
        )? {
            return Err(RuntimeError::SemanticIdentityConflict);
        }
        if proposal.base_revision != semantic_revision {
            return Err(RuntimeError::StaleCausalBase {
                expected: semantic_revision,
                actual: proposal.base_revision,
            });
        }

        let prepared = r7::prepare_production_user_stimulus_transition_v1(
            &r7_event,
            &field,
            &graph,
            semantic_revision,
        )
        .map_err(map_semantic_prepare_error)?;
        let next_revision = semantic_revision
            .checked_add(1)
            .ok_or(RuntimeError::SemanticRevisionOverflow)?;
        let state_before = state_digest(&field, &formula_digest);
        let state_after = state_digest(&prepared.next_field, &formula_digest);
        if state_before == state_after {
            return Err(RuntimeError::SemanticStateUnchanged);
        }
        let graph_after = graph_digest(&graph);
        let authority_digest = authority_projection_digest(&root_event);
        let receipt = TransitionReceipt {
            schema_version: 1,
            formula_digest,
            scope_digest: semantic_persona_scope,
            event_digest,
            authority_digest,
            base_revision: semantic_revision,
            next_revision,
            state_before,
            state_after,
            graph_after,
            action_contract: None,
            active_nodes: prepared.active_nodes,
            active_edges: graph.edges.len() as u32,
            residuals: fixed_zero_vector(),
            status: CommitStatus::Committed,
        };
        let state_bytes = encode_hot_state_v1(&formula_digest, &prepared.next_field, &graph);
        let _ = decode_hot_state_v1(&state_bytes, &formula_digest, &state_after, &graph_after)?;
        let chain_seed = self
            .store
            .last_chain_digest(&semantic_persona_scope)?
            .unwrap_or(initial_snapshot_digest);
        let commit = StatefulCommit {
            journal: CommitEnvelope {
                event_kind: wire::event_kind_name(&root_event).to_owned(),
                event_bytes: wire::encode_event(&root_event),
                receipt: receipt.clone(),
                chain_seed,
                delta_bytes: vec![],
            },
            state_bytes,
        };

        before_commit();
        match self.store.commit_stateful_journal(&commit) {
            Ok((revision, _row)) => {
                if let Some(hot) = self.hot.as_mut() {
                    hot.field = prepared.next_field;
                    hot.graph = graph;
                    hot.semantic_revision = revision;
                }
                Ok(PerceptionProposalDecisionV1 {
                    receipt,
                    revision,
                    deduplicated: false,
                })
            }
            Err(StoreError::DuplicateEvent(revision)) => {
                let row = self
                    .store
                    .lookup_event(&semantic_persona_scope, &event_digest)?
                    .ok_or(RuntimeError::RetryWait)?;
                if row.revision != revision {
                    return Err(RuntimeError::InvalidNeuralState);
                }
                let receipt = row
                    .decode_receipt()
                    .map_err(|error| RuntimeError::Store(StoreError::Sqlite(error.to_string())))?;
                if receipt.action_contract.is_some() {
                    return Err(RuntimeError::SemanticIdentityConflict);
                }
                self.bind_hot(scope.bot_token, scope.persona_token)?;
                Ok(PerceptionProposalDecisionV1 {
                    receipt,
                    revision,
                    deduplicated: true,
                })
            }
            Err(stale @ StoreError::StaleRevision { .. }) => {
                let Some(row) = self
                    .store
                    .lookup_event(&semantic_persona_scope, &event_digest)?
                else {
                    return Err(RuntimeError::Store(stale));
                };
                if row.event_digest != event_digest || row.scope_digest != semantic_persona_scope {
                    return Err(RuntimeError::InvalidNeuralState);
                }
                let revision = row.revision;
                let receipt = row
                    .decode_receipt()
                    .map_err(|error| RuntimeError::Store(StoreError::Sqlite(error.to_string())))?;
                if receipt.action_contract.is_some() {
                    return Err(RuntimeError::SemanticIdentityConflict);
                }
                if receipt.scope_digest != semantic_persona_scope
                    || receipt.event_digest != event_digest
                    || receipt.formula_digest != formula_digest
                    || receipt.schema_version != 1
                    || receipt.status != CommitStatus::Committed
                    || receipt.next_revision != revision
                    || receipt.base_revision >= receipt.next_revision
                    || receipt.base_revision.checked_add(1) != Some(receipt.next_revision)
                {
                    return Err(RuntimeError::InvalidNeuralState);
                }
                self.bind_hot(scope.bot_token, scope.persona_token)?;
                Ok(PerceptionProposalDecisionV1 {
                    receipt,
                    revision,
                    deduplicated: true,
                })
            }
            Err(other) => Err(RuntimeError::Store(other)),
        }
    }
}

#[cfg(test)]
trait PersonaScopeForRequest {
    fn scope_persona_scope(&self) -> ScopeRef;
}

#[cfg(test)]
impl PersonaScopeForRequest for ae_contracts::PersonaSourceRef {
    fn scope_persona_scope(&self) -> ScopeRef {
        ScopeRef {
            bot_token: self.scope.bot_token,
            persona_token: self.scope.persona_token,
            relation_token: None,
            session_token: [0; 16],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ae_contracts::{
        wire, AllostaticSetpoints, CausalRef, EpistemicPriors, EvidenceVector, ExpressionPhenotype,
        GenesisManifestProposal, PersonaScopeRef, PersonaSelectionKind, PersonaSourceRef,
        PersonalityVector, SemanticEstimate, SocialPriors, UserStimulus,
    };
    use ae_fixed::Fixed;

    #[test]
    fn private_r7_errors_collapse_to_one_non_payload_root_error() {
        let error: RuntimeError = r7::RuntimeError::PrivateProjectionWireBindingMismatch {
            field: "PRIVATE_BINDING_FIELD_SENTINEL",
        }
        .into();
        assert!(matches!(error, RuntimeError::PrivateProjectionUnavailable));
        assert_eq!(error.to_string(), "private projection unavailable");
    }

    pub(super) fn request(seed: u8) -> PersonaGenesisRequest {
        let scope = PersonaScopeRef {
            bot_token: [seed; 16],
            persona_token: [seed.wrapping_add(1); 16],
        };
        let source = PersonaSourceRef {
            scope,
            source_digest: [seed.wrapping_add(2); 32],
            capability_digest: [seed.wrapping_add(3); 32],
            selection: PersonaSelectionKind::Conversation,
            prompt_chars: 10,
            begin_dialog_count: 1,
            mood_dialog_count: 0,
        };
        let proposal = GenesisManifestProposal {
            schema_version: 1,
            source: source.clone(),
            traits: PersonalityVector {
                baseline_warmth: Fixed::from_raw(700_000),
                ..PersonalityVector::default()
            },
            trait_confidence: PersonalityVector {
                baseline_warmth: Fixed::from_raw(500_000),
                ..PersonalityVector::default()
            },
            expression: ExpressionPhenotype::default(),
            allostasis: AllostaticSetpoints::default(),
            epistemic: EpistemicPriors::default(),
            social: SocialPriors::default(),
            compiler_protocol_digest: [seed.wrapping_add(4); 32],
            compiler_model_digest: [seed.wrapping_add(5); 32],
        };
        PersonaGenesisRequest {
            source,
            proposal,
            formula_digest: [seed.wrapping_add(6); 32],
            incarnation_nonce: [seed.wrapping_add(7); 32],
            parent_incarnation_id: None,
            observed_at_ms: 1_700_000_000_000,
        }
    }

    fn stimulus(seed: u8, revision: u64, session: u8) -> CanonicalEvent {
        CanonicalEvent::UserStimulus(UserStimulus {
            event_id: [seed.wrapping_add(10); 16],
            scope: ScopeRef {
                bot_token: [seed; 16],
                persona_token: [seed.wrapping_add(1); 16],
                relation_token: None,
                session_token: [session; 16],
            },
            causal: CausalRef {
                turn_id: [seed.wrapping_add(11); 16],
                action_id: None,
                delivery_id: None,
                claim_id: None,
                base_revision: revision,
            },
            observed_at_ms: 1_700_000_000_100,
            evidence: SemanticEstimate {
                schema_version: 1,
                dimensions: EvidenceVector::default(),
                estimator_confidence: Fixed::ZERO,
                estimator_digest: [0; 32],
            },
        })
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ae-runtime-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn full_g0_vertical_slice() {
        let dir = temp_dir("slice");
        let mut runtime = AstrRuntime::open(&dir.join("store.db")).unwrap();
        let request = request(1);
        let receipt = runtime.ensure_genesis(&request).unwrap();
        assert_eq!(receipt.status, GenesisStatus::Committed);

        let decision = runtime
            .apply_event(&request.source.scope_persona_scope(), &stimulus(1, 0, 1))
            .unwrap();
        assert!(!decision.deduplicated);
        assert_eq!(decision.revision, 1);
        assert_eq!(decision.receipt.base_revision, 0);
        assert_eq!(decision.receipt.next_revision, 1);
        assert_eq!(decision.receipt.state_before, decision.receipt.state_after);

        let report = runtime
            .verify_replay(
                &request.source.scope.bot_token,
                &request.source.scope.persona_token,
            )
            .unwrap();
        assert!(report.ok, "{:?}", report.first_error);
        assert_eq!(report.checked, 1);

        let inspect = runtime
            .inspect(
                &request.source.scope.bot_token,
                &request.source.scope.persona_token,
            )
            .unwrap();
        assert!(inspect.bound);
        assert!(inspect.seed_code.starts_with("AE-S1-"));
        assert!(!inspect.observatory_genesis_unavailable);

        runtime.flush_and_close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_inputs_same_digests_across_runtime_instances() {
        let dir_a = temp_dir("det-a");
        let dir_b = temp_dir("det-b");
        let mut a = AstrRuntime::open(&dir_a.join("store.db")).unwrap();
        let mut b = AstrRuntime::open(&dir_b.join("store.db")).unwrap();
        let request = request(7);
        let receipt_a = a.ensure_genesis(&request).unwrap();
        let receipt_b = b.ensure_genesis(&request).unwrap();
        assert_eq!(receipt_a, receipt_b);
        assert_eq!(
            wire::genesis_receipt_digest(&receipt_a),
            wire::genesis_receipt_digest(&receipt_b)
        );

        let event = stimulus(7, 0, 9);
        let decision_a = a
            .apply_event(&request.source.scope_persona_scope(), &event)
            .unwrap();
        let decision_b = b
            .apply_event(&request.source.scope_persona_scope(), &event)
            .unwrap();
        assert_eq!(decision_a.contract, decision_b.contract);
        assert_eq!(
            wire::receipt_digest(&decision_a.receipt),
            wire::receipt_digest(&decision_b.receipt)
        );
        assert_eq!(
            wire::action_contract_digest(&decision_a.contract),
            wire::action_contract_digest(&decision_b.contract)
        );
        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    #[test]
    fn duplicate_event_is_applied_once() {
        let dir = temp_dir("dup");
        let mut runtime = AstrRuntime::open(&dir.join("store.db")).unwrap();
        let request = request(2);
        runtime.ensure_genesis(&request).unwrap();
        let event = stimulus(2, 0, 1);
        let first = runtime
            .apply_event(&request.source.scope_persona_scope(), &event)
            .unwrap();
        let second = runtime
            .apply_event(&request.source.scope_persona_scope(), &event)
            .unwrap();
        assert!(!first.deduplicated);
        assert!(second.deduplicated);
        assert_eq!(
            wire::receipt_digest(&first.receipt),
            wire::receipt_digest(&second.receipt)
        );
        let report = runtime
            .verify_replay(
                &request.source.scope.bot_token,
                &request.source.scope.persona_token,
            )
            .unwrap();
        assert_eq!(report.checked, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_causal_base_is_rejected() {
        let dir = temp_dir("stale");
        let mut runtime = AstrRuntime::open(&dir.join("store.db")).unwrap();
        let request = request(3);
        runtime.ensure_genesis(&request).unwrap();
        let error = runtime
            .apply_event(&request.source.scope_persona_scope(), &stimulus(3, 5, 1))
            .unwrap_err();
        assert!(matches!(
            error,
            RuntimeError::StaleCausalBase {
                expected: 0,
                actual: 5
            }
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn self_action_writes_zero_and_is_not_supported_in_g0() {
        let dir = temp_dir("selfaction");
        let mut runtime = AstrRuntime::open(&dir.join("store.db")).unwrap();
        let request = request(4);
        runtime.ensure_genesis(&request).unwrap();
        let candidate = CanonicalEvent::SelfActionCandidate(ae_contracts::SelfActionCandidate {
            event_id: [55; 16],
            scope: ScopeRef {
                bot_token: [4; 16],
                persona_token: [5; 16],
                relation_token: None,
                session_token: [1; 16],
            },
            causal: CausalRef {
                turn_id: [56; 16],
                action_id: None,
                delivery_id: None,
                claim_id: None,
                base_revision: 0,
            },
            visible_action_digest: [57; 32],
            claims: vec![],
        });
        assert!(matches!(
            runtime.apply_event(&request.source.scope_persona_scope(), &candidate),
            Err(RuntimeError::UnsupportedEvent("self_action_candidate"))
        ));
        // Zero production writes: journal is untouched.
        let report = runtime
            .verify_replay(
                &request.source.scope.bot_token,
                &request.source.scope.persona_token,
            )
            .unwrap();
        assert_eq!(report.checked, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn genesis_failure_creates_no_default_brain() {
        let dir = temp_dir("nobrain");
        let mut runtime = AstrRuntime::open(&dir.join("store.db")).unwrap();
        let mut broken = request(5);
        broken.proposal.source.source_digest = [99; 32];
        let error = runtime.ensure_genesis(&broken).unwrap_err();
        assert!(matches!(error, RuntimeError::Genesis(_)));
        // No lease, no incarnation, no binding: applying an event fails.
        assert!(matches!(
            runtime.apply_event(&broken.source.scope_persona_scope(), &stimulus(5, 0, 1)),
            Err(RuntimeError::PersonaGenesisRequired)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crash_recovery_reopens_and_replays() {
        let dir = temp_dir("crash");
        let path = dir.join("store.db");
        let mut runtime = AstrRuntime::open(&path).unwrap();
        let request = request(6);
        let receipt = runtime.ensure_genesis(&request).unwrap();
        let decision = runtime
            .apply_event(&request.source.scope_persona_scope(), &stimulus(6, 0, 1))
            .unwrap();
        assert_eq!(decision.revision, 1);
        drop(runtime); // crash without flush_and_close

        let mut reopened = AstrRuntime::open(&path).unwrap();
        let report = reopened
            .verify_replay(
                &request.source.scope.bot_token,
                &request.source.scope.persona_token,
            )
            .unwrap();
        assert!(report.ok, "{:?}", report.first_error);
        assert_eq!(report.checked, 1);

        // The next event continues at revision 2, not 1, and the birth was
        // not duplicated.
        let next = reopened
            .apply_event(&request.source.scope_persona_scope(), &stimulus(6, 1, 2))
            .unwrap();
        assert_eq!(next.revision, 2);
        assert_eq!(next.receipt.base_revision, 1);
        let again = reopened.ensure_genesis(&request).unwrap();
        assert_eq!(again, receipt);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn twenty_concurrent_ensure_genesis_calls_join_one_birth() {
        use std::sync::{Arc, Mutex};
        let dir = temp_dir("concurrent");
        let runtime = Arc::new(Mutex::new(
            AstrRuntime::open(&dir.join("store.db")).unwrap(),
        ));
        let request = Arc::new(request(8));

        let mut handles = Vec::new();
        for _ in 0..20 {
            let runtime = Arc::clone(&runtime);
            let request = Arc::clone(&request);
            handles.push(std::thread::spawn(move || {
                runtime.lock().unwrap().ensure_genesis(&request).unwrap()
            }));
        }
        let receipts: Vec<GenesisReceipt> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        for receipt in &receipts[1..] {
            assert_eq!(receipt, &receipts[0]);
        }
        let runtime = runtime.lock().unwrap();
        assert!(matches!(runtime.store.count_incarnations(), Ok(1)));
        assert!(matches!(runtime.store.count_leases(), Ok(1)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod spc1_native_ingress_red_tests {
    use super::*;
    use ae_contracts::r7::{EvidenceVector, PerceptionProposalV1};
    use ae_contracts::{CausalRef, SemanticEstimate, UserStimulus};
    use ae_fixed::Fixed;
    use std::collections::BTreeSet;

    fn scope(seed: u8, session: u8) -> ScopeRef {
        ScopeRef {
            bot_token: [seed; 16],
            persona_token: [seed.wrapping_add(1); 16],
            relation_token: None,
            session_token: [session; 16],
        }
    }

    fn proposal(seed: u8, base_revision: u64) -> PerceptionProposalV1 {
        PerceptionProposalV1 {
            schema_version: 1,
            event_id: [seed; 16],
            turn_id: [seed.wrapping_add(1); 16],
            observed_at_ms: 1_700_000_000_000 + u64::from(seed),
            base_revision,
            dimensions: EvidenceVector {
                positive: Fixed::from_raw(200_000 + i64::from(seed)),
                harm: Fixed::from_raw(100_000),
                boundary: Fixed::from_raw(150_000),
                epistemic_conflict: Fixed::from_raw(250_000),
                ..EvidenceVector::default()
            },
            estimator_confidence: Fixed::from_raw(800_000),
            protocol_version: 1,
            request_nonce_digest: [seed.wrapping_add(2); 32],
        }
    }

    fn set_dimension(proposal: &mut PerceptionProposalV1, index: usize, value: Fixed) {
        match index {
            0 => proposal.dimensions.positive = value,
            1 => proposal.dimensions.affiliation = value,
            2 => proposal.dimensions.harm = value,
            3 => proposal.dimensions.boundary = value,
            4 => proposal.dimensions.repair = value,
            5 => proposal.dimensions.repetition = value,
            6 => proposal.dimensions.new_information = value,
            7 => proposal.dimensions.constraint_instability = value,
            8 => proposal.dimensions.epistemic_conflict = value,
            9 => proposal.dimensions.self_responsibility = value,
            10 => proposal.dimensions.other_responsibility = value,
            11 => proposal.dimensions.hostility = value,
            12 => proposal.dimensions.publicness = value,
            13 => proposal.dimensions.engagement = value,
            14 => proposal.dimensions.rejection = value,
            _ => panic!("invalid evidence dimension index"),
        }
    }

    fn expected_estimator_digest(
        proposal: &PerceptionProposalV1,
        scope: &ae_contracts::r7::ScopeRef,
    ) -> Digest {
        let schema_version = proposal.schema_version.to_le_bytes();
        let values = [
            proposal.dimensions.positive.encode(),
            proposal.dimensions.affiliation.encode(),
            proposal.dimensions.harm.encode(),
            proposal.dimensions.boundary.encode(),
            proposal.dimensions.repair.encode(),
            proposal.dimensions.repetition.encode(),
            proposal.dimensions.new_information.encode(),
            proposal.dimensions.constraint_instability.encode(),
            proposal.dimensions.epistemic_conflict.encode(),
            proposal.dimensions.self_responsibility.encode(),
            proposal.dimensions.other_responsibility.encode(),
            proposal.dimensions.hostility.encode(),
            proposal.dimensions.publicness.encode(),
            proposal.dimensions.engagement.encode(),
            proposal.dimensions.rejection.encode(),
        ];
        let confidence = proposal.estimator_confidence.encode();
        let protocol_version = proposal.protocol_version.to_le_bytes();
        let base_revision = proposal.base_revision.to_le_bytes();
        let mut scope_body = Vec::with_capacity(16 * 4 + 1);
        scope_body.extend_from_slice(&scope.bot_token);
        scope_body.extend_from_slice(&scope.persona_token);
        match scope.relation_token {
            Some(relation) => {
                scope_body.push(1);
                scope_body.extend_from_slice(&relation);
            }
            None => scope_body.push(0),
        }
        scope_body.extend_from_slice(&scope.session_token);
        let scope_digest = ae_contracts::r7::wire::domain_hash(
            b"astr-embodiment/semantic-perception-scope-v1",
            &[&scope_body],
        );
        let mut fields: Vec<&[u8]> = Vec::with_capacity(22);
        fields.push(&schema_version);
        fields.extend(values.iter().map(|value| value.as_slice()));
        fields.push(&confidence);
        fields.push(&protocol_version);
        fields.push(&proposal.request_nonce_digest);
        fields.push(&proposal.event_id);
        fields.push(&scope_digest);
        fields.push(&proposal.turn_id);
        fields.push(&base_revision);
        ae_contracts::r7::wire::domain_hash(PerceptionProposalV1::DIGEST_DOMAIN_V1, &fields)
    }

    fn database(name: &str) -> std::path::PathBuf {
        let root = std::path::PathBuf::from(
            r"G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\test-runs\spc1-native-ingress",
        );
        std::fs::create_dir_all(&root).expect("test root");
        let path = root.join(format!("{name}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    fn runtime_for(seed: u8, name: &str) -> (AstrRuntime, ScopeRef) {
        let path = database(name);
        let mut runtime = AstrRuntime::open(&path).expect("open runtime");
        let genesis = super::tests::request(seed);
        runtime.ensure_genesis(&genesis).expect("genesis");
        (runtime, scope(seed, 90))
    }

    fn cleanup_database(name: &str) {
        let root = std::path::PathBuf::from(
            r"G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\test-runs\spc1-native-ingress",
        );
        let path = root.join(format!("{name}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(path);
    }

    fn g0_stimulus(seed: u8, revision: u64, session: u8) -> CanonicalEvent {
        CanonicalEvent::UserStimulus(UserStimulus {
            event_id: [seed.wrapping_add(10); 16],
            scope: scope(seed, session),
            causal: CausalRef {
                turn_id: [seed.wrapping_add(11); 16],
                action_id: None,
                delivery_id: None,
                claim_id: None,
                base_revision: revision,
            },
            observed_at_ms: 1_700_000_000_100,
            evidence: SemanticEstimate {
                schema_version: 1,
                dimensions: ae_contracts::EvidenceVector::default(),
                estimator_confidence: Fixed::ZERO,
                estimator_digest: [0; 32],
            },
        })
    }

    #[test]
    fn perception_proposal_has_a_closed_digest_and_semantic_cursor_api() {
        let scope = ae_contracts::r7::ScopeRef {
            bot_token: [31; 16],
            persona_token: [32; 16],
            relation_token: None,
            session_token: [3; 16],
        };
        let proposal = PerceptionProposalV1 {
            schema_version: 1,
            event_id: [4; 16],
            turn_id: [5; 16],
            observed_at_ms: 1,
            base_revision: 0,
            dimensions: EvidenceVector {
                positive: Fixed::from_raw(1),
                ..EvidenceVector::default()
            },
            estimator_confidence: Fixed::ONE,
            protocol_version: 1,
            request_nonce_digest: [6; 32],
        };
        assert_ne!(proposal.estimator_digest_v1(&scope), [0; 32]);

        let path = database("red");
        let mut runtime = AstrRuntime::open(&path).expect("open runtime");
        let genesis = super::tests::request(31);
        runtime.ensure_genesis(&genesis).expect("genesis");
        let runtime_scope = ScopeRef {
            bot_token: scope.bot_token,
            persona_token: scope.persona_token,
            relation_token: scope.relation_token,
            session_token: scope.session_token,
        };
        assert_eq!(runtime.semantic_revision_v1(&runtime_scope).unwrap(), 0);
        runtime.flush_and_close().expect("close runtime");
        drop(runtime);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn proposal_json_is_closed_and_fixed_digest_is_scope_bound() {
        let mut proposal = proposal(41, 0);
        for (index, raw) in (1..=15).map(|value| value * 10_000).enumerate() {
            set_dimension(&mut proposal, index, Fixed::from_raw(raw));
        }
        proposal.validate_v1().expect("valid proposal");
        let encoded = serde_json::to_value(&proposal).expect("encode proposal");
        let keys = encoded
            .as_object()
            .expect("proposal object")
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            keys,
            [
                "schema_version",
                "event_id",
                "turn_id",
                "observed_at_ms",
                "base_revision",
                "dimensions",
                "estimator_confidence",
                "protocol_version",
                "request_nonce_digest",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect()
        );
        assert!(encoded["event_id"].is_string());
        assert!(encoded["request_nonce_digest"].is_string());
        assert_eq!(
            encoded["dimensions"]["positive"],
            serde_json::json!(proposal.dimensions.positive.raw())
        );
        let mut unknown = encoded.clone();
        unknown["unknown"] = serde_json::json!("sentinel");
        assert!(serde_json::from_value::<PerceptionProposalV1>(unknown).is_err());
        let mut nested_unknown = encoded.clone();
        nested_unknown["dimensions"]["unknown"] = serde_json::json!(1);
        assert!(serde_json::from_value::<PerceptionProposalV1>(nested_unknown).is_err());
        for invalid_json_number in [serde_json::json!(0.5), serde_json::json!(true)] {
            let mut invalid = encoded.clone();
            invalid["dimensions"]["positive"] = invalid_json_number;
            assert!(serde_json::from_value::<PerceptionProposalV1>(invalid).is_err());
        }
        let r7_scope = ae_contracts::r7::ScopeRef {
            bot_token: [41; 16],
            persona_token: [42; 16],
            relation_token: None,
            session_token: [90; 16],
        };
        let mut other_scope = r7_scope.clone();
        other_scope.session_token = [91; 16];
        assert_ne!(
            proposal.estimator_digest_v1(&r7_scope),
            proposal.estimator_digest_v1(&other_scope)
        );
        let mut changed = proposal.clone();
        changed.dimensions.rejection = Fixed::from_raw(1);
        assert_ne!(
            proposal.estimator_digest_v1(&r7_scope),
            changed.estimator_digest_v1(&r7_scope)
        );
        assert_eq!(
            proposal.estimator_digest_v1(&r7_scope),
            expected_estimator_digest(&proposal, &r7_scope)
        );
        assert_eq!(
            proposal.estimator_digest_v1(&r7_scope),
            [
                0x40, 0x54, 0x37, 0x53, 0x26, 0xa6, 0x5e, 0xa9, 0x36, 0xe6, 0x5d, 0x79, 0x04, 0xac,
                0xcc, 0x9e, 0x50, 0x51, 0x80, 0x45, 0x0b, 0xcd, 0xc0, 0x63, 0x45, 0x27, 0x8f, 0x37,
                0xa4, 0xd3, 0x39, 0xa0,
            ]
        );
        let mut all_one = proposal.clone();
        for index in 0..15 {
            set_dimension(&mut all_one, index, Fixed::ONE);
        }
        all_one.estimator_confidence = Fixed::ONE;
        all_one.validate_v1().expect("inclusive one boundary");
        let mut high_confidence = all_one.clone();
        high_confidence.estimator_confidence = Fixed::from_raw(1_000_001);
        assert!(high_confidence.validate_v1().is_err());
        for index in 0..15 {
            let mut negative = all_one.clone();
            set_dimension(&mut negative, index, Fixed::from_raw(-1));
            assert!(negative.validate_v1().is_err());
            let mut high = all_one.clone();
            set_dimension(&mut high, index, Fixed::from_raw(1_000_001));
            assert!(high.validate_v1().is_err());
        }
    }

    #[test]
    fn semantic_commit_is_durable_idempotent_and_isolated_from_g0() {
        let path = database("lifecycle");
        let mut runtime = AstrRuntime::open(&path).expect("open runtime");
        let genesis = super::tests::request(51);
        runtime.ensure_genesis(&genesis).expect("genesis");
        let request_scope = scope(51, 90);
        assert_eq!(runtime.current_revision(&request_scope).unwrap(), 0);
        assert_eq!(runtime.semantic_revision_v1(&request_scope).unwrap(), 0);

        let first_proposal = proposal(61, 0);
        let first = runtime
            .apply_perception_proposal_v1(&request_scope, &first_proposal)
            .expect("first semantic commit");
        assert_eq!(first.revision, 1);
        assert!(!first.deduplicated);
        assert_eq!(first.receipt.action_contract, None);
        assert_ne!(first.receipt.state_before, first.receipt.state_after);
        assert_eq!(runtime.semantic_revision_v1(&request_scope).unwrap(), 1);
        assert_eq!(runtime.current_revision(&request_scope).unwrap(), 0);
        let semantic_scope =
            r7_semantic_persona_scope(&request_scope.bot_token, &request_scope.persona_token);
        let semantic_rows_after_first = runtime
            .store
            .read_journal(&semantic_scope)
            .expect("semantic rows");
        let semantic_snapshot_after_first = runtime
            .store
            .read_snapshot(&semantic_scope, 1)
            .expect("semantic snapshot")
            .expect("revision one snapshot");

        let retry = runtime
            .apply_perception_proposal_v1(&request_scope, &first_proposal)
            .expect("exact retry");
        assert!(retry.deduplicated);
        assert_eq!(retry.receipt, first.receipt);
        assert_eq!(runtime.semantic_revision_v1(&request_scope).unwrap(), 1);

        let mut modified = first_proposal.clone();
        modified.dimensions.positive = Fixed::from_raw(300_000);
        assert!(matches!(
            runtime.apply_perception_proposal_v1(&request_scope, &modified),
            Err(RuntimeError::SemanticIdentityConflict)
        ));
        assert_eq!(runtime.semantic_revision_v1(&request_scope).unwrap(), 1);

        let mut changed_turn = first_proposal.clone();
        changed_turn.turn_id = [99; 16];
        assert!(matches!(
            runtime.apply_perception_proposal_v1(&request_scope, &changed_turn),
            Err(RuntimeError::SemanticIdentityConflict)
        ));
        let alternate_session = scope(51, 91);
        assert!(matches!(
            runtime.apply_perception_proposal_v1(&alternate_session, &first_proposal),
            Err(RuntimeError::SemanticIdentityConflict)
        ));
        let stale = proposal(64, 0);
        assert!(matches!(
            runtime.apply_perception_proposal_v1(&request_scope, &stale),
            Err(RuntimeError::StaleCausalBase {
                expected: 1,
                actual: 0
            })
        ));
        assert_eq!(runtime.semantic_revision_v1(&request_scope).unwrap(), 1);
        assert_eq!(
            runtime.store.read_journal(&semantic_scope).unwrap(),
            semantic_rows_after_first
        );
        assert_eq!(
            runtime
                .store
                .read_snapshot(&semantic_scope, 1)
                .unwrap()
                .expect("revision one snapshot"),
            semantic_snapshot_after_first
        );

        let second_proposal = proposal(62, 1);
        let second = runtime
            .apply_perception_proposal_v1(&request_scope, &second_proposal)
            .expect("second semantic commit");
        assert_eq!(second.revision, 2);
        assert_ne!(second.receipt.state_after, first.receipt.state_after);
        assert_eq!(runtime.current_revision(&request_scope).unwrap(), 0);
        runtime
            .audit_durable_histories_v1(&request_scope.bot_token, &request_scope.persona_token)
            .expect("both lanes audit");
        let inspect = runtime
            .inspect(&request_scope.bot_token, &request_scope.persona_token)
            .expect("G0 inspect");
        assert_eq!(inspect.revision, 0);
        assert_eq!(inspect.journal_count, 0);
        let g0_replay = runtime
            .verify_replay(&request_scope.bot_token, &request_scope.persona_token)
            .expect("G0 replay");
        assert_eq!(g0_replay.checked, 0);
        runtime.flush_and_close().expect("close");
        drop(runtime);

        let mut reopened = AstrRuntime::open(&path).expect("reopen");
        assert_eq!(reopened.semantic_revision_v1(&request_scope).unwrap(), 2);
        assert_eq!(reopened.current_revision(&request_scope).unwrap(), 0);
        reopened
            .audit_durable_histories_v1(&request_scope.bot_token, &request_scope.persona_token)
            .expect("reopened semantic and G0 replay");
        let third = reopened
            .apply_perception_proposal_v1(&request_scope, &proposal(63, 2))
            .expect("post-reopen semantic commit");
        assert_eq!(third.revision, 3);
        assert_eq!(reopened.current_revision(&request_scope).unwrap(), 0);
        reopened.flush_and_close().expect("close reopened");
        drop(reopened);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn shared_runtime_instances_reconcile_semantic_hydration_and_continue() {
        let path = database("shared-runtime");
        let genesis = super::tests::request(91);
        let request_scope = scope(91, 190);
        let mut first_runtime = AstrRuntime::open(&path).expect("open first runtime");
        first_runtime.ensure_genesis(&genesis).expect("genesis");
        let mut second_runtime = AstrRuntime::open(&path).expect("open second runtime");
        assert_eq!(
            second_runtime
                .semantic_revision_v1(&request_scope)
                .expect("second runtime initial cursor"),
            0
        );

        let first_proposal = proposal(101, 0);
        let first = first_runtime
            .apply_perception_proposal_v1(&request_scope, &first_proposal)
            .expect("first runtime commit");
        assert_eq!(first.revision, 1);
        assert_eq!(
            second_runtime
                .semantic_revision_v1(&request_scope)
                .expect("second runtime reconciles durable cursor"),
            1
        );

        let duplicate = second_runtime
            .apply_perception_proposal_v1(&request_scope, &first_proposal)
            .expect("second runtime exact retry");
        assert!(duplicate.deduplicated);
        assert_eq!(duplicate.revision, 1);
        assert_eq!(duplicate.receipt, first.receipt);

        let second_proposal = proposal(102, 1);
        let second = second_runtime
            .apply_perception_proposal_v1(&request_scope, &second_proposal)
            .expect("second runtime continues from hydrated winner");
        assert_eq!(second.revision, 2);
        assert!(!second.deduplicated);
        assert_eq!(
            second_runtime
                .semantic_revision_v1(&request_scope)
                .expect("second runtime final cursor"),
            2
        );
        assert_eq!(
            first_runtime
                .semantic_revision_v1(&request_scope)
                .expect("first runtime reconciles second commit"),
            2
        );
        first_runtime
            .flush_and_close()
            .expect("close first runtime");
        second_runtime
            .flush_and_close()
            .expect("close second runtime");
        drop(first_runtime);
        drop(second_runtime);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn stale_exact_proposal_resolves_to_the_persisted_winner() {
        let path = database("stale-exact-race");
        let genesis = super::tests::request(111);
        let request_scope = scope(111, 210);
        let mut winner_runtime = AstrRuntime::open(&path).expect("open winner runtime");
        winner_runtime.ensure_genesis(&genesis).expect("genesis");
        let mut loser_runtime = AstrRuntime::open(&path).expect("open loser runtime");
        assert_eq!(
            loser_runtime
                .semantic_revision_v1(&request_scope)
                .expect("loser initial cursor"),
            0
        );

        let candidate = proposal(121, 0);
        let mut winner = None;
        let mut commit_winner = || {
            winner = Some(
                winner_runtime
                    .apply_perception_proposal_v1(&request_scope, &candidate)
                    .expect("winner commit"),
            );
        };
        let resolved = loser_runtime
            .apply_perception_proposal_v1_with_pre_commit_hook(
                &request_scope,
                &candidate,
                &mut commit_winner,
            )
            .expect("stale exact proposal resolves to winner");
        let winner = winner.expect("winner decision");
        assert!(!winner.deduplicated);
        assert!(resolved.deduplicated);
        assert_eq!(resolved.revision, winner.revision);
        assert_eq!(resolved.receipt, winner.receipt);
        assert_eq!(resolved.receipt.action_contract, None);
        assert_eq!(
            loser_runtime
                .semantic_revision_v1(&request_scope)
                .expect("loser hydrated cursor"),
            winner.revision
        );

        loser_runtime.flush_and_close().expect("close loser");
        winner_runtime.flush_and_close().expect("close winner");
        drop(loser_runtime);
        drop(winner_runtime);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn stale_different_proposal_preserves_cas_error_and_revision() {
        let path = database("stale-different-race");
        let genesis = super::tests::request(112);
        let request_scope = scope(112, 211);
        let mut winner_runtime = AstrRuntime::open(&path).expect("open winner runtime");
        winner_runtime.ensure_genesis(&genesis).expect("genesis");
        let mut loser_runtime = AstrRuntime::open(&path).expect("open loser runtime");
        assert_eq!(
            loser_runtime.semantic_revision_v1(&request_scope).unwrap(),
            0
        );

        let winner_proposal = proposal(122, 0);
        let stale_proposal = proposal(123, 0);
        let mut commit_winner = || {
            winner_runtime
                .apply_perception_proposal_v1(&request_scope, &winner_proposal)
                .expect("winner commit");
        };
        let error = loser_runtime
            .apply_perception_proposal_v1_with_pre_commit_hook(
                &request_scope,
                &stale_proposal,
                &mut commit_winner,
            )
            .expect_err("different event must remain stale");
        assert!(matches!(
            error,
            RuntimeError::Store(StoreError::StaleRevision {
                expected: 0,
                actual: 1
            })
        ));
        let semantic_scope =
            r7_semantic_persona_scope(&request_scope.bot_token, &request_scope.persona_token);
        let winner_event = canonical_event_from_r7(&r7_perception_event(
            &request_scope,
            &winner_proposal,
            winner_proposal.estimator_digest_v1(&r7_scope_from_root(&request_scope)),
        ))
        .expect("winner event");
        let winner_digest = wire::event_digest(&winner_event);
        assert_eq!(
            loser_runtime
                .store
                .current_revision(&semantic_scope)
                .expect("durable semantic cursor"),
            1
        );
        let rows = loser_runtime
            .store
            .read_journal(&semantic_scope)
            .expect("semantic rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].event_digest, winner_digest);

        loser_runtime.flush_and_close().expect("close loser");
        winner_runtime.flush_and_close().expect("close winner");
        drop(loser_runtime);
        drop(winner_runtime);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn g0_receipt_metadata_is_identical_after_semantic_hydration() {
        let (mut clean_runtime, clean_scope) = runtime_for(131, "g0-clean-metadata");
        let (mut semantic_runtime, semantic_scope) = runtime_for(131, "g0-semantic-metadata");
        assert_eq!(clean_scope, semantic_scope);

        let baseline_event = g0_stimulus(131, 0, 90);
        clean_runtime
            .apply_event(&clean_scope, &baseline_event)
            .expect("clean baseline G0 event");
        semantic_runtime
            .apply_event(&semantic_scope, &baseline_event)
            .expect("semantic baseline G0 event");
        semantic_runtime
            .apply_perception_proposal_v1(&semantic_scope, &proposal(141, 0))
            .expect("semantic commit");
        let mut event = g0_stimulus(131, 1, 90);
        if let CanonicalEvent::UserStimulus(stimulus) = &mut event {
            stimulus.event_id = [142; 16];
            stimulus.causal.turn_id = [143; 16];
        }
        let clean = clean_runtime
            .apply_event(&clean_scope, &event)
            .expect("clean G0 event");
        let after_semantic = semantic_runtime
            .apply_event(&semantic_scope, &event)
            .expect("G0 event after semantic hydration");

        assert_eq!(
            after_semantic.receipt.state_before,
            clean.receipt.state_before
        );
        assert_eq!(
            after_semantic.receipt.state_after,
            clean.receipt.state_after
        );
        assert_eq!(
            after_semantic.receipt.graph_after,
            clean.receipt.graph_after
        );
        assert_eq!(
            after_semantic.receipt.active_nodes,
            clean.receipt.active_nodes
        );
        assert_eq!(
            after_semantic.receipt.active_edges,
            clean.receipt.active_edges
        );

        clean_runtime
            .flush_and_close()
            .expect("close clean runtime");
        semantic_runtime
            .flush_and_close()
            .expect("close semantic runtime");
        drop(clean_runtime);
        drop(semantic_runtime);
        cleanup_database("g0-clean-metadata");
        cleanup_database("g0-semantic-metadata");
    }

    #[test]
    fn durable_audit_rejects_tampered_state_before_continuity() {
        let path = database("tampered-state-before");
        let genesis = super::tests::request(93);
        let request_scope = scope(93, 192);
        let mut runtime = AstrRuntime::open(&path).expect("open runtime");
        let genesis_receipt = runtime.ensure_genesis(&genesis).expect("genesis");
        drop(runtime);

        let mut store = Store::open(&path).expect("open store");
        let semantic_scope =
            r7_semantic_persona_scope(&request_scope.bot_token, &request_scope.persona_token);
        let legacy_scope = wire::persona_scope_digest(
            &request_scope.bot_token,
            &request_scope.persona_token,
            None,
        );
        let genesis_snapshot = store
            .read_snapshot(&legacy_scope, 0)
            .expect("read genesis snapshot")
            .expect("genesis snapshot");
        let event = CanonicalEvent::TimeAdvance(ae_contracts::TimeAdvance {
            event_id: [111; 16],
            scope: request_scope.clone(),
            elapsed_ms: 1,
        });
        let receipt = TransitionReceipt {
            schema_version: 1,
            formula_digest: genesis_receipt.formula_digest,
            scope_digest: semantic_scope,
            event_digest: wire::event_digest(&event),
            authority_digest: authority_projection_digest(&event),
            base_revision: 0,
            next_revision: 1,
            state_before: [0xA5; 32],
            state_after: genesis_receipt.initial_snapshot_digest,
            graph_after: genesis_receipt.graph_digest,
            action_contract: None,
            active_nodes: 0,
            active_edges: 0,
            residuals: fixed_zero_vector(),
            status: CommitStatus::Committed,
        };
        store
            .commit_stateful_journal(&StatefulCommit {
                journal: CommitEnvelope {
                    event_kind: wire::event_kind_name(&event).to_owned(),
                    event_bytes: wire::encode_event(&event),
                    receipt,
                    chain_seed: genesis_receipt.initial_snapshot_digest,
                    delta_bytes: Vec::new(),
                },
                state_bytes: genesis_snapshot.state_bytes,
            })
            .expect("install tampered but otherwise self-consistent row");
        drop(store);

        let mut reopened = AstrRuntime::open(&path).expect("reopen tampered runtime");
        assert!(
            reopened
                .audit_durable_histories_v1(&request_scope.bot_token, &request_scope.persona_token,)
                .is_err(),
            "audit must reject a state-before chain break"
        );
        drop(reopened);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn invalid_proposals_fail_closed_without_advancing_semantic_cursor() {
        let (mut runtime, request_scope) = runtime_for(71, "invalid");
        let base = proposal(81, 0);
        let semantic_scope =
            r7_semantic_persona_scope(&request_scope.bot_token, &request_scope.persona_token);
        let legacy_scope = wire::persona_scope_digest(
            &request_scope.bot_token,
            &request_scope.persona_token,
            None,
        );
        let semantic_rows_before = runtime.store.read_journal(&semantic_scope).unwrap();
        let legacy_rows_before = runtime.store.read_journal(&legacy_scope).unwrap();
        assert!(semantic_rows_before.is_empty());
        assert!(legacy_rows_before.is_empty());
        let mut invalid = Vec::new();
        let mut schema = base.clone();
        schema.schema_version = 2;
        invalid.push(schema);
        let mut protocol = base.clone();
        protocol.protocol_version = 2;
        invalid.push(protocol);
        let mut zero_vector = base.clone();
        zero_vector.dimensions = EvidenceVector::default();
        invalid.push(zero_vector);
        let mut four_load_noop = base.clone();
        four_load_noop.dimensions = EvidenceVector {
            affiliation: Fixed::ONE,
            ..EvidenceVector::default()
        };
        invalid.push(four_load_noop);
        let mut negative = base.clone();
        negative.dimensions.positive = Fixed::from_raw(-1);
        invalid.push(negative);
        let mut out_of_range = base.clone();
        out_of_range.dimensions.positive = Fixed::from_raw(1_000_001);
        invalid.push(out_of_range);
        let mut zero_confidence = base.clone();
        zero_confidence.estimator_confidence = Fixed::ZERO;
        invalid.push(zero_confidence);
        let mut zero_nonce = base.clone();
        zero_nonce.request_nonce_digest = [0; 32];
        invalid.push(zero_nonce);
        let mut zero_event = base.clone();
        zero_event.event_id = [0; 16];
        invalid.push(zero_event);
        let mut zero_turn = base.clone();
        zero_turn.turn_id = [0; 16];
        invalid.push(zero_turn);
        let mut zero_observed_at = base.clone();
        zero_observed_at.observed_at_ms = 0;
        invalid.push(zero_observed_at);
        let mut stale = base.clone();
        stale.base_revision = 9;
        invalid.push(stale);

        for candidate in invalid {
            assert!(runtime
                .apply_perception_proposal_v1(&request_scope, &candidate)
                .is_err());
            assert_eq!(runtime.semantic_revision_v1(&request_scope).unwrap(), 0);
            assert_eq!(runtime.current_revision(&request_scope).unwrap(), 0);
            assert_eq!(
                runtime.store.read_journal(&semantic_scope).unwrap(),
                semantic_rows_before
            );
            assert_eq!(
                runtime.store.read_journal(&legacy_scope).unwrap(),
                legacy_rows_before
            );
            assert!(runtime
                .store
                .read_snapshot(&semantic_scope, 1)
                .unwrap()
                .is_none());
        }
        let invalid_scope = ScopeRef {
            bot_token: [0; 16],
            persona_token: request_scope.persona_token,
            relation_token: None,
            session_token: request_scope.session_token,
        };
        assert!(matches!(
            runtime.semantic_revision_v1(&invalid_scope),
            Err(RuntimeError::InvalidPerceptionScope)
        ));
        runtime
            .audit_durable_histories_v1(&request_scope.bot_token, &request_scope.persona_token)
            .expect("failed proposals leave both histories valid");
        runtime.flush_and_close().expect("close");
        drop(runtime);
        let _ = std::fs::remove_file(database("invalid"));
    }
}

// Former external R7/scaffold consumers are compiled here so private runtime
// coverage remains active without preserving a public alternate authority.
#[cfg(test)]
include!("../tests/user_stimulus_state_transition.rs");
#[cfg(test)]
include!("../tests/durable_semantic_authority.rs");
#[cfg(test)]
include!("../tests/astrbot_v4273_tool_private_boundary.rs");
#[cfg(test)]
include!("../tests/lark_public_effect_boundary.rs");
#[cfg(test)]
include!("../../ae-organism-runtime/tests/support/private_projection_runtime.rs");
#[cfg(test)]
include!("../../ae-organism-runtime/tests/committed_semantic_projection_path.rs");
#[cfg(test)]
include!("../../ae-organism-runtime/tests/private_projection_payload_producer.rs");

#[cfg(test)]
mod internal_user_stimulus_state_transition_tests {
    user_stimulus_state_transition_test_contents!();
}

#[cfg(test)]
mod internal_durable_semantic_authority_tests {
    durable_semantic_authority_test_contents!();
}

#[cfg(test)]
mod internal_astrbot_v4273_tool_private_boundary_tests {
    astrbot_v4273_tool_private_boundary_test_contents!();
}

#[cfg(test)]
mod internal_lark_public_effect_boundary_tests {
    lark_public_effect_boundary_test_contents!();
}

#[cfg(test)]
#[allow(dead_code, unused_imports)]
mod internal_committed_semantic_projection_path_tests {
    committed_semantic_projection_path_test_contents!();
}

#[cfg(test)]
#[allow(dead_code, unused_imports)]
mod internal_private_projection_payload_producer_tests {
    private_projection_payload_producer_test_contents!();
}
