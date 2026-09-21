#![forbid(unsafe_code)]

//! AstrRuntime: the G0 vertical slice orchestrator.
//!
//! ensure_genesis -> Store-owned semantic evolution -> SQLite commit -> replay
//! verification. Python cannot reach any of this state directly; the PyO3
//! surface exposes only coarse calls.

pub mod matrix_time;
#[allow(dead_code)]
mod semantic;
pub mod semantic_dynamics_v2;
// Construction stays compiled but unreachable until the crate-owned event
// lane wires its verified inputs in a later forward-port task.
#[allow(dead_code)]
mod semantic_telemetry_v1;

use ae_agent::noop_action_contract;
use ae_continuum::ReplayReport;
use ae_contracts::{
    phase0_canonical_formula_digest_v1, wire, ActionContract, Alpha3ErrorCodeV1, CanonicalEvent, Digest, GenesisReceipt, GenesisStatus, Id128, InvariantResiduals,
    PersonaGenesisRequest, ScopeRef, SemanticAppraisalSettleRequestV1,
    SemanticAppraisalSettleResultV1, SemanticAppraisalSettleStatusV1, StateSubcodeV1,
    TransitionReceipt,
};
#[cfg(feature = "legacy-semantic-test-api")]
use ae_contracts::{PerceptionChallengeV1, PerceptionProposalV1};
use ae_neurofield::{
    graph_digest, initial_state_from_manifest, state_digest, NeuralField, SparseGraph,
};
#[cfg(feature = "legacy-semantic-test-api")]
use ae_store::SemanticCommitDispositionV1;
use ae_store::{
    ClaimOutcome, GenesisCommit, SemanticAppraisalSettlementStoreOutcomeV1,
    SemanticAppraisalStoreSettlementV1, Store, StoreError,
};
use std::path::Path;
use thiserror::Error;

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
    InvalidNeuralState(StateSubcodeV1),
    #[error("invalid closed semantic perception proposal")]
    InvalidPerceptionProposal,
    #[error("raw UserStimulus is unauthenticated; use a Store-minted perception proposal")]
    UnauthenticatedUserStimulus,
    #[error("legacy semantic snapshot has no v2 attestation")]
    LegacyUnattested,
    #[error("semantic revision overflow")]
    SemanticRevisionOverflow,
    #[error("semantic appraisal retry expired or unknown")]
    SemanticAppraisalRetryExpiredOrUnknown,
    #[error("autonomy error: {0}")]
    Autonomy(String),
    #[error("alpha3 error: {0:?}")]
    Alpha3(Alpha3ErrorCodeV1),
}

impl RuntimeError {
    pub const fn invalid_neural_state(subcode: StateSubcodeV1) -> Self {
        Self::InvalidNeuralState(subcode)
    }
}

