#![forbid(unsafe_code)]

//! PyO3 boundary: the ONLY surface Python may touch.
//!
//! Exposed: health, version, open, ensure_genesis, apply_event, inspect,
//! verify_replay, flush_and_close. There are no per-neuron getters, no
//! residual writers, no import-from-SeedCode entry point. JSON is exchanged
//! as closed, deny-unknown-field payloads; identity is computed in Rust.

use ae_context_projector::{ContextSummaryV1, DeliveryOutcome as ContextDeliveryOutcome};
use ae_contracts::{hex, wire, CanonicalEvent, PersonaGenesisRequest, ScopeRef};
use ae_store::{
    RebirthActionV1, RebirthAuditReceiptV1, RebirthOutcomeV1, RebirthPrepareRequestV1,
    RebirthResponseEnvelopeV1, RebirthResponseStateV1, UserAuthorizedRebirthV1,
};
use pyo3::create_exception;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use serde::Deserialize;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock};

create_exception!(_native, NativeCoreError, PyRuntimeError);

static CORE: OnceLock<Mutex<Option<ae_runtime::AstrRuntime>>> = OnceLock::new();

fn core() -> PyResult<MutexGuard<'static, Option<ae_runtime::AstrRuntime>>> {
    let mutex = CORE.get_or_init(|| Mutex::new(None));
    mutex
        .lock()
        .map_err(|_| NativeCoreError::new_err("POISONED::native core mutex poisoned"))
}

fn map_error(error: ae_runtime::RuntimeError) -> PyErr {
    let (code, message) = match &error {
        ae_runtime::RuntimeError::Genesis(_) => ("GENESIS_UNAVAILABLE", error.to_string()),
        ae_runtime::RuntimeError::RetryWait => ("RETRY_WAIT", error.to_string()),
        ae_runtime::RuntimeError::Rebirth(rebirth) => (rebirth.code(), error.to_string()),
        ae_runtime::RuntimeError::Store(ae_store::StoreError::SeedDigestCollision) => {
            ("SEED_DIGEST_COLLISION", error.to_string())
        }
        ae_runtime::RuntimeError::Store(ae_store::StoreError::StaleRevision { .. }) => {
            ("STALE_REVISION", error.to_string())
        }
        ae_runtime::RuntimeError::Store(ae_store::StoreError::DuplicateEvent(_)) => {
            ("DUPLICATE_EVENT", error.to_string())
        }
        ae_runtime::RuntimeError::Store(ae_store::StoreError::LeaseConflict) => {
            ("LEASE_CONFLICT", error.to_string())
        }
        ae_runtime::RuntimeError::Store(ae_store::StoreError::LeaseInFlight) => {
            ("LEASE_IN_FLIGHT", error.to_string())
        }
        ae_runtime::RuntimeError::Store(ae_store::StoreError::ManifestDigestMismatch)
        | ae_runtime::RuntimeError::Store(ae_store::StoreError::SeedCodeMismatch) => {
            ("IDENTITY_MISMATCH", error.to_string())
        }
        ae_runtime::RuntimeError::Store(_) => ("STORAGE", error.to_string()),
        ae_runtime::RuntimeError::PersonaGenesisRequired => ("GENESIS_REQUIRED", error.to_string()),
        ae_runtime::RuntimeError::GenesisManifestMismatch => {
            ("GENESIS_MANIFEST_MISMATCH", error.to_string())
        }
        ae_runtime::RuntimeError::StaleCausalBase { .. } => {
            ("STALE_CAUSAL_BASE", error.to_string())
        }
        ae_runtime::RuntimeError::UnsupportedEvent(_) => ("UNSUPPORTED_EVENT", error.to_string()),
        ae_runtime::RuntimeError::Closed => ("CLOSED", error.to_string()),
        ae_runtime::RuntimeError::InvalidNeuralState => ("INVALID_NEURAL_STATE", error.to_string()),
        ae_runtime::RuntimeError::ContextReceipt(_) => {
            ("CONTEXT_RECEIPT_INVALID", error.to_string())
        }
        ae_runtime::RuntimeError::ContextProjection(_) => ("CONTEXT_PROJECTION", error.to_string()),
        ae_runtime::RuntimeError::ContextCommitMissing => {
            ("CONTEXT_COMMIT_MISSING", error.to_string())
        }
        ae_runtime::RuntimeError::ContextCommitIntegrity => {
            ("CONTEXT_COMMIT_INTEGRITY", error.to_string())
        }
    };
    NativeCoreError::new_err(format!("{code}::{message}"))
}

fn closed_schema(message: String) -> PyErr {
    NativeCoreError::new_err(format!("CLOSED_SCHEMA::{message}"))
}

