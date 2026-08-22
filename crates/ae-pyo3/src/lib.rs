#![forbid(unsafe_code)]

//! PyO3 boundary: the ONLY surface Python may touch.
//!
//! Exposed: health, version, open, ensure_genesis, apply_event, inspect,
//! verify_replay, flush_and_close. There are no per-neuron getters, no
//! residual writers, no import-from-SeedCode entry point. JSON is exchanged
//! as closed, deny-unknown-field payloads; identity is computed in Rust.

use ae_contracts::r7::PerceptionProposalV1;
use ae_contracts::{hex, CanonicalEvent, PersonaGenesisRequest, ScopeRef};
use pyo3::create_exception;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use serde::{Deserialize, Serialize};
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
    let (code, message) = match error {
        ae_runtime::RuntimeError::Genesis(_) => ("GENESIS_UNAVAILABLE", "genesis unavailable"),
        ae_runtime::RuntimeError::RetryWait => ("RETRY_WAIT", "retry required"),
        ae_runtime::RuntimeError::Store(ae_store::StoreError::SeedDigestCollision) => {
            ("SEED_DIGEST_COLLISION", "seed digest collision")
        }
        ae_runtime::RuntimeError::Store(ae_store::StoreError::StaleRevision { .. }) => {
            ("STALE_REVISION", "stale revision")
        }
        ae_runtime::RuntimeError::Store(ae_store::StoreError::DuplicateEvent(_)) => {
            ("DUPLICATE_EVENT", "duplicate event")
        }
        ae_runtime::RuntimeError::Store(ae_store::StoreError::LeaseConflict) => {
            ("LEASE_CONFLICT", "lease conflict")
        }
        ae_runtime::RuntimeError::Store(ae_store::StoreError::LeaseInFlight) => {
            ("LEASE_IN_FLIGHT", "lease in flight")
        }
        ae_runtime::RuntimeError::Store(ae_store::StoreError::ManifestDigestMismatch)
        | ae_runtime::RuntimeError::Store(ae_store::StoreError::SeedCodeMismatch) => {
            ("IDENTITY_MISMATCH", "identity mismatch")
        }
        ae_runtime::RuntimeError::Store(_) => ("STORAGE", "storage unavailable"),
        ae_runtime::RuntimeError::PersonaGenesisRequired => {
            ("GENESIS_REQUIRED", "genesis required")
        }
        ae_runtime::RuntimeError::GenesisManifestMismatch => {
            ("GENESIS_MANIFEST_MISMATCH", "genesis manifest mismatch")
        }
        ae_runtime::RuntimeError::StaleCausalBase { .. } => {
            ("STALE_CAUSAL_BASE", "stale causal base")
        }
        ae_runtime::RuntimeError::UnsupportedEvent(_) => ("UNSUPPORTED_EVENT", "unsupported event"),
        ae_runtime::RuntimeError::Closed => ("CLOSED", "runtime closed"),
        ae_runtime::RuntimeError::InvalidNeuralState => {
            ("INVALID_NEURAL_STATE", "invalid neural state")
        }
        ae_runtime::RuntimeError::PrivateProjectionUnavailable => (
            "PRIVATE_PROJECTION_UNAVAILABLE",
            "private projection unavailable",
        ),
        ae_runtime::RuntimeError::InvalidPerceptionProposal => {
            ("INVALID_PERCEPTION_PROPOSAL", "invalid perception proposal")
        }
        ae_runtime::RuntimeError::InvalidPerceptionScope => {
            ("INVALID_PERCEPTION_SCOPE", "invalid perception scope")
        }
        ae_runtime::RuntimeError::SemanticIdentityConflict => (
            "SEMANTIC_IDENTITY_CONFLICT",
            "semantic proposal identity conflict",
        ),
        ae_runtime::RuntimeError::SemanticRevisionOverflow => {
            ("SEMANTIC_REVISION_OVERFLOW", "semantic revision overflow")
        }
        ae_runtime::RuntimeError::SemanticStateUnchanged => (
            "SEMANTIC_STATE_UNCHANGED",
            "semantic transition did not change state",
        ),
        ae_runtime::RuntimeError::LegacySemanticUnattested => (
            "LEGACY_UNATTESTED",
            "legacy semantic transition is unattested",
        ),
    };
    NativeCoreError::new_err(format!("{code}::{message}"))
}

