#![forbid(unsafe_code)]

//! PyO3 boundary: the ONLY surface Python may touch.
//!
//! Exposed: health, version, open, ensure_genesis, apply_event, inspect,
//! verify_replay, flush_and_close. There are no per-neuron getters, no
//! residual writers, no import-from-SeedCode entry point. JSON is exchanged
//! as closed, deny-unknown-field payloads; identity is computed in Rust.

use ae_context_projector::{ContextSummaryV1, DeliveryOutcome as ContextDeliveryOutcome};
use ae_contracts::{
    hex, wire, CanonicalEvent, EvidenceVector, PerceptionProposalV1, PersonaGenesisRequest,
    ScopeRef, SemanticLearningCompensationApplyV1, SemanticLearningCompensationClaimV1,
    SemanticLearningCompensationEnqueueV1, SemanticLearningCompensationTerminalV1,
};
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
        ae_runtime::RuntimeError::NativeGateUnavailable => {
            ("NATIVE_GATE_UNAVAILABLE", error.to_string())
        }
        ae_runtime::RuntimeError::InvalidLearningCompensation => {
            ("INVALID_LEARNING_COMPENSATION", error.to_string())
        }
        ae_runtime::RuntimeError::LearningCompensationUnavailable => {
            ("LEARNING_COMPENSATION_UNAVAILABLE", error.to_string())
        }
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

const EVIDENCE_DIMENSION_NAMES: [&str; 15] = [
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
];

fn strict_evidence_vector(value: Option<&serde_json::Value>, signed: bool) -> bool {
    let Some(object) = value.and_then(serde_json::Value::as_object) else {
        return false;
    };
    object.len() == EVIDENCE_DIMENSION_NAMES.len()
        && EVIDENCE_DIMENSION_NAMES.iter().all(|name| {
            object.get(*name).is_some_and(|value| {
                let Some(raw) = value.as_i64() else {
                    return false;
                };
                if signed {
                    (-250_000..=250_000).contains(&raw)
                } else {
                    (0..=1_000_000).contains(&raw)
                }
            })
        })
}

fn strict_digest_fields(
    object: &serde_json::Map<String, serde_json::Value>,
    names: &[&str],
) -> bool {
    names.iter().all(|name| {
        object
            .get(*name)
            .is_some_and(|value| strict_lower_hex(value, 64))
    })
}

fn parse_learning_enqueue_json(
    payload_json: &str,
) -> Result<SemanticLearningCompensationEnqueueV1, &'static str> {
    let payload: SemanticLearningCompensationEnqueueV1 = serde_json::from_str(payload_json)
        .map_err(|_| "enqueue must be a closed integer-only schema")?;
    let raw: serde_json::Value =
        serde_json::from_str(payload_json).map_err(|_| "enqueue must be valid JSON")?;
    let object = raw.as_object().ok_or("enqueue must be a JSON object")?;
    if !strict_digest_fields(
        object,
        &[
            "source_event_digest",
            "source_text_digest",
            "policy_digest",
            "provider_digest",
            "model_digest",
            "prompt_digest",
            "schema_digest",
            "formula_digest",
            "local_estimator_formula_digest",
            "source_telemetry_digest",
            "source_checkpoint_digest",
        ],
    ) || !strict_evidence_vector(object.get("local_vector"), false)
        || !object.contains_key("local_confidence_vector")
        || (!object
            .get("local_confidence_vector")
            .is_some_and(serde_json::Value::is_null)
            && !strict_evidence_vector(object.get("local_confidence_vector"), false))
        || !object
            .get("schema_version")
            .is_some_and(serde_json::Value::is_u64)
        || !object
            .get("source_revision")
            .is_some_and(serde_json::Value::is_u64)
        || !payload.validate_common()
    {
        return Err("enqueue fields are not canonical FxP6/digest values");
    }
    Ok(payload)
}

