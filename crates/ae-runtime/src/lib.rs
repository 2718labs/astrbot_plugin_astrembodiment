#![forbid(unsafe_code)]

//! AstrRuntime: the G0 vertical slice orchestrator.
//!
//! ensure_genesis -> deterministic no-op apply_event -> SQLite commit ->
//! replay verification. Python cannot reach any of this state directly; the
//! PyO3 surface exposes only coarse calls.

mod semantic;
pub mod semantic_dynamics_v2;
mod semantic_telemetry_v1;

pub use semantic::{
    ExpressionProfileFxP6, ExpressionProjectionV1, NodeObservabilityComponentV1,
    NodeObservabilityCountsV1, NodeObservabilityProjectionV1, NodeObservabilityRegionV1,
    NodeObservabilityResidualStateV1, NodeObservabilityResidualsV1,
};

use ae_agent::noop_action_contract;
use ae_authority::authority_projection_digest;
use ae_context_projector::{
    project_committed_receipt, ContextProjectionStateV1, ContextSummaryV1,
    DeliveryOutcome as ContextDeliveryOutcome, ReceiptCommitStatus, ReceiptEnvelopeV1,
    ReceiptValidationError, StoreError as ContextProjectorError, ValidatedCommittedReceiptV1,
};
use ae_continuum::{CommitEnvelope, ReplayReport};
use ae_contracts::{
    hex, perception_dimension_values, wire, ActionContract, CanonicalEvent, CausalRef,
    CommitStatus, Digest, GenesisManifestProposal, GenesisReceipt, GenesisStatus, Id128,
    InvariantResiduals, NativeTelemetryReceiptV1, PerceptionProposalV1, PersonaGenesisRequest,
    PersonalityVector, ScopeRef, SemanticEstimate, StateSubcodeV1, TransitionReceipt,
    TransitionReceiptV2, UserStimulus,
};
use ae_neurofield::{
    graph_digest, initial_state_from_manifest, state_digest, NeuralField, SparseGraph,
};
use ae_store::{
    phase0_formula_transition_delta_v1, ClaimOutcome, ContextCommitV1, ContinuityCommitBundleV1,
    GenesisCommit, GraphCommitV1, LegacySemanticFieldDomainUpgradeV1,
    LegacySemanticFormulaUpgradeReceiptV1, RebirthChildStageRequestV1, RebirthCommitPermitV1,
    RebirthLifecycleError, RebirthPreflightV1, RebirthPrepareRequestV1, RebirthPrepareResponseV1,
    RebirthResponseEnvelopeV1, SnapshotCommitV1, Store, StoreError, UserAuthorizedRebirthV1,
    VaultLifecycle, VaultMode, JOINT_MAX_LINEAR_FXP6_V1, LEGACY_FIELD_FXP6_SCALE,
    SEMANTIC_LANE_NAMESPACE_DOMAIN_V1,
};
use sha2::{Digest as Sha2Digest, Sha256};
use std::path::{Path, PathBuf};
use subtle::ConstantTimeEq;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("storage error: {0}")]
    Store(#[from] StoreError),
    #[error("genesis error: {0}")]
    Genesis(#[from] ae_genesis::GenesisError),
    #[error("rebirth lifecycle error: {0}")]
    Rebirth(#[from] RebirthLifecycleError),
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
    InvalidNeuralState(StateSubcodeV1),
    #[error("invalid closed semantic perception proposal")]
    InvalidPerceptionProposal,
    #[error("invalid semantic perception scope")]
    InvalidPerceptionScope,
    #[error("semantic event identity conflicts with a committed proposal")]
    SemanticIdentityConflict,
    #[error("semantic revision overflow")]
    SemanticRevisionOverflow,
    #[error("legacy semantic snapshot has no v2 attestation")]
    LegacyUnattested,
    #[error("context receipt validation error: {0}")]
    ContextReceipt(#[from] ReceiptValidationError),
    #[error("context projection error: {0}")]
    ContextProjection(#[from] ContextProjectorError),
    #[error("committed event is missing its context projection")]
    ContextCommitMissing,
    #[error("context projection does not match its committed integrity fence")]
    ContextCommitIntegrity,
}

impl RuntimeError {
    pub const fn invalid_neural_state(subcode: StateSubcodeV1) -> Self {
        Self::InvalidNeuralState(subcode)
    }
}

#[derive(Debug)]
pub struct ApplyDecision {
    pub contract: ActionContract,
    pub receipt: TransitionReceipt,
    pub revision: u64,
    pub context_summary: ContextSummaryV1,
    /// True when this exact event had already been applied; the state was not
    /// changed and the returned receipt is the originally committed one.
    pub deduplicated: bool,
}

#[derive(Clone, Debug)]
pub struct PerceptionProposalDecisionV1 {
    pub receipt: TransitionReceipt,
    pub semantic_vector_receipt: Option<TransitionReceiptV2>,
    pub semantic_telemetry_receipt: Option<NativeTelemetryReceiptV1>,
    pub node_observability: Option<NodeObservabilityProjectionV1>,
    pub revision: u64,
    pub deduplicated: bool,
    pub expression_projection: ExpressionProjectionV1,
    pub availability: SemanticClosureAvailabilityV1,
    /// Internal, closed migration telemetry.  The Python bridge intentionally
    /// keeps its established result schema while the native receipt remains
    /// the durable audit authority.
    pub field_migration: Option<SemanticFieldMigrationOutcomeV1>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticFieldMigrationOutcomeV1 {
    Applied,
    Replayed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticClosureAvailabilityV1 {
    Available,
    UnavailableLegacy,
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

struct HotBrain {
    bot_token: Id128,
    persona_token: Id128,
    persona_scope: Digest,
    identity: ae_genesis::GenesisIdentity,
    formula_digest: Digest,
    field: NeuralField,
    graph: SparseGraph,
    initial_snapshot_digest: Digest,
    revision: u64,
    semantic_scope: Digest,
    semantic_storage_scope: ScopeRef,
    semantic_field: NeuralField,
    semantic_graph: SparseGraph,
    semantic_revision: u64,
    semantic_legacy_upgrade: Option<LegacySemanticUpgradeSource>,
}

#[derive(Clone, Copy)]
struct LegacySemanticUpgradeSource {
    source_formula_digest: Digest,
    source_state_digest: Digest,
    source_graph_digest: Digest,
}

type SemanticSnapshot = (
    NeuralField,
    SparseGraph,
    Option<(TransitionReceipt, NativeTelemetryReceiptV1)>,
);

struct CommittedSemanticDecisionInput<'a> {
    semantic_scope: &'a Digest,
    formula_digest: &'a Digest,
    legacy_formula_digest: &'a Digest,
    baseline_field: &'a NeuralField,
    baseline_graph: &'a SparseGraph,
    manifest_digest: &'a Digest,
    development_seed_digest: &'a Digest,
    event_digest: &'a Digest,
    source_digest: Digest,
    proposal: &'a PerceptionProposalV1,
    deduplicated: bool,
}

struct LegacyAesem2FieldMigrationInput<'a> {
    semantic_scope: &'a Digest,
    semantic_storage_scope: &'a ScopeRef,
    legacy_formula_digest: &'a Digest,
    baseline_field: &'a NeuralField,
    baseline_graph: &'a SparseGraph,
    initial_snapshot_digest: Digest,
    semantic_revision: u64,
    source: Option<LegacySemanticUpgradeSource>,
    field: &'a NeuralField,
    graph: &'a SparseGraph,
}

pub struct AstrRuntime {
    store: Store,
    hot: Option<HotBrain>,
    legacy_authority_database: PathBuf,
    vault_root: PathBuf,
}

fn continuity_scope(scope: &ScopeRef) -> Digest {
    wire::persona_scope_digest(
        &scope.bot_token,
        &scope.persona_token,
        scope.relation_token.as_ref(),
    )
}

fn persona_scope_ref(bot_token: Id128, persona_token: Id128) -> ScopeRef {
    ScopeRef {
        bot_token,
        persona_token,
        relation_token: None,
        session_token: [0; 16],
    }
}

const REQUEST_NONCE_BINDING_DOMAIN_V1: &[u8] = b"astr-embodiment/spc1-request-nonce-binding-v1";

fn canonical_request_nonce_digest_v1(scope: &ScopeRef, proposal: &PerceptionProposalV1) -> Digest {
    let relation_token = scope
        .relation_token
        .as_ref()
        .map(|token| format!("\"{}\"", hex::encode16(token)))
        .unwrap_or_else(|| "null".to_owned());
    let scope_json = format!(
        "{{\"bot_token\":\"{}\",\"persona_token\":\"{}\",\"relation_token\":{},\"session_token\":\"{}\"}}",
        hex::encode16(&scope.bot_token),
        hex::encode16(&scope.persona_token),
        relation_token,
        hex::encode16(&scope.session_token),
    );
    let binding_json = format!(
        "{{\"base_revision\":{},\"event_id\":\"{}\",\"observed_at_ms\":{},\"scope\":{},\"turn_id\":\"{}\"}}",
        proposal.base_revision,
        hex::encode16(&proposal.event_id),
        proposal.observed_at_ms,
        scope_json,
        hex::encode16(&proposal.turn_id),
    );
    let mut hasher = Sha256::new();
    hasher.update(REQUEST_NONCE_BINDING_DOMAIN_V1);
    hasher.update([0]);
    hasher.update(binding_json.as_bytes());
    let digest: Digest = hasher.finalize().into();
    if digest != [0; 32] {
        return digest;
    }
    let mut fallback = Sha256::new();
    fallback.update(REQUEST_NONCE_BINDING_DOMAIN_V1);
    fallback.update([1]);
    fallback.update(binding_json.as_bytes());
    fallback.finalize().into()
}

fn request_nonce_binding_matches_v1(scope: &ScopeRef, proposal: &PerceptionProposalV1) -> bool {
    canonical_request_nonce_digest_v1(scope, proposal)
        .ct_eq(&proposal.request_nonce_digest)
        .into()
}

fn semantic_storage_scope(
    bot_token: Id128,
    persona_token: Id128,
    incarnation_id: &Digest,
    formula_digest: &Digest,
) -> ScopeRef {
    let root_scope = wire::persona_scope_digest(&bot_token, &persona_token, None);
    let binding = wire::domain_hash(
        SEMANTIC_LANE_NAMESPACE_DOMAIN_V1,
        &[&root_scope, incarnation_id, formula_digest],
    );
    let mut relation_token = [0; 16];
    relation_token.copy_from_slice(&binding[..16]);
    let mut session_token = [0; 16];
    session_token.copy_from_slice(&binding[16..]);
    ScopeRef {
        bot_token,
        persona_token,
        relation_token: Some(relation_token),
        session_token,
    }
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

fn perception_nonzero_dimension_count(proposal: &PerceptionProposalV1) -> u8 {
    perception_dimension_values(&proposal.dimensions)
        .into_iter()
        .filter(|value| *value != ae_fixed::Fixed::ZERO)
        .count() as u8
}

fn semantic_event(
    storage_scope: &ScopeRef,
    proposal: &PerceptionProposalV1,
    estimator_digest: Digest,
) -> CanonicalEvent {
    CanonicalEvent::UserStimulus(UserStimulus {
        event_id: proposal.event_id,
        scope: storage_scope.clone(),
        causal: CausalRef {
            turn_id: proposal.turn_id,
            action_id: None,
            delivery_id: None,
            claim_id: None,
            base_revision: proposal.base_revision,
        },
        observed_at_ms: proposal.observed_at_ms,
        evidence: SemanticEstimate {
            schema_version: proposal.schema_version,
            dimensions: proposal.dimensions.clone(),
            estimator_confidence: proposal.estimator_confidence,
            estimator_digest,
        },
    })
}

fn fully_confident_personality() -> PersonalityVector {
    PersonalityVector {
        baseline_warmth: ae_fixed::Fixed::ONE,
        baseline_patience: ae_fixed::Fixed::ONE,
        sensitivity: ae_fixed::Fixed::ONE,
        irritability: ae_fixed::Fixed::ONE,
        composure: ae_fixed::Fixed::ONE,
        epistemic_pride: ae_fixed::Fixed::ONE,
        epistemic_openness: ae_fixed::Fixed::ONE,
        boundary_strength: ae_fixed::Fixed::ONE,
        forgiveness: ae_fixed::Fixed::ONE,
        attachment_propensity: ae_fixed::Fixed::ONE,
        expression_drive: ae_fixed::Fixed::ONE,
        curiosity: ae_fixed::Fixed::ONE,
    }
}

impl AstrRuntime {
    pub fn open(path: &Path) -> Result<Self, RuntimeError> {
        let legacy_authority_database = path.to_path_buf();
        let storage_parent = path.parent().ok_or(RebirthLifecycleError::LocatorInvalid)?;
        std::fs::create_dir_all(storage_parent).map_err(|source| {
            RuntimeError::Store(StoreError::Io {
                context: "creating runtime storage directory",
                source,
            })
        })?;
        let vault_root = storage_parent.join("continuity-vault");
        let lifecycle = VaultLifecycle::open(&vault_root)?;
        let store = match lifecycle.vault_mode_v1()? {
            VaultMode::Unborn => Store::open(path)?,
            VaultMode::Ready => Store::open(&lifecycle.current_authority_database_path()?)?,
            VaultMode::Migrating
            | VaultMode::RecoveryRequired
            | VaultMode::ReadOnlyRecovery
            | VaultMode::WriteRefusedIncompatible => {
                return Err(RebirthLifecycleError::BootstrapConflict.into())
            }
        };
        Ok(Self {
            store,
            hot: None,
            legacy_authority_database,
            vault_root,
        })
    }

    fn lifecycle(&self) -> Result<VaultLifecycle, RuntimeError> {
        Ok(VaultLifecycle::open(&self.vault_root)?)
    }

    /// Select the Store named by the lifecycle owner, never by deriving or
    /// mutating locator state in runtime.  The old connection is flushed
    /// before it is dropped so bootstrap and explicit rebirth cannot lose a
    /// committed authority to a WAL-only view.
    fn reopen_authoritative_store(
        &mut self,
        lifecycle: &VaultLifecycle,
        scope: &ScopeRef,
    ) -> Result<(), RuntimeError> {
        self.store.flush()?;
        let database = lifecycle.current_authority_database_path()?;
        let store = Store::open(&database)?;
        self.store = store;
        self.hot = None;
        self.bind_hot(scope.bot_token, scope.persona_token)
    }

    /// The legacy direct Store becomes lifecycle authority only after it has
    /// already committed a real Genesis.  A Ready vault is selected through
    /// its owner; every recovery or incompatible state fails closed rather
    /// than falling back to the legacy file or manufacturing a birth.
    fn select_rebirth_authority(
        &mut self,
        scope: &ScopeRef,
    ) -> Result<VaultLifecycle, RuntimeError> {
        let lifecycle = self.lifecycle()?;
        match lifecycle.vault_mode_v1()? {
            VaultMode::Unborn => {
                self.store.flush()?;
                lifecycle.bootstrap_legacy_store_v1(&self.legacy_authority_database)?;
            }
            VaultMode::Ready => {}
            VaultMode::Migrating
            | VaultMode::RecoveryRequired
            | VaultMode::ReadOnlyRecovery
            | VaultMode::WriteRefusedIncompatible => {
                return Err(RebirthLifecycleError::BootstrapConflict.into())
            }
        }
        self.reopen_authoritative_store(&lifecycle, scope)?;
        Ok(lifecycle)
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
                self.select_rebirth_authority(&persona_scope_ref(
                    request.source.scope.bot_token,
                    request.source.scope.persona_token,
                ))?;
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
                    return Err(RuntimeError::invalid_neural_state(
                        StateSubcodeV1::BaselineStateInvalid,
                    ));
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
                    state_bytes: Self::encode_state(&field, &graph),
                    graph_digest,
                };

                match self.store.commit_genesis(&commit) {
                    Ok(()) => {
                        let semantic_storage_scope = semantic_storage_scope(
                            effective.source.scope.bot_token,
                            effective.source.scope.persona_token,
                            &identity.incarnation_id,
                            &effective.formula_digest,
                        );
                        let semantic_scope = continuity_scope(&semantic_storage_scope);
                        let semantic_field = field.clone();
                        let semantic_graph = graph.clone();
                        self.hot = Some(HotBrain {
                            bot_token: effective.source.scope.bot_token,
                            persona_token: effective.source.scope.persona_token,
                            persona_scope: wire::persona_scope_digest(
                                &effective.source.scope.bot_token,
                                &effective.source.scope.persona_token,
                                None,
                            ),
                            identity,
                            formula_digest: effective.formula_digest,
                            field,
                            graph,
                            initial_snapshot_digest,
                            revision: 0,
                            semantic_scope,
                            semantic_storage_scope,
                            semantic_field,
                            semantic_graph,
                            semantic_revision: 0,
                            semantic_legacy_upgrade: None,
                        });
                        self.select_rebirth_authority(&persona_scope_ref(
                            request.source.scope.bot_token,
                            request.source.scope.persona_token,
                        ))?;
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
                        self.select_rebirth_authority(&persona_scope_ref(
                            request.source.scope.bot_token,
                            request.source.scope.persona_token,
                        ))?;
                        Ok(committed.receipt)
                    }
                    Err(other) => Err(RuntimeError::Store(other)),
                }
            }
        }
    }

    fn encode_state(field: &NeuralField, graph: &SparseGraph) -> Vec<u8> {
        // G0 snapshot bytes: the canonical fixed-layout field encoding plus
        // the graph body; nothing else is needed to re-derive every digest.
        let mut body = Vec::with_capacity(16_384 * 8 * 8 + 65_540);
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
        body
    }

    fn bind_hot(&mut self, bot_token: Id128, persona_token: Id128) -> Result<(), RuntimeError> {
        let committed = self
            .store
            .lookup_bound_genesis(&bot_token, &persona_token)?
            .ok_or(RuntimeError::PersonaGenesisRequired)?;
        let (field, graph) = initial_state_from_manifest(
            &committed.manifest,
            &committed.receipt.formula_digest,
            &committed.receipt.development_seed_digest,
        );
        let identity = ae_genesis::GenesisIdentity {
            manifest: committed.manifest,
            manifest_digest: committed.receipt.manifest_digest,
            seed_code_digest: committed.receipt.seed_code_digest,
            incarnation_id: committed.receipt.incarnation_id,
            development_seed_digest: committed.receipt.development_seed_digest,
        };
        let persona_scope = wire::persona_scope_digest(&bot_token, &persona_token, None);
        let revision = self.store.current_revision(&persona_scope)?;
        let semantic_storage_scope = semantic_storage_scope(
            bot_token,
            persona_token,
            &identity.incarnation_id,
            &committed.receipt.formula_digest,
        );
        let semantic_scope = continuity_scope(&semantic_storage_scope);
        let semantic_revision = self.store.current_revision(&semantic_scope)?;
        let semantic_formula_digest =
            semantic::phase0_semantic_formula_digest_v1(&committed.receipt.formula_digest)?;
        let (semantic_field, semantic_graph, semantic_legacy_upgrade) = if semantic_revision == 0 {
            (field.clone(), graph.clone(), None)
        } else {
            let row = self
                .store
                .read_journal(&semantic_scope)?
                .into_iter()
                .find(|row| row.revision == semantic_revision)
                .ok_or(RuntimeError::LegacyUnattested)?;
            let receipt = row
                .decode_receipt()
                .map_err(|error| RuntimeError::Store(StoreError::Sqlite(error.to_string())))?;
            let snapshot = self
                .store
                .read_snapshot(&semantic_scope, semantic_revision)?
                .ok_or(RuntimeError::LegacyUnattested)?;
            if semantic::snapshot_is_aesem2(&snapshot.state_bytes) {
                // The frozen AESEM2 decoder is still authoritative for the
                // historical field and graph.  A fresh proposal may cross
                // to Phase 0 only through the Store's one-time receipt.
                if receipt.schema_version != 1
                    || receipt.status != CommitStatus::Committed
                    || receipt.action_contract.is_some()
                    || receipt.scope_digest != semantic_scope
                    || receipt.formula_digest != committed.receipt.formula_digest
                    || receipt.next_revision != semantic_revision
                    || receipt.base_revision.checked_add(1) != Some(semantic_revision)
                    || snapshot.state_digest != receipt.state_after
                {
                    return Err(RuntimeError::LegacyUnattested);
                }
                let (legacy_field, legacy_graph, _) = semantic::decode_semantic_snapshot_v2(
                    &snapshot.state_bytes,
                    &receipt.formula_digest,
                    &receipt.state_after,
                    &receipt.graph_after,
                    &receipt,
                )?;
                (
                    legacy_field,
                    legacy_graph,
                    Some(LegacySemanticUpgradeSource {
                        source_formula_digest: receipt.formula_digest,
                        source_state_digest: snapshot.state_digest,
                        source_graph_digest: receipt.graph_after,
                    }),
                )
            } else {
                if receipt.schema_version != 1
                    || receipt.status != CommitStatus::Committed
                    || receipt.action_contract.is_some()
                    || receipt.scope_digest != semantic_scope
                    || receipt.formula_digest != semantic_formula_digest
                    || receipt.next_revision != semantic_revision
                {
                    return Err(RuntimeError::LegacyUnattested);
                }
                let (field, graph, _) = semantic::decode_semantic_snapshot_v3(
                    &snapshot.state_bytes,
                    &semantic_formula_digest,
                    &receipt.state_after,
                    &receipt.graph_after,
                    &receipt,
                )?;
                (field, graph, None)
            }
        };
        self.hot = Some(HotBrain {
            bot_token,
            persona_token,
            persona_scope,
            identity,
            formula_digest: committed.receipt.formula_digest,
            field,
            graph,
            initial_snapshot_digest: committed.receipt.initial_snapshot_digest,
            revision,
            semantic_scope,
            semantic_storage_scope,
            semantic_field,
            semantic_graph,
            semantic_revision,
            semantic_legacy_upgrade,
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
            let (semantic_scope, semantic_revision) = {
                let hot = self
                    .hot
                    .as_ref()
                    .ok_or(RuntimeError::PersonaGenesisRequired)?;
                (hot.semantic_scope, hot.semantic_revision)
            };
            if self.store.current_revision(&semantic_scope)? != semantic_revision {
                self.bind_hot(scope.bot_token, scope.persona_token)?;
            }
        }
        self.hot
            .as_mut()
            .ok_or(RuntimeError::PersonaGenesisRequired)
    }

    fn committed_context_receipt(
        event: &CanonicalEvent,
        relation_scope_token: Id128,
        source_continuum_revision: u64,
    ) -> Result<ValidatedCommittedReceiptV1, RuntimeError> {
        let (event_id, dimensions_fxp6, unresolved_boundary, unresolved_repair, delivery_outcome) =
            match event {
                CanonicalEvent::UserStimulus(stimulus) => {
                    let dimensions = &stimulus.evidence.dimensions;
                    let bounded = |value: ae_fixed::Fixed| {
                        value
                            .raw()
                            .clamp(0, ValidatedCommittedReceiptV1::MAX_DIMENSION_FXP6)
                    };
                    (
                        stimulus.event_id,
                        [
                            bounded(dimensions.positive),
                            bounded(dimensions.affiliation),
                            bounded(dimensions.harm),
                            bounded(dimensions.boundary),
                            bounded(dimensions.repair),
                            bounded(dimensions.repetition),
                            bounded(dimensions.new_information),
                            bounded(dimensions.constraint_instability),
                            bounded(dimensions.epistemic_conflict),
                            bounded(dimensions.self_responsibility),
                            bounded(dimensions.other_responsibility),
                            bounded(dimensions.hostility),
                            bounded(dimensions.publicness),
                            bounded(dimensions.engagement),
                            bounded(dimensions.rejection),
                        ],
                        dimensions.boundary.raw() > 0,
                        dimensions.repair.raw() > 0,
                        ContextDeliveryOutcome::Pending,
                    )
                }
                CanonicalEvent::DeliveryOutcome(outcome) => (
                    outcome.event_id,
                    [0; 15],
                    false,
                    false,
                    if outcome.delivered {
                        ContextDeliveryOutcome::Delivered
                    } else {
                        ContextDeliveryOutcome::Failed
                    },
                ),
                CanonicalEvent::TimeAdvance(advance) => (
                    advance.event_id,
                    [0; 15],
                    false,
                    false,
                    ContextDeliveryOutcome::Pending,
                ),
                _ => return Err(RuntimeError::UnsupportedEvent(wire::event_kind_name(event))),
            };
        Ok(ValidatedCommittedReceiptV1::try_from_envelope(
            ReceiptEnvelopeV1 {
                commit_status: ReceiptCommitStatus::Committed,
                event_id,
                relation_token: relation_scope_token,
                source_continuum_revision,
                dimensions_fxp6,
                unresolved_boundary,
                unresolved_repair,
                repetition_increment: 1,
                delivery_outcome,
            },
        )?)
    }

    fn context_summary_for_persona_scope(
        &self,
        persona_scope: &Digest,
        relation_scope_token: &Id128,
    ) -> Result<Option<ContextSummaryV1>, RuntimeError> {
        let Some(row) = self
            .store
            .read_context_commit(persona_scope, relation_scope_token)?
        else {
            return Ok(None);
        };
        if row.scope_digest != *persona_scope || row.relation_scope_token != *relation_scope_token {
            return Err(RuntimeError::ContextCommitIntegrity);
        }
        if ae_store::continuity_context_digest(&row.canonical_state_bytes) != row.context_digest {
            return Err(RuntimeError::ContextCommitIntegrity);
        }
        let projection =
            ContextProjectionStateV1::try_from_canonical_state_bytes(&row.canonical_state_bytes)?;
        if projection.relation_hmac() != row.relation_hmac
            || projection.summary().source_continuum_revision != row.revision
        {
            return Err(RuntimeError::ContextCommitIntegrity);
        }
        Ok(Some(projection.summary().clone()))
    }

    /// Return the committed aggregate-only context for the relation selected
    /// by this scope.  The Store remains the authority: absent or malformed
    /// bytes never fall back to an in-memory or standalone projector state.
    pub fn context_summary_for_scope(
        &mut self,
        scope: &ScopeRef,
    ) -> Result<Option<ContextSummaryV1>, RuntimeError> {
        self.hot_for(scope)?;
        let continuity_scope = continuity_scope(scope);
        let relation_scope_token = scope.relation_token.unwrap_or(scope.session_token);
        self.context_summary_for_persona_scope(&continuity_scope, &relation_scope_token)
    }

    // ------------------------------------------------------------- rebirth

    /// Build the complete revision-zero child transaction from the currently
    /// selected parent.  The lifecycle owner supplies both child identity and
    /// nonce digest through its permit helpers; runtime never retains or
    /// persists a raw confirmation nonce.
    fn fresh_child_genesis(
        &mut self,
        scope: &ScopeRef,
        permit: &RebirthCommitPermitV1,
    ) -> Result<GenesisCommit, RuntimeError> {
        if permit.scope_token != continuity_scope(scope) {
            return Err(RebirthLifecycleError::FenceStale.into());
        }
        self.hot_for(scope)?;
        let committed = self
            .store
            .lookup_bound_genesis(&scope.bot_token, &scope.persona_token)?
            .ok_or(RuntimeError::PersonaGenesisRequired)?;
        if committed.receipt.incarnation_id != permit.parent_authority.incarnation_id
            || self.store.current_revision(&permit.scope_token)? != permit.parent_authority.revision
        {
            return Err(RebirthLifecycleError::FenceStale.into());
        }

        let compiled_at_ms = committed.born_at_ms;
        let parent_receipt = committed.receipt;
        let source = committed.source;
        let mut manifest = committed.manifest;
        if wire::manifest_body_digest(&manifest) != parent_receipt.manifest_digest {
            return Err(RebirthLifecycleError::ChildInvalid.into());
        }
        // The durable manifest body deliberately excludes its self-digest;
        // restore the receipt-attested digest before comparing it to a freshly
        // derived GenesisIdentity.
        manifest.manifest_digest = parent_receipt.manifest_digest;
        let child_nonce_digest = VaultLifecycle::child_genesis_nonce_digest_for_permit(permit);
        let child_request = PersonaGenesisRequest {
            source: source.clone(),
            proposal: GenesisManifestProposal {
                schema_version: manifest.schema_version,
                source: source.clone(),
                traits: manifest.traits.clone(),
                trait_confidence: fully_confident_personality(),
                expression: manifest.expression.clone(),
                allostasis: manifest.allostasis.clone(),
                epistemic: manifest.epistemic.clone(),
                social: manifest.social.clone(),
                compiler_protocol_digest: parent_receipt.compiler_protocol_digest,
                compiler_model_digest: parent_receipt.compiler_model_digest,
            },
            formula_digest: parent_receipt.formula_digest,
            incarnation_nonce: child_nonce_digest,
            parent_incarnation_id: Some(permit.parent_authority.incarnation_id),
            observed_at_ms: compiled_at_ms,
        };
        let child_identity =
            ae_genesis::derive_identity(&child_request, &ae_genesis::GenesisPrior::default())?;
        if child_identity.manifest != manifest
            || child_identity.seed_code_digest != parent_receipt.seed_code_digest
            || child_identity.incarnation_id == permit.parent_authority.incarnation_id
        {
            return Err(RebirthLifecycleError::ChildInvalid.into());
        }
        let (field, graph) = initial_state_from_manifest(
            &child_identity.manifest,
            &parent_receipt.formula_digest,
            &child_identity.development_seed_digest,
        );
        if !field.validate() || !graph.validate() {
            return Err(RuntimeError::invalid_neural_state(
                StateSubcodeV1::BaselineStateInvalid,
            ));
        }
        let initial_snapshot_digest = state_digest(&field, &parent_receipt.formula_digest);
        let initial_graph_digest = graph_digest(&graph);
        let receipt = GenesisReceipt {
            schema_version: 1,
            seed_code_digest: child_identity.seed_code_digest,
            manifest_digest: child_identity.manifest_digest,
            incarnation_id: child_identity.incarnation_id,
            formula_digest: parent_receipt.formula_digest,
            persona_source_digest: parent_receipt.persona_source_digest,
            compiler_protocol_digest: parent_receipt.compiler_protocol_digest,
            compiler_model_digest: parent_receipt.compiler_model_digest,
            development_seed_digest: child_identity.development_seed_digest,
            initial_snapshot_digest,
            graph_digest: initial_graph_digest,
            equilibrium_residual: ae_fixed::Fixed::ZERO,
            energy_residual: ae_fixed::Fixed::ZERO,
            capacity_residual: ae_fixed::Fixed::ZERO,
            sample_fit_residual: ae_fixed::Fixed::ZERO,
            status: GenesisStatus::Committed,
        };
        Ok(GenesisCommit {
            scope_key: ae_genesis::genesis_scope_key(
                &source.scope.bot_token,
                &source.scope.persona_token,
                &source.source_digest,
                &parent_receipt.formula_digest,
            ),
            // Store owns child lease allocation and overwrites this field
            // while staging its non-authoritative candidate generation.
            lease_epoch: 0,
            nonce_digest: child_nonce_digest,
            manifest_body: wire::encode_manifest_body(&child_identity.manifest),
            seed_code_digest: child_identity.seed_code_digest,
            incarnation_id: child_identity.incarnation_id,
            formula_digest: parent_receipt.formula_digest,
            source,
            compiler_protocol_digest: parent_receipt.compiler_protocol_digest,
            compiler_model_digest: parent_receipt.compiler_model_digest,
            compiled_at_ms,
            receipt,
            initial_snapshot_digest,
            state_bytes: Self::encode_state(&field, &graph),
            graph_digest: initial_graph_digest,
            manifest: child_identity.manifest,
        })
    }

    /// First explicit destructive action: create only a durable challenge.
    /// The caller's scope token must exactly name the active lifecycle lane.
    pub fn prepare_rebirth_v1(
        &mut self,
        scope: &ScopeRef,
        request: &RebirthPrepareRequestV1,
    ) -> Result<RebirthPrepareResponseV1, RuntimeError> {
        if request.scope_token != continuity_scope(scope) {
            return Err(RebirthLifecycleError::FenceStale.into());
        }
        self.hot_for(scope)?;
        let lifecycle = self.select_rebirth_authority(scope)?;
        Ok(lifecycle.prepare_rebirth(request.clone())?)
    }

    /// Second explicit destructive action: preflight/replay in the lifecycle
    /// owner, stage exactly one complete child, then atomically switch its
    /// authority.  A replay is returned before child staging.
    pub fn confirm_rebirth_v1(
        &mut self,
        scope: &ScopeRef,
        confirmation: &UserAuthorizedRebirthV1,
    ) -> Result<RebirthResponseEnvelopeV1, RuntimeError> {
        if confirmation.scope_token != continuity_scope(scope) {
            return Err(RebirthLifecycleError::FenceStale.into());
        }
        self.hot_for(scope)?;
        let lifecycle = self.select_rebirth_authority(scope)?;
        match lifecycle.preflight_rebirth_confirmation(confirmation)? {
            RebirthPreflightV1::Replayed(envelope) => Ok(envelope),
            RebirthPreflightV1::Stage(permit) => {
                let genesis = self.fresh_child_genesis(scope, &permit)?;
                let child = lifecycle
                    .stage_rebirth_child_v1(&permit, RebirthChildStageRequestV1 { genesis })?;
                let envelope = lifecycle.commit_rebirth(&permit, &child)?;
                self.reopen_authoritative_store(&lifecycle, scope)?;
                Ok(envelope)
            }
        }
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
            formula_digest,
            manifest_digest,
            initial_snapshot_digest,
            state_before,
            graph_after,
            snapshot_state_bytes,
            graph_replay_state_bytes,
            active_nodes,
            active_edges,
        ) = {
            let hot = self.hot_for(scope)?;
            (
                hot.bot_token,
                hot.persona_token,
                hot.formula_digest,
                hot.identity.manifest_digest,
                hot.initial_snapshot_digest,
                state_digest(&hot.field, &hot.formula_digest),
                graph_digest(&hot.graph),
                Self::encode_state(&hot.field, &hot.graph),
                hot.graph.canonical_bytes(),
                hot.field.active_node_count(),
                hot.graph.edges.len() as u32,
            )
        };
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
        let relation_scope_token = event_scope
            .relation_token
            .unwrap_or(event_scope.session_token);
        let continuity_scope = continuity_scope(event_scope);
        let current_revision = self.store.current_revision(&continuity_scope)?;

        let event_bytes = wire::encode_event(event);
        let event_digest = wire::event_digest(event);
        let contract = noop_action_contract(&manifest_digest, &event_digest, turn_id);
        let contract_digest = wire::action_contract_digest(&contract);

        // Idempotency: an event that was already applied is never applied
        // twice; the original receipt is returned unchanged.
        if let Some(row) = self.store.lookup_event(&continuity_scope, &event_digest)? {
            let receipt = row
                .decode_receipt()
                .map_err(|error| RuntimeError::Store(StoreError::Sqlite(error.to_string())))?;
            let context_summary = self
                .context_summary_for_persona_scope(&continuity_scope, &relation_scope_token)?
                .ok_or(RuntimeError::ContextCommitMissing)?;
            return Ok(ApplyDecision {
                contract,
                receipt,
                revision: row.revision,
                context_summary,
                deduplicated: true,
            });
        }

        let causal_base = match event {
            CanonicalEvent::UserStimulus(e) => e.causal.base_revision,
            CanonicalEvent::DeliveryOutcome(e) => e.causal.base_revision,
            CanonicalEvent::TimeAdvance(_) => current_revision,
            _ => unreachable!(),
        };
        if causal_base != current_revision {
            return Err(RuntimeError::StaleCausalBase {
                expected: current_revision,
                actual: causal_base,
            });
        }

        let authority_digest = authority_projection_digest(event);
        let receipt = TransitionReceipt {
            schema_version: 1,
            formula_digest,
            scope_digest: continuity_scope,
            event_digest,
            authority_digest,
            base_revision: current_revision,
            next_revision: current_revision + 1,
            state_before,
            state_after: state_before,
            graph_after,
            action_contract: Some(contract_digest),
            active_nodes,
            active_edges,
            residuals: InvariantResiduals::default(),
            status: CommitStatus::Committed,
        };

        let context_receipt =
            Self::committed_context_receipt(event, relation_scope_token, receipt.next_revision)?;
        let previous_context = self
            .store
            .read_context_commit(&continuity_scope, &relation_scope_token)?;
        if previous_context.is_some()
            && self
                .context_summary_for_persona_scope(&continuity_scope, &relation_scope_token)?
                .is_none()
        {
            return Err(RuntimeError::ContextCommitIntegrity);
        }
        let context_projection = project_committed_receipt(
            previous_context
                .as_ref()
                .map(|row| row.canonical_state_bytes.as_slice()),
            &context_receipt,
        )?;
        let context_summary = context_projection.summary().clone();
        let canonical_context_state = context_projection.canonical_state_bytes();
        let context_commit = ContextCommitV1 {
            relation_scope_token,
            relation_hmac: context_projection.relation_hmac(),
            source_continuum_revision: receipt.next_revision,
            context_digest: ae_store::continuity_context_digest(&canonical_context_state),
            canonical_state_bytes: canonical_context_state,
        };

        // An empty journal is the normal first-turn case: start the chain at
        // the committed Genesis snapshot. Only a store error should fail the
        // event; ``Ok(None)`` is not evidence that Genesis is missing.
        let chain_seed = self
            .store
            .last_chain_digest(&continuity_scope)?
            .unwrap_or(initial_snapshot_digest);
        let envelope = CommitEnvelope {
            event_kind: wire::event_kind_name(event).to_string(),
            event_bytes,
            receipt: receipt.clone(),
            chain_seed,
            delta_bytes: vec![],
        };
        let bundle = ContinuityCommitBundleV1 {
            envelope,
            snapshot: SnapshotCommitV1 {
                state_digest: state_before,
                state_bytes: snapshot_state_bytes,
            },
            graph: GraphCommitV1 {
                base_graph_digest: graph_after,
                graph_digest: graph_after,
                formula_digest,
                delta_bytes: vec![],
                replay_state_bytes: graph_replay_state_bytes,
            },
            context: context_commit,
        };

        match self.store.commit_continuity_bundle(&bundle) {
            Ok((revision, _row)) => {
                if let Some(hot) = self.hot.as_mut() {
                    if hot.persona_scope == continuity_scope {
                        hot.revision = revision;
                    }
                }
                Ok(ApplyDecision {
                    contract,
                    receipt,
                    revision,
                    context_summary,
                    deduplicated: false,
                })
            }
            Err(StoreError::DuplicateEvent(revision)) => {
                let row = self
                    .store
                    .lookup_event(&continuity_scope, &event_digest)?
                    .ok_or(RuntimeError::RetryWait)?;
                let receipt = row
                    .decode_receipt()
                    .map_err(|error| RuntimeError::Store(StoreError::Sqlite(error.to_string())))?;
                let context_summary = self
                    .context_summary_for_persona_scope(&continuity_scope, &relation_scope_token)?
                    .ok_or(RuntimeError::ContextCommitMissing)?;
                Ok(ApplyDecision {
                    contract,
                    receipt,
                    revision,
                    context_summary,
                    deduplicated: true,
                })
            }
            Err(other) => Err(RuntimeError::Store(other)),
        }
    }

    fn semantic_snapshot_at(
        &self,
        semantic_scope: &Digest,
        formula_digest: &Digest,
        baseline_field: &NeuralField,
        baseline_graph: &SparseGraph,
        revision: u64,
    ) -> Result<SemanticSnapshot, RuntimeError> {
        if revision == 0 {
            return Ok((baseline_field.clone(), baseline_graph.clone(), None));
        }
        let row = self
            .store
            .read_journal(semantic_scope)?
            .into_iter()
            .find(|row| row.revision == revision)
            .ok_or(RuntimeError::LegacyUnattested)?;
        let receipt = row
            .decode_receipt()
            .map_err(|error| RuntimeError::Store(StoreError::Sqlite(error.to_string())))?;
        if row.scope_digest != *semantic_scope
            || row.base_revision != receipt.base_revision
            || receipt.schema_version != 1
            || receipt.status != CommitStatus::Committed
            || receipt.action_contract.is_some()
            || receipt.scope_digest != *semantic_scope
            || receipt.next_revision != revision
            || receipt.base_revision.checked_add(1) != Some(revision)
        {
            return Err(RuntimeError::LegacyUnattested);
        }
        let snapshot = self
            .store
            .read_snapshot(semantic_scope, revision)?
            .ok_or(RuntimeError::LegacyUnattested)?;
        if snapshot.state_digest != receipt.state_after {
            return Err(RuntimeError::invalid_neural_state(
                StateSubcodeV1::SnapshotAttestationMismatch,
            ));
        }
        if semantic::snapshot_is_aesem2(&snapshot.state_bytes) {
            let (field, graph, _) = semantic::decode_semantic_snapshot_v2(
                &snapshot.state_bytes,
                &receipt.formula_digest,
                &receipt.state_after,
                &receipt.graph_after,
                &receipt,
            )?;
            return Ok((field, graph, None));
        }
        if receipt.formula_digest != *formula_digest {
            return Err(RuntimeError::LegacyUnattested);
        }
        let (field, graph, telemetry_receipt) = semantic::decode_semantic_snapshot_v3(
            &snapshot.state_bytes,
            formula_digest,
            &receipt.state_after,
            &receipt.graph_after,
            &receipt,
        )?;
        Ok((field, graph, Some((receipt, telemetry_receipt))))
    }

    fn legacy_field_domain_metadata(
        normalization: semantic::LegacyFieldDomainNormalizationV1,
    ) -> LegacySemanticFieldDomainUpgradeV1 {
        LegacySemanticFieldDomainUpgradeV1 {
            algorithm: JOINT_MAX_LINEAR_FXP6_V1,
            fxp6_scale: LEGACY_FIELD_FXP6_SCALE,
            source_common_max: normalization.source_common_max,
            out_of_range_count: normalization.out_of_range_count,
            potential_out_of_range_count: normalization.potential_out_of_range_count,
            excitation_out_of_range_count: normalization.excitation_out_of_range_count,
            signal_mass_before: normalization.signal_mass_before,
            signal_mass_after: normalization.signal_mass_after,
        }
    }

    /// Full replay of the only old writer that can be normalized.  The caller
    /// invokes this only after the latest AESEM2 field proves it needs the
    /// finite P/E transform; all other legacy states retain the normal strict
    /// failure behavior.
    fn attest_legacy_aesem2_field_history(
        &self,
        input: &LegacyAesem2FieldMigrationInput<'_>,
        source: LegacySemanticUpgradeSource,
    ) -> Result<(), RuntimeError> {
        if input.semantic_revision == 0
            || !input.baseline_field.validate()
            || !input.baseline_graph.validate()
        {
            return Err(RuntimeError::LegacyUnattested);
        }
        let rows = self.store.read_journal(input.semantic_scope)?;
        if u64::try_from(rows.len()).ok() != Some(input.semantic_revision) {
            return Err(RuntimeError::LegacyUnattested);
        }
        let mut replay_field = input.baseline_field.clone();
        let replay_graph = input.baseline_graph.clone();
        let mut chain_seed = input.initial_snapshot_digest;
        for (index, row) in rows.iter().enumerate() {
            let revision = u64::try_from(index)
                .ok()
                .and_then(|value| value.checked_add(1))
                .ok_or(RuntimeError::LegacyUnattested)?;
            if row.revision != revision || row.base_revision.checked_add(1) != Some(revision) {
                return Err(RuntimeError::LegacyUnattested);
            }
            let event =
                wire::decode_event(&row.event_bytes).map_err(|_| RuntimeError::LegacyUnattested)?;
            if wire::encode_event(&event) != row.event_bytes
                || wire::event_digest(&event) != row.event_digest
            {
                return Err(RuntimeError::LegacyUnattested);
            }
            let CanonicalEvent::UserStimulus(stimulus) = event else {
                return Err(RuntimeError::LegacyUnattested);
            };
            if stimulus.scope != *input.semantic_storage_scope
                || stimulus.causal.base_revision != row.base_revision
                || stimulus.evidence.schema_version != PerceptionProposalV1::SCHEMA_VERSION
            {
                return Err(RuntimeError::LegacyUnattested);
            }
            let receipt = row
                .decode_receipt()
                .map_err(|_| RuntimeError::LegacyUnattested)?;
            if wire::encode_transition_receipt(&receipt) != row.receipt_bytes
                || receipt.schema_version != 1
                || receipt.status != CommitStatus::Committed
                || receipt.action_contract.is_some()
                || receipt.scope_digest != *input.semantic_scope
                || receipt.event_digest != row.event_digest
                || receipt.formula_digest != *input.legacy_formula_digest
                || receipt.base_revision != row.base_revision
                || receipt.next_revision != revision
                || receipt.authority_digest
                    != authority_projection_digest(&CanonicalEvent::UserStimulus(stimulus.clone()))
                || row.chain_digest
                    != ae_continuum::chain_link(&chain_seed, &row.event_bytes, &row.receipt_bytes)
            {
                return Err(RuntimeError::LegacyUnattested);
            }
            let snapshot = self
                .store
                .read_snapshot(input.semantic_scope, revision)?
                .ok_or(RuntimeError::LegacyUnattested)?;
            if !semantic::snapshot_is_aesem2(&snapshot.state_bytes)
                || snapshot.state_digest != receipt.state_after
            {
                return Err(RuntimeError::LegacyUnattested);
            }
            let (snapshot_field, snapshot_graph, _) = semantic::decode_semantic_snapshot_v2(
                &snapshot.state_bytes,
                input.legacy_formula_digest,
                &receipt.state_after,
                &receipt.graph_after,
                &receipt,
            )?;
            let replay = semantic::replay_legacy_aesem2_transition_v1(
                &replay_field,
                input.baseline_field,
                &stimulus.evidence.dimensions,
                stimulus.evidence.estimator_confidence,
            )?;
            let theoretical_limit = i128::from(revision)
                .checked_add(1)
                .and_then(|value| value.checked_mul(i128::from(semantic::LEGACY_FIELD_FXP6_SCALE)))
                .ok_or(RuntimeError::LegacyUnattested)?;
            let p_and_e_in_theoretical_domain = snapshot_field
                .potential
                .iter()
                .chain(snapshot_field.excitation.iter())
                .all(|value| {
                    let raw = value.raw();
                    raw >= 0 && i128::from(raw) <= theoretical_limit
                });
            if !p_and_e_in_theoretical_domain
                || state_digest(&snapshot_field, input.legacy_formula_digest)
                    != state_digest(&replay.next_field, input.legacy_formula_digest)
                || graph_digest(&snapshot_graph) != graph_digest(&replay_graph)
                || receipt.state_before != state_digest(&replay_field, input.legacy_formula_digest)
                || receipt.state_after
                    != state_digest(&replay.next_field, input.legacy_formula_digest)
                || receipt.graph_after != graph_digest(&replay_graph)
                || receipt.active_nodes != replay.active_nodes
                || receipt.active_edges != 0
                || receipt.residuals != InvariantResiduals::default()
            {
                return Err(RuntimeError::LegacyUnattested);
            }
            replay_field = replay.next_field;
            chain_seed = row.chain_digest;
        }
        if state_digest(&replay_field, input.legacy_formula_digest)
            != state_digest(input.field, input.legacy_formula_digest)
            || graph_digest(&replay_graph) != graph_digest(input.graph)
            || state_digest(input.field, input.legacy_formula_digest) != source.source_state_digest
            || graph_digest(input.graph) != source.source_graph_digest
            || chain_seed
                != self
                    .store
                    .last_chain_digest(input.semantic_scope)?
                    .ok_or(RuntimeError::LegacyUnattested)?
        {
            return Err(RuntimeError::LegacyUnattested);
        }
        Ok(())
    }

    fn normalize_attested_legacy_aesem2_field(
        &self,
        input: LegacyAesem2FieldMigrationInput<'_>,
    ) -> Result<(NeuralField, Option<LegacySemanticFieldDomainUpgradeV1>), RuntimeError> {
        let Some((normalized, normalization)) =
            semantic::normalize_legacy_aesem2_field_domain_v1(input.field)?
        else {
            return Ok((input.field.clone(), None));
        };
        let source = input.source.ok_or(RuntimeError::LegacyUnattested)?;
        if source.source_formula_digest != *input.legacy_formula_digest {
            return Err(RuntimeError::LegacyUnattested);
        }
        self.attest_legacy_aesem2_field_history(&input, source)?;
        Ok((
            normalized,
            Some(Self::legacy_field_domain_metadata(normalization)),
        ))
    }

    fn semantic_identity_conflict(
        &self,
        semantic_scope: &Digest,
        event_id: &Id128,
        event_digest: &Digest,
    ) -> Result<bool, RuntimeError> {
        for row in self.store.read_journal(semantic_scope)? {
            let event = wire::decode_event(&row.event_bytes)
                .map_err(|error| RuntimeError::Store(StoreError::Sqlite(error.to_string())))?;
            if let CanonicalEvent::UserStimulus(stimulus) = event {
                if stimulus.event_id == *event_id && row.event_digest != *event_digest {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn committed_semantic_decision(
        &self,
        input: CommittedSemanticDecisionInput<'_>,
    ) -> Result<PerceptionProposalDecisionV1, RuntimeError> {
        let CommittedSemanticDecisionInput {
            semantic_scope,
            formula_digest,
            legacy_formula_digest,
            baseline_field,
            baseline_graph,
            manifest_digest,
            development_seed_digest,
            event_digest,
            source_digest,
            proposal,
            deduplicated,
        } = input;
        let row = self
            .store
            .lookup_event(semantic_scope, event_digest)?
            .ok_or(RuntimeError::RetryWait)?;
        let receipt = row
            .decode_receipt()
            .map_err(|error| RuntimeError::Store(StoreError::Sqlite(error.to_string())))?;
        let snapshot = self
            .store
            .read_snapshot(semantic_scope, row.revision)?
            .ok_or(RuntimeError::LegacyUnattested)?;
        if semantic::snapshot_is_aesem2(&snapshot.state_bytes) {
            let (legacy_field, _, _) = semantic::decode_semantic_snapshot_v2(
                &snapshot.state_bytes,
                &receipt.formula_digest,
                &receipt.state_after,
                &receipt.graph_after,
                &receipt,
            )?;
            return Ok(PerceptionProposalDecisionV1 {
                expression_projection: semantic::expression_projection_from_field_v1(
                    &legacy_field,
                    row.revision,
                )?,
                receipt,
                semantic_vector_receipt: None,
                semantic_telemetry_receipt: None,
                node_observability: None,
                revision: row.revision,
                deduplicated,
                availability: SemanticClosureAvailabilityV1::UnavailableLegacy,
                field_migration: None,
            });
        }
        if receipt.event_digest != *event_digest
            || receipt.scope_digest != *semantic_scope
            || receipt.formula_digest != *formula_digest
            || receipt.status != CommitStatus::Committed
            || receipt.action_contract.is_some()
            || receipt.next_revision != row.revision
        {
            return Err(RuntimeError::SemanticIdentityConflict);
        }
        let (before, before_graph, _) = self.semantic_snapshot_at(
            semantic_scope,
            formula_digest,
            baseline_field,
            baseline_graph,
            receipt.base_revision,
        )?;
        let (before_for_phase0, field_migration) =
            match semantic::normalize_legacy_aesem2_field_domain_v1(&before)? {
                None => (before.clone(), None),
                Some((normalized, normalization)) => {
                    let upgrade = self
                        .store
                        .read_legacy_semantic_formula_upgrade_v1(
                            semantic_scope,
                            legacy_formula_digest,
                            formula_digest,
                        )?
                        .ok_or(RuntimeError::LegacyUnattested)?;
                    if upgrade.base_revision != receipt.base_revision
                        || upgrade.next_revision != receipt.next_revision
                        || upgrade.event_digest != *event_digest
                        || upgrade.source_state_digest
                            != state_digest(&before, legacy_formula_digest)
                        || upgrade.source_graph_digest != graph_digest(&before_graph)
                        || upgrade.target_state_before != receipt.state_before
                        || upgrade.field_domain
                            != Some(Self::legacy_field_domain_metadata(normalization))
                    {
                        return Err(RuntimeError::LegacyUnattested);
                    }
                    (normalized, Some(SemanticFieldMigrationOutcomeV1::Replayed))
                }
            };
        let (after, after_graph, telemetry_receipt) = self.semantic_snapshot_at(
            semantic_scope,
            formula_digest,
            baseline_field,
            baseline_graph,
            row.revision,
        )?;
        let telemetry_receipt = telemetry_receipt
            .map(|(_, receipt)| receipt)
            .ok_or(RuntimeError::LegacyUnattested)?;
        let prepared = semantic::prepare_semantic_transition_v2(
            &before_for_phase0,
            baseline_field,
            &before_graph,
            manifest_digest,
            development_seed_digest,
            proposal,
        )?;
        if state_digest(&prepared.next_field, formula_digest) != receipt.state_after
            || graph_digest(&prepared.next_graph) != receipt.graph_after
            || prepared.active_nodes != receipt.active_nodes
            || state_digest(&after, formula_digest)
                != state_digest(&prepared.next_field, formula_digest)
            || graph_digest(&after_graph) != graph_digest(&prepared.next_graph)
        {
            return Err(RuntimeError::SemanticIdentityConflict);
        }
        let expected_telemetry = semantic_telemetry_v1::prepare_native_telemetry_v1(
            *formula_digest,
            *semantic_scope,
            *event_digest,
            source_digest,
            receipt.base_revision,
            receipt.next_revision,
            state_digest(&before_for_phase0, formula_digest),
            state_digest(&after, formula_digest),
            graph_digest(&before_graph),
            graph_digest(&after_graph),
            &prepared.local_by_region,
            &prepared.dynamics,
            &prepared.full_vector_load,
        )?;
        if telemetry_receipt != expected_telemetry {
            return Err(RuntimeError::SemanticIdentityConflict);
        }
        let expected_semantic_receipt = semantic::semantic_vector_receipt_v2(
            &receipt,
            prepared.full_vector_load.evaluated_dimension_count,
            prepared.full_vector_load.injected_dimension_count,
            perception_nonzero_dimension_count(proposal),
        )?;
        let node_observability =
            semantic::node_observability_projection_v2(&before_for_phase0, &after, row.revision)?;
        if (node_observability.counts.changed_node_count > 0)
            != expected_semantic_receipt.semantic_vector.state_changed
        {
            return Err(RuntimeError::invalid_neural_state(
                StateSubcodeV1::SemanticClosureInvalid,
            ));
        }
        let expression_projection =
            semantic::expression_projection_from_field_v1(&after, row.revision)?;
        Ok(PerceptionProposalDecisionV1 {
            receipt,
            semantic_vector_receipt: Some(expected_semantic_receipt),
            semantic_telemetry_receipt: Some(telemetry_receipt),
            node_observability: Some(node_observability),
            revision: row.revision,
            deduplicated,
            expression_projection,
            availability: SemanticClosureAvailabilityV1::Available,
            field_migration,
        })
    }

    /// Read the independent per-persona semantic cursor. This never aliases
    /// the ordinary G0 continuity revision.
    pub fn semantic_revision_v1(&mut self, scope: &ScopeRef) -> Result<u64, RuntimeError> {
        validate_perception_scope(scope)?;
        Ok(self.hot_for(scope)?.semantic_revision)
    }

    /// Validate and atomically apply a closed fifteen-dimensional semantic
    /// proposal. It owns no provider, text, policy, tool, or send authority.
    pub fn apply_perception_proposal_v1(
        &mut self,
        scope: &ScopeRef,
        proposal: &PerceptionProposalV1,
    ) -> Result<PerceptionProposalDecisionV1, RuntimeError> {
        validate_perception_scope(scope)?;
        proposal
            .validate_v1()
            .map_err(|_| RuntimeError::InvalidPerceptionProposal)?;
        if !request_nonce_binding_matches_v1(scope, proposal) {
            return Err(RuntimeError::InvalidPerceptionProposal);
        }
        let nonzero_evidence_dimension_count = perception_nonzero_dimension_count(proposal);
        let (
            hot_bot_token,
            hot_persona_token,
            semantic_scope,
            semantic_storage_scope,
            semantic_revision,
            genesis_formula_digest,
            initial_snapshot_digest,
            manifest,
            manifest_digest,
            development_seed_digest,
            field,
            graph,
            semantic_legacy_upgrade,
        ) = {
            let hot = self.hot_for(scope)?;
            (
                hot.bot_token,
                hot.persona_token,
                hot.semantic_scope,
                hot.semantic_storage_scope.clone(),
                hot.semantic_revision,
                hot.formula_digest,
                hot.initial_snapshot_digest,
                hot.identity.manifest.clone(),
                hot.identity.manifest_digest,
                hot.identity.development_seed_digest,
                hot.semantic_field.clone(),
                hot.semantic_graph.clone(),
                hot.semantic_legacy_upgrade,
            )
        };
        if scope.bot_token != hot_bot_token || scope.persona_token != hot_persona_token {
            return Err(RuntimeError::GenesisManifestMismatch);
        }
        let (baseline_field, baseline_graph) = initial_state_from_manifest(
            &manifest,
            &genesis_formula_digest,
            &development_seed_digest,
        );
        if !baseline_field.validate() || !baseline_graph.validate() {
            return Err(RuntimeError::invalid_neural_state(
                StateSubcodeV1::BaselineStateInvalid,
            ));
        }
        let formula_digest = semantic::phase0_semantic_formula_digest_v1(&genesis_formula_digest)?;
        let estimator_digest = proposal.estimator_digest_v1(scope);
        let event = semantic_event(&semantic_storage_scope, proposal, estimator_digest);
        let event_digest = wire::event_digest(&event);

        if self
            .store
            .lookup_event(&semantic_scope, &event_digest)?
            .is_some()
        {
            return self.committed_semantic_decision(CommittedSemanticDecisionInput {
                semantic_scope: &semantic_scope,
                formula_digest: &formula_digest,
                legacy_formula_digest: &genesis_formula_digest,
                baseline_field: &baseline_field,
                baseline_graph: &baseline_graph,
                manifest_digest: &manifest_digest,
                development_seed_digest: &development_seed_digest,
                event_digest: &event_digest,
                source_digest: estimator_digest,
                proposal,
                deduplicated: true,
            });
        }
        if self.semantic_identity_conflict(&semantic_scope, &proposal.event_id, &event_digest)? {
            return Err(RuntimeError::SemanticIdentityConflict);
        }
        if proposal.base_revision != semantic_revision {
            return Err(RuntimeError::StaleCausalBase {
                expected: semantic_revision,
                actual: proposal.base_revision,
            });
        }

        let (field_for_phase0, field_domain_upgrade) = self
            .normalize_attested_legacy_aesem2_field(LegacyAesem2FieldMigrationInput {
                semantic_scope: &semantic_scope,
                semantic_storage_scope: &semantic_storage_scope,
                legacy_formula_digest: &genesis_formula_digest,
                baseline_field: &baseline_field,
                baseline_graph: &baseline_graph,
                initial_snapshot_digest,
                semantic_revision,
                source: semantic_legacy_upgrade,
                field: &field,
                graph: &graph,
            })?;

        let prepared = semantic::prepare_semantic_transition_v2(
            &field_for_phase0,
            &baseline_field,
            &graph,
            &manifest_digest,
            &development_seed_digest,
            proposal,
        )?;
        let next_revision = semantic_revision
            .checked_add(1)
            .ok_or(RuntimeError::SemanticRevisionOverflow)?;
        let state_before = state_digest(&field_for_phase0, &formula_digest);
        let state_after = state_digest(&prepared.next_field, &formula_digest);
        let graph_before = graph_digest(&graph);
        let graph_after = graph_digest(&prepared.next_graph);
        let telemetry_receipt = semantic_telemetry_v1::prepare_native_telemetry_v1(
            formula_digest,
            semantic_scope,
            event_digest,
            estimator_digest,
            semantic_revision,
            next_revision,
            state_before,
            state_after,
            graph_before,
            graph_after,
            &prepared.local_by_region,
            &prepared.dynamics,
            &prepared.full_vector_load,
        )?;
        let receipt = TransitionReceipt {
            schema_version: 1,
            formula_digest,
            scope_digest: semantic_scope,
            event_digest,
            authority_digest: authority_projection_digest(&event),
            base_revision: semantic_revision,
            next_revision,
            state_before,
            state_after,
            graph_after,
            action_contract: None,
            active_nodes: prepared.active_nodes,
            active_edges: prepared.dynamics.propagated_edge_count,
            residuals: telemetry_receipt.residuals.clone(),
            status: CommitStatus::Committed,
        };
        let semantic_vector_receipt = semantic::semantic_vector_receipt_v2(
            &receipt,
            prepared.full_vector_load.evaluated_dimension_count,
            prepared.full_vector_load.injected_dimension_count,
            nonzero_evidence_dimension_count,
        )?;
        let node_observability = semantic::node_observability_projection_v2(
            &field_for_phase0,
            &prepared.next_field,
            next_revision,
        )?;
        if (node_observability.counts.changed_node_count > 0)
            != semantic_vector_receipt.semantic_vector.state_changed
        {
            return Err(RuntimeError::invalid_neural_state(
                StateSubcodeV1::SemanticClosureInvalid,
            ));
        }
        let state_bytes = semantic::encode_semantic_snapshot_v3(
            &formula_digest,
            &prepared.next_field,
            &prepared.next_graph,
            &telemetry_receipt,
        )?;
        let _ = semantic::decode_semantic_snapshot_v3(
            &state_bytes,
            &formula_digest,
            &state_after,
            &graph_after,
            &receipt,
        )?;

        let relation_scope_token =
            semantic_storage_scope
                .relation_token
                .ok_or(RuntimeError::invalid_neural_state(
                    StateSubcodeV1::RelationScopeMissing,
                ))?;
        let context_receipt =
            Self::committed_context_receipt(&event, relation_scope_token, next_revision)?;
        let previous_context = self
            .store
            .read_context_commit(&semantic_scope, &relation_scope_token)?;
        if previous_context.is_some()
            && self
                .context_summary_for_persona_scope(&semantic_scope, &relation_scope_token)?
                .is_none()
        {
            return Err(RuntimeError::ContextCommitIntegrity);
        }
        let context_projection = project_committed_receipt(
            previous_context
                .as_ref()
                .map(|row| row.canonical_state_bytes.as_slice()),
            &context_receipt,
        )?;
        let canonical_context_state = context_projection.canonical_state_bytes();
        let context = ContextCommitV1 {
            relation_scope_token,
            relation_hmac: context_projection.relation_hmac(),
            source_continuum_revision: next_revision,
            context_digest: ae_store::continuity_context_digest(&canonical_context_state),
            canonical_state_bytes: canonical_context_state,
        };
        let chain_seed = self
            .store
            .last_chain_digest(&semantic_scope)?
            .unwrap_or(initial_snapshot_digest);
        let formula_transition_delta = if let Some(upgrade_source) = semantic_legacy_upgrade {
            if semantic_revision == 0
                || upgrade_source.source_formula_digest != genesis_formula_digest
                || formula_digest
                    != semantic::phase0_semantic_formula_digest_v1(
                        &upgrade_source.source_formula_digest,
                    )?
                || state_digest(&field, &upgrade_source.source_formula_digest)
                    != upgrade_source.source_state_digest
                || graph_before != upgrade_source.source_graph_digest
            {
                return Err(RuntimeError::LegacyUnattested);
            }
            match field_domain_upgrade {
                Some(field_domain) => {
                    LegacySemanticFormulaUpgradeReceiptV1::from_transition_receipt_with_field_domain(
                        &receipt,
                        upgrade_source.source_state_digest,
                        upgrade_source.source_graph_digest,
                        upgrade_source.source_formula_digest,
                        chain_seed,
                        field_domain,
                    )
                }
                None => LegacySemanticFormulaUpgradeReceiptV1::from_transition_receipt(
                    &receipt,
                    upgrade_source.source_state_digest,
                    upgrade_source.source_graph_digest,
                    upgrade_source.source_formula_digest,
                    chain_seed,
                ),
            }
            .canonical_bytes()
        } else if semantic_revision == 0 && formula_digest != genesis_formula_digest {
            phase0_formula_transition_delta_v1(&receipt, graph_before, genesis_formula_digest)
        } else {
            vec![]
        };
        let bundle = ContinuityCommitBundleV1 {
            envelope: CommitEnvelope {
                event_kind: wire::event_kind_name(&event).to_owned(),
                event_bytes: wire::encode_event(&event),
                receipt: receipt.clone(),
                chain_seed,
                delta_bytes: formula_transition_delta.clone(),
            },
            snapshot: SnapshotCommitV1 {
                state_digest: state_after,
                state_bytes,
            },
            graph: GraphCommitV1 {
                base_graph_digest: graph_before,
                graph_digest: graph_after,
                formula_digest,
                delta_bytes: formula_transition_delta,
                replay_state_bytes: prepared.next_graph.canonical_bytes(),
            },
            context,
        };

        match self.store.commit_continuity_bundle(&bundle) {
            Ok((revision, _)) if revision == next_revision => {
                let expression_projection =
                    semantic::expression_projection_from_field_v1(&prepared.next_field, revision)?;
                if let Some(hot) = self.hot.as_mut() {
                    if hot.semantic_scope == semantic_scope {
                        hot.semantic_field = prepared.next_field;
                        hot.semantic_graph = prepared.next_graph;
                        hot.semantic_revision = revision;
                        hot.semantic_legacy_upgrade = None;
                    }
                }
                Ok(PerceptionProposalDecisionV1 {
                    receipt,
                    semantic_vector_receipt: Some(semantic_vector_receipt),
                    semantic_telemetry_receipt: Some(telemetry_receipt),
                    node_observability: Some(node_observability),
                    revision,
                    deduplicated: false,
                    expression_projection,
                    availability: SemanticClosureAvailabilityV1::Available,
                    field_migration: field_domain_upgrade
                        .map(|_| SemanticFieldMigrationOutcomeV1::Applied),
                })
            }
            Ok((_revision, _)) => {
                self.bind_hot(scope.bot_token, scope.persona_token)?;
                self.committed_semantic_decision(CommittedSemanticDecisionInput {
                    semantic_scope: &semantic_scope,
                    formula_digest: &formula_digest,
                    legacy_formula_digest: &genesis_formula_digest,
                    baseline_field: &baseline_field,
                    baseline_graph: &baseline_graph,
                    manifest_digest: &manifest_digest,
                    development_seed_digest: &development_seed_digest,
                    event_digest: &event_digest,
                    source_digest: estimator_digest,
                    proposal,
                    deduplicated: true,
                })
            }
            Err(stale @ StoreError::StaleRevision { .. }) => {
                if self
                    .store
                    .lookup_event(&semantic_scope, &event_digest)?
                    .is_none()
                {
                    return Err(RuntimeError::Store(stale));
                }
                self.bind_hot(scope.bot_token, scope.persona_token)?;
                self.committed_semantic_decision(CommittedSemanticDecisionInput {
                    semantic_scope: &semantic_scope,
                    formula_digest: &formula_digest,
                    legacy_formula_digest: &genesis_formula_digest,
                    baseline_field: &baseline_field,
                    baseline_graph: &baseline_graph,
                    manifest_digest: &manifest_digest,
                    development_seed_digest: &development_seed_digest,
                    event_digest: &event_digest,
                    source_digest: estimator_digest,
                    proposal,
                    deduplicated: true,
                })
            }
            Err(error) => Err(RuntimeError::Store(error)),
        }
    }

    // ------------------------------------------------------------ observatory

    pub fn inspect(
        &mut self,
        bot_token: &Id128,
        persona_token: &Id128,
    ) -> Result<InspectReport, RuntimeError> {
        let bound = self
            .store
            .lookup_bound_genesis(bot_token, persona_token)?
            .map(|committed| {
                let persona_scope = wire::persona_scope_digest(bot_token, persona_token, None);
                let revision = self.store.current_revision(&persona_scope).unwrap_or(0);
                let last_chain = self.store.last_chain_digest(&persona_scope).unwrap_or(None);
                let journal_count = self.store.count_journal().unwrap_or(0);
                InspectReport {
                    bound: true,
                    bot_token: *bot_token,
                    persona_token: *persona_token,
                    seed_code: ae_genesis::format_seed_code(&committed.receipt.seed_code_digest),
                    seed_code_short: ae_genesis::format_short_seed_code(
                        &committed.receipt.seed_code_digest,
                    ),
                    incarnation_id: ae_genesis::format_incarnation_id(
                        &committed.receipt.incarnation_id,
                    ),
                    revision,
                    initial_snapshot_digest: committed.receipt.initial_snapshot_digest,
                    last_chain_digest: last_chain,
                    journal_count,
                    observatory_genesis_unavailable: false,
                }
            })
            .unwrap_or(InspectReport {
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
        Ok(bound)
    }

    pub fn verify_replay(
        &mut self,
        bot_token: &Id128,
        persona_token: &Id128,
    ) -> Result<ReplayReport, RuntimeError> {
        let committed = self
            .store
            .lookup_bound_genesis(bot_token, persona_token)?
            .ok_or(RuntimeError::PersonaGenesisRequired)?;
        let persona_scope = wire::persona_scope_digest(bot_token, persona_token, None);
        let rows = self.store.read_journal(&persona_scope)?;
        Ok(ae_continuum::verify_replay(
            committed.receipt.initial_snapshot_digest,
            &rows,
        ))
    }

    /// Drain the writer: snapshot the current state, checkpoint WAL and close
    /// the store. Later calls fail with Closed.
    pub fn flush_and_close(&mut self) -> Result<(), RuntimeError> {
        if let Some(hot) = self.hot.take() {
            let state = state_digest(&hot.field, &hot.formula_digest);
            self.store.write_snapshot(
                &hot.persona_scope,
                hot.revision,
                &state,
                &Self::encode_state(&hot.field, &hot.graph),
            )?;
        }
        self.store.flush()?;
        Ok(())
    }

    pub fn closed(&self) -> bool {
        matches!(self.store.count_leases(), Err(StoreError::Closed))
    }

    pub fn current_revision(&mut self, scope: &ScopeRef) -> Result<u64, RuntimeError> {
        self.hot_for(scope)?;
        Ok(self.store.current_revision(&continuity_scope(scope))?)
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
        GenesisManifestProposal, PerceptionProposalV1, PersonaScopeRef, PersonaSelectionKind,
        PersonaSourceRef, PersonalityVector, SemanticEstimate, SocialPriors, UserStimulus,
    };
    use ae_fixed::Fixed;

    fn request(seed: u8) -> PersonaGenesisRequest {
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

    fn semantic_proposal(scope: &ScopeRef, seed: u8, base_revision: u64) -> PerceptionProposalV1 {
        let mut proposal = PerceptionProposalV1 {
            schema_version: 1,
            event_id: [seed; 16],
            turn_id: [seed.wrapping_add(1); 16],
            observed_at_ms: 1_700_000_000_200 + u64::from(seed),
            base_revision,
            dimensions: EvidenceVector::default(),
            estimator_confidence: Fixed::ONE,
            protocol_version: 1,
            request_nonce_digest: [1; 32],
        };
        proposal.request_nonce_digest = canonical_request_nonce_digest_v1(scope, &proposal);
        proposal
    }

    struct LegacyAesem2History {
        semantic_scope: Digest,
        legacy_formula_digest: Digest,
        phase0_formula_digest: Digest,
        latest_field: NeuralField,
        latest_graph: SparseGraph,
        latest_state_digest: Digest,
        latest_graph_digest: Digest,
        latest_chain_digest: Digest,
    }

    /// Build a persisted r=2 predecessor lane using the frozen AESEM2 wire
    /// layout.  It deliberately uses the pre-Phase-0 formula attached to the
    /// active incarnation and is reopened through the current runtime.
    fn seed_legacy_aesem2_history(
        runtime: &mut AstrRuntime,
        request: &PersonaGenesisRequest,
        scope: &ScopeRef,
        inject_known_legacy_field_overflow: bool,
    ) -> LegacyAesem2History {
        let (
            semantic_scope,
            semantic_storage_scope,
            legacy_formula_digest,
            phase0_formula_digest,
            initial_snapshot_digest,
            baseline_field,
            baseline_graph,
        ) = {
            let hot = runtime.hot_for(scope).expect("genesis binds hot state");
            (
                hot.semantic_scope,
                hot.semantic_storage_scope.clone(),
                hot.formula_digest,
                semantic::phase0_semantic_formula_digest_v1(&hot.formula_digest)
                    .expect("Phase-0 formula derives"),
                hot.initial_snapshot_digest,
                hot.semantic_field.clone(),
                hot.semantic_graph.clone(),
            )
        };
        assert_eq!(legacy_formula_digest, request.formula_digest);
        assert_ne!(legacy_formula_digest, phase0_formula_digest);

        let relation_scope_token = semantic_storage_scope
            .relation_token
            .expect("semantic lane owns a relation token");
        let mut field = baseline_field.clone();
        let graph = baseline_graph.clone();
        let mut latest_state_digest = state_digest(&field, &legacy_formula_digest);
        let mut latest_graph_digest = graph_digest(&graph);
        let mut latest_chain_digest = initial_snapshot_digest;

        for base_revision in 0..2_u64 {
            let mut proposal = semantic_proposal(scope, 90 + base_revision as u8, base_revision);
            if inject_known_legacy_field_overflow {
                proposal.dimensions = EvidenceVector {
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
                proposal.request_nonce_digest = canonical_request_nonce_digest_v1(scope, &proposal);
            }
            let estimator_digest = proposal.estimator_digest_v1(scope);
            let event = semantic_event(&semantic_storage_scope, &proposal, estimator_digest);
            let event_digest = wire::event_digest(&event);
            let prepared =
                semantic::prepare_legacy_aesem2_transition_v1(&field, &baseline_field, &proposal)
                    .expect("predecessor fixture transition is valid");
            let next_revision = base_revision + 1;
            let next_field = prepared.next_field;
            let state_before = state_digest(&field, &legacy_formula_digest);
            let state_after = state_digest(&next_field, &legacy_formula_digest);
            let graph_before = graph_digest(&graph);
            let graph_after = graph_digest(&graph);
            let receipt = TransitionReceipt {
                schema_version: 1,
                formula_digest: legacy_formula_digest,
                scope_digest: semantic_scope,
                event_digest,
                authority_digest: authority_projection_digest(&event),
                base_revision,
                next_revision,
                state_before,
                state_after,
                graph_after,
                action_contract: None,
                active_nodes: prepared.active_nodes,
                active_edges: 0,
                residuals: InvariantResiduals::default(),
                status: CommitStatus::Committed,
            };
            let semantic_receipt = semantic::semantic_vector_receipt_v2(
                &receipt,
                15,
                15,
                perception_nonzero_dimension_count(&proposal),
            )
            .expect("frozen semantic receipt closes");
            let state_bytes = semantic::encode_semantic_snapshot_v2_for_test(
                &legacy_formula_digest,
                &next_field,
                &graph,
                &semantic_receipt,
            )
            .expect("frozen AESEM2 snapshot encodes");
            let decoded = semantic::decode_semantic_snapshot_v2(
                &state_bytes,
                &legacy_formula_digest,
                &state_after,
                &graph_after,
                &receipt,
            )
            .expect("frozen AESEM2 snapshot replays");
            assert_eq!(
                state_digest(&decoded.0, &legacy_formula_digest),
                state_after
            );
            assert_eq!(graph_digest(&decoded.1), graph_after);

            let context_receipt =
                AstrRuntime::committed_context_receipt(&event, relation_scope_token, next_revision)
                    .expect("legacy context receipt closes");
            let previous_context = runtime
                .store
                .read_context_commit(&semantic_scope, &relation_scope_token)
                .expect("read predecessor context");
            let context_projection = project_committed_receipt(
                previous_context
                    .as_ref()
                    .map(|row| row.canonical_state_bytes.as_slice()),
                &context_receipt,
            )
            .expect("legacy context projection closes");
            let canonical_context_state = context_projection.canonical_state_bytes();
            let bundle = ContinuityCommitBundleV1 {
                envelope: CommitEnvelope {
                    event_kind: wire::event_kind_name(&event).to_owned(),
                    event_bytes: wire::encode_event(&event),
                    receipt: receipt.clone(),
                    chain_seed: latest_chain_digest,
                    delta_bytes: vec![],
                },
                snapshot: SnapshotCommitV1 {
                    state_digest: state_after,
                    state_bytes,
                },
                graph: GraphCommitV1 {
                    base_graph_digest: graph_before,
                    graph_digest: graph_after,
                    formula_digest: legacy_formula_digest,
                    delta_bytes: vec![],
                    replay_state_bytes: graph.canonical_bytes(),
                },
                context: ContextCommitV1 {
                    relation_scope_token,
                    relation_hmac: context_projection.relation_hmac(),
                    source_continuum_revision: next_revision,
                    context_digest: ae_store::continuity_context_digest(&canonical_context_state),
                    canonical_state_bytes: canonical_context_state,
                },
            };
            let (committed_revision, row) = runtime
                .store
                .commit_continuity_bundle(&bundle)
                .expect("frozen AESEM2 authority commits");
            assert_eq!(committed_revision, next_revision);
            latest_chain_digest = row.chain_digest;
            latest_state_digest = state_after;
            latest_graph_digest = graph_after;
            field = next_field;
        }

        LegacyAesem2History {
            semantic_scope,
            legacy_formula_digest,
            phase0_formula_digest,
            latest_field: field,
            latest_graph: graph,
            latest_state_digest,
            latest_graph_digest,
            latest_chain_digest,
        }
    }

    fn legacy_upgrade_bundle_for_test(
        runtime: &mut AstrRuntime,
        history: &LegacyAesem2History,
        scope: &ScopeRef,
        proposal: &PerceptionProposalV1,
        from_formula_digest: Digest,
    ) -> ContinuityCommitBundleV1 {
        let (semantic_storage_scope, manifest_digest, development_seed_digest, baseline_field) = {
            let hot = runtime.hot_for(scope).expect("legacy state rebinds");
            let (baseline_field, _) = initial_state_from_manifest(
                &hot.identity.manifest,
                &hot.formula_digest,
                &hot.identity.development_seed_digest,
            );
            (
                hot.semantic_storage_scope.clone(),
                hot.identity.manifest_digest,
                hot.identity.development_seed_digest,
                baseline_field,
            )
        };
        let estimator_digest = proposal.estimator_digest_v1(scope);
        let event = semantic_event(&semantic_storage_scope, proposal, estimator_digest);
        let event_digest = wire::event_digest(&event);
        let prepared = semantic::prepare_semantic_transition_v2(
            &history.latest_field,
            &baseline_field,
            &history.latest_graph,
            &manifest_digest,
            &development_seed_digest,
            proposal,
        )
        .expect("candidate proposal is valid");
        let state_before = state_digest(&history.latest_field, &history.phase0_formula_digest);
        let state_after = state_digest(&prepared.next_field, &history.phase0_formula_digest);
        let graph_before = graph_digest(&history.latest_graph);
        let graph_after = graph_digest(&prepared.next_graph);
        let telemetry_receipt = semantic_telemetry_v1::prepare_native_telemetry_v1(
            history.phase0_formula_digest,
            history.semantic_scope,
            event_digest,
            estimator_digest,
            proposal.base_revision,
            proposal.base_revision + 1,
            state_before,
            state_after,
            graph_before,
            graph_after,
            &prepared.local_by_region,
            &prepared.dynamics,
            &prepared.full_vector_load,
        )
        .expect("candidate telemetry closes");
        let receipt = TransitionReceipt {
            schema_version: 1,
            formula_digest: history.phase0_formula_digest,
            scope_digest: history.semantic_scope,
            event_digest,
            authority_digest: authority_projection_digest(&event),
            base_revision: proposal.base_revision,
            next_revision: proposal.base_revision + 1,
            state_before,
            state_after,
            graph_after,
            action_contract: None,
            active_nodes: prepared.active_nodes,
            active_edges: prepared.dynamics.propagated_edge_count,
            residuals: telemetry_receipt.residuals.clone(),
            status: CommitStatus::Committed,
        };
        let state_bytes = semantic::encode_semantic_snapshot_v3(
            &history.phase0_formula_digest,
            &prepared.next_field,
            &prepared.next_graph,
            &telemetry_receipt,
        )
        .expect("candidate AESEM3 snapshot encodes");
        let relation_scope_token = semantic_storage_scope
            .relation_token
            .expect("semantic lane owns a relation token");
        let context_receipt = AstrRuntime::committed_context_receipt(
            &event,
            relation_scope_token,
            receipt.next_revision,
        )
        .expect("candidate context receipt closes");
        let previous_context = runtime
            .store
            .read_context_commit(&history.semantic_scope, &relation_scope_token)
            .expect("read legacy context");
        let context_projection = project_committed_receipt(
            previous_context
                .as_ref()
                .map(|row| row.canonical_state_bytes.as_slice()),
            &context_receipt,
        )
        .expect("candidate context projection closes");
        let canonical_context_state = context_projection.canonical_state_bytes();
        let upgrade = LegacySemanticFormulaUpgradeReceiptV1::from_transition_receipt(
            &receipt,
            history.latest_state_digest,
            history.latest_graph_digest,
            from_formula_digest,
            history.latest_chain_digest,
        );
        let delta_bytes = upgrade.canonical_bytes();
        ContinuityCommitBundleV1 {
            envelope: CommitEnvelope {
                event_kind: wire::event_kind_name(&event).to_owned(),
                event_bytes: wire::encode_event(&event),
                receipt: receipt.clone(),
                chain_seed: history.latest_chain_digest,
                delta_bytes: delta_bytes.clone(),
            },
            snapshot: SnapshotCommitV1 {
                state_digest: state_after,
                state_bytes,
            },
            graph: GraphCommitV1 {
                base_graph_digest: graph_before,
                graph_digest: graph_after,
                formula_digest: history.phase0_formula_digest,
                delta_bytes,
                replay_state_bytes: prepared.next_graph.canonical_bytes(),
            },
            context: ContextCommitV1 {
                relation_scope_token,
                relation_hmac: context_projection.relation_hmac(),
                source_continuum_revision: receipt.next_revision,
                context_digest: ae_store::continuity_context_digest(&canonical_context_state),
                canonical_state_bytes: canonical_context_state,
            },
        }
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

    #[test]
    fn semantic_neutral_proposal_commits_full_vector_and_same_revision_expression() {
        let root = std::env::var_os("AE_CARD_R_TEMP_ROOT")
            .map(std::path::PathBuf::from)
            .expect("AE_CARD_R_TEMP_ROOT must name the card-R task directory");
        let dir = root.join(format!("focused-semantic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let mut runtime = AstrRuntime::open(&dir.join("store.db")).unwrap();
        let request = request(41);
        let genesis = runtime.ensure_genesis(&request).unwrap();
        let phase0_formula =
            semantic::phase0_semantic_formula_digest_v1(&request.formula_digest).unwrap();
        assert_ne!(genesis.formula_digest, phase0_formula);
        let scope = ScopeRef {
            bot_token: request.source.scope.bot_token,
            persona_token: request.source.scope.persona_token,
            relation_token: None,
            session_token: [42; 16],
        };
        let proposal = PerceptionProposalV1 {
            schema_version: 1,
            event_id: [43; 16],
            turn_id: [44; 16],
            observed_at_ms: 1_700_000_000_200,
            base_revision: 0,
            dimensions: EvidenceVector::default(),
            estimator_confidence: Fixed::ONE,
            protocol_version: 1,
            request_nonce_digest: [
                0xa8, 0xd3, 0x8b, 0x2c, 0xa2, 0x8a, 0xaf, 0x6d, 0x3a, 0xba, 0xd2, 0x18, 0x20, 0x02,
                0x16, 0xe6, 0xb5, 0x59, 0x32, 0x40, 0x76, 0x10, 0xa4, 0xf1, 0x61, 0x1b, 0xef, 0x05,
                0xd6, 0x91, 0x02, 0xe5,
            ],
        };

        let decision = runtime
            .apply_perception_proposal_v1(&scope, &proposal)
            .unwrap();
        assert!(!decision.deduplicated);
        assert_eq!(decision.revision, 1);
        assert_eq!(decision.receipt.formula_digest, phase0_formula);
        assert_eq!(runtime.semantic_revision_v1(&scope).unwrap(), 1);
        assert_eq!(decision.receipt.base_revision, 0);
        assert_eq!(decision.receipt.next_revision, 1);
        assert_eq!(decision.receipt.status, CommitStatus::Committed);

        let semantic_receipt = decision.semantic_vector_receipt.as_ref().unwrap();
        assert_eq!(semantic_receipt.next_revision, decision.revision);
        assert_eq!(semantic_receipt.semantic_vector.dimension_slot_count, 15);
        assert_eq!(
            semantic_receipt.semantic_vector.evaluated_dimension_count,
            15
        );
        assert_eq!(
            semantic_receipt.semantic_vector.injected_dimension_count,
            15
        );
        assert_eq!(
            semantic_receipt
                .semantic_vector
                .nonzero_evidence_dimension_count,
            0
        );
        assert_eq!(
            semantic_receipt
                .semantic_vector
                .neutral_baseline_dimension_count,
            15
        );
        assert_eq!(
            semantic_receipt.semantic_vector.unavailable_dimension_count,
            0
        );

        let journal = runtime
            .store
            .read_journal(&semantic_receipt.scope_digest)
            .unwrap();
        assert_eq!(journal.len(), 1);
        let snapshot = runtime
            .store
            .read_snapshot(&semantic_receipt.scope_digest, decision.revision)
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.state_digest, semantic_receipt.state_after);

        let expression = &decision.expression_projection;
        assert_eq!(expression.revision, decision.revision);
        for value in [
            expression.profile_fxp6.warmth,
            expression.profile_fxp6.sensitivity,
            expression.profile_fxp6.guardedness,
            expression.profile_fxp6.repair_orientation,
            expression.profile_fxp6.engagement,
            expression.profile_fxp6.epistemic_caution,
        ] {
            assert!(value <= 1_000_000);
        }
    }

    #[test]
    fn semantic_followup_keeps_formula_and_graph_continuity() {
        let root = std::env::var_os("AE_CARD_R_TEMP_ROOT")
            .map(std::path::PathBuf::from)
            .expect("AE_CARD_R_TEMP_ROOT must name the continuity task directory");
        let dir = root.join(format!(
            "focused-semantic-continuity-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let mut runtime = AstrRuntime::open(&dir.join("store.db")).unwrap();
        let request = request(51);
        let genesis = runtime.ensure_genesis(&request).unwrap();
        let phase0_formula =
            semantic::phase0_semantic_formula_digest_v1(&request.formula_digest).unwrap();
        assert_ne!(genesis.formula_digest, phase0_formula);
        let scope = ScopeRef {
            bot_token: request.source.scope.bot_token,
            persona_token: request.source.scope.persona_token,
            relation_token: None,
            session_token: [52; 16],
        };
        let first = runtime
            .apply_perception_proposal_v1(&scope, &semantic_proposal(&scope, 53, 0))
            .unwrap();
        let second = runtime
            .apply_perception_proposal_v1(&scope, &semantic_proposal(&scope, 54, first.revision))
            .unwrap();

        assert_eq!(first.revision, 1);
        assert_eq!(second.revision, 2);
        assert_eq!(second.receipt.base_revision, first.revision);
        assert_eq!(second.receipt.next_revision, 2);
        assert_eq!(first.receipt.formula_digest, phase0_formula);
        assert_eq!(second.receipt.formula_digest, first.receipt.formula_digest);
        assert_eq!(second.receipt.state_before, first.receipt.state_after);

        let first_telemetry = first.semantic_telemetry_receipt.as_ref().unwrap();
        let second_telemetry = second.semantic_telemetry_receipt.as_ref().unwrap();
        assert_eq!(
            second_telemetry.formula_digest,
            first_telemetry.formula_digest
        );
        assert_eq!(second_telemetry.graph_before, first_telemetry.graph_after);

        let journal = runtime
            .store
            .read_journal(&first.receipt.scope_digest)
            .unwrap();
        assert_eq!(journal.len(), 2);
        assert_eq!(journal[1].decode_receipt().unwrap(), second.receipt);
        assert_eq!(runtime.semantic_revision_v1(&scope).unwrap(), 2);
    }

    #[test]
    fn legacy_aesem2_revision_two_upgrades_once_without_reset_or_rebirth() {
        let root = std::env::var_os("AE_CARD_R_TEMP_ROOT")
            .map(std::path::PathBuf::from)
            .expect("AE_CARD_R_TEMP_ROOT must name the legacy-upgrade task directory");
        let dir = root.join(format!("legacy-aesem2-upgrade-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("store.db");
        let request = request(61);
        let scope = ScopeRef {
            bot_token: request.source.scope.bot_token,
            persona_token: request.source.scope.persona_token,
            relation_token: None,
            session_token: [62; 16],
        };

        let (history, original_genesis, legacy_journal, legacy_snapshot) = {
            let mut runtime = AstrRuntime::open(&path).unwrap();
            let original_genesis = runtime.ensure_genesis(&request).unwrap();
            let history = seed_legacy_aesem2_history(&mut runtime, &request, &scope, false);
            let legacy_journal = runtime.store.read_journal(&history.semantic_scope).unwrap();
            let legacy_snapshot = runtime
                .store
                .read_snapshot(&history.semantic_scope, 2)
                .unwrap()
                .expect("r=2 AESEM2 snapshot persists");
            assert_eq!(legacy_journal.len(), 2);
            assert!(semantic::snapshot_is_aesem2(&legacy_snapshot.state_bytes));
            (history, original_genesis, legacy_journal, legacy_snapshot)
        };

        let mut reopened = AstrRuntime::open(&path).unwrap();
        assert_eq!(reopened.semantic_revision_v1(&scope).unwrap(), 2);
        let upgrade_proposal = semantic_proposal(&scope, 96, 2);
        let upgraded = reopened
            .apply_perception_proposal_v1(&scope, &upgrade_proposal)
            .expect("first fresh proposal upgrades the AESEM2 lane");
        assert_eq!(upgraded.revision, 3);
        assert_eq!(upgraded.receipt.base_revision, 2);
        assert_eq!(upgraded.receipt.next_revision, 3);
        assert_eq!(
            upgraded.receipt.formula_digest,
            history.phase0_formula_digest
        );
        assert_eq!(
            upgraded.receipt.state_before,
            state_digest(&history.latest_field, &history.phase0_formula_digest)
        );
        assert_eq!(
            upgraded
                .semantic_telemetry_receipt
                .as_ref()
                .expect("upgrade emits current telemetry")
                .graph_before,
            history.latest_graph_digest
        );
        assert_eq!(
            graph_digest(&history.latest_graph),
            history.latest_graph_digest
        );
        assert_ne!(upgraded.receipt.state_before, history.latest_state_digest);
        assert_eq!(reopened.semantic_revision_v1(&scope).unwrap(), 3);

        let after_upgrade = reopened
            .store
            .read_journal(&history.semantic_scope)
            .unwrap();
        assert_eq!(after_upgrade.len(), 3);
        assert_eq!(&after_upgrade[..2], legacy_journal.as_slice());
        assert_eq!(history.legacy_formula_digest, request.formula_digest);
        assert_eq!(
            history.latest_chain_digest,
            legacy_journal
                .last()
                .expect("legacy history has a tail")
                .chain_digest
        );
        let upgrade_receipt = reopened
            .store
            .read_legacy_semantic_formula_upgrade_v1(
                &history.semantic_scope,
                &history.legacy_formula_digest,
                &history.phase0_formula_digest,
            )
            .unwrap()
            .expect("one explicit legacy-upgrade receipt persists");
        assert_eq!(upgrade_receipt.base_revision, 2);
        assert_eq!(upgrade_receipt.next_revision, 3);
        assert_eq!(upgrade_receipt.event_digest, upgraded.receipt.event_digest);
        assert_eq!(
            upgrade_receipt.receipt_digest,
            wire::receipt_digest(&upgraded.receipt)
        );
        assert_eq!(
            upgrade_receipt.source_state_digest,
            history.latest_state_digest
        );
        assert_eq!(
            upgrade_receipt.target_state_before,
            upgraded.receipt.state_before
        );
        assert_eq!(
            upgrade_receipt.source_graph_digest,
            history.latest_graph_digest
        );
        assert_eq!(
            upgrade_receipt.prior_chain_digest,
            history.latest_chain_digest
        );
        assert_eq!(
            reopened
                .store
                .read_snapshot(&history.semantic_scope, 2)
                .unwrap()
                .expect("legacy snapshot remains")
                .state_bytes,
            legacy_snapshot.state_bytes
        );
        assert_eq!(reopened.ensure_genesis(&request).unwrap(), original_genesis);

        drop(reopened);
        let mut continued = AstrRuntime::open(&path).unwrap();
        let deduplicated = continued
            .apply_perception_proposal_v1(&scope, &upgrade_proposal)
            .expect("persisted upgrade event deduplicates after reopen");
        assert!(deduplicated.deduplicated);
        assert_eq!(deduplicated.revision, 3);
        assert_eq!(
            wire::receipt_digest(&deduplicated.receipt),
            wire::receipt_digest(&upgraded.receipt)
        );
        let followup = continued
            .apply_perception_proposal_v1(&scope, &semantic_proposal(&scope, 97, 3))
            .expect("current formula continues after the one-time upgrade");
        assert_eq!(followup.revision, 4);
        assert_eq!(
            followup.receipt.formula_digest,
            history.phase0_formula_digest
        );
        assert_eq!(continued.semantic_revision_v1(&scope).unwrap(), 4);
        assert_eq!(
            continued
                .store
                .read_legacy_semantic_formula_upgrade_v1(
                    &history.semantic_scope,
                    &history.legacy_formula_digest,
                    &history.phase0_formula_digest,
                )
                .unwrap(),
            Some(upgrade_receipt)
        );
    }

    #[test]
    fn finite_legacy_aesem2_field_overflow_migrates_with_the_first_real_event() {
        let root = std::env::var_os("AE_CARD_R_TEMP_ROOT")
            .map(std::path::PathBuf::from)
            .expect("AE_CARD_R_TEMP_ROOT must name the field-migration task directory");
        let dir = root.join(format!(
            "legacy-aesem2-field-domain-migration-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("store.db");
        let request = request(81);
        let scope = ScopeRef {
            bot_token: request.source.scope.bot_token,
            persona_token: request.source.scope.persona_token,
            relation_token: None,
            session_token: [82; 16],
        };

        let (history, legacy_journal, legacy_snapshot) = {
            let mut runtime = AstrRuntime::open(&path).unwrap();
            runtime.ensure_genesis(&request).unwrap();
            let history = seed_legacy_aesem2_history(&mut runtime, &request, &scope, true);
            assert!(history.latest_field.potential[0] > Fixed::ONE);
            assert!(history.latest_field.excitation[0] > Fixed::ONE);
            let journal = runtime.store.read_journal(&history.semantic_scope).unwrap();
            let snapshot = runtime
                .store
                .read_snapshot(&history.semantic_scope, 2)
                .unwrap()
                .expect("overflowing AESEM2 snapshot persists");
            (history, journal, snapshot)
        };

        let mut reopened = AstrRuntime::open(&path).unwrap();
        let proposal = semantic_proposal(&scope, 83, 2);
        let migrated = reopened
            .apply_perception_proposal_v1(&scope, &proposal)
            .expect("known finite legacy overflow must migrate inside r=2 -> r=3");

        assert_eq!(migrated.revision, 3);
        assert_eq!(migrated.receipt.base_revision, 2);
        assert_eq!(migrated.receipt.next_revision, 3);
        assert_eq!(
            migrated.field_migration,
            Some(SemanticFieldMigrationOutcomeV1::Applied)
        );
        assert_eq!(
            migrated.receipt.formula_digest,
            history.phase0_formula_digest
        );
        assert_ne!(
            migrated.receipt.state_before,
            state_digest(&history.latest_field, &history.phase0_formula_digest)
        );
        let after = reopened
            .store
            .read_journal(&history.semantic_scope)
            .unwrap();
        assert_eq!(after.len(), 3);
        assert_eq!(&after[..2], legacy_journal.as_slice());
        assert_eq!(
            reopened
                .store
                .read_snapshot(&history.semantic_scope, 2)
                .unwrap()
                .expect("historical snapshot remains immutable")
                .state_bytes,
            legacy_snapshot.state_bytes
        );
        let upgrade = reopened
            .store
            .read_legacy_semantic_formula_upgrade_v1(
                &history.semantic_scope,
                &history.legacy_formula_digest,
                &history.phase0_formula_digest,
            )
            .unwrap()
            .expect("field migration has one durable receipt");
        assert!(upgrade.field_domain.is_some());
        drop(reopened);
        let mut resumed = AstrRuntime::open(&path).unwrap();
        let deduplicated = resumed
            .apply_perception_proposal_v1(&scope, &proposal)
            .expect("the same event replays its normalized precondition");
        assert!(deduplicated.deduplicated);
        assert_eq!(
            deduplicated.field_migration,
            Some(SemanticFieldMigrationOutcomeV1::Replayed)
        );
        assert_eq!(
            wire::receipt_digest(&deduplicated.receipt),
            wire::receipt_digest(&migrated.receipt)
        );
    }

    #[test]
    fn tampered_legacy_formula_upgrade_receipt_writes_nothing() {
        let root = std::env::var_os("AE_CARD_R_TEMP_ROOT")
            .map(std::path::PathBuf::from)
            .expect("AE_CARD_R_TEMP_ROOT must name the legacy-upgrade task directory");
        let dir = root.join(format!("legacy-aesem2-tamper-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut runtime = AstrRuntime::open(&dir.join("store.db")).unwrap();
        let request = request(71);
        runtime.ensure_genesis(&request).unwrap();
        let scope = ScopeRef {
            bot_token: request.source.scope.bot_token,
            persona_token: request.source.scope.persona_token,
            relation_token: None,
            session_token: [72; 16],
        };
        let history = seed_legacy_aesem2_history(&mut runtime, &request, &scope, false);
        let tampered_from_formula = [0xa5; 32];
        assert_ne!(tampered_from_formula, history.legacy_formula_digest);
        let bundle = legacy_upgrade_bundle_for_test(
            &mut runtime,
            &history,
            &scope,
            &semantic_proposal(&scope, 98, 2),
            tampered_from_formula,
        );
        let before = runtime.store.read_journal(&history.semantic_scope).unwrap();
        assert_eq!(before.len(), 2);

        assert!(matches!(
            runtime.store.commit_continuity_bundle(&bundle),
            Err(StoreError::ContinuityFence("graph_current_formula"))
        ));
        assert_eq!(
            runtime.store.read_journal(&history.semantic_scope).unwrap(),
            before
        );
        assert!(runtime
            .store
            .read_snapshot(&history.semantic_scope, 3)
            .unwrap()
            .is_none());
        assert!(runtime
            .store
            .read_legacy_semantic_formula_upgrade_v1(
                &history.semantic_scope,
                &history.legacy_formula_digest,
                &history.phase0_formula_digest,
            )
            .unwrap()
            .is_none());
        assert_eq!(
            runtime
                .store
                .current_revision(&history.semantic_scope)
                .unwrap(),
            2
        );
    }

    #[test]
    fn semantic_nonce_binding_mismatch_is_rejected_without_semantic_write() {
        let root = std::env::var_os("AE_CARD_R_TEMP_ROOT")
            .map(std::path::PathBuf::from)
            .expect("AE_CARD_R_TEMP_ROOT must name the nonce-binding task directory");
        let dir = root.join(format!("focused-semantic-nonce-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let mut runtime = AstrRuntime::open(&dir.join("store.db")).unwrap();
        let request = request(41);
        runtime.ensure_genesis(&request).unwrap();
        let scope = ScopeRef {
            bot_token: request.source.scope.bot_token,
            persona_token: request.source.scope.persona_token,
            relation_token: None,
            session_token: [42; 16],
        };
        let canonical = PerceptionProposalV1 {
            schema_version: 1,
            event_id: [43; 16],
            turn_id: [44; 16],
            observed_at_ms: 1_700_000_000_200,
            base_revision: 0,
            dimensions: EvidenceVector::default(),
            estimator_confidence: Fixed::ONE,
            protocol_version: 1,
            request_nonce_digest: [
                0xa8, 0xd3, 0x8b, 0x2c, 0xa2, 0x8a, 0xaf, 0x6d, 0x3a, 0xba, 0xd2, 0x18, 0x20, 0x02,
                0x16, 0xe6, 0xb5, 0x59, 0x32, 0x40, 0x76, 0x10, 0xa4, 0xf1, 0x61, 0x1b, 0xef, 0x05,
                0xd6, 0x91, 0x02, 0xe5,
            ],
        };
        let mut mismatched = canonical.clone();
        mismatched.request_nonce_digest = [0x01; 32];
        assert_ne!(
            mismatched.request_nonce_digest,
            canonical.request_nonce_digest
        );

        assert_eq!(runtime.semantic_revision_v1(&scope).unwrap(), 0);
        assert!(matches!(
            runtime.apply_perception_proposal_v1(&scope, &mismatched),
            Err(RuntimeError::InvalidPerceptionProposal)
        ));
        assert_eq!(runtime.semantic_revision_v1(&scope).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
