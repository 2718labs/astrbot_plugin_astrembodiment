#![forbid(unsafe_code)]

//! PyO3 boundary: the ONLY surface Python may touch.
//!
//! Exposed: health, version, open, ensure_genesis, apply_event, inspect,
//! verify_replay, flush_and_close. There are no per-neuron getters, no
//! residual writers, no import-from-SeedCode entry point. JSON is exchanged
//! as closed, deny-unknown-field payloads; identity is computed in Rust.

use ae_context_projector::{ContextSummaryV1, DeliveryOutcome as ContextDeliveryOutcome};
use ae_contracts::{
    hex, node_observability_contract_info_v1, wire, CanonicalEvent, PerceptionProposalV1,
    PersonaGenesisRequest, ScopeRef, StateSubcodeV1,
};
use ae_store::{
    RebirthActionV1, RebirthAuditReceiptV1, RebirthOutcomeV1, RebirthPrepareRequestV1,
    RebirthResponseEnvelopeV1, RebirthResponseStateV1, SeedConfigAckStateV1,
    SeedConfigLifecycleError, SeedConfigObservationV1, SeedConfigOriginV1,
    SeedConfigReconcileRequestV1, SeedConfigStateV1, SeedConfigWritebackAckV1,
    SemanticOutboxCryptoError, SemanticOutboxCryptoStatusV1, SemanticOutboxCryptoStatusValueV1,
    UserAuthorizedRebirthV1, SEMANTIC_OUTBOX_KEY_VERSION_V1, SEMANTIC_OUTBOX_MAX_AAD_BYTES_V1,
    SEMANTIC_OUTBOX_MAX_ENVELOPE_BYTES_V1, SEMANTIC_OUTBOX_MAX_PLAINTEXT_BYTES_V1,
};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use pyo3::create_exception;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use serde::Deserialize;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock};

create_exception!(_native, NativeCoreError, PyRuntimeError);

static CORE: OnceLock<Mutex<Option<ae_runtime::AstrRuntime>>> = OnceLock::new();

fn invalid_neural_state_message(subcode: StateSubcodeV1) -> String {
    format!("INVALID_NEURAL_STATE::{}", subcode.as_str())
}

fn invalid_neural_state_error(subcode: StateSubcodeV1) -> PyErr {
    let error = NativeCoreError::new_err(invalid_neural_state_message(subcode));
    Python::attach(|py| {
        let value = error.value(py);
        let _ = value.setattr("code", "INVALID_NEURAL_STATE");
        let _ = value.setattr("state_subcode", subcode.as_str());
        let _ = value.setattr("state_diagnostic", py.None());
    });
    error
}

fn seed_config_error(code: &'static str) -> PyErr {
    let error = NativeCoreError::new_err(code);
    Python::attach(|py| {
        let value = error.value(py);
        let _ = value.setattr("code", code);
    });
    error
}

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
        ae_runtime::RuntimeError::SeedConfig(seed) => return seed_config_error(seed.code()),
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
        ae_runtime::RuntimeError::InvalidNeuralState(subcode) => {
            return invalid_neural_state_error(*subcode)
        }
        ae_runtime::RuntimeError::InvalidPerceptionProposal => {
            ("INVALID_PERCEPTION_PROPOSAL", error.to_string())
        }
        ae_runtime::RuntimeError::InvalidPerceptionScope => {
            ("INVALID_PERCEPTION_SCOPE", error.to_string())
        }
        ae_runtime::RuntimeError::SemanticIdentityConflict => {
            ("SEMANTIC_IDENTITY_CONFLICT", error.to_string())
        }
        ae_runtime::RuntimeError::SemanticRevisionOverflow => {
            ("SEMANTIC_REVISION_OVERFLOW", error.to_string())
        }
        ae_runtime::RuntimeError::LegacyUnattested => ("LEGACY_UNATTESTED", error.to_string()),
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

/// The semantic lane adds only the frozen aggregate migration classification
/// to its existing native error.  `state_subcode` remains intact; neither
/// error text nor this attribute carries source content, identity, paths, or
/// raw field data.
fn map_semantic_perception_error(error: ae_runtime::RuntimeError) -> PyErr {
    let migration_subcode = error.migration_subcode_v1();
    let mapped = map_error(error);
    Python::attach(|py| {
        let value = mapped.value(py);
        let _ = value.setattr("migration_subcode", migration_subcode.as_str());
    });
    mapped
}

fn closed_schema(message: String) -> PyErr {
    NativeCoreError::new_err(format!("CLOSED_SCHEMA::{message}"))
}