fn context_summary_payload(summary: &ContextSummaryV1) -> serde_json::Value {
    let delivery_outcome = match summary.delivery_outcome {
        ContextDeliveryOutcome::Pending => "pending",
        ContextDeliveryOutcome::Delivered => "delivered",
        ContextDeliveryOutcome::Failed => "failed",
    };
    serde_json::json!({
        "schema": "astrembodiment.context-summary.v1",
        "summary_revision": summary.summary_revision,
        "source_continuum_revision": summary.source_continuum_revision,
        "dimensions_ema_fxp6": summary.dimensions_ema_fxp6,
        "unresolved_boundary": summary.unresolved_boundary,
        "unresolved_repair": summary.unresolved_repair,
        "repetition_count": summary.repetition_count,
        "delivery_outcome": delivery_outcome,
        "summary_digest": hex::encode32(&summary.summary_digest),
    })
}

fn rebirth_action_name(action: &RebirthActionV1) -> &'static str {
    match action {
        RebirthActionV1::Rebirth => "REBIRTH",
        RebirthActionV1::ClearActiveState => "CLEAR_ACTIVE_STATE",
    }
}

fn rebirth_response_state_name(state: &RebirthResponseStateV1) -> &'static str {
    match state {
        RebirthResponseStateV1::ConfirmationPending => "CONFIRMATION_PENDING",
        RebirthResponseStateV1::Committed => "COMMITTED",
        RebirthResponseStateV1::Replayed => "REPLAYED",
    }
}

fn rebirth_outcome_name(outcome: &RebirthOutcomeV1) -> &'static str {
    match outcome {
        RebirthOutcomeV1::Committed => "COMMITTED",
    }
}

fn rebirth_receipt_payload(receipt: &RebirthAuditReceiptV1) -> serde_json::Value {
    serde_json::json!({
        "receipt_id": hex::encode32(&receipt.receipt_id),
        "action": rebirth_action_name(&receipt.action),
        "scope_token_short": receipt.scope_token_short.as_str(),
        "request_nonce_digest": hex::encode32(&receipt.request_nonce_digest),
        "parent_incarnation_short": receipt.parent_incarnation_short.as_str(),
        "child_incarnation_short": receipt.child_incarnation_short.as_str(),
        "before_revision": receipt.before_revision,
        "after_revision": receipt.after_revision,
        "outcome": rebirth_outcome_name(&receipt.outcome),
        "audit_time_ms": receipt.audit_time_ms,
    })
}