fn parse_learning_claim_json(
    payload_json: &str,
) -> Result<SemanticLearningCompensationClaimV1, &'static str> {
    let payload: SemanticLearningCompensationClaimV1 =
        serde_json::from_str(payload_json).map_err(|_| "claim must be a closed schema")?;
    let raw: serde_json::Value =
        serde_json::from_str(payload_json).map_err(|_| "claim must be valid JSON")?;
    let object = raw.as_object().ok_or("claim must be a JSON object")?;
    if !strict_digest_fields(object, &["job_id", "expected_request_digest"])
        || !object.contains_key("previous_lease_token")
        || (!object
            .get("previous_lease_token")
            .is_some_and(serde_json::Value::is_null)
            && !object
                .get("previous_lease_token")
                .is_some_and(|value| strict_lower_hex(value, 64)))
        || !object
            .get("schema_version")
            .is_some_and(serde_json::Value::is_u64)
        || !payload.validate()
    {
        return Err("claim fields are not canonical");
    }
    Ok(payload)
}

fn parse_learning_apply_json(
    payload_json: &str,
) -> Result<SemanticLearningCompensationApplyV1, &'static str> {
    let payload: SemanticLearningCompensationApplyV1 = serde_json::from_str(payload_json)
        .map_err(|_| "apply must be a closed integer-only schema")?;
    let raw: serde_json::Value =
        serde_json::from_str(payload_json).map_err(|_| "apply must be valid JSON")?;
    let object = raw.as_object().ok_or("apply must be a JSON object")?;
    if !strict_digest_fields(
        object,
        &[
            "job_id",
            "lease_token",
            "expected_request_digest",
            "expected_formula_digest",
            "expected_telemetry_digest",
            "expected_checkpoint_digest",
            "provider_digest",
            "model_digest",
            "prompt_digest",
        ],
    ) || !strict_evidence_vector(object.get("teacher_vector"), false)
        || !strict_evidence_vector(object.get("teacher_confidence_vector"), false)
        || !object
            .get("schema_version")
            .is_some_and(serde_json::Value::is_u64)
        || !object
            .get("expected_base_revision")
            .is_some_and(serde_json::Value::is_u64)
        || !payload.validate()
    {
        return Err("apply fields are not canonical FxP6/digest values");
    }
    Ok(payload)
}

fn parse_learning_terminal_json(
    payload_json: &str,
) -> Result<SemanticLearningCompensationTerminalV1, &'static str> {
    let payload: SemanticLearningCompensationTerminalV1 =
        serde_json::from_str(payload_json).map_err(|_| "terminal must be a closed schema")?;
    let raw: serde_json::Value =
        serde_json::from_str(payload_json).map_err(|_| "terminal must be valid JSON")?;
    let object = raw.as_object().ok_or("terminal must be a JSON object")?;
    if !strict_digest_fields(
        object,
        &[
            "job_id",
            "lease_token",
            "expected_request_digest",
            "reason_digest",
            "checkpoint_digest",
        ],
    ) || !object
        .get("schema_version")
        .is_some_and(serde_json::Value::is_u64)
        || !payload.validate()
    {
        return Err("terminal fields are not canonical");
    }
    Ok(payload)
}