fn closed_schema(message: String) -> PyErr {
    NativeCoreError::new_err(format!("CLOSED_SCHEMA::{message}"))
}

fn semantic_perception_payload(
    decision: &ae_runtime::PerceptionProposalDecisionV1,
) -> Result<serde_json::Value, serde_json::Error> {
    let mut receipt = serde_json::to_value(&decision.receipt)?;
    if let Some(receipt_object) = receipt.as_object_mut() {
        // The canonical root receipt keeps ActionContract for G0 compatibility,
        // but SPC1's Python contract is an explicit closed allow-list.
        const CLOSED_RECEIPT_FIELDS: &[&str] = &[
            "schema_version",
            "formula_digest",
            "scope_digest",
            "event_digest",
            "authority_digest",
            "base_revision",
            "next_revision",
            "state_before",
            "state_after",
            "graph_after",
            "active_nodes",
            "active_edges",
            "residuals",
            "status",
        ];
        receipt_object.retain(|key, _| CLOSED_RECEIPT_FIELDS.contains(&key.as_str()));
    }
    let semantic_vector = &decision.semantic_vector_receipt.semantic_vector;
    let node_observability = serde_json::to_value(&decision.node_observability)?;
    Ok(serde_json::json!({
        "schema": "astrembodiment.semantic-perception-closure.v1",
        "receipt": receipt,
        "semantic_vector_receipt": {
            "schema": ae_contracts::SEMANTIC_VECTOR_RECEIPT_SCHEMA_V2,
            "formula": semantic_vector.formula,
            "dimension_slot_count": semantic_vector.dimension_slot_count,
            "evaluated_dimension_count": semantic_vector.evaluated_dimension_count,
            "injected_dimension_count": semantic_vector.injected_dimension_count,
            "nonzero_evidence_dimension_count": semantic_vector.nonzero_evidence_dimension_count,
            "neutral_baseline_dimension_count": semantic_vector.neutral_baseline_dimension_count,
            "unavailable_dimension_count": semantic_vector.unavailable_dimension_count,
            "state_changed": semantic_vector.state_changed,
        },
        "node_observability": node_observability,
        "revision": decision.revision,
        "deduplicated": decision.deduplicated,
        "expression_projection": {
            "schema": ae_runtime::EXPRESSION_PROJECTION_SCHEMA_V1,
            "revision": decision.expression_projection.revision,
            "profile_fxp6": {
                "warmth": decision.expression_projection.profile_fxp6.warmth,
                "sensitivity": decision.expression_projection.profile_fxp6.sensitivity,
                "guardedness": decision.expression_projection.profile_fxp6.guardedness,
                "repair_orientation": decision.expression_projection.profile_fxp6.repair_orientation,
                "engagement": decision.expression_projection.profile_fxp6.engagement,
                "epistemic_caution": decision.expression_projection.profile_fxp6.epistemic_caution,
            },
        },
    }))
}

fn encode_json_for_boundary<T: Serialize>(value: &T) -> PyResult<String> {
    serde_json::to_string(value)
        .map_err(|_| NativeCoreError::new_err("ENCODING::serialization failed"))
}

fn parse_semantic_proposal_json(proposal_json: &str) -> Result<PerceptionProposalV1, &'static str> {
    serde_json::from_str(proposal_json).map_err(|_| "invalid perception proposal")
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
    let request: PersonaGenesisRequest = serde_json::from_str(request_json)
        .map_err(|_| closed_schema("invalid genesis request".to_owned()))?;
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
    encode_json_for_boundary(&payload)
}

/// Apply one closed canonical event through the deterministic G0 no-op lane.
#[pyfunction]
fn apply_event(scope_json: &str, event_json: &str) -> PyResult<String> {
    let scope: FfiScope =
        serde_json::from_str(scope_json).map_err(|_| closed_schema("invalid scope".to_owned()))?;
    let scope_ref = scope
        .scope_ref()
        .map_err(|_| closed_schema("invalid scope".to_owned()))?;
    let envelope: FfiEventEnvelope = serde_json::from_str(event_json)
        .map_err(|_| closed_schema("invalid event envelope".to_owned()))?;
    if is_known_g0_unsupported_event(&envelope.kind) {
        return Err(NativeCoreError::new_err(
            "UNSUPPORTED_EVENT::unsupported event",
        ));
    }
    let event: CanonicalEvent =
        serde_json::from_str(event_json).map_err(|_| closed_schema("invalid event".to_owned()))?;
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
    encode_json_for_boundary(&payload)
}

