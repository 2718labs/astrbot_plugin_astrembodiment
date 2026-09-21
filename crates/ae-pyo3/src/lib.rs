#![forbid(unsafe_code)]

//! PyO3 boundary: the ONLY surface Python may touch.
//!
//! Exposed: lifecycle/event APIs plus the closed body autonomy, projection,
//! wake, dispatch and settlement surface registered at the bottom of this
//! module. Retired world, dream, consent-admin and ecosystem operations are
//! absent from the strict alpha3 request decoder.
//! There are no per-neuron getters, no residual writers, and no
//! import-from-SeedCode entry point. JSON is exchanged as closed,
//! deny-unknown-field payloads; identity is computed in Rust.

use ae_contracts::{hex, PersonaGenesisRequest, ScopeRef, SemanticAppraisalSettleRequestV1};
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
        ae_runtime::RuntimeError::Store(ae_store::StoreError::AutonomyConflict(message))
            if message.starts_with("OBSERVE_INVALID_REQUEST::") =>
        {
            ("OBSERVE_INVALID_REQUEST", error.to_string())
        }
        ae_runtime::RuntimeError::Store(ae_store::StoreError::AutonomyConflict(message))
            if message.starts_with("OBSERVE_INVALID_CURSOR::") =>
        {
            ("OBSERVE_INVALID_CURSOR", error.to_string())
        }
        ae_runtime::RuntimeError::Store(ae_store::StoreError::AutonomyConflict(message))
            if message.starts_with("OBSERVE_PROJECTION_UNAVAILABLE::") =>
        {
            ("OBSERVE_PROJECTION_UNAVAILABLE", error.to_string())
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
        ae_runtime::RuntimeError::InvalidNeuralState(_) => {
            ("INVALID_NEURAL_STATE", error.to_string())
        }
        ae_runtime::RuntimeError::InvalidPerceptionProposal => {
            ("INVALID_PERCEPTION_PROPOSAL", error.to_string())
        }
        ae_runtime::RuntimeError::UnauthenticatedUserStimulus => {
            ("UNAUTHENTICATED_USER_STIMULUS", error.to_string())
        }
        ae_runtime::RuntimeError::LegacyUnattested => ("LEGACY_UNATTESTED", error.to_string()),
        ae_runtime::RuntimeError::SemanticRevisionOverflow => {
            ("SEMANTIC_REVISION_OVERFLOW", error.to_string())
        }
        ae_runtime::RuntimeError::SemanticAppraisalRetryExpiredOrUnknown => (
            "SEMANTIC_APPRAISAL_RETRY_EXPIRED_OR_UNKNOWN",
            error.to_string(),
        ),
        ae_runtime::RuntimeError::Autonomy(_) => ("AUTONOMY", error.to_string()),
        ae_runtime::RuntimeError::Alpha3(_) => ("ALPHA3", error.to_string()),
    };
    NativeCoreError::new_err(format!("{code}::{message}"))
}