fn evidence_vector_payload(vector: &EvidenceVector) -> serde_json::Value {
    serde_json::json!({
        "positive": vector.positive.raw(),
        "affiliation": vector.affiliation.raw(),
        "harm": vector.harm.raw(),
        "boundary": vector.boundary.raw(),
        "repair": vector.repair.raw(),
        "repetition": vector.repetition.raw(),
        "new_information": vector.new_information.raw(),
        "constraint_instability": vector.constraint_instability.raw(),
        "epistemic_conflict": vector.epistemic_conflict.raw(),
        "self_responsibility": vector.self_responsibility.raw(),
        "other_responsibility": vector.other_responsibility.raw(),
        "hostility": vector.hostility.raw(),
        "publicness": vector.publicness.raw(),
        "engagement": vector.engagement.raw(),
        "rejection": vector.rejection.raw(),
    })
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
    node: &ae_runtime::NodeObservabilityProjectionV1,
) -> serde_json::Value {
    let residuals = match node.residuals.state {
        ae_runtime::NodeObservabilityResidualStateV1::NotComputed => {
            serde_json::json!({"state":"NOT_COMPUTED","formula":null,"values_fxp6":null})
        }
    };
    let regions: Vec<serde_json::Value> = node
        .regions
        .iter()
        .map(|region| {
            serde_json::json!({
                "region_id": region.region_id,
                "region_name": region.region_name,
                "node_capacity": region.node_capacity,
                "selected_node_count": region.selected_node_count,
                "activated_node_count": region.activated_node_count,
                "changed_node_count": region.changed_node_count,
                "potential": {
                    "before_mean_fxp6": region.potential.before_mean_fxp6,
                    "after_mean_fxp6": region.potential.after_mean_fxp6,
                    "delta_mean_fxp6": region.potential.delta_mean_fxp6,
                    "changed_node_count": region.potential.changed_node_count,
                    "nonzero_after_count": region.potential.nonzero_after_count,
                },
                "excitation": {
                    "before_mean_fxp6": region.excitation.before_mean_fxp6,
                    "after_mean_fxp6": region.excitation.after_mean_fxp6,
                    "delta_mean_fxp6": region.excitation.delta_mean_fxp6,
                    "changed_node_count": region.excitation.changed_node_count,
                    "nonzero_after_count": region.excitation.nonzero_after_count,
                },
            })
        })
        .collect();
    serde_json::json!({
        "schema": "astr-embodiment.node-observability.v1",
        "formula": "spc1-node-observability-v1",
        "revision": node.revision,
        "field_node_capacity": node.field_node_capacity,
        "region_layout": "regions-v1",
        "counts": {
            "selected_node_count": node.counts.selected_node_count,
            "activated_node_count": node.counts.activated_node_count,
            "changed_node_count": node.counts.changed_node_count,
            "potential_nonzero_after_count": node.counts.potential_nonzero_after_count,
            "excitation_nonzero_after_count": node.counts.excitation_nonzero_after_count,
            "signal_nonzero_after_count": node.counts.signal_nonzero_after_count,
        },
        "residuals": residuals,
        "regions": regions,
    })
}

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
                || telemetry.native_gate.raw() == 0
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
                "node_observability": node_observability_payload(node),
                "revision": decision.revision,
                "deduplicated": decision.deduplicated,
                "expression_projection": expression_projection,
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
        .map_err(map_error)?;
    let payload = semantic_perception_payload(&decision)?;
    serde_json::to_string(&payload)
        .map_err(|error| NativeCoreError::new_err(format!("ENCODING::{error}")))
}

fn optional_digest_payload(value: Option<ae_contracts::Digest>) -> Option<String> {
    value.map(|digest| hex::encode32(&digest))
}

fn learning_enqueue_payload(
    decision: &ae_runtime::LearningCompensationEnqueueDecisionV1,
) -> serde_json::Value {
    serde_json::json!({
        "schema": "astrembodiment.learning-compensation-enqueue.v1",
        "availability": decision.availability.as_str(),
        "status": decision.status.as_str(),
        "job_id": optional_digest_payload(decision.job_id),
        "source_event_digest": hex::encode32(&decision.source_event_digest),
        "source_text_digest": hex::encode32(&decision.source_text_digest),
        "source_revision": decision.source_revision,
        "formula_digest": hex::encode32(&decision.formula_digest),
        "local_estimator_formula_digest": hex::encode32(&decision.local_estimator_formula_digest),
        "learning_formula_digest": optional_digest_payload(decision.learning_formula_digest),
        "policy_digest": hex::encode32(&decision.policy_digest),
        "request_digest": optional_digest_payload(decision.request_digest),
        "terminal_status": decision.terminal_status.map(terminal_status_name),
        "receipt_digest": optional_digest_payload(decision.receipt_digest),
        "expression_projection": null,
    })
}