/// Return the content-free cursor for the separate SPC1 semantic lane.
#[pyfunction]
fn semantic_revision_v1(scope_json: &str) -> PyResult<String> {
    let scope: FfiScope = serde_json::from_str(scope_json)
        .map_err(|_| closed_schema("invalid perception scope".to_owned()))?;
    let scope_ref = scope
        .scope_ref()
        .map_err(|_| closed_schema("invalid perception scope".to_owned()))?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let revision = runtime
        .semantic_revision_v1(&scope_ref)
        .map_err(map_error)?;
    let payload = serde_json::json!({
        "schema": "astrembodiment.semantic-revision.v1",
        "revision": revision,
    });
    encode_json_for_boundary(&payload)
}

/// Apply one closed SPC1 perception proposal.  The result intentionally
/// exposes only the receipt, semantic revision, and deduplication flag.
#[pyfunction]
fn apply_perception_proposal_v1(scope_json: &str, proposal_json: &str) -> PyResult<String> {
    let scope: FfiScope = serde_json::from_str(scope_json)
        .map_err(|_| closed_schema("invalid perception scope".to_owned()))?;
    let scope_ref = scope
        .scope_ref()
        .map_err(|_| closed_schema("invalid perception scope".to_owned()))?;
    let proposal = parse_semantic_proposal_json(proposal_json)
        .map_err(|message| closed_schema(message.to_owned()))?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let decision = runtime
        .apply_perception_proposal_v1(&scope_ref, &proposal)
        .map_err(map_error)?;
    let payload = semantic_perception_payload(&decision)
        .map_err(|_| NativeCoreError::new_err("ENCODING::serialization failed"))?;
    encode_json_for_boundary(&payload)
}

/// Content-free observatory projection for one (Bot, Persona) binding.
#[pyfunction]
fn inspect(scope_json: &str) -> PyResult<String> {
    let scope: FfiScope =
        serde_json::from_str(scope_json).map_err(|_| closed_schema("invalid scope".to_owned()))?;
    let scope_ref = scope
        .scope_ref()
        .map_err(|_| closed_schema("invalid scope".to_owned()))?;
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
    encode_json_for_boundary(&payload)
}

/// Mechanical replay verification of the committed journal.
#[pyfunction]
fn verify_replay(scope_json: &str) -> PyResult<String> {
    let scope: FfiScope =
        serde_json::from_str(scope_json).map_err(|_| closed_schema("invalid scope".to_owned()))?;
    let scope_ref = scope
        .scope_ref()
        .map_err(|_| closed_schema("invalid scope".to_owned()))?;
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
    encode_json_for_boundary(&payload)
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
    module.add_function(wrap_pyfunction!(semantic_revision_v1, module)?)?;
    module.add_function(wrap_pyfunction!(apply_perception_proposal_v1, module)?)?;
    module.add_function(wrap_pyfunction!(inspect, module)?)?;
    module.add_function(wrap_pyfunction!(verify_replay, module)?)?;
    module.add_function(wrap_pyfunction!(flush_and_close, module)?)?;
    module.add("NativeCoreError", module.py().get_type::<NativeCoreError>())?;
    Ok(())
}

#[cfg(test)]
mod native_private_projection_absence_tests {
    use super::*;
    use ae_genesis::GenesisError;
    use ae_store::StoreError;
    use pyo3::types::PyModule;
    use serde::{Serialize, Serializer};
    use std::collections::BTreeSet;

