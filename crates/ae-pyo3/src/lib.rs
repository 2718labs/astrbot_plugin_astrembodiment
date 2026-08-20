#![forbid(unsafe_code)]

//! PyO3 boundary: the ONLY surface Python may touch.
//!
//! Exposed: health, version, open, ensure_genesis, apply_event, inspect,
//! verify_replay, flush_and_close. There are no per-neuron getters, no
//! residual writers, no import-from-SeedCode entry point. JSON is exchanged
//! as closed, deny-unknown-field payloads; identity is computed in Rust.

use ae_contracts::{hex, CanonicalEvent, PersonaGenesisRequest, ScopeRef};
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FfiEventEnvelope {
    kind: String,
    #[serde(rename = "payload")]
    _payload: serde_json::Value,
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
    module.add_function(wrap_pyfunction!(apply_event, module)?)?;
    module.add_function(wrap_pyfunction!(inspect, module)?)?;
    module.add_function(wrap_pyfunction!(verify_replay, module)?)?;
    module.add_function(wrap_pyfunction!(flush_and_close, module)?)?;
    module.add("NativeCoreError", module.py().get_type::<NativeCoreError>())?;
    Ok(())
}

// Reuses exact typed R7 authority fixtures only in the Rust test target.  The
// file is not part of the extension-module production surface.
#[cfg(test)]
#[path = "../../ae-organism-runtime/tests/committed_semantic_projection_path.rs"]
mod r7_typed_fixture;

#[cfg(test)]
mod native_r7_atomic_ingress_tests {
    use super::*;
    use ae_contracts::r7::CanonicalEvent as R7CanonicalEvent;
    use ae_runtime::r7::{
        PrivateProjectionPayloadWireErrorV1, R7PreOutputProjectionInputV1,
        RuntimeDecision as R7RuntimeDecision, RuntimeError as R7RuntimeError,
    };
    use pyo3::types::PyModule;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_STORE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    /// This bridge is deliberately Rust-private: it owns the legacy runtime
    /// instance and delegates only closed R7 typed sources into its additive
    /// atomic ingress.  It is neither a PyO3 class nor registered function.
    struct NativeAtomicIngressV1 {
        runtime: ae_runtime::AstrRuntime,
    }

    impl NativeAtomicIngressV1 {
        fn open_for_test() -> Self {
            let serial = TEST_STORE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let directory = std::env::temp_dir().join(format!(
                "astrembodiment-core-r7-atomic-ingress-{}-{serial}",
                std::process::id()
            ));
            std::fs::create_dir_all(&directory).expect("test store directory");
            let runtime = ae_runtime::AstrRuntime::open(&directory.join("store.db"))
                .expect("legacy runtime owns the Rust-only R7 ingress");
            Self { runtime }
        }

        fn produce(
            &mut self,
            event: &R7CanonicalEvent,
            input: &R7PreOutputProjectionInputV1,
        ) -> Result<R7RuntimeDecision, R7RuntimeError> {
            self.runtime
                .apply_user_stimulus_with_private_projection_wire_v1(event, input)
        }
    }

    #[test]
    fn native_atomic_ingress_commits_once_after_a_typed_retry_and_emits_one_shot_wire() {
        let mut ingress = NativeAtomicIngressV1::open_for_test();
        let (rejected_event, rejected_input) =
            super::r7_typed_fixture::rejected_first_transition_fixture();
        assert!(matches!(
            ingress.produce(&rejected_event, &rejected_input),
            Err(R7RuntimeError::PrivateProjectionWireUnavailable)
        ));

        let (event, input) = super::r7_typed_fixture::matching_first_transition_fixture();
        let decision = ingress
            .produce(&event, &input)
            .expect("the same runtime accepts a fully bound typed retry");
        assert_eq!(decision.receipt.base_revision, 0);
        assert_eq!(decision.receipt.next_revision, 1);

        let mut wire = decision.into_private_projection_wire();
        assert!(!wire
            .consume_once()
            .expect("opaque wire permits exactly one native consumption")
            .is_empty());
        assert!(matches!(
            wire.consume_once(),
            Err(PrivateProjectionPayloadWireErrorV1::AlreadyConsumed)
        ));
    }

    #[test]
    fn python_module_keeps_r7_atomic_and_legacy_raw_sealers_unmounted() {
        Python::initialize();
        Python::attach(|py| {
            let module = PyModule::new(py, "_native").expect("test module");
            _native(&module).expect("legacy module registration");
            assert!(module.getattr("apply_event").is_ok());
            for forbidden in [
                "apply_user_stimulus_with_private_projection_wire_v1",
                "_PrivateProjectionPayloadWireV1",
                "_PrivateProjectionPayloadProducerV1",
                "_PrivateProjectionPayloadIngressV1",
                "_consume_private_projection_payload_wire_v1",
                "_astrbot_host_private_projection_wire_capability_v1",
                "PrivateProjectionPayloadWireV1",
                "seal_private_projection_payload_wire_v1",
            ] {
                assert!(
                    module.getattr(forbidden).is_err(),
                    "Python must not expose R7 raw bytes/json/repr or a legacy sealer: {forbidden}"
                );
            }
        });
    }
}