fn learning_claim_payload(
    decision: &ae_runtime::LearningCompensationClaimDecisionV1,
) -> serde_json::Value {
    serde_json::json!({
        "schema": "astrembodiment.learning-compensation-claim.v1",
        "job_id": hex::encode32(&decision.job_id),
        "status": decision.status.as_str(),
        "lease_token": optional_digest_payload(decision.lease_token),
        "lease_epoch": decision.lease_epoch,
        "source_event_digest": optional_digest_payload(decision.source_event_digest),
        "source_text_digest": optional_digest_payload(decision.source_text_digest),
        "source_revision": decision.source_revision,
        "request_digest": optional_digest_payload(decision.request_digest),
        "base_revision": decision.base_revision,
        "formula_digest": optional_digest_payload(decision.formula_digest),
        "local_estimator_formula_digest": optional_digest_payload(decision.local_estimator_formula_digest),
        "learning_formula_digest": optional_digest_payload(decision.learning_formula_digest),
        "policy_digest": optional_digest_payload(decision.policy_digest),
        "provider_digest": optional_digest_payload(decision.provider_digest),
        "model_digest": optional_digest_payload(decision.model_digest),
        "prompt_digest": optional_digest_payload(decision.prompt_digest),
        "schema_digest": optional_digest_payload(decision.schema_digest),
        "telemetry_digest": optional_digest_payload(decision.telemetry_digest),
        "checkpoint_digest": optional_digest_payload(decision.checkpoint_digest),
        "terminal_status": decision.terminal_status.map(terminal_status_name),
        "receipt_digest": optional_digest_payload(decision.receipt_digest),
        "expression_projection": null,
    })
}

fn learning_receipt_payload(
    status: &str,
    receipt: &ae_contracts::LearningCompensationReceiptV1,
) -> serde_json::Value {
    serde_json::json!({
        "schema": "astrembodiment.learning-compensation-receipt.v1",
        "job_id": hex::encode32(&receipt.job_id),
        "status": status,
        "source_event_digest": hex::encode32(&receipt.source_event_digest),
        "source_text_digest": hex::encode32(&receipt.source_text_digest),
        "source_revision": receipt.source_revision,
        "base_revision": receipt.base_revision,
        "next_checkpoint_revision": receipt.next_checkpoint_revision,
        "formula_digest": hex::encode32(&receipt.formula_digest),
        "local_estimator_formula_digest": hex::encode32(&receipt.local_estimator_formula_digest),
        "learning_formula_digest": hex::encode32(&receipt.learning_formula_digest),
        "telemetry_digest": hex::encode32(&receipt.telemetry_digest),
        "checkpoint_digest": hex::encode32(&receipt.checkpoint_digest),
        "compensation_digest": hex::encode32(&receipt.compensation_digest),
        "policy_digest": hex::encode32(&receipt.policy_digest),
        "provider_digest": hex::encode32(&receipt.provider_digest),
        "model_digest": hex::encode32(&receipt.model_digest),
        "prompt_digest": hex::encode32(&receipt.prompt_digest),
        "schema_digest": hex::encode32(&receipt.schema_digest),
        "teacher_digest": hex::encode32(&receipt.teacher_digest),
        "request_digest": hex::encode32(&receipt.request_digest),
        "eligible_dimension_count": receipt.eligible_dimension_count,
        "changed_dimension_count": receipt.changed_dimension_count,
        "u_next": evidence_vector_payload(&receipt.u_next),
        "receipt_digest": hex::encode32(&receipt.receipt_digest),
        "expression_projection": null,
    })
}