    fn semantic_test_receipt() -> ae_contracts::TransitionReceipt {
        ae_contracts::TransitionReceipt {
            schema_version: 1,
            formula_digest: [1; 32],
            scope_digest: [2; 32],
            event_digest: [3; 32],
            authority_digest: [4; 32],
            base_revision: 0,
            next_revision: 1,
            state_before: [5; 32],
            state_after: [6; 32],
            graph_after: [7; 32],
            action_contract: None,
            active_nodes: 2_048,
            active_edges: 0,
            residuals: ae_contracts::InvariantResiduals::default(),
            status: ae_contracts::CommitStatus::Committed,
        }
    }

    fn semantic_test_vector_receipt() -> ae_contracts::TransitionReceiptV2 {
        ae_contracts::TransitionReceiptV2::from_legacy(
            &semantic_test_receipt(),
            ae_contracts::SemanticVectorReceiptV2 {
                schema_version: 2,
                formula: ae_contracts::SemanticVectorFormulaV2::FullVectorRouteNeutralRelaxationV1,
                dimension_slot_count: 15,
                evaluated_dimension_count: 15,
                injected_dimension_count: 15,
                nonzero_evidence_dimension_count: 3,
                neutral_baseline_dimension_count: 12,
                unavailable_dimension_count: 0,
                state_changed: true,
            },
        )
        .expect("test v2 receipt")
    }

    fn semantic_test_node_observability() -> ae_runtime::NodeObservabilityProjectionV1 {
        let regions = [
            ("interoception_allostasis", 2_048_u32),
            ("affective_valuation", 2_048_u32),
            ("salience", 1_024_u32),
            ("epistemic_fallibility", 2_048_u32),
            ("social_boundary", 2_048_u32),
            ("temper_inhibitory", 1_024_u32),
            ("world_model_imagination", 4_096_u32),
            ("global_workspace", 1_024_u32),
            ("action_expression", 1_024_u32),
        ]
        .into_iter()
        .enumerate()
        .map(|(region_id, (region_name, node_capacity))| {
            let selected_node_count = if region_id == 0 { 2_048 } else { 0 };
            let component = ae_runtime::NodeObservabilityComponentV1 {
                before_mean_fxp6: 0,
                after_mean_fxp6: i64::from(selected_node_count > 0),
                delta_mean_fxp6: i64::from(selected_node_count > 0),
                changed_node_count: selected_node_count,
                nonzero_after_count: selected_node_count,
            };
            ae_runtime::NodeObservabilityRegionV1 {
                region_id: region_id as u8,
                region_name,
                node_capacity,
                selected_node_count,
                activated_node_count: selected_node_count,
                changed_node_count: selected_node_count,
                potential: component.clone(),
                excitation: component,
            }
        })
        .collect();
        ae_runtime::NodeObservabilityProjectionV1 {
            schema: ae_runtime::NODE_OBSERVABILITY_SCHEMA_V1,
            formula: ae_runtime::NODE_OBSERVABILITY_FORMULA_V1,
            revision: 1,
            field_node_capacity: 16_384,
            region_layout: "regions-v1",
            counts: ae_runtime::NodeObservabilityCountsV1 {
                selected_node_count: 2_048,
                activated_node_count: 2_048,
                changed_node_count: 2_048,
                potential_nonzero_after_count: 2_048,
                excitation_nonzero_after_count: 2_048,
                signal_nonzero_after_count: 2_048,
            },
            residuals: ae_runtime::NodeObservabilityResidualsV1 {
                state: ae_runtime::NodeObservabilityResidualStateV1::NotComputed,
                formula: None,
                values_fxp6: None,
            },
            regions,
        }
    }

    #[test]
    fn private_projection_failure_maps_to_one_fixed_non_payload_error() {
        Python::initialize();
        Python::attach(|py| {
            let error = map_error(ae_runtime::RuntimeError::PrivateProjectionUnavailable);
            assert_eq!(
                error.value(py).to_string(),
                "PRIVATE_PROJECTION_UNAVAILABLE::private projection unavailable"
            );
        });
    }