fn rebirth_envelope_payload(envelope: &RebirthResponseEnvelopeV1) -> serde_json::Value {
    serde_json::json!({
        "schema": "astrembodiment.rebirth-response.v1",
        "state": rebirth_response_state_name(&envelope.state),
        "receipt": envelope.receipt.as_ref().map(rebirth_receipt_payload),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FfiScope {
    bot_token: String,
    persona_token: String,
    session_token: String,
    relation_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FfiEventEnvelope {
    kind: String,
    #[serde(rename = "payload")]
    _payload: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum FfiRebirthActionV1 {
    Rebirth,
    ClearActiveState,
}

impl FfiRebirthActionV1 {
    fn into_runtime(self) -> RebirthActionV1 {
        match self {
            Self::Rebirth => RebirthActionV1::Rebirth,
            Self::ClearActiveState => RebirthActionV1::ClearActiveState,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FfiRebirthPrepareRequestV1 {
    scope: FfiScope,
    expected_incarnation_id: String,
    expected_revision: u64,
    action: FfiRebirthActionV1,
}

impl FfiRebirthPrepareRequestV1 {
    fn into_runtime(self) -> Result<(ScopeRef, RebirthPrepareRequestV1), String> {
        let scope = self.scope.scope_ref()?;
        let scope_token = wire::persona_scope_digest(
            &scope.bot_token,
            &scope.persona_token,
            scope.relation_token.as_ref(),
        );
        Ok((
            scope,
            RebirthPrepareRequestV1 {
                scope_token,
                expected_incarnation_id: hex::decode32(&self.expected_incarnation_id)?,
                expected_revision: self.expected_revision,
                action: self.action.into_runtime(),
            },
        ))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FfiUserAuthorizedRebirthV1 {
    scope: FfiScope,
    expected_incarnation_id: String,
    expected_revision: u64,
    request_nonce: String,
    action: FfiRebirthActionV1,
    confirmed: Option<bool>,
}

impl FfiUserAuthorizedRebirthV1 {
    fn into_runtime(self) -> Result<(ScopeRef, UserAuthorizedRebirthV1), String> {
        let scope = self.scope.scope_ref()?;
        let scope_token = wire::persona_scope_digest(
            &scope.bot_token,
            &scope.persona_token,
            scope.relation_token.as_ref(),
        );
        Ok((
            scope,
            UserAuthorizedRebirthV1 {
                scope_token,
                expected_incarnation_id: hex::decode32(&self.expected_incarnation_id)?,
                expected_revision: self.expected_revision,
                request_nonce: hex::decode32(&self.request_nonce)?,
                action: self.action.into_runtime(),
                // Missing consent deliberately becomes false so the durable
                // lifecycle owner emits REBIRTH_CONFIRMATION_REQUIRED rather
                // than serde text or an implicit Python/Rust default-true.
                confirmed: self.confirmed.unwrap_or(false),
            },
        ))
    }
}

fn is_known_g0_unsupported_event(kind: &str) -> bool {
    matches!(
        kind,
        "user_reaction"
            | "correction_claim"
            | "correction_verdict"
            | "self_action_candidate"
            | "settlement_evidence"
            | "admin_action"
    )
}

impl FfiScope {
    fn scope_ref(&self) -> Result<ScopeRef, String> {
        Ok(ScopeRef {
            bot_token: hex::decode16(&self.bot_token)?,
            persona_token: hex::decode16(&self.persona_token)?,
            relation_token: self
                .relation_token
                .as_deref()
                .map(hex::decode16)
                .transpose()?,
            session_token: hex::decode16(&self.session_token)?,
        })
    }
}

#[pyfunction]
fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[pyfunction]
fn health() -> String {
    r#"{"status":"g0-ready","formula":"aster-ccn-v1","neuron_slots":16384,"version":"1.0.0"}"#
        .to_owned()
}

/// Open (or replace) the single production runtime with its own SQLite store.
#[pyfunction]
fn open(data_dir: &str) -> PyResult<()> {
    let mut guard = core()?;
    if let Some(mut previous) = guard.take() {
        previous.flush_and_close().map_err(map_error)?;
    }
    let runtime = ae_runtime::AstrRuntime::open(Path::new(data_dir)).map_err(map_error)?;
    *guard = Some(runtime);
    Ok(())
}

/// Submit one closed PersonaGenesisRequest. Python may compile the proposal,
/// but only Rust projects the Manifest, derives SeedCode/IncarnationId and
/// commits the birth. Concurrent callers join the same committed receipt.
#[pyfunction]
fn ensure_genesis(request_json: &str) -> PyResult<String> {
    let request: PersonaGenesisRequest =
        serde_json::from_str(request_json).map_err(|error| closed_schema(error.to_string()))?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let receipt = runtime.ensure_genesis(&request).map_err(map_error)?;
    let payload = serde_json::json!({
        "schema": "astrembodiment.genesis-receipt.v1",
        "lease_status": "committed",
        "receipt": receipt,
        "manifest": receipt.manifest_digest,
        "seed_code": ae_genesis::format_seed_code(&receipt.seed_code_digest),
        "seed_code_short": ae_genesis::format_short_seed_code(&receipt.seed_code_digest),
        "incarnation_id": ae_genesis::format_incarnation_id(&receipt.incarnation_id),
    });
    serde_json::to_string(&payload)
        .map_err(|error| NativeCoreError::new_err(format!("ENCODING::{error}")))
}

/// Create a durable confirmation challenge for one explicit destructive
/// lifecycle action. The raw nonce is emitted only in this immediate response.
#[pyfunction]
fn prepare_rebirth_v1(request_json: &str) -> PyResult<String> {
    let request: FfiRebirthPrepareRequestV1 =
        serde_json::from_str(request_json).map_err(|error| closed_schema(error.to_string()))?;
    let (scope, request) = request.into_runtime().map_err(closed_schema)?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let response = runtime
        .prepare_rebirth_v1(&scope, &request)
        .map_err(map_error)?;
    let payload = serde_json::json!({
        "schema": "astrembodiment.rebirth-prepare.v1",
        "state": rebirth_response_state_name(&response.state),
        "request_nonce": hex::encode32(&response.request_nonce),
        "request_nonce_digest": hex::encode32(&response.request_nonce_digest),
        "binding_digest": hex::encode32(&response.binding_digest),
    });
    serde_json::to_string(&payload)
        .map_err(|error| NativeCoreError::new_err(format!("ENCODING::{error}")))
}

/// Confirm a previously prepared destructive lifecycle action. Missing or
/// false consent stays a fixed Rust rejection; this boundary never supplies it.
#[pyfunction]
fn confirm_rebirth_v1(request_json: &str) -> PyResult<String> {
    let request: FfiUserAuthorizedRebirthV1 =
        serde_json::from_str(request_json).map_err(|error| closed_schema(error.to_string()))?;
    let (scope, confirmation) = request.into_runtime().map_err(closed_schema)?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let response = runtime
        .confirm_rebirth_v1(&scope, &confirmation)
        .map_err(map_error)?;
    serde_json::to_string(&rebirth_envelope_payload(&response))
        .map_err(|error| NativeCoreError::new_err(format!("ENCODING::{error}")))
}

/// Apply one closed canonical event through the deterministic G0 no-op lane.
#[pyfunction]
fn apply_event(scope_json: &str, event_json: &str) -> PyResult<String> {
    let scope: FfiScope =
        serde_json::from_str(scope_json).map_err(|error| closed_schema(error.to_string()))?;
    let scope_ref = scope.scope_ref().map_err(closed_schema)?;
    let envelope: FfiEventEnvelope =
        serde_json::from_str(event_json).map_err(|error| closed_schema(error.to_string()))?;
    if is_known_g0_unsupported_event(&envelope.kind) {
        return Err(NativeCoreError::new_err(format!(
            "UNSUPPORTED_EVENT::event kind {} is not supported by the G0 no-op lane",
            envelope.kind
        )));
    }
    let event: CanonicalEvent =
        serde_json::from_str(event_json).map_err(|error| closed_schema(error.to_string()))?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let decision = runtime.apply_event(&scope_ref, &event).map_err(map_error)?;
    let payload = serde_json::json!({
        "schema": "astrembodiment.decision.v1",
        "contract": decision.contract,
        "receipt": decision.receipt,
        "revision": decision.revision,
        "deduplicated": decision.deduplicated,
        "context_summary": context_summary_payload(&decision.context_summary),
    });
    serde_json::to_string(&payload)
        .map_err(|error| NativeCoreError::new_err(format!("ENCODING::{error}")))
}

/// Content-free observatory projection for one (Bot, Persona) binding.
#[pyfunction]
fn inspect(scope_json: &str) -> PyResult<String> {
    let scope: FfiScope =
        serde_json::from_str(scope_json).map_err(|error| closed_schema(error.to_string()))?;
    let scope_ref = scope.scope_ref().map_err(closed_schema)?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let report = runtime
        .inspect(&scope_ref.bot_token, &scope_ref.persona_token)
        .map_err(map_error)?;
    let payload = serde_json::json!({
        "schema": "astrembodiment.inspect.v1",
        "bound": report.bound,
        "bot_token": hex::encode16(&report.bot_token),
        "persona_token": hex::encode16(&report.persona_token),
        "seed_code": report.seed_code,
        "seed_code_short": report.seed_code_short,
        "incarnation_id": report.incarnation_id,
        "revision": report.revision,
        "initial_snapshot_digest": hex::encode32(&report.initial_snapshot_digest),
        "last_chain_digest": report.last_chain_digest.map(|d| hex::encode32(&d)),
        "journal_count": report.journal_count,
        "observatory": {
            "genesis_unavailable": report.observatory_genesis_unavailable,
        },
    });
    serde_json::to_string(&payload)
        .map_err(|error| NativeCoreError::new_err(format!("ENCODING::{error}")))
}

/// Mechanical replay verification of the committed journal.
#[pyfunction]
fn verify_replay(scope_json: &str) -> PyResult<String> {
    let scope: FfiScope =
        serde_json::from_str(scope_json).map_err(|error| closed_schema(error.to_string()))?;
    let scope_ref = scope.scope_ref().map_err(closed_schema)?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let report = runtime
        .verify_replay(&scope_ref.bot_token, &scope_ref.persona_token)
        .map_err(map_error)?;
    let payload = serde_json::json!({
        "schema": "astrembodiment.replay-report.v1",
        "checked": report.checked,
        "ok": report.ok,
        "base_revision": report.base_revision,
        "final_revision": report.final_revision,
        "final_chain_digest": hex::encode32(&report.final_chain_digest),
        "first_error": report.first_error,
    });
    serde_json::to_string(&payload)
        .map_err(|error| NativeCoreError::new_err(format!("ENCODING::{error}")))
}

/// Drain the writer: snapshot, WAL checkpoint, close the store.
#[pyfunction]
fn flush_and_close() -> PyResult<()> {
    let mut guard = core()?;
    if let Some(mut runtime) = guard.take() {
        runtime.flush_and_close().map_err(map_error)?;
    }
    Ok(())
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(version, module)?)?;
    module.add_function(wrap_pyfunction!(health, module)?)?;
    module.add_function(wrap_pyfunction!(open, module)?)?;
    module.add_function(wrap_pyfunction!(ensure_genesis, module)?)?;
    module.add_function(wrap_pyfunction!(prepare_rebirth_v1, module)?)?;
    module.add_function(wrap_pyfunction!(confirm_rebirth_v1, module)?)?;
    module.add_function(wrap_pyfunction!(apply_event, module)?)?;
    module.add_function(wrap_pyfunction!(inspect, module)?)?;
    module.add_function(wrap_pyfunction!(verify_replay, module)?)?;
    module.add_function(wrap_pyfunction!(flush_and_close, module)?)?;
    module.add("NativeCoreError", module.py().get_type::<NativeCoreError>())?;
    Ok(())
}