fn learning_no_change_payload(
    receipt: &ae_contracts::LearningCompensationNoChangeReceiptV1,
) -> serde_json::Value {
    serde_json::json!({
        "schema": "astrembodiment.learning-compensation-receipt.v1",
        "job_id": hex::encode32(&receipt.job_id),
        "status": "NO_CHANGE",
        "source_event_digest": hex::encode32(&receipt.source_event_digest),
        "source_text_digest": hex::encode32(&receipt.source_text_digest),
        "source_revision": receipt.source_revision,
        "base_revision": receipt.base_revision,
        "next_checkpoint_revision": null,
        "formula_digest": hex::encode32(&receipt.formula_digest),
        "local_estimator_formula_digest": hex::encode32(&receipt.local_estimator_formula_digest),
        "learning_formula_digest": hex::encode32(&receipt.learning_formula_digest),
        "telemetry_digest": hex::encode32(&receipt.telemetry_digest),
        "checkpoint_digest": hex::encode32(&receipt.checkpoint_digest),
        "compensation_digest": null,
        "policy_digest": hex::encode32(&receipt.policy_digest),
        "provider_digest": hex::encode32(&receipt.provider_digest),
        "model_digest": hex::encode32(&receipt.model_digest),
        "prompt_digest": hex::encode32(&receipt.prompt_digest),
        "schema_digest": hex::encode32(&receipt.schema_digest),
        "teacher_digest": hex::encode32(&receipt.teacher_digest),
        "request_digest": hex::encode32(&receipt.request_digest),
        "eligible_dimension_count": receipt.eligible_dimension_count,
        "changed_dimension_count": receipt.changed_dimension_count,
        "u_next": null,
        "receipt_digest": hex::encode32(&receipt.receipt_digest),
        "expression_projection": null,
    })
}

fn unavailable_learning_apply_payload(
    job_id: &ae_contracts::Digest,
    status: &str,
) -> serde_json::Value {
    serde_json::json!({
        "schema": "astrembodiment.learning-compensation-receipt.v1",
        "job_id": hex::encode32(job_id),
        "status": status,
        "source_event_digest": null,
        "source_text_digest": null,
        "source_revision": null,
        "base_revision": null,
        "next_checkpoint_revision": null,
        "formula_digest": null,
        "local_estimator_formula_digest": null,
        "learning_formula_digest": null,
        "telemetry_digest": null,
        "checkpoint_digest": null,
        "compensation_digest": null,
        "policy_digest": null,
        "provider_digest": null,
        "model_digest": null,
        "prompt_digest": null,
        "schema_digest": null,
        "teacher_digest": null,
        "request_digest": null,
        "eligible_dimension_count": null,
        "changed_dimension_count": null,
        "u_next": null,
        "receipt_digest": null,
        "expression_projection": null,
    })
}

fn terminal_status_name(
    status: ae_contracts::LearningCompensationTerminalStatusV1,
) -> &'static str {
    match status {
        ae_contracts::LearningCompensationTerminalStatusV1::AbandonedInputUnavailable => {
            "ABANDONED_INPUT_UNAVAILABLE"
        }
        ae_contracts::LearningCompensationTerminalStatusV1::Rejected => "REJECTED",
        ae_contracts::LearningCompensationTerminalStatusV1::Expired => "EXPIRED",
    }
}

fn learning_terminal_payload(
    decision: &ae_runtime::LearningCompensationTerminalDecisionV1,
) -> serde_json::Value {
    let receipt = &decision.receipt;
    serde_json::json!({
        "schema": "astrembodiment.learning-compensation-terminal.v1",
        "job_id": hex::encode32(&receipt.job_id),
        "status": terminal_status_name(receipt.status),
        "source_event_digest": hex::encode32(&receipt.source_event_digest),
        "source_text_digest": hex::encode32(&receipt.source_text_digest),
        "source_revision": receipt.source_revision,
        "request_digest": hex::encode32(&receipt.request_digest),
        "formula_digest": hex::encode32(&receipt.formula_digest),
        "local_estimator_formula_digest": hex::encode32(&receipt.local_estimator_formula_digest),
        "learning_formula_digest": hex::encode32(&receipt.learning_formula_digest),
        "policy_digest": hex::encode32(&receipt.policy_digest),
        "provider_digest": hex::encode32(&receipt.provider_digest),
        "model_digest": hex::encode32(&receipt.model_digest),
        "prompt_digest": hex::encode32(&receipt.prompt_digest),
        "schema_digest": hex::encode32(&receipt.schema_digest),
        "reason_digest": hex::encode32(&receipt.reason_digest),
        "checkpoint_digest": hex::encode32(&receipt.checkpoint_digest),
        "receipt_digest": hex::encode32(&receipt.receipt_digest),
        "expression_projection": null,
    })
}

