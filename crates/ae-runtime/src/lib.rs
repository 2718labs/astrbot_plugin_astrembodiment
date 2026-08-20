#![forbid(unsafe_code)]

//! AstrRuntime: the G0 vertical slice orchestrator.
//!
//! ensure_genesis -> deterministic no-op apply_event -> SQLite commit ->
//! replay verification. Python cannot reach any of this state directly; the
//! PyO3 surface exposes only coarse calls.

use ae_agent::noop_action_contract;
use ae_authority::authority_projection_digest;
use ae_continuum::{CommitEnvelope, ReplayReport};
use ae_contracts::{
    wire, ActionContract, CanonicalEvent, CommitStatus, Digest, GenesisReceipt, GenesisStatus,
    Id128, InvariantResiduals, PersonaGenesisRequest, ScopeRef, TransitionReceipt,
};
use ae_fixed::Fixed;
use ae_neurofield::{
    graph_digest, initial_state_from_manifest, state_digest, NeuralField, SparseGraph, Synapse,
    EDGE_CAPACITY, NEURON_SLOTS,
};
use ae_store::{ClaimOutcome, GenesisCommit, Store, StoreError};
use std::path::Path;
use thiserror::Error;

/// Native R7 atomic projection remains a Rust-only additive namespace.
pub mod r7;

const CANONICAL_HOT_STATE_MAGIC_V1: [u8; 8] = *b"AEHOTST\0";
const CANONICAL_HOT_STATE_SCHEMA_V1: u16 = 1;
const CANONICAL_HOT_STATE_VECTOR_COUNT: usize = 8;
const SYNAPSE_WIRE_BYTES: usize = 16;

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
}

pub struct AstrRuntime {
    store: Store,
    hot: Option<HotBrain>,
    // This is deliberately independent of the durable G0 store.  R7 owns its
    // typed semantic transaction and opaque wire; the legacy shell cannot
    // construct, inspect, or mutate that transaction directly.
    r7_runtime: r7::AstrRuntime,
}

fn fixed_zero_vector() -> InvariantResiduals {
    InvariantResiduals::default()
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

impl AstrRuntime {
    pub fn open(path: &Path) -> Result<Self, RuntimeError> {
        Ok(Self {
            store: Store::open(path)?,
            hot: None,
            r7_runtime: r7::AstrRuntime::scaffold(),
        })
    }

    /// Rust-only additive R7 ingress.  It takes the authority's closed typed
    /// source and returns only its opaque, one-shot decision capability.
    /// Python keeps its unchanged G0 compatibility surface and has no route
    /// to this method, a raw wire, or a source-state mutation API.
    pub fn apply_user_stimulus_with_private_projection_wire_v1(
        &mut self,
        event: &ae_contracts::r7::CanonicalEvent,
        input: &r7::R7PreOutputProjectionInputV1,
    ) -> Result<r7::RuntimeDecision, r7::RuntimeError> {
        self.r7_runtime
            .apply_user_stimulus_with_private_projection_wire_v1(event, input)
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
        let persona_scope = wire::persona_scope_digest(&bot_token, &persona_token, None);
        let revision = self.store.current_revision(&persona_scope)?;
        let snapshot = self
            .store
            .read_snapshot(&persona_scope, 0)?
            .ok_or(RuntimeError::InvalidNeuralState)?;
        if snapshot.state_digest != committed.receipt.initial_snapshot_digest {
            return Err(RuntimeError::InvalidNeuralState);
        }
        let (field, graph) = decode_hot_state_v1(
            &snapshot.state_bytes,
            &committed.receipt.formula_digest,
            &committed.receipt.initial_snapshot_digest,
            &committed.receipt.graph_digest,
        )?;
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
        }
        self.hot
            .as_mut()
            .ok_or(RuntimeError::PersonaGenesisRequired)
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
            persona_scope,
            hot_revision,
            formula_digest,
            manifest_digest,
            initial_snapshot_digest,
            state_before,
            graph_after,
            active_nodes,
            active_edges,
        ) = {
            let hot = self.hot_for(scope)?;
            (
                hot.bot_token,
                hot.persona_token,
                hot.persona_scope,
                hot.revision,
                hot.formula_digest,
                hot.identity.manifest_digest,
                hot.initial_snapshot_digest,
                state_digest(&hot.field, &hot.formula_digest),
                graph_digest(&hot.graph),
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

        let event_bytes = wire::encode_event(event);
        let event_digest = wire::event_digest(event);
        let contract = noop_action_contract(&manifest_digest, &event_digest, turn_id);
        let contract_digest = wire::action_contract_digest(&contract);

        // Idempotency: an event that was already applied is never applied
        // twice; the original receipt is returned unchanged.
        if let Some(row) = self.store.lookup_event(&persona_scope, &event_digest)? {
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
            CanonicalEvent::TimeAdvance(_) => hot_revision,
            _ => unreachable!(),
        };
        if causal_base != hot_revision {
            return Err(RuntimeError::StaleCausalBase {
                expected: hot_revision,
                actual: causal_base,
            });
        }

        let authority_digest = authority_projection_digest(event);
        let receipt = TransitionReceipt {
            schema_version: 1,
            formula_digest,
            scope_digest: persona_scope,
            event_digest,
            authority_digest,
            base_revision: hot_revision,
            next_revision: hot_revision + 1,
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
            .last_chain_digest(&persona_scope)?
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
                    hot.revision = revision;
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
                    .lookup_event(&persona_scope, &event_digest)?
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
                &encode_hot_state_v1(&hot.formula_digest, &hot.field, &hot.graph),
            )?;
        }
        self.store.flush()?;
        Ok(())
    }

    pub fn closed(&self) -> bool {
        matches!(self.store.count_leases(), Err(StoreError::Closed))
    }

    pub fn current_revision(&mut self, scope: &ScopeRef) -> Result<u64, RuntimeError> {
        let hot = self.hot_for(scope)?;
        Ok(hot.revision)
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