    #[test]
    fn python_module_keeps_r7_atomic_and_legacy_raw_sealers_unmounted() {
        Python::initialize();
        Python::attach(|py| {
            let module = PyModule::new(py, "_native").expect("test module");
            _native(&module).expect("legacy module registration");
            assert!(module.getattr("apply_event").is_ok());
            assert!(module.getattr("semantic_revision_v1").is_ok());
            assert!(module.getattr("apply_perception_proposal_v1").is_ok());
            for forbidden in [
                "apply_user_stimulus_with_private_projection_wire_v1",
                "R7PreOutputProjectionInputV1",
                "_PrivateProjectionPayloadWireV1",
                "_PrivateProjectionPayloadProducerV1",
                "_PrivateProjectionPayloadIngressV1",
                "_PrivateProjectionTransferV1",
                "_consume_private_projection_payload_wire_v1",
                "_discard_private_projection_transfer_v1",
                "_astrbot_host_private_projection_wire_capability_v1",
                "PrivateProjectionPayloadWireV1",
                "PrivateProjectionPayloadProducerV1",
                "PrivateProjectionPayloadIngressV1",
                "PrivateProjectionTransferV1",
                "seal_private_projection_payload_wire_v1",
                "materialize_private_projection_payload_v1",
                "private_projection_payload_callback_v1",
                "repr_private_projection_payload_wire_v1",
                "pickle_private_projection_payload_wire_v1",
                "buffer_private_projection_payload_wire_v1",
                "consume_once",
                "wire_digest",
                "to_bytes",
                "as_bytes",
                "__bytes__",
                "__buffer__",
            ] {
                assert!(
                    module.getattr(forbidden).is_err(),
                    "Python must not expose R7 raw bytes/json/repr or a legacy sealer: {forbidden}"
                );
            }
        });
    }

    #[test]
    fn semantic_perception_payload_exposes_bounded_v2_receipt_and_observability() {
        let decision = ae_runtime::PerceptionProposalDecisionV1 {
            receipt: semantic_test_receipt(),
            semantic_vector_receipt: semantic_test_vector_receipt(),
            node_observability: semantic_test_node_observability(),
            revision: 1,
            deduplicated: false,
            expression_projection: ae_runtime::ExpressionProjectionV1 {
                revision: 1,
                profile_fxp6: ae_runtime::ExpressionProfileFxP6 {
                    warmth: 700_000,
                    sensitivity: 200_000,
                    guardedness: 100_000,
                    repair_orientation: 300_000,
                    engagement: 600_000,
                    epistemic_caution: 400_000,
                },
            },
        };
        let payload = semantic_perception_payload(&decision).expect("closed receipt payload");
        let object = payload.as_object().expect("object payload");
        assert_eq!(object.len(), 7);
        assert!(object.contains_key("schema"));
        assert!(object.contains_key("receipt"));
        assert!(object.contains_key("semantic_vector_receipt"));
        assert!(object.contains_key("node_observability"));
        assert!(object.contains_key("revision"));
        assert!(object.contains_key("deduplicated"));
        assert!(object.contains_key("expression_projection"));
        assert!(payload.get("contract").is_none());
        assert_eq!(
            payload["expression_projection"],
            serde_json::json!({
                "schema": "astr-embodiment.expression-projection.v1",
                "revision": 1,
                "profile_fxp6": {
                    "warmth": 700_000,
                    "sensitivity": 200_000,
                    "guardedness": 100_000,
                    "repair_orientation": 300_000,
                    "engagement": 600_000,
                    "epistemic_caution": 400_000,
                },
            })
        );
        let receipt_keys = payload["receipt"]
            .as_object()
            .expect("receipt object")
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            receipt_keys,
            [
                "schema_version",
                "formula_digest",
                "scope_digest",
                "event_digest",
                "authority_digest",
                "base_revision",
                "next_revision",
                "state_before",
                "state_after",
                "graph_after",
                "active_nodes",
                "active_edges",
                "residuals",
                "status",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect()
        );
        assert_eq!(
            payload["semantic_vector_receipt"],
            serde_json::json!({
                "schema": "astr-embodiment.semantic-vector-receipt.v2",
                "formula": "full-vector-route-neutral-relaxation-v1",
                "dimension_slot_count": 15,
                "evaluated_dimension_count": 15,
                "injected_dimension_count": 15,
                "nonzero_evidence_dimension_count": 3,
                "neutral_baseline_dimension_count": 12,
                "unavailable_dimension_count": 0,
                "state_changed": true,
            })
        );
        let vector_receipt_keys = payload["semantic_vector_receipt"]
            .as_object()
            .expect("closed v2 semantic vector receipt")
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            vector_receipt_keys,
            [
                "schema",
                "formula",
                "dimension_slot_count",
                "evaluated_dimension_count",
                "injected_dimension_count",
                "nonzero_evidence_dimension_count",
                "neutral_baseline_dimension_count",
                "unavailable_dimension_count",
                "state_changed",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect()
        );
        assert_eq!(
            payload["node_observability"]["schema"],
            ae_runtime::NODE_OBSERVABILITY_SCHEMA_V1
        );
        assert_eq!(payload["node_observability"]["regions"].as_array().map(Vec::len), Some(9));
        assert_eq!(
            payload["node_observability"]["residuals"],
            serde_json::json!({"state": "NOT_COMPUTED", "formula": null, "values_fxp6": null})
        );

        let serialized = serde_json::to_string(&payload).expect("serialize payload");
        assert!(serialized.len() <= 16_384);
        for forbidden in [
            "RAW_TEXT_SENTINEL",
            "PROVIDER_PAYLOAD_SENTINEL",
            "REQUEST_NONCE_SENTINEL",
            "WIRE_BYTES_SENTINEL",
            "event_bytes",
            "provider_payload",
            "request_nonce_digest",
            "action_contract",
            "private_wire",
            "ActionContract",
            "node_id",
            "node_values",
            "node_deltas",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "semantic output leaked forbidden material: {forbidden}"
            );
        }