const OUTBOX_CRYPTO_STATUS_SCHEMA: &str = "astrembodiment.semantic-outbox-crypto-status.v1";
const OUTBOX_SEAL_REQUEST_SCHEMA: &str = "astrembodiment.semantic-outbox-seal-request.v1";
const OUTBOX_SEALED_SCHEMA: &str = "astrembodiment.semantic-outbox-sealed.v1";
const OUTBOX_OPEN_REQUEST_SCHEMA: &str = "astrembodiment.semantic-outbox-open-request.v1";
const OUTBOX_OPENED_SCHEMA: &str = "astrembodiment.semantic-outbox-opened.v1";
const OUTBOX_REQUEST_JSON_OVERHEAD_BYTES: usize = 512;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FfiSemanticOutboxSealRequestV1 {
    schema: String,
    key_version: u32,
    aad_b64: String,
    plaintext_b64: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FfiSemanticOutboxOpenRequestV1 {
    schema: String,
    aad_b64: String,
    envelope_b64: String,
}

fn async_crypto_error(error: SemanticOutboxCryptoError) -> PyErr {
    let code = match error {
        SemanticOutboxCryptoError::Unavailable => "ASYNC_KEY_UNAVAILABLE",
        SemanticOutboxCryptoError::PayloadAuthFailed => "ASYNC_PAYLOAD_AUTH_FAILED",
        SemanticOutboxCryptoError::KeyVersionUnsupported => "ASYNC_KEY_VERSION_UNSUPPORTED",
    };
    let result = NativeCoreError::new_err(code);
    Python::attach(|py| {
        let _ = result.value(py).setattr("code", code);
    });
    result
}

const fn max_canonical_outbox_base64_chars(max_decoded_bytes: usize) -> usize {
    max_decoded_bytes.saturating_add(2) / 3 * 4
}

const OUTBOX_SEAL_REQUEST_MAX_JSON_BYTES: usize =
    max_canonical_outbox_base64_chars(SEMANTIC_OUTBOX_MAX_AAD_BYTES_V1)
        + max_canonical_outbox_base64_chars(SEMANTIC_OUTBOX_MAX_PLAINTEXT_BYTES_V1)
        + OUTBOX_REQUEST_JSON_OVERHEAD_BYTES;
const OUTBOX_OPEN_REQUEST_MAX_JSON_BYTES: usize =
    max_canonical_outbox_base64_chars(SEMANTIC_OUTBOX_MAX_AAD_BYTES_V1)
        + max_canonical_outbox_base64_chars(SEMANTIC_OUTBOX_MAX_ENVELOPE_BYTES_V1)
        + OUTBOX_REQUEST_JSON_OVERHEAD_BYTES;

fn canonical_outbox_base64(
    value: &str,
    max_decoded_bytes: usize,
) -> Result<Vec<u8>, SemanticOutboxCryptoError> {
    if value.len() > max_canonical_outbox_base64_chars(max_decoded_bytes) {
        return Err(SemanticOutboxCryptoError::PayloadAuthFailed);
    }
    let mut decoded = BASE64_STANDARD
        .decode(value)
        .map_err(|_| SemanticOutboxCryptoError::PayloadAuthFailed)?;
    if decoded.len() > max_decoded_bytes || BASE64_STANDARD.encode(&decoded) != value {
        decoded.fill(0);
        return Err(SemanticOutboxCryptoError::PayloadAuthFailed);
    }
    Ok(decoded)
}

fn semantic_outbox_status_payload(value: SemanticOutboxCryptoStatusV1) -> serde_json::Value {
    let status = match value.status {
        SemanticOutboxCryptoStatusValueV1::Ready => "READY",
        SemanticOutboxCryptoStatusValueV1::Unavailable => "UNAVAILABLE",
        SemanticOutboxCryptoStatusValueV1::KeyVersionUnsupported => "KEY_VERSION_UNSUPPORTED",
    };
    serde_json::json!({
        "schema": OUTBOX_CRYPTO_STATUS_SCHEMA,
        "status": status,
        "key_version": value.key_version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ae_contracts::StateSubcodeV1;

    #[test]
    fn invalid_neural_state_messages_are_exact_closed_set() {
        let cases = [
            (
                StateSubcodeV1::FieldStateInvalid,
                "INVALID_NEURAL_STATE::FIELD_STATE_INVALID",
            ),
            (
                StateSubcodeV1::Aesem3RetiredCompensationNonzero,
                "INVALID_NEURAL_STATE::AESEM3_RETIRED_COMPENSATION_NONZERO",
            ),
            (
                StateSubcodeV1::SnapshotWireInvalid,
                "INVALID_NEURAL_STATE::SNAPSHOT_WIRE_INVALID",
            ),
            (
                StateSubcodeV1::UnknownInvalidNeuralState,
                "INVALID_NEURAL_STATE::UNKNOWN_INVALID_NEURAL_STATE",
            ),
        ];

        for (subcode, expected) in cases {
            assert_eq!(invalid_neural_state_message(subcode), expected);
        }
    }

    #[test]
    fn semantic_outbox_pyo3_shape_rejects_oversize_base64_before_and_after_decode() {
        fn error_code(error: PyErr) -> String {
            Python::attach(|py| error.value(py).getattr("code").unwrap().extract().unwrap())
        }

        let aad = BASE64_STANDARD.encode([0u8]);
        let too_long_before_decode = "A"
            .repeat(max_canonical_outbox_base64_chars(SEMANTIC_OUTBOX_MAX_PLAINTEXT_BYTES_V1) + 1);
        let seal_before_decode = serde_json::json!({
            "schema": OUTBOX_SEAL_REQUEST_SCHEMA,
            "key_version": SEMANTIC_OUTBOX_KEY_VERSION_V1,
            "aad_b64": aad,
            "plaintext_b64": too_long_before_decode,
        })
        .to_string();
        assert_eq!(
            error_code(semantic_outbox_seal_v1(&seal_before_decode).unwrap_err()),
            "ASYNC_PAYLOAD_AUTH_FAILED"
        );

        let too_large_plaintext =
            BASE64_STANDARD.encode(vec![0u8; SEMANTIC_OUTBOX_MAX_PLAINTEXT_BYTES_V1 + 1]);
        let seal_after_decode = serde_json::json!({
            "schema": OUTBOX_SEAL_REQUEST_SCHEMA,
            "key_version": SEMANTIC_OUTBOX_KEY_VERSION_V1,
            "aad_b64": BASE64_STANDARD.encode([0u8]),
            "plaintext_b64": too_large_plaintext,
        })
        .to_string();
        assert_eq!(
            error_code(semantic_outbox_seal_v1(&seal_after_decode).unwrap_err()),
            "ASYNC_PAYLOAD_AUTH_FAILED"
        );

        let too_large_aad = BASE64_STANDARD.encode(vec![0u8; SEMANTIC_OUTBOX_MAX_AAD_BYTES_V1 + 1]);
        let seal_oversized_aad = serde_json::json!({
            "schema": OUTBOX_SEAL_REQUEST_SCHEMA,
            "key_version": SEMANTIC_OUTBOX_KEY_VERSION_V1,
            "aad_b64": too_large_aad,
            "plaintext_b64": BASE64_STANDARD.encode([0u8]),
        })
        .to_string();
        assert_eq!(
            error_code(semantic_outbox_seal_v1(&seal_oversized_aad).unwrap_err()),
            "ASYNC_PAYLOAD_AUTH_FAILED"
        );

        let too_large_envelope =
            BASE64_STANDARD.encode(vec![0u8; SEMANTIC_OUTBOX_MAX_ENVELOPE_BYTES_V1 + 1]);
        let open_after_decode = serde_json::json!({
            "schema": OUTBOX_OPEN_REQUEST_SCHEMA,
            "aad_b64": BASE64_STANDARD.encode([0u8]),
            "envelope_b64": too_large_envelope,
        })
        .to_string();
        assert_eq!(
            error_code(semantic_outbox_open_v1(&open_after_decode).unwrap_err()),
            "ASYNC_PAYLOAD_AUTH_FAILED"
        );
    }
}

fn strict_lower_hex(value: &serde_json::Value, expected_chars: usize) -> bool {
    value.as_str().is_some_and(|text| {
        text.len() == expected_chars
            && text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn parse_semantic_scope_json(scope_json: &str) -> Result<ScopeRef, String> {
    let scope: FfiScope = serde_json::from_str(scope_json).map_err(|error| error.to_string())?;
    let raw: serde_json::Value =
        serde_json::from_str(scope_json).map_err(|error| error.to_string())?;
    let object = raw
        .as_object()
        .ok_or_else(|| "scope must be a JSON object".to_owned())?;
    for name in ["bot_token", "persona_token", "session_token"] {
        if !object
            .get(name)
            .is_some_and(|value| strict_lower_hex(value, 32))
        {
            return Err(format!("{name} must be exact lower hex"));
        }
    }
    if object
        .get("relation_token")
        .is_some_and(|value| !value.is_null() && !strict_lower_hex(value, 32))
    {
        return Err("relation_token must be null or exact lower hex".to_owned());
    }
    scope.scope_ref()
}

fn parse_semantic_proposal_json(proposal_json: &str) -> Result<PerceptionProposalV1, &'static str> {
    // Deserialize the typed closed schema first: serde rejects unknown and
    // duplicate fields at both proposal and dimensions levels.
    let proposal: PerceptionProposalV1 = serde_json::from_str(proposal_json)
        .map_err(|_| "proposal must be a closed integer-only schema")?;
    let raw: serde_json::Value =
        serde_json::from_str(proposal_json).map_err(|_| "proposal must be valid JSON")?;
    let object = raw.as_object().ok_or("proposal must be a JSON object")?;
    for name in ["event_id", "turn_id"] {
        if !object
            .get(name)
            .is_some_and(|value| strict_lower_hex(value, 32))
        {
            return Err("proposal identity must be exact lower hex");
        }
    }
    if !object
        .get("request_nonce_digest")
        .is_some_and(|value| strict_lower_hex(value, 64))
    {
        return Err("request_nonce_digest must be exact lower hex");
    }
    let dimensions = object
        .get("dimensions")
        .and_then(serde_json::Value::as_object)
        .ok_or("dimensions must be an object")?;
    for name in [
        "positive",
        "affiliation",
        "harm",
        "boundary",
        "repair",
        "repetition",
        "new_information",
        "constraint_instability",
        "epistemic_conflict",
        "self_responsibility",
        "other_responsibility",
        "hostility",
        "publicness",
        "engagement",
        "rejection",
    ] {
        if !dimensions.get(name).is_some_and(serde_json::Value::is_i64) {
            return Err("dimensions must use integer FxP6 values");
        }
    }
    if !object
        .get("estimator_confidence")
        .is_some_and(serde_json::Value::is_i64)
    {
        return Err("estimator_confidence must be an integer FxP6 value");
    }
    proposal
        .validate_v1()
        .map_err(|_| "proposal values or nonce binding are invalid")?;
    Ok(proposal)
}

fn commit_status_name(status: ae_contracts::CommitStatus) -> &'static str {
    match status {
        ae_contracts::CommitStatus::Committed => "committed",
        ae_contracts::CommitStatus::Rejected => "rejected",
        ae_contracts::CommitStatus::Superseded => "superseded",
        ae_contracts::CommitStatus::Stale => "stale",
    }
}

fn legacy_semantic_receipt_payload(receipt: &ae_contracts::TransitionReceipt) -> serde_json::Value {
    let residuals = &receipt.residuals;
    serde_json::json!({
        "schema_version": receipt.schema_version,
        "formula_digest": hex::encode32(&receipt.formula_digest),
        "scope_digest": hex::encode32(&receipt.scope_digest),
        "event_digest": hex::encode32(&receipt.event_digest),
        "authority_digest": hex::encode32(&receipt.authority_digest),
        "base_revision": receipt.base_revision,
        "next_revision": receipt.next_revision,
        "state_before": hex::encode32(&receipt.state_before),
        "state_after": hex::encode32(&receipt.state_after),
        "graph_after": hex::encode32(&receipt.graph_after),
        "active_nodes": receipt.active_nodes,
        "active_edges": receipt.active_edges,
        "residuals": {
            "authority": residuals.authority.raw(),
            "continuity": residuals.continuity.raw(),
            "energy": residuals.energy.raw(),
            "renormalization": residuals.renormalization.raw(),
            "capacity": residuals.capacity.raw(),
        },
        "status": commit_status_name(receipt.status),
    })
}

fn expression_projection_payload(
    projection: &ae_runtime::ExpressionProjectionV1,
) -> serde_json::Value {
    serde_json::json!({
        "schema": "astr-embodiment.expression-projection.v1",
        "revision": projection.revision,
        "profile_fxp6": {
            "warmth": projection.profile_fxp6.warmth,
            "sensitivity": projection.profile_fxp6.sensitivity,
            "guardedness": projection.profile_fxp6.guardedness,
            "repair_orientation": projection.profile_fxp6.repair_orientation,
            "engagement": projection.profile_fxp6.engagement,
            "epistemic_caution": projection.profile_fxp6.epistemic_caution,
        },
    })
}

fn semantic_vector_receipt_payload(
    receipt: &ae_contracts::TransitionReceiptV2,
) -> serde_json::Value {
    serde_json::json!({
        "schema": "astr-embodiment.semantic-vector-receipt.v2",
        "formula": "full-vector-route-neutral-relaxation-v1",
        "dimension_slot_count": receipt.semantic_vector.dimension_slot_count,
        "evaluated_dimension_count": receipt.semantic_vector.evaluated_dimension_count,
        "injected_dimension_count": receipt.semantic_vector.injected_dimension_count,
        "nonzero_evidence_dimension_count": receipt.semantic_vector.nonzero_evidence_dimension_count,
        "neutral_baseline_dimension_count": receipt.semantic_vector.neutral_baseline_dimension_count,
        "unavailable_dimension_count": receipt.semantic_vector.unavailable_dimension_count,
        "state_changed": receipt.semantic_vector.state_changed,
    })
}

fn node_observability_payload(
    node: &ae_runtime::NodeObservabilityProjectionWireV2,
) -> PyResult<serde_json::Value> {
    if !node.validate() {
        return Err(NativeCoreError::new_err(
            "SEMANTIC_CLOSURE_INVALID::node observability projection",
        ));
    }
    serde_json::to_value(node)
        .map_err(|_| NativeCoreError::new_err("ENCODING::node observability projection"))
}

/// The two historical digest fields remain in telemetry JSON for wire-shape
/// compatibility; native-only validation fixes the reserved vector digest to
/// its canonical zero value and derives the checkpoint digest from telemetry.
fn native_telemetry_payload(receipt: &ae_contracts::NativeTelemetryReceiptV1) -> serde_json::Value {
    serde_json::json!({
        "schema": receipt.schema,
        "formula": "phase0-native-propagation-fxp6-v1",
        "formula_digest": hex::encode32(&receipt.formula_digest),
        "scope_digest": hex::encode32(&receipt.scope_digest),
        "event_digest": hex::encode32(&receipt.event_digest),
        "source_digest": hex::encode32(&receipt.source_digest),
        "base_revision": receipt.base_revision,
        "next_revision": receipt.next_revision,
        "phase": "PREPARE",
        "state_before": hex::encode32(&receipt.state_before),
        "state_after": hex::encode32(&receipt.state_after),
        "graph_before": hex::encode32(&receipt.graph_before),
        "graph_after": hex::encode32(&receipt.graph_after),
        "local_digest": hex::encode32(&receipt.local_digest),
        "compensation_digest": hex::encode32(&receipt.compensation_digest),
        "effective_digest": hex::encode32(&receipt.effective_digest),
        "energy": {
            "reserve_before": receipt.energy.reserve_before.raw(),
            "reserve_after": receipt.energy.reserve_after.raw(),
            "recovered": receipt.energy.recovered.raw(),
            "spent": receipt.energy.spent.raw(),
            "headroom": receipt.energy.headroom.raw(),
            "residual": receipt.energy.residual.raw(),
        },
        "capacity": {
            "upper_saturated_nodes": receipt.capacity.upper_saturated_nodes,
            "node_limit": receipt.capacity.node_limit,
            "node_headroom": receipt.capacity.node_headroom.raw(),
            "edge_used": receipt.capacity.edge_used,
            "edge_limit": receipt.capacity.edge_limit,
            "edge_headroom": receipt.capacity.edge_headroom.raw(),
            "headroom": receipt.capacity.headroom.raw(),
            "residual": receipt.capacity.residual.raw(),
        },
        "residuals": {
            "authority": receipt.residuals.authority.raw(),
            "continuity": receipt.residuals.continuity.raw(),
            "energy": receipt.residuals.energy.raw(),
            "renormalization": receipt.residuals.renormalization.raw(),
            "capacity": receipt.residuals.capacity.raw(),
        },
        "residual_health": receipt.residual_health.raw(),
        "native_gate": receipt.native_gate.raw(),
        "checkpoint_digest": hex::encode32(&receipt.checkpoint_digest),
        "telemetry_digest": hex::encode32(&receipt.telemetry_digest),
    })
}

fn semantic_perception_payload(
    decision: &ae_runtime::PerceptionProposalDecisionV1,
) -> PyResult<serde_json::Value> {
    let receipt = legacy_semantic_receipt_payload(&decision.receipt);
    match decision.availability {
        ae_runtime::SemanticClosureAvailabilityV1::UnavailableLegacy => {
            if decision.semantic_vector_receipt.is_some()
                || decision.semantic_telemetry_receipt.is_some()
                || decision.node_observability.is_some()
                || decision.expression_projection.revision != decision.revision
            {
                return Err(NativeCoreError::new_err(
                    "LEGACY_UNATTESTED::legacy closure carried Phase 0 artifacts",
                ));
            }
            Ok(serde_json::json!({
                "schema": "astrembodiment.semantic-perception-closure.v2",
                "availability": "UNAVAILABLE_LEGACY",
                "receipt": receipt,
                "telemetry_receipt": null,
                "semantic_vector_receipt": null,
                "node_observability": null,
                "revision": decision.revision,
                "deduplicated": decision.deduplicated,
                "expression_projection": null,
                "migration_subcode": decision.migration_subcode.map(|value| value.as_str()),
            }))
        }
        ae_runtime::SemanticClosureAvailabilityV1::Available => {
            let expression_projection =
                expression_projection_payload(&decision.expression_projection);
            let semantic_receipt = decision.semantic_vector_receipt.as_ref().ok_or_else(|| {
                NativeCoreError::new_err("LEGACY_UNATTESTED::semantic receipt is absent")
            })?;
            let telemetry = decision
                .semantic_telemetry_receipt
                .as_ref()
                .ok_or_else(|| {
                    NativeCoreError::new_err("LEGACY_UNATTESTED::native telemetry is absent")
                })?;
            let node = decision.node_observability.as_ref().ok_or_else(|| {
                NativeCoreError::new_err("LEGACY_UNATTESTED::node observability is absent")
            })?;
            if decision.receipt.status != ae_contracts::CommitStatus::Committed
                || !semantic_receipt.validate()
                || !telemetry.validate()
                || semantic_receipt.semantic_vector.dimension_slot_count != 15
                || semantic_receipt.semantic_vector.evaluated_dimension_count != 15
                || semantic_receipt.semantic_vector.injected_dimension_count != 15
                || semantic_receipt.semantic_vector.unavailable_dimension_count != 0
                || semantic_receipt.formula_digest != decision.receipt.formula_digest
                || semantic_receipt.scope_digest != decision.receipt.scope_digest
                || semantic_receipt.event_digest != decision.receipt.event_digest
                || semantic_receipt.base_revision != decision.receipt.base_revision
                || semantic_receipt.next_revision != decision.receipt.next_revision
                || semantic_receipt.state_before != decision.receipt.state_before
                || semantic_receipt.state_after != decision.receipt.state_after
                || semantic_receipt.graph_after != decision.receipt.graph_after
                || semantic_receipt.residuals != decision.receipt.residuals
                || telemetry.formula_digest != decision.receipt.formula_digest
                || telemetry.scope_digest != decision.receipt.scope_digest
                || telemetry.event_digest != decision.receipt.event_digest
                || telemetry.base_revision != decision.receipt.base_revision
                || telemetry.next_revision != decision.receipt.next_revision
                || telemetry.state_before != decision.receipt.state_before
                || telemetry.state_after != decision.receipt.state_after
                || telemetry.graph_after != decision.receipt.graph_after
                || telemetry.residuals != decision.receipt.residuals
                || semantic_receipt.next_revision != decision.revision
                || telemetry.next_revision != decision.revision
                || node.revision != decision.revision
                || decision.expression_projection.revision != decision.revision
            {
                return Err(NativeCoreError::new_err(
                    "LEGACY_UNATTESTED::semantic closure failed boundary validation",
                ));
            }
            Ok(serde_json::json!({
                "schema": "astrembodiment.semantic-perception-closure.v2",
                "availability": "AVAILABLE",
                "receipt": receipt,
                "telemetry_receipt": native_telemetry_payload(telemetry),
                "semantic_vector_receipt": semantic_vector_receipt_payload(semantic_receipt),
                "node_observability": node_observability_payload(node)?,
                "revision": decision.revision,
                "deduplicated": decision.deduplicated,
                "expression_projection": expression_projection,
                "migration_subcode": decision.migration_subcode.map(|value| value.as_str()),
            }))
        }
    }
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

fn seed_config_state_name(state: SeedConfigStateV1) -> &'static str {
    match state {
        SeedConfigStateV1::Unchanged => "UNCHANGED",
        SeedConfigStateV1::WriteMirror => "WRITE_MIRROR",
        SeedConfigStateV1::Deferred => "DEFERRED",
        SeedConfigStateV1::RebirthCommitted => "REBIRTH_COMMITTED",
        SeedConfigStateV1::RebirthReplayed => "REBIRTH_REPLAYED",
    }
}

fn seed_config_ack_state_name(state: SeedConfigAckStateV1) -> &'static str {
    match state {
        SeedConfigAckStateV1::MirrorActive => "MIRROR_ACTIVE",
        SeedConfigAckStateV1::Replayed => "REPLAYED",
        SeedConfigAckStateV1::Stale => "STALE",
    }
}

fn seed_config_result_payload(result: ae_store::SeedConfigReconcileResultV1) -> serde_json::Value {
    let writeback = result.writeback.map(|value| {
        serde_json::json!({
            "seed_code": value.seed_code,
            "mirror_guard": value.mirror_guard,
            "writeback_token": value.writeback_token,
        })
    });
    serde_json::json!({
        "schema": "astrembodiment.seed-config-result.v1",
        "state": seed_config_state_name(result.state),
        "writeback": writeback,
        "before_revision": result.before_revision,
        "after_revision": result.after_revision,
        "reason": result.reason,
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
struct FfiSeedConfigScopeV1 {
    bot_token: String,
    persona_token: String,
    relation_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum FfiSeedConfigObservationV1 {
    PresentNonempty,
    PresentEmpty,
    Missing,
    ReadFailed,
}

impl FfiSeedConfigObservationV1 {
    fn into_runtime(self) -> SeedConfigObservationV1 {
        match self {
            Self::PresentNonempty => SeedConfigObservationV1::PresentNonempty,
            Self::PresentEmpty => SeedConfigObservationV1::PresentEmpty,
            Self::Missing => SeedConfigObservationV1::Missing,
            Self::ReadFailed => SeedConfigObservationV1::ReadFailed,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum FfiSeedConfigOriginV1 {
    UserSaveEvent,
    StartupRead,
    PluginWriteback,
    LegacyConfigMigration,
}

impl FfiSeedConfigOriginV1 {
    fn into_runtime(self) -> SeedConfigOriginV1 {
        match self {
            Self::UserSaveEvent => SeedConfigOriginV1::UserSaveEvent,
            Self::StartupRead => SeedConfigOriginV1::StartupRead,
            Self::PluginWriteback => SeedConfigOriginV1::PluginWriteback,
            Self::LegacyConfigMigration => SeedConfigOriginV1::LegacyConfigMigration,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FfiSeedConfigReconcileRequestV1 {
    schema: String,
    scope: FfiSeedConfigScopeV1,
    observation: FfiSeedConfigObservationV1,
    origin: FfiSeedConfigOriginV1,
    seed_code: Option<String>,
    mirror_guard: Option<String>,
    previous_observation: Option<FfiSeedConfigObservationV1>,
    package_epoch: String,
    config_schema_version: u16,
    host_config_revision: u64,
}

impl FfiSeedConfigReconcileRequestV1 {
    fn into_runtime(self) -> Result<(ScopeRef, SeedConfigReconcileRequestV1), ()> {
        if self.schema != "astrembodiment.seed-config-observation.v1" {
            return Err(());
        }
        let scope = self.scope.scope_ref()?;
        let scope_token = wire::persona_scope_digest(
            &scope.bot_token,
            &scope.persona_token,
            scope.relation_token.as_ref(),
        );
        let observation = self.observation.into_runtime();
        let seed_code = self.seed_code;
        let mirror_guard = self.mirror_guard;
        if mirror_guard
            .as_deref()
            .is_some_and(|value| !strict_seed_config_capability(value))
        {
            return Err(());
        }
        if seed_code.as_deref().is_some_and(|value| value.len() > 256) {
            return Err(());
        }
        match observation {
            SeedConfigObservationV1::PresentNonempty => {
                if !seed_code
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                {
                    return Err(());
                }
            }
            SeedConfigObservationV1::PresentEmpty
            | SeedConfigObservationV1::Missing
            | SeedConfigObservationV1::ReadFailed => {
                if seed_code.is_some() {
                    return Err(());
                }
            }
        }
        if self.package_epoch.is_empty()
            || self.package_epoch.len() > 128
            || !self
                .package_epoch
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            || self.config_schema_version != 1
            || self.previous_observation.as_ref().is_some_and(|previous| {
                !matches!(previous, FfiSeedConfigObservationV1::PresentNonempty)
            })
        {
            return Err(());
        }
        Ok((
            scope,
            SeedConfigReconcileRequestV1 {
                scope_token,
                observation,
                origin: self.origin.into_runtime(),
                seed_code,
                mirror_guard,
                previous_observation: self
                    .previous_observation
                    .map(FfiSeedConfigObservationV1::into_runtime),
                package_epoch: self.package_epoch,
                config_schema_version: self.config_schema_version,
                host_config_revision: self.host_config_revision,
            },
        ))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FfiSeedConfigWritebackAckV1 {
    schema: String,
    scope: FfiSeedConfigScopeV1,
    writeback_token: String,
    write_succeeded: bool,
    host_config_revision: u64,
}

impl FfiSeedConfigWritebackAckV1 {
    fn into_runtime(self) -> Result<(ScopeRef, SeedConfigWritebackAckV1), ()> {
        if self.schema != "astrembodiment.seed-config-writeback-ack.v1"
            || !self.write_succeeded
            || !strict_seed_config_capability(&self.writeback_token)
        {
            return Err(());
        }
        let scope = self.scope.scope_ref()?;
        let scope_token = wire::persona_scope_digest(
            &scope.bot_token,
            &scope.persona_token,
            scope.relation_token.as_ref(),
        );
        Ok((
            scope,
            SeedConfigWritebackAckV1 {
                scope_token,
                writeback_token: self.writeback_token,
                write_succeeded: self.write_succeeded,
                host_config_revision: self.host_config_revision,
            },
        ))
    }
}

impl FfiSeedConfigScopeV1 {
    fn scope_ref(&self) -> Result<ScopeRef, ()> {
        if !strict_seed_config_token(&self.bot_token)
            || !strict_seed_config_token(&self.persona_token)
            || self
                .relation_token
                .as_deref()
                .is_some_and(|value| !strict_seed_config_token(value))
        {
            return Err(());
        }
        Ok(ScopeRef {
            bot_token: hex::decode16(&self.bot_token).map_err(|_| ())?,
            persona_token: hex::decode16(&self.persona_token).map_err(|_| ())?,
            relation_token: self
                .relation_token
                .as_deref()
                .map(hex::decode16)
                .transpose()
                .map_err(|_| ())?,
            // Seed configuration owns only the Bot/Persona scope.  Session
            // is deliberately absent from this ABI and is fixed to zero.
            session_token: [0; 16],
        })
    }
}

fn strict_seed_config_token(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn strict_seed_config_capability(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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

/// Return the opaque, native-owned node-observability contract declaration.
/// It carries no field values, identity, or model content.
#[pyfunction]
fn contract_info() -> PyResult<String> {
    let contract = node_observability_contract_info_v1();
    if !contract.validate() {
        return Err(NativeCoreError::new_err(
            "SEMANTIC_CLOSURE_INVALID::node observability contract",
        ));
    }
    serde_json::to_string(&contract)
        .map_err(|_| NativeCoreError::new_err("ENCODING::node observability contract"))
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

/// Reconcile a tri-state host SeedCode observation. The only destructive
/// authority is inside the dedicated Rust seed-config lifecycle; this JSON
/// boundary accepts no incarnation/revision fence and no manual consent flag.
#[pyfunction]
fn reconcile_seed_config_v1(request_json: &str) -> PyResult<String> {
    let request: FfiSeedConfigReconcileRequestV1 = serde_json::from_str(request_json)
        .map_err(|_| seed_config_error(SeedConfigLifecycleError::SchemaInvalid.code()))?;
    let (scope, request) = request
        .into_runtime()
        .map_err(|_| seed_config_error(SeedConfigLifecycleError::SchemaInvalid.code()))?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| seed_config_error(SeedConfigLifecycleError::Unknown.code()))?;
    let result = runtime
        .reconcile_seed_config_v1(&scope, &request)
        .map_err(map_error)?;
    serde_json::to_string(&seed_config_result_payload(result))
        .map_err(|_| seed_config_error(SeedConfigLifecycleError::Unknown.code()))
}

/// Mark a native pending mirror active only after AstrBot reports that the
/// exact configuration writeback succeeded.
#[pyfunction]
fn ack_seed_config_writeback_v1(request_json: &str) -> PyResult<String> {
    let request: FfiSeedConfigWritebackAckV1 = serde_json::from_str(request_json)
        .map_err(|_| seed_config_error(SeedConfigLifecycleError::SchemaInvalid.code()))?;
    let (scope, request) = request
        .into_runtime()
        .map_err(|_| seed_config_error(SeedConfigLifecycleError::SchemaInvalid.code()))?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| seed_config_error(SeedConfigLifecycleError::Unknown.code()))?;
    let result = runtime
        .ack_seed_config_writeback_v1(&scope, &request)
        .map_err(map_error)?;
    serde_json::to_string(&serde_json::json!({
        "schema": "astrembodiment.seed-config-ack.v1",
        "state": seed_config_ack_state_name(result.state),
    }))
    .map_err(|_| seed_config_error(SeedConfigLifecycleError::Unknown.code()))
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

/// Return the closed availability surface for the native-only async key.
#[pyfunction]
fn semantic_outbox_crypto_status_v1() -> PyResult<String> {
    let unavailable = SemanticOutboxCryptoStatusV1 {
        status: SemanticOutboxCryptoStatusValueV1::Unavailable,
        key_version: SEMANTIC_OUTBOX_KEY_VERSION_V1,
    };
    let status = match core() {
        Ok(guard) => guard
            .as_ref()
            .map(ae_runtime::AstrRuntime::semantic_outbox_crypto_status_v1)
            .unwrap_or(unavailable),
        Err(_) => unavailable,
    };
    serde_json::to_string(&semantic_outbox_status_payload(status))
        .map_err(|_| async_crypto_error(SemanticOutboxCryptoError::Unavailable))
}

/// Seal one closed async payload without exposing the installation key or its
/// filesystem/DPAPI representation to Python.
#[pyfunction]
fn semantic_outbox_seal_v1(request_json: &str) -> PyResult<String> {
    if request_json.len() > OUTBOX_SEAL_REQUEST_MAX_JSON_BYTES {
        return Err(async_crypto_error(
            SemanticOutboxCryptoError::PayloadAuthFailed,
        ));
    }
    let request: FfiSemanticOutboxSealRequestV1 = serde_json::from_str(request_json)
        .map_err(|_| async_crypto_error(SemanticOutboxCryptoError::PayloadAuthFailed))?;
    if request.schema != OUTBOX_SEAL_REQUEST_SCHEMA {
        return Err(async_crypto_error(
            SemanticOutboxCryptoError::PayloadAuthFailed,
        ));
    }
    if request.key_version != SEMANTIC_OUTBOX_KEY_VERSION_V1 {
        return Err(async_crypto_error(
            SemanticOutboxCryptoError::KeyVersionUnsupported,
        ));
    }
    let aad = canonical_outbox_base64(&request.aad_b64, SEMANTIC_OUTBOX_MAX_AAD_BYTES_V1)
        .map_err(async_crypto_error)?;
    let plaintext = canonical_outbox_base64(
        &request.plaintext_b64,
        SEMANTIC_OUTBOX_MAX_PLAINTEXT_BYTES_V1,
    )
    .map_err(async_crypto_error)?;
    let mut guard =
        core().map_err(|_| async_crypto_error(SemanticOutboxCryptoError::Unavailable))?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| async_crypto_error(SemanticOutboxCryptoError::Unavailable))?;
    let envelope = runtime
        .semantic_outbox_seal_v1(request.key_version, &aad, &plaintext)
        .map_err(async_crypto_error)?;
    serde_json::to_string(&serde_json::json!({
        "schema": OUTBOX_SEALED_SCHEMA,
        "key_version": SEMANTIC_OUTBOX_KEY_VERSION_V1,
        "envelope_b64": BASE64_STANDARD.encode(envelope),
    }))
    .map_err(|_| async_crypto_error(SemanticOutboxCryptoError::Unavailable))
}

/// Authenticate and open one closed async envelope.  Malformed base64 and
/// authenticated-decryption failures intentionally share one stable code.
#[pyfunction]
fn semantic_outbox_open_v1(request_json: &str) -> PyResult<String> {
    if request_json.len() > OUTBOX_OPEN_REQUEST_MAX_JSON_BYTES {
        return Err(async_crypto_error(
            SemanticOutboxCryptoError::PayloadAuthFailed,
        ));
    }
    let request: FfiSemanticOutboxOpenRequestV1 = serde_json::from_str(request_json)
        .map_err(|_| async_crypto_error(SemanticOutboxCryptoError::PayloadAuthFailed))?;
    if request.schema != OUTBOX_OPEN_REQUEST_SCHEMA {
        return Err(async_crypto_error(
            SemanticOutboxCryptoError::PayloadAuthFailed,
        ));
    }
    let aad = canonical_outbox_base64(&request.aad_b64, SEMANTIC_OUTBOX_MAX_AAD_BYTES_V1)
        .map_err(async_crypto_error)?;
    let envelope =
        canonical_outbox_base64(&request.envelope_b64, SEMANTIC_OUTBOX_MAX_ENVELOPE_BYTES_V1)
            .map_err(async_crypto_error)?;
    let mut guard =
        core().map_err(|_| async_crypto_error(SemanticOutboxCryptoError::Unavailable))?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| async_crypto_error(SemanticOutboxCryptoError::Unavailable))?;
    let plaintext = runtime
        .semantic_outbox_open_v1(SEMANTIC_OUTBOX_KEY_VERSION_V1, &aad, &envelope)
        .map_err(async_crypto_error)?;
    serde_json::to_string(&serde_json::json!({
        "schema": OUTBOX_OPENED_SCHEMA,
        "plaintext_b64": BASE64_STANDARD.encode(plaintext),
    }))
    .map_err(|_| async_crypto_error(SemanticOutboxCryptoError::Unavailable))
}

/// Read the closed per-persona semantic-lane cursor. This is intentionally
/// separate from the legacy G0 continuity revision.
#[pyfunction]
fn semantic_revision_v1(scope_json: &str) -> PyResult<String> {
    let scope_ref = parse_semantic_scope_json(scope_json)
        .map_err(|error| NativeCoreError::new_err(format!("INVALID_PERCEPTION_SCOPE::{error}")))?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let revision = runtime
        .semantic_revision_v1(&scope_ref)
        .map_err(map_error)?;
    serde_json::to_string(&serde_json::json!({
        "schema": "astrembodiment.semantic-revision.v1",
        "revision": revision,
    }))
    .map_err(|error| NativeCoreError::new_err(format!("ENCODING::{error}")))
}

/// Apply a complete text-free semantic proposal. The FFI parser is the closed
/// trust boundary: malformed, non-integer, unknown, duplicate, or noncanonical
/// proposal fields never reach native state.
#[pyfunction]
fn apply_perception_proposal_v1(scope_json: &str, proposal_json: &str) -> PyResult<String> {
    let scope_ref = parse_semantic_scope_json(scope_json)
        .map_err(|error| NativeCoreError::new_err(format!("INVALID_PERCEPTION_SCOPE::{error}")))?;
    let proposal = parse_semantic_proposal_json(proposal_json).map_err(|error| {
        NativeCoreError::new_err(format!("INVALID_PERCEPTION_PROPOSAL::{error}"))
    })?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let decision = runtime
        .apply_perception_proposal_v1(&scope_ref, &proposal)
        .map_err(map_semantic_perception_error)?;
    let payload = semantic_perception_payload(&decision)?;
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
    module.add_function(wrap_pyfunction!(contract_info, module)?)?;
    module.add_function(wrap_pyfunction!(health, module)?)?;
    module.add_function(wrap_pyfunction!(open, module)?)?;
    module.add_function(wrap_pyfunction!(ensure_genesis, module)?)?;
    module.add_function(wrap_pyfunction!(prepare_rebirth_v1, module)?)?;
    module.add_function(wrap_pyfunction!(confirm_rebirth_v1, module)?)?;
    module.add_function(wrap_pyfunction!(reconcile_seed_config_v1, module)?)?;
    module.add_function(wrap_pyfunction!(ack_seed_config_writeback_v1, module)?)?;
    module.add_function(wrap_pyfunction!(apply_event, module)?)?;
    module.add_function(wrap_pyfunction!(semantic_outbox_crypto_status_v1, module)?)?;
    module.add_function(wrap_pyfunction!(semantic_outbox_seal_v1, module)?)?;
    module.add_function(wrap_pyfunction!(semantic_outbox_open_v1, module)?)?;
    module.add_function(wrap_pyfunction!(semantic_revision_v1, module)?)?;
    module.add_function(wrap_pyfunction!(apply_perception_proposal_v1, module)?)?;
    module.add_function(wrap_pyfunction!(inspect, module)?)?;
    module.add_function(wrap_pyfunction!(verify_replay, module)?)?;
    module.add_function(wrap_pyfunction!(flush_and_close, module)?)?;
    module.add("NativeCoreError", module.py().get_type::<NativeCoreError>())?;
    Ok(())
}