/// Persist a text-free, locally-attested 15D compensation request. Native
/// rejects inferred confidences and binds immutable source provenance before
/// the host submits any teacher material.
#[pyfunction]
fn enqueue_learning_compensation_v1(scope_json: &str, payload_json: &str) -> PyResult<String> {
    let scope_ref = parse_semantic_scope_json(scope_json)
        .map_err(|error| NativeCoreError::new_err(format!("INVALID_PERCEPTION_SCOPE::{error}")))?;
    let request = parse_learning_enqueue_json(payload_json).map_err(|error| {
        NativeCoreError::new_err(format!("INVALID_LEARNING_COMPENSATION::{error}"))
    })?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let decision = runtime
        .enqueue_learning_compensation_v1(&scope_ref, &request)
        .map_err(map_error)?;
    serde_json::to_string(&learning_enqueue_payload(&decision))
        .map_err(|error| NativeCoreError::new_err(format!("ENCODING::{error}")))
}

/// Claim the current native telemetry/cursor triple for one previously queued
/// text-free job. A stale retry must present its prior lease token.
#[pyfunction]
fn claim_learning_compensation_v1(scope_json: &str, payload_json: &str) -> PyResult<String> {
    let scope_ref = parse_semantic_scope_json(scope_json)
        .map_err(|error| NativeCoreError::new_err(format!("INVALID_PERCEPTION_SCOPE::{error}")))?;
    let claim = parse_learning_claim_json(payload_json).map_err(|error| {
        NativeCoreError::new_err(format!("INVALID_LEARNING_COMPENSATION::{error}"))
    })?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let decision = runtime
        .claim_learning_compensation_v1(&scope_ref, &claim)
        .map_err(map_error)?;
    serde_json::to_string(&learning_claim_payload(&decision))
        .map_err(|error| NativeCoreError::new_err(format!("ENCODING::{error}")))
}

/// Independently recompute Candidate B and atomically append a compensation
/// checkpoint. It never returns an expression projection or mutates the
/// semantic field/graph.
#[pyfunction]
fn apply_learning_compensation_v1(scope_json: &str, payload_json: &str) -> PyResult<String> {
    let scope_ref = parse_semantic_scope_json(scope_json)
        .map_err(|error| NativeCoreError::new_err(format!("INVALID_PERCEPTION_SCOPE::{error}")))?;
    let apply = parse_learning_apply_json(payload_json).map_err(|error| {
        NativeCoreError::new_err(format!("INVALID_LEARNING_COMPENSATION::{error}"))
    })?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let decision = runtime
        .apply_learning_compensation_v1(&scope_ref, &apply)
        .map_err(map_error)?;
    let payload = match &decision {
        ae_runtime::LearningCompensationApplyDecisionV1::Committed(receipt) => {
            learning_receipt_payload("COMMITTED", receipt)
        }
        ae_runtime::LearningCompensationApplyDecisionV1::Replayed(receipt) => {
            learning_receipt_payload("REPLAYED", receipt)
        }
        ae_runtime::LearningCompensationApplyDecisionV1::NoChange(receipt) => {
            learning_no_change_payload(receipt)
        }
        ae_runtime::LearningCompensationApplyDecisionV1::StaleRetry { job_id } => {
            unavailable_learning_apply_payload(job_id, "STALE_RETRY")
        }
        ae_runtime::LearningCompensationApplyDecisionV1::Unavailable { job_id } => {
            unavailable_learning_apply_payload(job_id, "UNAVAILABLE")
        }
    };
    serde_json::to_string(&payload)
        .map_err(|error| NativeCoreError::new_err(format!("ENCODING::{error}")))
}