impl From<semantic_dynamics_v2::DynamicsError> for RuntimeError {
    fn from(error: semantic_dynamics_v2::DynamicsError) -> Self {
        let subcode = match error {
            semantic_dynamics_v2::DynamicsError::FieldStateInvalid => {
                StateSubcodeV1::FieldStateInvalid
            }
            semantic_dynamics_v2::DynamicsError::GraphStateInvalid => {
                StateSubcodeV1::GraphStateInvalid
            }
            semantic_dynamics_v2::DynamicsError::InvalidInput
            | semantic_dynamics_v2::DynamicsError::Arithmetic => StateSubcodeV1::DynamicsInvalid,
        };
        Self::invalid_neural_state(subcode)
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

#[derive(Clone, Debug)]
pub struct InspectReport {
    pub bound: bool,
    pub bot_token: Id128,
    pub persona_token: Id128,
    pub persona_scope: Digest,
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
    semantic_formula_digest: Digest,
    field: NeuralField,
    graph: SparseGraph,
    initial_snapshot_digest: Digest,
    canonical_revision: u64,
    semantic_revision: u64,
}

pub struct AstrRuntime {
    store: Store,
    hot: Option<HotBrain>,
}

fn fixed_zero_vector() -> InvariantResiduals {
    InvariantResiduals::default()
}

impl AstrRuntime {
    pub fn open(path: &Path) -> Result<Self, RuntimeError> {
        let runtime = Self {
            store: Store::open(path)?,
            hot: None,
        };
        // Store has already authenticated v9. Opening a core runtime performs
        // no retired autonomy reconstruction or relation-state mutation.
        Ok(runtime)
    }

    fn evict_core_scope(&mut self, scope: &ae_contracts::PersonaScopeRef) {
        if self.hot.as_ref().is_some_and(|h|h.bot_token==scope.bot_token && h.persona_token==scope.persona_token) {
            self.hot=None;
        }
    }

    pub fn list_embodiment_personas_v1(&mut self,request:&ae_contracts::ListEmbodimentPersonasV1)->Result<ae_contracts::EmbodimentPersonaInventoryPageV1,RuntimeError>{Ok(self.store.list_embodiment_personas_v1(request)?)}
    pub fn read_embodiment_profile_v1(&mut self,scope:&ae_contracts::PersonaScopeRef)->Result<ae_contracts::EmbodimentProfileReadV1,RuntimeError>{Ok(self.store.read_embodiment_profile_v1(scope)?)}
    pub fn get_embodiment_persona_v1(&mut self,scope:&ae_contracts::PersonaScopeRef)->Result<ae_contracts::EmbodimentPersonaLookupV1,RuntimeError>{Ok(self.store.get_embodiment_persona_v1(scope)?)}
    pub fn create_embodiment_persona_if_missing_v1(&mut self,request:&ae_contracts::CreateEmbodimentPersonaIfMissingV1)->Result<ae_contracts::EmbodimentPersonaCreateOutcomeV1,RuntimeError>{let result=self.store.create_embodiment_persona_if_missing_v1(request)?;self.evict_core_scope(&request.scope);Ok(result)}
    pub fn embodiment_clock_status_v1(&mut self,request:&ae_contracts::EmbodimentClockStatusRequestV1)->Result<ae_contracts::EmbodimentClockStatusV1,RuntimeError>{Ok(self.store.embodiment_clock_status_v1(request)?)}
    pub fn advance_embodiment_time_v1(&mut self,request:&[u8])->Result<ae_contracts::EmbodimentClockCommitOutcomeV1,RuntimeError>{let result=self.store.advance_embodiment_time_v1(request)?;self.evict_core_scope(&result.receipt.result.scope);Ok(result)}
    pub fn compare_and_swap_embodiment_profile_v1(&mut self,request:&ae_contracts::CompareAndSwapEmbodimentProfileV1)->Result<ae_contracts::EmbodimentClockCommitOutcomeV1,RuntimeError>{let result=self.store.compare_and_swap_embodiment_profile_v1(request)?;self.evict_core_scope(&request.scope);Ok(result)}

    pub fn commit_core_inbound_v1(&mut self, request:&ae_contracts::CommitCoreInboundV1) -> Result<ae_contracts::CoreInboundCommitOutcomeV1,RuntimeError> {
        let result=self.store.commit_core_inbound_v1(request)?;
        // ReloadRequired is intentional: no fallible hydration after commit,
        // and an unrelated resident persona is never modified.
        self.evict_core_scope(&result.initial_receipt.event.scope);
        Ok(result)
    }

    pub fn commit_core_delivery_outcome_v1(&mut self, request:&ae_contracts::CommitCoreDeliveryOutcomeV1) -> Result<ae_contracts::CoreDeliveryCommitOutcomeV1,RuntimeError> {
        let result=self.store.commit_core_delivery_outcome_v1(request)?;
        self.evict_core_scope(&result.receipt.scope);
        Ok(result)
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
                    state_bytes: self.encode_state(&field, &graph),
                    graph_digest,
                };

                match self.store.commit_genesis(&commit) {
                    Ok(()) => {
                        self.hot = Some(HotBrain {
                            bot_token: effective.source.scope.bot_token,
                            persona_token: effective.source.scope.persona_token,
                            persona_scope: wire::persona_scope_digest(
                                &effective.source.scope.bot_token,
                                &effective.source.scope.persona_token,
                                None,
                            ),
                            identity,
                            semantic_formula_digest: phase0_canonical_formula_digest_v1(
                                &effective.formula_digest,
                            ),
                            field,
                            graph,
                            initial_snapshot_digest,
                            canonical_revision: 0,
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

    fn encode_state(&self, field: &NeuralField, graph: &SparseGraph) -> Vec<u8> {
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
        let (baseline_field, baseline_graph) = initial_state_from_manifest(
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
        let canonical_revision = self.store.current_revision(&persona_scope)?;
        let hydrated = self
            .store
            .hydrated_semantic_state_v1(&committed.source.scope)?;
        let (semantic_formula_digest, field, graph, semantic_revision) =
            if let Some(state) = hydrated {
                (
                    state.formula_digest,
                    state.field,
                    state.graph,
                    state.semantic_revision,
                )
            } else {
                (
                    phase0_canonical_formula_digest_v1(&committed.receipt.formula_digest),
                    baseline_field,
                    baseline_graph,
                    0,
                )
            };
        self.hot = Some(HotBrain {
            bot_token,
            persona_token,
            persona_scope,
            identity,
            semantic_formula_digest,
            field,
            graph,
            initial_snapshot_digest: committed.receipt.initial_snapshot_digest,
            canonical_revision,
            semantic_revision,
        });
        Ok(())
    }

    fn hot_for(&mut self, scope: &ScopeRef) -> Result<&mut HotBrain, RuntimeError> {
        let identity_matches = self
            .hot
            .as_ref()
            .map(|hot| hot.bot_token == scope.bot_token && hot.persona_token == scope.persona_token)
            .unwrap_or(false);
        let revisions_match = if let Some(hot) = self.hot.as_ref().filter(|_| identity_matches) {
            let persona_scope = hot.persona_scope;
            self.store.current_revision(&persona_scope)? == hot.canonical_revision
                && self.store.semantic_revision_v1(&persona_scope)? == hot.semantic_revision
        } else {
            false
        };
        if !identity_matches || !revisions_match {
            self.bind_hot(scope.bot_token, scope.persona_token)?;
        }
        self.hot
            .as_mut()
            .ok_or(RuntimeError::PersonaGenesisRequired)
    }

    // --------------------------------------------------------------- events

    /// Host authority bridge: mint only from an interaction observation that
    /// is already committed by the dedicated interaction transaction.
    #[cfg(feature = "legacy-semantic-test-api")]
    pub fn mint_perception_challenge_from_committed_inbound_v1(
        &mut self,
        origin_event_digest: Digest,
    ) -> Result<PerceptionChallengeV1, RuntimeError> {
        self.store
            .mint_perception_challenge_from_committed_inbound_v1(origin_event_digest)
            .map_err(RuntimeError::Store)
    }

    /// Production semantic write entry. The Store authenticates and consumes
    /// the challenge in the same immediate transaction that writes the event
    /// and semantic sidecars; hot state is replaced only after commit.
    #[cfg(feature = "legacy-semantic-test-api")]
    pub fn apply_perception_proposal_v1(
        &mut self,
        scope: &ScopeRef,
        proposal: &PerceptionProposalV1,
    ) -> Result<ApplyDecision, RuntimeError> {
        proposal
            .validate_v1()
            .map_err(|_| RuntimeError::InvalidPerceptionProposal)?;
        let result = match self.store.commit_perception_proposal_v1(scope, proposal) {
            Ok(result) => result,
            Err(StoreError::StaleRevision { expected, actual }) => {
                return Err(RuntimeError::StaleCausalBase {
                    expected: actual,
                    actual: expected,
                });
            }
            Err(StoreError::SemanticInvalid(_)) | Err(StoreError::SemanticIdentityConflict) => {
                return Err(RuntimeError::InvalidPerceptionProposal)
            }
            Err(other) => return Err(RuntimeError::Store(other)),
        };
        let receipt = result
            .committed
            .journal
            .decode_receipt()
            .map_err(|error| RuntimeError::Store(StoreError::Sqlite(error.to_string())))?;
        let turn_id = match wire::decode_event(&result.committed.journal.event_bytes)
            .map_err(|error| RuntimeError::Store(StoreError::Sqlite(error.to_string())))?
        {
            CanonicalEvent::UserStimulus(stimulus) => stimulus.causal.turn_id,
            _ => return Err(RuntimeError::InvalidPerceptionProposal),
        };
        let contract = noop_action_contract(
            &result.committed.manifest_digest,
            &result.committed.event_digest,
            turn_id,
        );
        let revision = result.committed.journal_revision;
        let deduplicated = result.disposition == SemanticCommitDispositionV1::Existing;
        self.bind_hot(scope.bot_token, scope.persona_token)?;
        Ok(ApplyDecision {
            contract,
            receipt,
            revision,
            deduplicated,
        })
    }

    /// Settle one Store-owned semantic appraisal claim.  The Host supplies
    /// neither origin, nonce authority nor causal base: all three are reloaded
    /// and attested by Store inside the serialized writer transaction.
    pub fn settle_semantic_appraisal_v1(
        &mut self,
        request: &SemanticAppraisalSettleRequestV1,
    ) -> Result<SemanticAppraisalSettleResultV1, RuntimeError> {
        if !request.validate_v1() {
            return Err(RuntimeError::InvalidPerceptionProposal);
        }
        let outcome = match self.store.settle_semantic_appraisal_v1(request) {
            Ok(outcome) => outcome,
            Err(StoreError::StaleRevision { expected, actual }) => {
                return Err(RuntimeError::StaleCausalBase {
                    expected: actual,
                    actual: expected,
                });
            }
            Err(StoreError::SemanticInvalid(_)) | Err(StoreError::SemanticIdentityConflict) => {
                return Err(RuntimeError::InvalidPerceptionProposal);
            }
            Err(other) => return Err(RuntimeError::Store(other)),
        };
        let settlement = match outcome {
            SemanticAppraisalSettlementStoreOutcomeV1::Completed(settlement) => settlement,
            SemanticAppraisalSettlementStoreOutcomeV1::RetryExpiredOrUnknown => {
                return Err(RuntimeError::SemanticAppraisalRetryExpiredOrUnknown)
            }
        };
        match settlement {
            SemanticAppraisalStoreSettlementV1::Committed {
                result,
                charged_tokens,
                reply_affect,
            } => {
                let turn_id = match wire::decode_event(&result.committed.journal.event_bytes)
                    .map_err(|error| RuntimeError::Store(StoreError::Sqlite(error.to_string())))?
                {
                    CanonicalEvent::UserStimulus(stimulus) => stimulus.causal.turn_id,
                    _ => return Err(RuntimeError::InvalidPerceptionProposal),
                };
                let contract = noop_action_contract(
                    &result.committed.manifest_digest,
                    &result.committed.event_digest,
                    turn_id,
                );
                let canonical_revision = result.committed.journal_revision;
                let semantic_revision = result.committed.semantic_revision;
                self.bind_hot(request.scope.bot_token, request.scope.persona_token)?;
                Ok(SemanticAppraisalSettleResultV1 {
                    status: SemanticAppraisalSettleStatusV1::Committed,
                    canonical_revision,
                    semantic_revision: Some(semantic_revision),
                    charged_tokens,
                    contract: Some(contract),
                    reply_affect,
                })
            }
            SemanticAppraisalStoreSettlementV1::ZeroMutation {
                canonical_revision,
                charged_tokens,
                reply_affect,
            } => Ok(SemanticAppraisalSettleResultV1 {
                status: SemanticAppraisalSettleStatusV1::ZeroMutation,
                canonical_revision,
                semantic_revision: None,
                charged_tokens,
                contract: None,
                reply_affect,
            }),
        }
    }

    /// Apply one canonical event. User stimuli enter the Store-owned semantic
    /// lane; delivery outcomes remain journal-only and cannot fabricate
    /// evidence or advance the independent semantic cursor.

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
                    persona_scope,
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
                persona_scope: [0; 32],
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

    /// Drain the writer, checkpoint WAL and close the store. Semantic state is
    /// already durable in the atomic AESEM3 sidecar transaction; writing a
    /// second generic snapshot here would conflate journal and semantic cursors.
    pub fn flush_and_close(&mut self) -> Result<(), RuntimeError> {
        self.hot.take();
        self.store.flush()?;
        Ok(())
    }

    pub fn closed(&self) -> bool {
        matches!(self.store.count_leases(), Err(StoreError::Closed))
    }

    pub fn current_revision(&mut self, scope: &ScopeRef) -> Result<u64, RuntimeError> {
        let hot = self.hot_for(scope)?;
        Ok(hot.canonical_revision)
    }

    pub fn semantic_revision_v1(&mut self, scope: &ScopeRef) -> Result<u64, RuntimeError> {
        let hot = self.hot_for(scope)?;
        Ok(hot.semantic_revision)
    }

    pub fn audit_semantic_integrity_v1(&mut self) -> Result<(), RuntimeError> {
        self.store
            .audit_semantic_integrity_v1()
            .map_err(RuntimeError::Store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ae_contracts::{
        wire, AllostaticSetpoints, AutonomyJournalDeltaV1, CausalRef, ChronotypeV1,
        DispatchOutcomeV1, DispatchSettleV1, EpistemicPriors, ExpressionPhenotype,
        ExternalizationOutcomeV1, ExternalizationSettleV1, FrozenTimeInputV1,
        GateAndClaimDispatchRequestV1, GateAndClaimExternalizationRequestV1,
        GenesisManifestProposal, HostCapabilitySnapshotV1, IntentionStateV1,
        OutboundTargetEnvelopeV1, PersonaScopeRef, PersonaSelectionKind, PersonaSourceRef,
        PersonaTemporalProfileV1, PersonalityVector, RelationTemporalPolicyV1,
        SemanticTransitionKindV1, SocialPriors, TargetKindV1, TimeAdvanceV1, TimezoneSourceV1,
        AUTONOMY_SCHEMA_VERSION,
    };
    #[cfg(feature = "legacy-semantic-test-api")]
    use ae_contracts::{
        DeliveryOutcome, EvidenceVector, InteractionFactBatchV1, InteractionFactKindV1,
        InteractionFactV1, InteractionSourceAuthorityV1, PerceptionProposalV1,
        ALPHA3_SCHEMA_VERSION,
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

    #[cfg(feature = "legacy-semantic-test-api")]
    fn stimulus_scope(seed: u8, session: u8) -> ScopeRef {
        ScopeRef {
            bot_token: [seed; 16],
            persona_token: [seed.wrapping_add(1); 16],
            relation_token: Some([seed.wrapping_add(30); 16]),
            session_token: [session; 16],
        }
    }

    #[cfg(feature = "legacy-semantic-test-api")]
    fn commit_inbound_and_mint_challenge(
        runtime: &mut AstrRuntime,
        seed: u8,
        session: u8,
    ) -> Result<(ScopeRef, PerceptionChallengeV1), RuntimeError> {
        let scope = stimulus_scope(seed, session);
        let persona_scope =
            wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
        let relation_scope = wire::persona_scope_digest(
            &scope.bot_token,
            &scope.persona_token,
            scope.relation_token.as_ref(),
        );
        runtime.bootstrap_autonomy(
            &scope,
            &PersonaTemporalProfileV1 {
                schema_version: AUTONOMY_SCHEMA_VERSION,
                persona_scope,
                home_timezone: "UTC".into(),
                current_timezone: "UTC".into(),
                chronotype: ChronotypeV1::Intermediate,
                preferred_sleep_local_minute: 1_380,
                preferred_wake_local_minute: 420,
                sleep_flex_minutes: 90,
                entrainment_rate_minutes_per_day: 60,
                revision: 1,
            },
            Some(&RelationTemporalPolicyV1 {
                schema_version: AUTONOMY_SCHEMA_VERSION,
                relation_scope,
                user_timezone: "UTC".into(),
                timezone_source: TimezoneSourceV1::Explicit,
                quiet_hours_start_minute: 0,
                quiet_hours_end_minute: 0,
                quiet_hours_emergency_bypass: false,
                proactive_enabled: false,
                proactive_daily_max: 0,
                min_proactive_cooldown_ms: 0,
                intention_ttl_ms: 86_400_000,
                unanswered_backoff_base_ms: 0,
                unanswered_hard_stop: 3,
                emergency_threshold: Fixed::ONE,
                daily_submitted: 0,
                consecutive_unanswered: 0,
                last_inbound_utc_ms: None,
                last_proactive_submitted_utc_ms: None,
                revision: 1,
                auto_policy_version: 0,
                next_claim_reservation_tokens: 256,
            }),
        )?;
        let base_revision = runtime.current_revision(&scope)?;
        let batch = InteractionFactBatchV1 {
            schema_version: ALPHA3_SCHEMA_VERSION,
            event_id: [seed.wrapping_add(10).wrapping_add(session); 16],
            scope: scope.clone(),
            causal: CausalRef {
                turn_id: [seed.wrapping_add(11).wrapping_add(session); 16],
                action_id: None,
                delivery_id: None,
                claim_id: None,
                base_revision,
            },
            facts: vec![InteractionFactV1 {
                fact_id: [seed.wrapping_add(20).wrapping_add(session); 16],
                kind: InteractionFactKindV1::InboundObserved,
                observed_at_utc_ms: 1_700_000_000_100 + u64::from(session),
                source_authority: InteractionSourceAuthorityV1::AstrbotMetadata,
                source_digest: [seed.wrapping_add(12).wrapping_add(session); 32],
                extractor_digest: [seed.wrapping_add(13); 32],
                confidence: Fixed::ONE,
                value_code: None,
                subject_public_ref: None,
                consent_terms: None,
                scheduled_at_utc_ms: None,
                expires_at_utc_ms: None,
            }],
        };
        let committed = runtime.apply_interaction_fact_batch_v1(&batch)?;
        let challenge = runtime
            .mint_perception_challenge_from_committed_inbound_v1(committed.receipt.event_digest)?;
        Ok((scope, challenge))
    }

    #[cfg(feature = "legacy-semantic-test-api")]
    fn proposal(challenge: &PerceptionChallengeV1) -> PerceptionProposalV1 {
        PerceptionProposalV1 {
            schema_version: PerceptionProposalV1::SCHEMA_VERSION,
            origin_digest: challenge.origin.origin_digest,
            dimensions: EvidenceVector {
                positive: Fixed::from_raw(500_000),
                affiliation: Fixed::from_raw(250_000),
                engagement: Fixed::from_raw(600_000),
                ..EvidenceVector::default()
            },
            estimator_confidence: Fixed::from_raw(800_000),
            protocol_version: PerceptionProposalV1::PROTOCOL_VERSION,
            request_nonce_digest: challenge.request_nonce_digest,
        }
    }

    #[cfg(feature = "legacy-semantic-test-api")]
    fn apply_authenticated_stimulus(
        runtime: &mut AstrRuntime,
        seed: u8,
        session: u8,
    ) -> Result<ApplyDecision, RuntimeError> {
        let (scope, challenge) = commit_inbound_and_mint_challenge(runtime, seed, session)?;
        runtime.apply_perception_proposal_v1(&scope, &proposal(&challenge))
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ae-runtime-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    #[cfg(feature = "legacy-semantic-test-api")]
    fn full_g0_vertical_slice() {
        let dir = temp_dir("slice");
        let mut runtime = AstrRuntime::open(&dir.join("store.db")).unwrap();
        let request = request(1);
        let receipt = runtime.ensure_genesis(&request).unwrap();
        assert_eq!(receipt.status, GenesisStatus::Committed);

        let decision = apply_authenticated_stimulus(&mut runtime, 1, 1).unwrap();
        assert!(!decision.deduplicated);
        assert_eq!(decision.revision, 2);
        assert_eq!(decision.receipt.base_revision, 1);
        assert_eq!(decision.receipt.next_revision, 2);
        assert_ne!(decision.receipt.state_before, decision.receipt.state_after);

        let report = runtime
            .verify_replay(
                &request.source.scope.bot_token,
                &request.source.scope.persona_token,
            )
            .unwrap();
        assert!(report.ok, "{:?}", report.first_error);
        assert_eq!(report.checked, 2);

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
    #[cfg(feature = "legacy-semantic-test-api")]
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

        let decision_a = apply_authenticated_stimulus(&mut a, 7, 9).unwrap();
        let decision_b = apply_authenticated_stimulus(&mut b, 7, 9).unwrap();
        // Store-minted challenge nonces intentionally make authority/event and
        // action IDs database-local. The deterministic semantic computation
        // and action policy projection remain identical.
        assert_eq!(
            decision_a.receipt.formula_digest,
            decision_b.receipt.formula_digest
        );
        assert_eq!(
            decision_a.receipt.state_before,
            decision_b.receipt.state_before
        );
        assert_eq!(
            decision_a.receipt.state_after,
            decision_b.receipt.state_after
        );
        assert_eq!(
            decision_a.receipt.graph_after,
            decision_b.receipt.graph_after
        );
        let mut normalized_b = decision_b.contract.clone();
        normalized_b.action_id = decision_a.contract.action_id;
        assert_eq!(decision_a.contract, normalized_b);
        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    #[test]
    #[cfg(feature = "legacy-semantic-test-api")]
    fn duplicate_event_is_applied_once() {
        let dir = temp_dir("dup");
        let mut runtime = AstrRuntime::open(&dir.join("store.db")).unwrap();
        let request = request(2);
        runtime.ensure_genesis(&request).unwrap();
        let (scope, challenge) = commit_inbound_and_mint_challenge(&mut runtime, 2, 1).unwrap();
        let proposal = proposal(&challenge);
        let first = runtime
            .apply_perception_proposal_v1(&scope, &proposal)
            .unwrap();
        let second = runtime
            .apply_perception_proposal_v1(&scope, &proposal)
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
        assert_eq!(report.checked, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(feature = "legacy-semantic-test-api")]
    fn stale_causal_base_is_rejected() {
        let dir = temp_dir("stale");
        let mut runtime = AstrRuntime::open(&dir.join("store.db")).unwrap();
        let request = request(3);
        runtime.ensure_genesis(&request).unwrap();
        let (scope, challenge) = commit_inbound_and_mint_challenge(&mut runtime, 3, 1).unwrap();
        let delivery = CanonicalEvent::DeliveryOutcome(DeliveryOutcome {
            event_id: [0xD3; 16],
            scope: scope.clone(),
            causal: CausalRef {
                turn_id: [0xD4; 16],
                action_id: None,
                delivery_id: None,
                claim_id: None,
                base_revision: 1,
            },
            delivered: true,
            visible_action_digest: [0xD5; 32],
            delivered_at_ms: 1_700_000_000_200,
        });
        runtime
            .apply_event(&request.source.scope_persona_scope(), &delivery)
            .unwrap();
        let error = runtime
            .apply_perception_proposal_v1(&scope, &proposal(&challenge))
            .unwrap_err();
        assert!(
            matches!(&error, RuntimeError::InvalidPerceptionProposal),
            "unexpected stale challenge error: {error:?}"
        );
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
    #[cfg(feature = "legacy-semantic-test-api")]
    fn genesis_failure_creates_no_default_brain() {
        let dir = temp_dir("nobrain");
        let mut runtime = AstrRuntime::open(&dir.join("store.db")).unwrap();
        let mut broken = request(5);
        broken.proposal.source.source_digest = [99; 32];
        let error = runtime.ensure_genesis(&broken).unwrap_err();
        assert!(matches!(error, RuntimeError::Genesis(_)));
        // No lease, incarnation or binding: even the authority-owned inbound
        // lane fails before a challenge/proposal can exist.
        let inbound_error = commit_inbound_and_mint_challenge(&mut runtime, 5, 1).unwrap_err();
        assert!(
            matches!(&inbound_error, RuntimeError::PersonaGenesisRequired),
            "unexpected missing-genesis error: {inbound_error:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(feature = "legacy-semantic-test-api")]
    fn crash_recovery_reopens_and_replays() {
        let dir = temp_dir("crash");
        let path = dir.join("store.db");
        let mut runtime = AstrRuntime::open(&path).unwrap();
        let request = request(6);
        let receipt = runtime.ensure_genesis(&request).unwrap();
        let decision = apply_authenticated_stimulus(&mut runtime, 6, 1).unwrap();
        assert_eq!(decision.revision, 2);
        drop(runtime); // crash without flush_and_close

        let mut reopened = AstrRuntime::open(&path).unwrap();
        let report = reopened
            .verify_replay(
                &request.source.scope.bot_token,
                &request.source.scope.persona_token,
            )
            .unwrap();
        assert!(report.ok, "{:?}", report.first_error);
        assert_eq!(report.checked, 2);

        // The next event continues at revision 2, not 1, and the birth was
        // not duplicated.
        let next = apply_authenticated_stimulus(&mut reopened, 6, 2).unwrap();
        assert_eq!(next.revision, 4);
        assert_eq!(next.receipt.base_revision, 3);
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
    fn autonomous_time_advance_changes_state_and_replays() {
        let dir = temp_dir("autonomy");
        let path = dir.join("store.db");
        let mut runtime = AstrRuntime::open(&path).unwrap();
        let genesis = request(9);
        runtime.ensure_genesis(&genesis).unwrap();
        let scope = genesis.source.scope_persona_scope();
        let persona_scope =
            wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
        let profile = PersonaTemporalProfileV1 {
            schema_version: AUTONOMY_SCHEMA_VERSION,
            persona_scope,
            home_timezone: "America/Los_Angeles".into(),
            current_timezone: "America/Los_Angeles".into(),
            chronotype: ChronotypeV1::NightOwl,
            preferred_sleep_local_minute: 90,
            preferred_wake_local_minute: 570,
            sleep_flex_minutes: 120,
            entrainment_rate_minutes_per_day: 90,
            revision: 1,
        };
        let initial = runtime.bootstrap_autonomy(&scope, &profile, None).unwrap();
        let frozen = FrozenTimeInputV1 {
            schema_version: AUTONOMY_SCHEMA_VERSION,
            observed_now_utc_ms: 1_000,
            effective_now_utc_ms: 1_000,
            persona_tzid: "America/Los_Angeles".into(),
            persona_utc_offset_seconds: -25_200,
            persona_local_minute: 300,
            persona_day_ordinal: 739_854,
            relation_tzid: "Asia/Shanghai".into(),
            relation_utc_offset_seconds: 28_800,
            relation_local_minute: 1_200,
            relation_day_ordinal: 739_854,
            budget_day_start_utc_ms: 0,
            budget_next_day_start_utc_ms: 86_400_000,
            next_timezone_transition_utc_ms: None,
            tzdb_fingerprint: [4; 32],
        };
        let event = TimeAdvanceV1 {
            event_id: [7; 16],
            scope: scope.clone(),
            expected_generation: initial.generation,
            frozen_input_digest: frozen_time_input_digest(&frozen),
            frozen,
            stimulus: Default::default(),
        };
        let claim = runtime.claim_wake(&scope, &event).unwrap();
        assert!(claim.proposal.intentions.is_empty());
        let settled = runtime.settle_wake(&claim.claim_token).unwrap();
        assert_eq!(settled.generation, initial.generation + 1);
        assert_ne!(settled.process_s, initial.process_s);
        let semantic = runtime
            .store
            .latest_semantic_v1(&persona_scope)
            .unwrap()
            .unwrap();
        assert_eq!(semantic.transition_kind, SemanticTransitionKindV1::Time);
        assert!(semantic.evidence_digest.is_none());
        assert!(semantic.estimator_digest.is_none());
        assert!(semantic.receipt_bytes.is_none());
        assert!(semantic.telemetry_bytes.is_none());
        assert!(semantic.time_authority.is_some());
        // The consumed claim is no longer required: exact retries resolve the
        // committed settlement and must not create a second journal/semantic row.
        assert_eq!(runtime.settle_wake(&claim.claim_token).unwrap(), settled);
        assert_eq!(runtime.current_revision(&scope).unwrap(), 1);
        assert_eq!(runtime.semantic_revision_v1(&scope).unwrap(), 1);
        let replay = runtime
            .verify_replay(&scope.bot_token, &scope.persona_token)
            .unwrap();
        assert!(replay.ok, "{:?}", replay.first_error);
        assert_eq!(replay.checked, 1);
        let projection = runtime.verify_autonomy_projection(&scope).unwrap();
        assert!(projection.ok, "{:?}", projection.first_error);
        let snapshot = runtime
            .store
            .read_autonomy_snapshot(&persona_scope, 1)
            .unwrap()
            .unwrap();
        let snapshot_state: ae_contracts::AutonomousRuntimeStateV1 =
            serde_json::from_slice(&snapshot.1).unwrap();
        assert_eq!(snapshot_state, settled);
        drop(runtime);

        let mut reopened = AstrRuntime::open(&path).unwrap();
        let status = reopened.autonomy_status(Some(&scope)).unwrap();
        assert_eq!(status, vec![(scope, settled)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_persona_wake_preserves_relation_bindings_without_contact_work() {
        let dir = temp_dir("two-relations");
        let path = dir.join("store.db");
        let mut runtime = AstrRuntime::open(&path).unwrap();
        let genesis = request(19);
        runtime.ensure_genesis(&genesis).unwrap();
        let runtime_scope = genesis.source.scope_persona_scope();
        let persona_scope = wire::persona_scope_digest(
            &runtime_scope.bot_token,
            &runtime_scope.persona_token,
            None,
        );
        let profile = PersonaTemporalProfileV1 {
            schema_version: 1,
            persona_scope,
            home_timezone: "America/Los_Angeles".into(),
            current_timezone: "America/Los_Angeles".into(),
            chronotype: ChronotypeV1::NightOwl,
            preferred_sleep_local_minute: 90,
            preferred_wake_local_minute: 570,
            sleep_flex_minutes: 120,
            entrainment_rate_minutes_per_day: 90,
            revision: 1,
        };
        let make_policy = |relation_scope| RelationTemporalPolicyV1 {
            schema_version: 1,
            relation_scope,
            user_timezone: "Asia/Shanghai".into(),
            timezone_source: TimezoneSourceV1::Explicit,
            quiet_hours_start_minute: 30,
            quiet_hours_end_minute: 510,
            quiet_hours_emergency_bypass: true,
            proactive_enabled: true,
            proactive_daily_max: 2,
            min_proactive_cooldown_ms: 21_600_000,
            intention_ttl_ms: 86_400_000,
            unanswered_backoff_base_ms: 21_600_000,
            unanswered_hard_stop: 3,
            emergency_threshold: Fixed::from_raw(900_000),
            daily_submitted: 0,
            consecutive_unanswered: 0,
            last_inbound_utc_ms: Some(1),
            last_proactive_submitted_utc_ms: None,
            revision: 1,
            auto_policy_version: 0,
            next_claim_reservation_tokens: 256,
        };
        let mut first_scope = runtime_scope.clone();
        first_scope.relation_token = Some([51; 16]);
        first_scope.session_token = [52; 16];
        let first_relation = wire::persona_scope_digest(
            &first_scope.bot_token,
            &first_scope.persona_token,
            first_scope.relation_token.as_ref(),
        );
        let initial = runtime
            .bootstrap_autonomy(&first_scope, &profile, Some(&make_policy(first_relation)))
            .unwrap();
        let mut second_scope = runtime_scope.clone();
        second_scope.relation_token = Some([53; 16]);
        second_scope.session_token = [54; 16];
        let second_relation = wire::persona_scope_digest(
            &second_scope.bot_token,
            &second_scope.persona_token,
            second_scope.relation_token.as_ref(),
        );
        runtime
            .bootstrap_autonomy(&second_scope, &profile, Some(&make_policy(second_relation)))
            .unwrap();
        let now = 10 * 86_400_000;
        let frozen = FrozenTimeInputV1 {
            schema_version: 1,
            observed_now_utc_ms: now,
            effective_now_utc_ms: now,
            persona_tzid: "America/Los_Angeles".into(),
            persona_utc_offset_seconds: -25_200,
            persona_local_minute: 300,
            persona_day_ordinal: 739_854,
            relation_tzid: "Asia/Shanghai".into(),
            relation_utc_offset_seconds: 28_800,
            relation_local_minute: 1_200,
            relation_day_ordinal: 739_854,
            budget_day_start_utc_ms: now - now % 86_400_000,
            budget_next_day_start_utc_ms: now - now % 86_400_000 + 86_400_000,
            next_timezone_transition_utc_ms: None,
            tzdb_fingerprint: [4; 32],
        };
        let event = TimeAdvanceV1 {
            event_id: [55; 16],
            scope: runtime_scope.clone(),
            expected_generation: initial.generation,
            frozen_input_digest: frozen_time_input_digest(&frozen),
            frozen,
            stimulus: Default::default(),
        };
        let claim = runtime.claim_wake(&runtime_scope, &event).unwrap();
        assert!(claim.proposal.intentions.is_empty());
        runtime.settle_wake(&claim.claim_token).unwrap();
        assert_eq!(runtime.autonomy_status(None).unwrap().len(), 1);
        assert_eq!(
            runtime.autonomy_work_scopes(&persona_scope).unwrap().len(),
            2
        );
        assert_eq!(
            runtime
                .pending_autonomy_work(&first_scope)
                .unwrap()
                .intentions
                .len(),
            0
        );
        assert_eq!(
            runtime
                .pending_autonomy_work(&second_scope)
                .unwrap()
                .intentions
                .len(),
            0
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "legacy proactive lane is intentionally inactive after Task8"]
    fn endogenous_wake_forms_one_deduplicated_intention() {
        let dir = temp_dir("endogenous");
        let store_path = dir.join("store.db");
        let mut runtime = AstrRuntime::open(&store_path).unwrap();
        let genesis = request(10);
        runtime.ensure_genesis(&genesis).unwrap();
        let mut scope = genesis.source.scope_persona_scope();
        scope.relation_token = Some([42; 16]);
        let persona_scope =
            wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
        let relation_scope = wire::persona_scope_digest(
            &scope.bot_token,
            &scope.persona_token,
            scope.relation_token.as_ref(),
        );
        let profile = PersonaTemporalProfileV1 {
            schema_version: 1,
            persona_scope,
            home_timezone: "America/Los_Angeles".into(),
            current_timezone: "America/Los_Angeles".into(),
            chronotype: ChronotypeV1::NightOwl,
            preferred_sleep_local_minute: 90,
            preferred_wake_local_minute: 570,
            sleep_flex_minutes: 120,
            entrainment_rate_minutes_per_day: 90,
            revision: 1,
        };
        let now = 10 * 86_400_000;
        let relation = RelationTemporalPolicyV1 {
            schema_version: 1,
            relation_scope,
            user_timezone: "Asia/Shanghai".into(),
            timezone_source: TimezoneSourceV1::Explicit,
            quiet_hours_start_minute: 30,
            quiet_hours_end_minute: 510,
            quiet_hours_emergency_bypass: true,
            proactive_enabled: true,
            proactive_daily_max: 2,
            min_proactive_cooldown_ms: 21_600_000,
            intention_ttl_ms: 86_400_000,
            unanswered_backoff_base_ms: 21_600_000,
            unanswered_hard_stop: 3,
            emergency_threshold: Fixed::from_raw(900_000),
            daily_submitted: 0,
            consecutive_unanswered: 0,
            last_inbound_utc_ms: Some(1),
            last_proactive_submitted_utc_ms: None,
            revision: 1,
            auto_policy_version: 0,
            next_claim_reservation_tokens: 256,
        };
        let initial = runtime
            .bootstrap_autonomy(&scope, &profile, Some(&relation))
            .unwrap();
        let frozen = FrozenTimeInputV1 {
            schema_version: 1,
            observed_now_utc_ms: now,
            effective_now_utc_ms: now,
            persona_tzid: "America/Los_Angeles".into(),
            persona_utc_offset_seconds: -25_200,
            persona_local_minute: 300,
            persona_day_ordinal: 739_854,
            relation_tzid: "Asia/Shanghai".into(),
            relation_utc_offset_seconds: 28_800,
            relation_local_minute: 1_200,
            relation_day_ordinal: 739_854,
            budget_day_start_utc_ms: now - now % 86_400_000,
            budget_next_day_start_utc_ms: now - now % 86_400_000 + 86_400_000,
            next_timezone_transition_utc_ms: None,
            tzdb_fingerprint: [4; 32],
        };
        let event = TimeAdvanceV1 {
            event_id: [8; 16],
            scope: scope.clone(),
            expected_generation: initial.generation,
            frozen_input_digest: frozen_time_input_digest(&frozen),
            frozen,
            stimulus: Default::default(),
        };
        let first = runtime.claim_wake(&scope, &event).unwrap();
        let repeated = runtime.claim_wake(&scope, &event).unwrap();
        assert_eq!(first.claim_token, repeated.claim_token);
        assert_eq!(first.proposal.intentions.len(), 1);
        assert_eq!(first.proposal.intentions, repeated.proposal.intentions);
        let intention = first.proposal.intentions[0].clone();
        runtime.settle_wake(&first.claim_token).unwrap();
        let journal = runtime.store.read_journal(&persona_scope).unwrap();
        assert_eq!(journal.len(), 3);
        assert_eq!(journal[0].event_kind, "time_advance");
        assert_eq!(journal[1].event_kind, "self_action_candidate");
        assert_eq!(journal[2].event_kind, "operational_checkpoint_v1");
        let wake_delta: AutonomyJournalDeltaV1 =
            serde_json::from_slice(&journal[0].delta_bytes).unwrap();
        let action_delta: AutonomyJournalDeltaV1 =
            serde_json::from_slice(&journal[1].delta_bytes).unwrap();
        assert!(wake_delta.intention.is_none());
        assert_eq!(action_delta.intention.as_ref(), Some(&intention));
        let projection = runtime.verify_autonomy_projection(&scope).unwrap();
        assert!(projection.ok, "{:?}", projection.first_error);
        assert_eq!(projection.checked_rows, 3);
        drop(runtime);
        let connection = rusqlite::Connection::open(&store_path).unwrap();
        connection
            .execute_batch(
                "DELETE FROM autonomous_runtime_state;
                 DELETE FROM autonomy_snapshot;
                 DELETE FROM wake_schedule;
                 DELETE FROM inner_event;
                 DELETE FROM durable_intention;
                 DELETE FROM autonomy_scope_binding;",
            )
            .unwrap();
        drop(connection);
        let mut runtime = AstrRuntime::open(&store_path).unwrap();
        assert_eq!(
            runtime.autonomy_status(Some(&scope)).unwrap()[0]
                .1
                .generation,
            1
        );
        assert_eq!(
            runtime
                .pending_autonomy_work(&scope)
                .unwrap()
                .intentions
                .len(),
            1
        );
        assert!(runtime.verify_autonomy_projection(&scope).unwrap().ok);
        assert!(runtime.rebuild_autonomy_projection(&scope).unwrap().ok);
        assert!(runtime.rebuild_autonomy_projection(&scope).unwrap().ok);
        drop(runtime);
        let connection = rusqlite::Connection::open(&store_path).unwrap();
        connection
            .execute(
                "UPDATE autonomous_runtime_state SET body_json='corrupt' WHERE persona_scope=?1",
                rusqlite::params![persona_scope.to_vec()],
            )
            .unwrap();
        drop(connection);
        let mut runtime = AstrRuntime::open(&store_path).unwrap();
        assert_eq!(
            runtime.autonomy_status(Some(&scope)).unwrap()[0]
                .1
                .generation,
            1
        );
        assert!(runtime.verify_autonomy_projection(&scope).unwrap().ok);
        drop(runtime);
        let mut forged_scope = scope.clone();
        forged_scope.bot_token = [99; 16];
        let connection = rusqlite::Connection::open(&store_path).unwrap();
        connection
            .execute(
                "UPDATE autonomy_scope_binding SET scope_json=?2 WHERE persona_scope=?1",
                rusqlite::params![
                    persona_scope.to_vec(),
                    serde_json::to_string(&forged_scope).unwrap()
                ],
            )
            .unwrap();
        drop(connection);
        let mut runtime = AstrRuntime::open(&store_path).unwrap();
        assert!(runtime
            .store
            .list_autonomy_scopes()
            .unwrap()
            .iter()
            .any(|value| value == &scope));
        let mut target = OutboundTargetEnvelopeV1 {
            schema_version: 1,
            target_kind: TargetKindV1::Private,
            umo_ciphertext: vec![1; 32],
            umo_nonce: vec![2; 12],
            key_id: "test-key".into(),
            umo_digest: [3; 32],
            platform_token: [4; 16],
            bot_token: scope.bot_token,
            persona_token: scope.persona_token,
            relation_token: scope.relation_token.unwrap(),
            session_token: scope.session_token,
            bound_at_utc_ms: now,
            binding_generation: 1,
            binding_digest: [0; 32],
        };
        target.binding_digest = outbound_target_binding_digest(&target);
        runtime.bind_outbound_target(&scope, &target).unwrap();
        let mut capability = HostCapabilitySnapshotV1 {
            schema_version: 1,
            astrbot_send_available: true,
            credential_store_available: true,
            platform_idempotent: false,
            provider_identifier: "test-provider".into(),
            config_source_digest: [5; 32],
            config_revision: 1,
            content_boundary_version: 1,
            policy_version: 1,
            snapshot_digest: [0; 32],
        };
        capability.snapshot_digest = host_capability_snapshot_digest(&capability);
        let _externalization = runtime
            .gate_and_claim_externalization(&GateAndClaimExternalizationRequestV1 {
                scope: scope.clone(),
                intention_id: intention.intention_id,
                attempt_no: 1,
                expected_revision: 0,
                caller_incarnation: [55; 32],
                capability: capability.clone(),
                frozen: event.frozen.clone(),
            })
            .unwrap()
            .claim
            .unwrap();
        drop(runtime);
        let connection = rusqlite::Connection::open(&store_path).unwrap();
        connection
            .execute_batch(
                "DELETE FROM durable_intention;
                 DELETE FROM outbound_attempt;
                 DELETE FROM autonomy_claim;
                 DELETE FROM externalization_budget_claim;
                 DELETE FROM externalization_budget;",
            )
            .unwrap();
        drop(connection);
        let mut runtime = AstrRuntime::open(&store_path).unwrap();
        let recovered = runtime.pending_autonomy_work(&scope).unwrap().intentions;
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].intention.state, IntentionStateV1::Deferred);
        assert_eq!(recovered[0].intention.externalization_attempts, 1);
        assert_eq!(
            runtime
                .store
                .externalization_reserved_tokens(
                    &intention.relation_scope,
                    event.frozen.budget_day_start_utc_ms,
                )
                .unwrap(),
            512
        );
        let externalization = runtime
            .gate_and_claim_externalization(&GateAndClaimExternalizationRequestV1 {
                scope: scope.clone(),
                intention_id: intention.intention_id,
                attempt_no: 2,
                expected_revision: 2,
                caller_incarnation: [59; 32],
                capability: capability.clone(),
                frozen: event.frozen.clone(),
            })
            .unwrap()
            .claim
            .unwrap();
        let materialized = runtime
            .settle_externalization(&ExternalizationSettleV1 {
                claim_token: externalization.claim_token,
                caller_incarnation: [59; 32],
                outcome: ExternalizationOutcomeV1::Success,
                used_tokens: Some(100),
                candidate_digest: Some([56; 32]),
                candidate_ciphertext: Some(vec![57; 48]),
            })
            .unwrap();
        assert_eq!(materialized.state, IntentionStateV1::DispatchPending);
        let outbound = runtime
            .pending_autonomy_work(&scope)
            .unwrap()
            .outbounds
            .pop()
            .unwrap();
        let dispatch = runtime
            .gate_and_claim_dispatch(&GateAndClaimDispatchRequestV1 {
                scope: scope.clone(),
                outbound_id: outbound.outbound_id,
                expected_target_digest: target.binding_digest,
                caller_incarnation: [58; 32],
                capability: capability.clone(),
                frozen: event.frozen.clone(),
            })
            .unwrap()
            .claim
            .unwrap();
        runtime
            .settle_dispatch(&DispatchSettleV1 {
                claim_token: dispatch.claim_token,
                outcome: DispatchOutcomeV1::AdapterSubmitted,
                settled_at_utc_ms: event.frozen.effective_now_utc_ms,
                receipt_digest: None,
                caller_incarnation: [58; 32],
            })
            .unwrap();
        drop(runtime);
        let connection = rusqlite::Connection::open(&store_path).unwrap();
        connection
            .execute_batch(
                "DELETE FROM durable_intention;
                 DELETE FROM outbound_attempt;
                 DELETE FROM autonomy_claim;
                 DELETE FROM externalization_budget_claim;
                 DELETE FROM externalization_budget;",
            )
            .unwrap();
        drop(connection);
        let mut runtime = AstrRuntime::open(&store_path).unwrap();
        assert!(runtime
            .pending_autonomy_work(&scope)
            .unwrap()
            .intentions
            .is_empty());
        assert!(runtime
            .pending_autonomy_work(&scope)
            .unwrap()
            .outbounds
            .is_empty());
        assert_eq!(
            runtime
                .store
                .outbound_submission_metrics(
                    &intention.relation_scope,
                    event.frozen.budget_day_start_utc_ms,
                )
                .unwrap()
                .0,
            1
        );
        assert_eq!(
            runtime
                .store
                .externalization_reserved_tokens(
                    &intention.relation_scope,
                    event.frozen.budget_day_start_utc_ms,
                )
                .unwrap(),
            612
        );
        assert!(runtime.rebuild_autonomy_projection(&scope).unwrap().ok);
        assert!(runtime.rebuild_autonomy_projection(&scope).unwrap().ok);
        drop(runtime);
        let mut tampered = intention.clone();
        tampered.state = IntentionStateV1::Ready;
        let connection = rusqlite::Connection::open(&store_path).unwrap();
        connection
            .execute(
                "UPDATE durable_intention SET state='ready',revision=0,body_json=?2 WHERE intention_id=?1",
                rusqlite::params![
                    intention.intention_id.to_vec(),
                    serde_json::to_string(&tampered).unwrap()
                ],
            )
            .unwrap();
        drop(connection);
        let runtime = AstrRuntime::open(&store_path).unwrap();
        assert!(runtime
            .pending_autonomy_work(&scope)
            .unwrap()
            .intentions
            .is_empty());
        drop(runtime);
        let connection = rusqlite::Connection::open(&store_path).unwrap();
        connection
            .execute(
                "UPDATE autonomy_operational_authority_head SET head_digest=?2 WHERE persona_scope=?1",
                rusqlite::params![persona_scope.to_vec(), [0xEE_u8; 32].to_vec()],
            )
            .unwrap();
        drop(connection);
        drop(AstrRuntime::open(&store_path).unwrap());
        let connection = rusqlite::Connection::open(&store_path).unwrap();
        let repaired_head: Vec<u8> = connection
            .query_row(
                "SELECT head_digest FROM autonomy_operational_authority_head WHERE persona_scope=?1",
                rusqlite::params![persona_scope.to_vec()],
                |row| row.get(0),
            )
            .unwrap();
        assert_ne!(repaired_head, vec![0xEE_u8; 32]);
        drop(connection);
        let _ = std::fs::remove_dir_all(&dir);
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