fn closed_schema(message: String) -> PyErr {
    NativeCoreError::new_err(format!("CLOSED_SCHEMA::{message}"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FfiScope {
    bot_token: String,
    persona_token: String,
    session_token: String,
    relation_token: Option<String>,
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
    format!(
        r#"{{"status":"g0-ready","formula":"aster-ccn-v1","neuron_slots":16384,"version":"{}"}}"#,
        env!("CARGO_PKG_VERSION")
    )
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

/// Atomically commit the sole Host-observed inbound interaction, mint its
/// Native-only challenge and reserve the semantic Provider budget.
#[pyfunction]
fn commit_core_inbound_v1(request_json: &str) -> PyResult<String> {
    if request_json.len() > 65536 {
        return Err(closed_schema("core request exceeds 64 KiB".into()));
    }
    let request: ae_contracts::CommitCoreInboundV1 =
        serde_json::from_str(request_json).map_err(|e| closed_schema(e.to_string()))?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let result = runtime
        .commit_core_inbound_v1(&request)
        .map_err(map_error)?;
    serde_json::to_string(&result).map_err(|e| NativeCoreError::new_err(format!("ENCODING::{e}")))
}

#[pyfunction]
fn commit_core_delivery_outcome_v1(request_json: &str) -> PyResult<String> {
    if request_json.len() > 65536 {
        return Err(closed_schema("core request exceeds 64 KiB".into()));
    }
    let request: ae_contracts::CommitCoreDeliveryOutcomeV1 =
        serde_json::from_str(request_json).map_err(|e| closed_schema(e.to_string()))?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let result = runtime
        .commit_core_delivery_outcome_v1(&request)
        .map_err(map_error)?;
    serde_json::to_string(&result).map_err(|e| NativeCoreError::new_err(format!("ENCODING::{e}")))
}

#[pyfunction]
fn get_embodiment_persona_v1(request_json: &str) -> PyResult<String> {
    if request_json.len() > 4096 {
        return Err(closed_schema("persona lookup exceeds limit".into()));
    }
    let request: ae_contracts::PersonaScopeRef =
        serde_json::from_str(request_json).map_err(|e| closed_schema(e.to_string()))?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let result = runtime
        .get_embodiment_persona_v1(&request)
        .map_err(map_error)?;
    serde_json::to_string(&result).map_err(|e| NativeCoreError::new_err(format!("ENCODING::{e}")))
}
#[pyfunction]
fn embodiment_clock_status_v1(request_json: &str) -> PyResult<String> {
    if request_json.len() > 16384 {
        return Err(closed_schema("clock status exceeds limit".into()));
    }
    let request: ae_contracts::EmbodimentClockStatusRequestV1 =
        serde_json::from_str(request_json).map_err(|e| closed_schema(e.to_string()))?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let result = runtime
        .embodiment_clock_status_v1(&request)
        .map_err(map_error)?;
    serde_json::to_string(&result).map_err(|e| NativeCoreError::new_err(format!("ENCODING::{e}")))
}
#[pyfunction]
fn advance_embodiment_time_v1(request_bytes: Vec<u8>) -> PyResult<String> {
    if request_bytes.len() > 65536 {
        return Err(closed_schema("clock advance exceeds limit".into()));
    }
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let result = runtime
        .advance_embodiment_time_v1(&request_bytes)
        .map_err(map_error)?;
    serde_json::to_string(&result).map_err(|e| NativeCoreError::new_err(format!("ENCODING::{e}")))
}
#[pyfunction]
fn compare_and_swap_embodiment_profile_v1(request_json: &str) -> PyResult<String> {
    if request_json.len() > 32768 {
        return Err(closed_schema("profile CAS exceeds limit".into()));
    }
    let request: ae_contracts::CompareAndSwapEmbodimentProfileV1 =
        serde_json::from_str(request_json).map_err(|e| closed_schema(e.to_string()))?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let result = runtime
        .compare_and_swap_embodiment_profile_v1(&request)
        .map_err(map_error)?;
    serde_json::to_string(&result).map_err(|e| NativeCoreError::new_err(format!("ENCODING::{e}")))
}
#[pyfunction]
fn create_embodiment_persona_if_missing_v1(request_json: &str) -> PyResult<String> {
    if request_json.len() > 32768 {
        return Err(closed_schema("persona creation exceeds limit".into()));
    }
    let request: ae_contracts::CreateEmbodimentPersonaIfMissingV1 =
        serde_json::from_str(request_json).map_err(|e| closed_schema(e.to_string()))?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let result = runtime
        .create_embodiment_persona_if_missing_v1(&request)
        .map_err(map_error)?;
    serde_json::to_string(&result).map_err(|e| NativeCoreError::new_err(format!("ENCODING::{e}")))
}
#[pyfunction]
fn list_embodiment_personas_v1(request_json: &str) -> PyResult<String> {
    if request_json.len() > 4096 {
        return Err(closed_schema("inventory request exceeds limit".into()));
    }
    let request: ae_contracts::ListEmbodimentPersonasV1 =
        serde_json::from_str(request_json).map_err(|e| closed_schema(e.to_string()))?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let result = runtime
        .list_embodiment_personas_v1(&request)
        .map_err(map_error)?;
    serde_json::to_string(&result).map_err(|e| NativeCoreError::new_err(format!("ENCODING::{e}")))
}
#[pyfunction]
fn read_embodiment_profile_v1(request_json: &str) -> PyResult<String> {
    if request_json.len() > 4096 {
        return Err(closed_schema("profile request exceeds limit".into()));
    }
    let request: ae_contracts::PersonaScopeRef =
        serde_json::from_str(request_json).map_err(|e| closed_schema(e.to_string()))?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let result = runtime
        .read_embodiment_profile_v1(&request)
        .map_err(map_error)?;
    serde_json::to_string(&result).map_err(|e| NativeCoreError::new_err(format!("ENCODING::{e}")))
}

/// Atomically settle one semantic Provider attempt. Native owns claim lookup,
/// conservative accounting, proposal authority and commit-time causal rebase.
#[pyfunction]
fn settle_semantic_appraisal_v1(request_json: &str) -> PyResult<String> {
    let request: SemanticAppraisalSettleRequestV1 =
        serde_json::from_str(request_json).map_err(|error| closed_schema(error.to_string()))?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let result = runtime
        .settle_semantic_appraisal_v1(&request)
        .map_err(map_error)?;
    serde_json::to_string(&result)
        .map_err(|error| NativeCoreError::new_err(format!("ENCODING::{error}")))
}

/// Pure Host request compilation: no Runtime mutex, storage or execution authority.
#[pyfunction]
fn compile_core_host_request_v1(operation: &str, request_json: &str) -> PyResult<String> {
    if ae_contracts::classify_retired_operation_v1(operation) == "UNSUPPORTED_CORE_BOUNDARY" {
        return Err(NativeCoreError::new_err("UNSUPPORTED_CORE_BOUNDARY"));
    }
    if operation.len() > 64
        || !operation.is_ascii()
        || !["inbound", "delivery", "create", "profile_cas"].contains(&operation)
    {
        return Err(NativeCoreError::new_err("UNKNOWN_OPERATION"));
    }
    use ae_contracts::*;
    fn parse<T: serde::de::DeserializeOwned>(s: &str) -> PyResult<T> {
        serde_json::from_str(s).map_err(|e| closed_schema(e.to_string()))
    }
    fn output<T: serde::Serialize>(v: &T) -> PyResult<String> {
        serde_json::to_string(v).map_err(|e| closed_schema(e.to_string()))
    }
    if request_json.len() > 65536 {
        return Err(closed_schema("CORE_HOST_REQUEST_LIMIT".into()));
    }
    match operation {
        "inbound" => {
            let mut r: CommitCoreInboundV1 = parse(request_json)?;
            let p = core_persona_digest(&r.observation.scope);
            r.observation.turn_id = core_id(
                b"ae.core-inbound.turn-id.v1",
                &[&p, &r.observation.astrbot_event_identity_digest],
            );
            r.observation.operation_id = core_id(
                b"ae.core-inbound.operation-id.v1",
                &[&p, &r.observation.turn_id],
            );
            r.validate_v1().map_err(|e| closed_schema(e.into()))?;
            output(&r)
        }
        "delivery" => {
            let mut r: CommitCoreDeliveryOutcomeV1 = parse(request_json)?;
            r.operation_id = core_id(
                b"ae.core-delivery.operation-id.v1",
                &[
                    &core_persona_digest(&r.scope),
                    &r.turn_id,
                    &r.inbound_operation_id,
                ],
            );
            r.validate_v1().map_err(|e| closed_schema(e.into()))?;
            output(&r)
        }
        "create" => {
            let mut r: CreateEmbodimentPersonaIfMissingV1 = parse(request_json)?;
            r.operation_id = core_id(
                b"ae.embodiment.create.operation-id.v1",
                &[&core_persona_digest(&r.scope), &r.incarnation_digest],
            );
            output(&r)
        }
        "profile_cas" => {
            let mut r: CompareAndSwapEmbodimentProfileV1 = parse(request_json)?;
            let pb = core_encode(&r.replacement_profile).map_err(closed_schema)?;
            let sb = core_encode(&r.replacement_schedule).map_err(closed_schema)?;
            let pd = wire::domain_hash(b"ae.embodiment.profile.v1", &[&pb]);
            let sd = wire::domain_hash(b"ae.embodiment.sleep-schedule.v1", &[&sb]);
            r.operation_id = core_id(
                b"ae.embodiment.profile-cas.operation-id.v1",
                &[
                    &core_persona_digest(&r.scope),
                    &r.expected_profile_revision.to_le_bytes(),
                    &r.expected_schedule_revision.to_le_bytes(),
                    &pd,
                    &sd,
                ],
            );
            output(&r)
        }
        _ => Err(closed_schema("CORE_HOST_OPERATION_INVALID".into())),
    }
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
        "persona_scope": hex::encode32(&report.persona_scope),
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

/// Local source builds explicitly lack the clean release provenance supplied by Task 12B.
#[pyfunction]
fn build_info_v1() -> String {
    include_str!(concat!(env!("OUT_DIR"), "/build_identity.json")).to_owned()
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(build_info_v1, module)?)?;
    module.add_function(wrap_pyfunction!(version, module)?)?;
    module.add_function(wrap_pyfunction!(health, module)?)?;
    module.add_function(wrap_pyfunction!(open, module)?)?;
    module.add_function(wrap_pyfunction!(ensure_genesis, module)?)?;
    module.add_function(wrap_pyfunction!(commit_core_inbound_v1, module)?)?;
    module.add_function(wrap_pyfunction!(commit_core_delivery_outcome_v1, module)?)?;
    module.add_function(wrap_pyfunction!(list_embodiment_personas_v1, module)?)?;
    module.add_function(wrap_pyfunction!(get_embodiment_persona_v1, module)?)?;
    module.add_function(wrap_pyfunction!(embodiment_clock_status_v1, module)?)?;
    module.add_function(wrap_pyfunction!(advance_embodiment_time_v1, module)?)?;
    module.add_function(wrap_pyfunction!(
        compare_and_swap_embodiment_profile_v1,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        create_embodiment_persona_if_missing_v1,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(read_embodiment_profile_v1, module)?)?;
    module.add_function(wrap_pyfunction!(settle_semantic_appraisal_v1, module)?)?;
    module.add_function(wrap_pyfunction!(compile_core_host_request_v1, module)?)?;
    module.add_function(wrap_pyfunction!(inspect, module)?)?;
    module.add_function(wrap_pyfunction!(verify_replay, module)?)?;
    module.add_function(wrap_pyfunction!(flush_and_close, module)?)?;
    module.add("NativeCoreError", module.py().get_type::<NativeCoreError>())?;
    Ok(())
}