#[pyfunction]
fn abandon_learning_compensation_v1(scope_json: &str, payload_json: &str) -> PyResult<String> {
    let scope_ref = parse_semantic_scope_json(scope_json)
        .map_err(|error| NativeCoreError::new_err(format!("INVALID_PERCEPTION_SCOPE::{error}")))?;
    let terminal = parse_learning_terminal_json(payload_json).map_err(|error| {
        NativeCoreError::new_err(format!("INVALID_LEARNING_COMPENSATION::{error}"))
    })?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let decision = runtime
        .abandon_learning_compensation_v1(&scope_ref, &terminal)
        .map_err(map_error)?;
    serde_json::to_string(&learning_terminal_payload(&decision))
        .map_err(|error| NativeCoreError::new_err(format!("ENCODING::{error}")))
}

#[pyfunction]
fn reject_learning_compensation_v1(scope_json: &str, payload_json: &str) -> PyResult<String> {
    let scope_ref = parse_semantic_scope_json(scope_json)
        .map_err(|error| NativeCoreError::new_err(format!("INVALID_PERCEPTION_SCOPE::{error}")))?;
    let terminal = parse_learning_terminal_json(payload_json).map_err(|error| {
        NativeCoreError::new_err(format!("INVALID_LEARNING_COMPENSATION::{error}"))
    })?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let decision = runtime
        .reject_learning_compensation_v1(&scope_ref, &terminal)
        .map_err(map_error)?;
    serde_json::to_string(&learning_terminal_payload(&decision))
        .map_err(|error| NativeCoreError::new_err(format!("ENCODING::{error}")))
}

/// Seal a valid claimed job as the first-class `EXPIRED` terminal state. The
/// worker must use this for TTL expiry rather than translating expiry into an
/// abandonment reason string.
#[pyfunction]
fn expire_learning_compensation_v1(scope_json: &str, payload_json: &str) -> PyResult<String> {
    let scope_ref = parse_semantic_scope_json(scope_json)
        .map_err(|error| NativeCoreError::new_err(format!("INVALID_PERCEPTION_SCOPE::{error}")))?;
    let terminal = parse_learning_terminal_json(payload_json).map_err(|error| {
        NativeCoreError::new_err(format!("INVALID_LEARNING_COMPENSATION::{error}"))
    })?;
    let mut guard = core()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
    let decision = runtime
        .expire_learning_compensation_v1(&scope_ref, &terminal)
        .map_err(map_error)?;
    serde_json::to_string(&learning_terminal_payload(&decision))
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
    module.add_function(wrap_pyfunction!(semantic_revision_v1, module)?)?;
    module.add_function(wrap_pyfunction!(apply_perception_proposal_v1, module)?)?;
    module.add_function(wrap_pyfunction!(enqueue_learning_compensation_v1, module)?)?;
    module.add_function(wrap_pyfunction!(claim_learning_compensation_v1, module)?)?;
    module.add_function(wrap_pyfunction!(apply_learning_compensation_v1, module)?)?;
    module.add_function(wrap_pyfunction!(abandon_learning_compensation_v1, module)?)?;
    module.add_function(wrap_pyfunction!(reject_learning_compensation_v1, module)?)?;
    module.add_function(wrap_pyfunction!(expire_learning_compensation_v1, module)?)?;
    module.add_function(wrap_pyfunction!(inspect, module)?)?;
    module.add_function(wrap_pyfunction!(verify_replay, module)?)?;
    module.add_function(wrap_pyfunction!(flush_and_close, module)?)?;
    module.add("NativeCoreError", module.py().get_type::<NativeCoreError>())?;
    Ok(())
}