        let malformed = r#"{"schema_version":1,"event_id":"RAW_TEXT_SENTINEL"}"#;
        let parse_error = parse_semantic_proposal_json(malformed).expect_err("malformed proposal");
        assert_eq!(parse_error, "invalid perception proposal");
        assert!(!parse_error.contains("RAW_TEXT_SENTINEL"));
    }

    struct AlwaysFailsSerialization;

    impl Serialize for AlwaysFailsSerialization {
        fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            Err(serde::ser::Error::custom("SERIALIZER_RAW_SENTINEL"))
        }
    }

    #[test]
    fn python_error_boundary_is_fixed_and_encoding_is_fallible() {
        Python::initialize();
        let cases = [
            (
                ae_runtime::RuntimeError::Store(StoreError::Sqlite(
                    "C:\\secret\\state.db SQL RAW_SENTINEL".to_owned(),
                )),
                "STORAGE::storage unavailable",
            ),
            (
                ae_runtime::RuntimeError::Store(StoreError::Io {
                    context: "opening RAW_PATH_SENTINEL",
                    source: std::io::Error::other("OS_RAW_SENTINEL"),
                }),
                "STORAGE::storage unavailable",
            ),
            (
                ae_runtime::RuntimeError::Genesis(GenesisError::CapsuleInvalid(
                    "GENESIS_RAW_SENTINEL",
                )),
                "GENESIS_UNAVAILABLE::genesis unavailable",
            ),
            (
                ae_runtime::RuntimeError::StaleCausalBase {
                    expected: 7,
                    actual: 3,
                },
                "STALE_CAUSAL_BASE::stale causal base",
            ),
            (
                ae_runtime::RuntimeError::UnsupportedEvent("RAW_EVENT_SENTINEL"),
                "UNSUPPORTED_EVENT::unsupported event",
            ),
            (
                ae_runtime::RuntimeError::SemanticIdentityConflict,
                "SEMANTIC_IDENTITY_CONFLICT::semantic proposal identity conflict",
            ),
        ];
        Python::attach(|py| {
            for (error, expected) in cases {
                let mapped = map_error(error);
                let rendered = mapped.value(py).to_string();
                assert_eq!(rendered, expected);
                assert!(!rendered.contains("RAW_SENTINEL"));
                assert!(!rendered.contains("SQL"));
                assert!(!rendered.contains("state.db"));
            }

            let encoding_error = encode_json_for_boundary(&AlwaysFailsSerialization)
                .expect_err("serializer failure must become a PyErr");
            assert_eq!(
                encoding_error.value(py).to_string(),
                "ENCODING::serialization failed"
            );
            assert!(!encoding_error
                .value(py)
                .to_string()
                .contains("SERIALIZER_RAW_SENTINEL"));
        });
    }
}
