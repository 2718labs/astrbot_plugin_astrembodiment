//! Bounded native R7 organism projection runtime.
//!
//! The runtime owns one immutable typed identity constitution and accepts only
//! revision-bound products from the native R7 source cores. It delegates final
//! cross-source certification to `ae-cognitive-envelope` and advances its
//! in-memory revision only after that fail-closed compiler succeeds.
//!
//! This crate deliberately has no JSON, text, neural/KV-array, provider-payload,
//! persistence, Python, Host-wire, or delivery input. The assembled cognitive
//! envelope stays behind an opaque value; safe callers can observe only a
//! closure-scoped digest view.

#[cfg(test)]
use super::private_projection_wire::{
    certificate_digest_v1, payload_digest_v1, wire_digest_v1,
    PRIVATE_PROJECTION_PAYLOAD_MAX_BYTES_V1, PRIVATE_PROJECTION_PAYLOAD_WIRE_HEADER_LEN_V1,
    PRIVATE_PROJECTION_PAYLOAD_WIRE_MAGIC_V1, PRIVATE_PROJECTION_PAYLOAD_WIRE_SCHEMA_VERSION_V1,
};
use super::private_projection_wire::{
    seal_cognitive_envelope_v1, seal_pre_output_cognitive_envelope_v1,
    PrivateProjectionPayloadWireBindingMetadataV1, PrivateProjectionPayloadWireErrorV1,
    PrivateProjectionPayloadWireV1,
};
use ae_action_contract::{ActionContractV1, ActionRealizationV1};
use ae_cognitive_envelope::{
    compile_pre_output_projection_v1, compile_projection_v1, AffordanceCatalogV1,
    CognitiveEnvelopeV1, CognitiveKvViewV1, ExactTurnAnchorsV1, OrganismSnapshotRefV1,
    PreOutputCognitiveEnvelopeV1, PreOutputProjectionInputV1, ProjectionErrorV1, ProjectionInput,
    ProjectionPreconditionsV1, ProjectionSourceKindV1, ProviderProfileV1, RelationScopeV1,
    SourceCapsuleV1,
};
use ae_contracts::r7::{wire, Digest, Id128};
use ae_efference_copy::EfferenceCopyV1;
use ae_epistemic_state::EpistemicProjectionV1;
use ae_genesis::r7::IdentityConstitutionV1;
use ae_morph::MorphAffordanceCatalogV1;
use ae_soma::{SomaClassificationIngressV1, SomaStateV1};
use serde_json::{Map, Value};
use thiserror::Error;

const PRIVATE_PROJECTION_PAYLOAD_BINDING_DOMAIN_V1: &[u8] =
    b"astr-embodiment/r7/private-projection-payload-binding-v1";

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub(crate) enum OrganismRuntimeErrorV1 {
    #[error("identity source capsule has wrong kind: {actual:?}")]
    WrongIdentitySourceKind { actual: ProjectionSourceKindV1 },
    #[error("identity source capsule digest does not match the typed constitution")]
    IdentityConstitutionDigestMismatch,
    #[error("revision {incoming} is stale or replayed; current revision is {current}")]
    StaleOrReplayedRevision { current: u64, incoming: u64 },
    #[error("{field} revision {actual} does not match update revision {expected}")]
    RevisionBindingMismatch {
        field: &'static str,
        expected: u64,
        actual: u64,
    },
    #[error("{field} identity does not match the immutable runtime identity")]
    IdentityBindingMismatch { field: &'static str },
    #[error("prepared semantic transition revision does not match the pre-output update")]
    PreparedTransitionRevisionMismatch,
    #[error("prepared semantic transition source state does not match the organism snapshot")]
    PreparedTransitionStateBindingMismatch,
    #[error("prepared semantic transition turn does not match the organism snapshot")]
    PreparedTransitionTurnBindingMismatch,
    #[error("prepared semantic transition scope does not match a pre-output source: {field}")]
    PreparedTransitionScopeBindingMismatch { field: &'static str },
    #[error(
        "prepared semantic transition causal binding does not match a pre-output source: {field}"
    )]
    PreparedTransitionCausalBindingMismatch { field: &'static str },
    #[error("cognitive projection was rejected: {0}")]
    ProjectionRejected(#[from] ProjectionErrorV1),
}

/// The six bounded reference capsules that have no native producer in this
/// crate. Values remain typed and bounded by `ae-cognitive-envelope`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BoundedProjectionReferencesV1 {
    organism_snapshot: SourceCapsuleV1<OrganismSnapshotRefV1>,
    cognitive_kv_view: SourceCapsuleV1<CognitiveKvViewV1>,
    exact_turn_anchors: SourceCapsuleV1<ExactTurnAnchorsV1>,
    relation_scope: SourceCapsuleV1<RelationScopeV1>,
    affordance_catalog: SourceCapsuleV1<AffordanceCatalogV1>,
    provider_profile: SourceCapsuleV1<ProviderProfileV1>,
}

impl BoundedProjectionReferencesV1 {
    pub(crate) fn new(
        organism_snapshot: SourceCapsuleV1<OrganismSnapshotRefV1>,
        cognitive_kv_view: SourceCapsuleV1<CognitiveKvViewV1>,
        exact_turn_anchors: SourceCapsuleV1<ExactTurnAnchorsV1>,
        relation_scope: SourceCapsuleV1<RelationScopeV1>,
        affordance_catalog: SourceCapsuleV1<AffordanceCatalogV1>,
        provider_profile: SourceCapsuleV1<ProviderProfileV1>,
    ) -> Self {
        Self {
            organism_snapshot,
            cognitive_kv_view,
            exact_turn_anchors,
            relation_scope,
            affordance_catalog,
            provider_profile,
        }
    }
}

/// One complete native update. No optional/default source exists: callers must
/// provide every typed source and every projection precondition explicitly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeProjectionUpdateV1 {
    revision: u64,
    references: BoundedProjectionReferencesV1,
    action_contract: SourceCapsuleV1<ActionContractV1>,
    soma_state: SourceCapsuleV1<SomaStateV1>,
    soma_classification_ingress: SomaClassificationIngressV1,
    epistemic_projection: EpistemicProjectionV1,
    action_realization: ActionRealizationV1,
    efference_copy: EfferenceCopyV1,
    preconditions: ProjectionPreconditionsV1,
}

impl NativeProjectionUpdateV1 {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        revision: u64,
        references: BoundedProjectionReferencesV1,
        action_contract: SourceCapsuleV1<ActionContractV1>,
        soma_state: SourceCapsuleV1<SomaStateV1>,
        soma_classification_ingress: SomaClassificationIngressV1,
        epistemic_projection: EpistemicProjectionV1,
        action_realization: ActionRealizationV1,
        efference_copy: EfferenceCopyV1,
        preconditions: ProjectionPreconditionsV1,
    ) -> Self {
        Self {
            revision,
            references,
            action_contract,
            soma_state,
            soma_classification_ingress,
            epistemic_projection,
            action_realization,
            efference_copy,
            preconditions,
        }
    }
}

/// Typed R7 sources available before provider output. In particular, this type
/// contains no `ActionRealizationV1` or `EfferenceCopyV1` field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreOutputProjectionUpdateV1 {
    revision: u64,
    references: BoundedProjectionReferencesV1,
    action_contract: SourceCapsuleV1<ActionContractV1>,
    soma_state: SourceCapsuleV1<SomaStateV1>,
    soma_classification_ingress: SomaClassificationIngressV1,
    epistemic_projection: EpistemicProjectionV1,
    preconditions: ProjectionPreconditionsV1,
}

impl PreOutputProjectionUpdateV1 {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        revision: u64,
        references: BoundedProjectionReferencesV1,
        action_contract: SourceCapsuleV1<ActionContractV1>,
        soma_state: SourceCapsuleV1<SomaStateV1>,
        soma_classification_ingress: SomaClassificationIngressV1,
        epistemic_projection: EpistemicProjectionV1,
        preconditions: ProjectionPreconditionsV1,
    ) -> Self {
        Self {
            revision,
            references,
            action_contract,
            soma_state,
            soma_classification_ingress,
            epistemic_projection,
            preconditions,
        }
    }
}

/// Opaque native pre-output projection. It stays digest-observable only and
/// cannot be rendered directly by callers.
struct PrivatePreOutputCognitiveProjectionV1 {
    revision: u64,
    turn_id: Id128,
    turn_binding: Digest,
    envelope: PreOutputCognitiveEnvelopeV1,
}

#[cfg(test)]
mod private_projection_wire_validation_tests {
    use super::*;

    fn digest(seed: u8) -> Digest {
        [seed; 32]
    }

    fn validation() -> PrivateProjectionWireValidationV1 {
        PrivateProjectionWireValidationV1 {
            revision: 9,
            epistemic_revision: 9,
            turn_id: [1; 16],
            epistemic_turn_id: [1; 16],
            turn_binding: digest(2),
            projection_digest: digest(3),
            identity_digest: digest(4),
            epistemic_identity_digest: digest(4),
            source_state_digest: digest(5),
            soma_state_digest: digest(5),
            epistemic_state_digest: digest(5),
            action_contract_digest: digest(6),
            certificate_action_contract_digest: digest(6),
            action_realization_digest: digest(7),
            certificate_action_realization_digest: digest(7),
            epistemic_projection_digest: digest(8),
            certificate_epistemic_projection_digest: digest(8),
        }
    }

    #[test]
    fn invalid_or_mismatched_projection_identity_is_refused_before_encoding() {
        let mut invalid = validation();
        invalid.projection_digest = [0; 32];
        assert!(matches!(
            validate_private_projection_for_wire_v1(&invalid),
            Err(PrivateProjectionPayloadWireErrorV1::ZeroIdentity {
                field: "projection_digest"
            })
        ));

        for (field, mismatched) in [
            (
                "action_contract_digest",
                PrivateProjectionWireValidationV1 {
                    certificate_action_contract_digest: digest(99),
                    ..validation()
                },
            ),
            (
                "turn_id",
                PrivateProjectionWireValidationV1 {
                    epistemic_turn_id: [99; 16],
                    ..validation()
                },
            ),
            (
                "source_state_digest",
                PrivateProjectionWireValidationV1 {
                    epistemic_state_digest: digest(99),
                    ..validation()
                },
            ),
        ] {
            assert!(matches!(
                validate_private_projection_for_wire_v1(&mismatched),
                Err(PrivateProjectionPayloadWireErrorV1::ProjectionBindingMismatch { field: actual })
                    if actual == field
            ));
        }
    }

    fn hex_digest(seed: u8) -> String {
        encode_hex_v1(&digest(seed))
    }

    fn semantic_payload() -> Value {
        let included = (20..20 + ProjectionInput::FIELD_COUNT as u8)
            .map(hex_digest)
            .collect::<Vec<_>>();
        serde_json::json!({
            "schema": "astrembodiment.cognitive-envelope.v1",
            "turn": {
                "turn_ref": "turn:9",
                "incarnation_ref": "incarnation:1",
                "scope_ref": "scope:current"
            },
            "identity": {
                "operational_commitments": ["accept_correction"],
                "anti_goals": ["avoid_fabrication"],
                "expression_basis": ["directness:high"],
                "correction_boundary_constitution": ["respect_boundary"],
                "relational_play_limits": ["current_scope_only"],
                "persona_revision_digest": hex_digest(40),
                "incarnation_digest": hex_digest(41),
                "constitution_digest": hex_digest(4)
            },
            "subjective_present": [{
                "axis": "energy",
                "band": "moderate",
                "trend": "stable",
                "behavioral_effect": "prefer_bounded_effort",
                "disclosure": "BEHAVIORAL_ONLY",
                "confidence": "high",
                "cause_ref": null
            }],
            "relation": {"fields": [{
                "name": "boundary",
                "value": "current_scope_only",
                "source_digest": hex_digest(42)
            }]},
            "epistemics": {
                "turn_id": encode_hex_v1(&[1; 16]),
                "state_digest": hex_digest(5),
                "revision": 9,
                "identity_digest": hex_digest(4),
                "claim_under_challenge": null,
                "source_estimate_digest": hex_digest(43),
                "classification_is_caller_provided": true,
                "projection_digest": hex_digest(8)
            },
            "praxis": {"fields": [{
                "name": "objective",
                "value": "answer_current_turn",
                "source_digest": hex_digest(44)
            }]},
            "affordances": [],
            "agency": {
                "action_contract": {
                    "schema": "astrembodiment.action-contract.v1",
                    "action_id": encode_hex_v1(&[13; 16]),
                    "turn_binding": hex_digest(2),
                    "base_revision": 9,
                    "source_state_digest": hex_digest(5),
                    "identity_constitution_digest": hex_digest(4),
                    "disposition": "speech",
                    "speech_act": "answer",
                    "requirements": {"must": [], "should": [], "may": [], "must_not": []},
                    "allowed_tools": [],
                    "allowed_disclosures": [],
                    "confidence_ceiling": 0.5,
                    "expires_at_ms": 1,
                    "contract_digest": hex_digest(6)
                },
                "action_contract_digest": hex_digest(6),
                "efference_copy_digest": hex_digest(9)
            },
            "realization": {
                "schema": "astrembodiment.action-realization.v1",
                "action_id": encode_hex_v1(&[13; 16]),
                "contract_digest": hex_digest(6),
                "speech_act": "answer",
                "owned_claims": [],
                "proposed_tools": [],
                "disclosures_used": [],
                "manifest_confidence": 0.5
            },
            "exact_anchors": [],
            "projection_certificate": {
                "source_state_digest": hex_digest(5),
                "soma_state_digest": hex_digest(5),
                "subjective_present_digest": hex_digest(10),
                "kv_snapshot_digest": hex_digest(11),
                "epistemic_projection_digest": hex_digest(8),
                "action_contract_digest": hex_digest(6),
                "action_realization_digest": hex_digest(7),
                "efference_copy_digest": hex_digest(9),
                "provider_profile_digest": hex_digest(12),
                "included_capsule_digests": included,
                "exact_anchor_residual": 0,
                "scope_residual": 0,
                "action_sensitivity_residual": 0,
                "action_sensitivity_bound": 0,
                "disclosure_residual": 0,
                "token_budget_used": 1
            }
        })
    }

    fn payload_wire(body: &Value) -> Vec<u8> {
        let validation = validation();
        let payload = serde_json::to_vec(body).expect("test payload encoding");
        let certificate =
            serde_json::to_vec(&body["projection_certificate"]).expect("test certificate encoding");
        let certificate_digest = certificate_digest_v1(&certificate);
        let binding_digest = projection_payload_binding_digest_v1(&validation, &digest(9));
        let payload_digest = payload_digest_v1(&payload);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(PRIVATE_PROJECTION_PAYLOAD_WIRE_MAGIC_V1);
        bytes.extend_from_slice(&PRIVATE_PROJECTION_PAYLOAD_WIRE_SCHEMA_VERSION_V1.to_be_bytes());
        bytes.extend_from_slice(
            &u16::try_from(PRIVATE_PROJECTION_PAYLOAD_WIRE_HEADER_LEN_V1)
                .expect("header length")
                .to_be_bytes(),
        );
        bytes.extend_from_slice(
            &u32::try_from(payload.len())
                .expect("payload length")
                .to_be_bytes(),
        );
        bytes.extend_from_slice(&validation.revision.to_be_bytes());
        bytes.extend_from_slice(&validation.turn_id);
        bytes.extend_from_slice(&validation.turn_binding);
        bytes.extend_from_slice(&validation.projection_digest);
        bytes.extend_from_slice(&certificate_digest);
        bytes.extend_from_slice(&binding_digest);
        bytes.extend_from_slice(&payload_digest);
        assert_eq!(bytes.len(), PRIVATE_PROJECTION_PAYLOAD_WIRE_HEADER_LEN_V1);
        bytes.extend_from_slice(&payload);
        let wire_digest = wire_digest_v1(&bytes);
        bytes.extend_from_slice(&wire_digest);
        bytes
    }

    #[test]
    fn semantic_payload_rejects_unknown_raw_and_numeric_array_values() {
        let valid = semantic_payload();
        validate_closed_payload_shape_v1(&valid).expect("closed fixture");
        validate_safe_payload_values_v1(&valid, None).expect("safe fixture");

        let mut unknown = valid.clone();
        unknown
            .as_object_mut()
            .expect("root object")
            .insert("unknown_field".to_owned(), Value::Bool(true));
        assert!(matches!(
            validate_private_projection_payload_wire_bytes_v1(&payload_wire(&unknown)),
            Err(PrivateProjectionPayloadWireErrorV1::UnsafePayloadShape)
        ));

        let mut raw_text = valid.clone();
        raw_text["turn"]["turn_ref"] = Value::String("raw user conversation".to_owned());
        assert!(matches!(
            validate_private_projection_payload_wire_bytes_v1(&payload_wire(&raw_text)),
            Err(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue)
        ));

        let mut numeric_array = valid;
        numeric_array["identity"]["operational_commitments"] = serde_json::json!([1, 2, 3]);
        assert!(matches!(
            validate_private_projection_payload_wire_bytes_v1(&payload_wire(&numeric_array)),
            Err(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue)
        ));

        let mut mismatched = semantic_payload();
        mismatched["identity"]["constitution_digest"] = Value::String(hex_digest(99));
        assert!(matches!(
            validate_private_projection_payload_wire_bytes_v1(&payload_wire(&mismatched)),
            Err(
                PrivateProjectionPayloadWireErrorV1::ProjectionBindingMismatch {
                    field: "identity_digest"
                }
            )
        ));
    }

    #[test]
    fn malformed_or_digest_mismatched_payload_frames_are_rejected() {
        let valid = payload_wire(&semantic_payload());
        validate_private_projection_payload_wire_bytes_v1(&valid).expect("valid frame");

        let mut bad_magic = valid.clone();
        bad_magic[0] ^= 1;
        assert!(matches!(
            validate_private_projection_payload_wire_bytes_v1(&bad_magic),
            Err(PrivateProjectionPayloadWireErrorV1::MalformedWire)
        ));

        let mut bad_length = valid.clone();
        bad_length[15] ^= 1;
        assert!(matches!(
            validate_private_projection_payload_wire_bytes_v1(&bad_length),
            Err(PrivateProjectionPayloadWireErrorV1::MalformedWire)
        ));

        let mut unbounded = valid.clone();
        unbounded[12..16].copy_from_slice(
            &u32::try_from(PRIVATE_PROJECTION_PAYLOAD_MAX_BYTES_V1 + 1)
                .expect("wire bound")
                .to_be_bytes(),
        );
        assert!(matches!(
            validate_private_projection_payload_wire_bytes_v1(&unbounded),
            Err(PrivateProjectionPayloadWireErrorV1::PayloadTooLarge)
        ));

        for (offset, field) in [
            (104, "certificate_digest"),
            (136, "binding_digest"),
            (168, "payload_digest"),
            (valid.len() - 32, "wire_digest"),
        ] {
            let mut corrupted = valid.clone();
            corrupted[offset] ^= 1;
            assert!(matches!(
                validate_private_projection_payload_wire_bytes_v1(&corrupted),
                Err(PrivateProjectionPayloadWireErrorV1::DigestMismatch { field: actual })
                    if actual == field
            ));
        }
    }
}

/// Runtime-private result retained only long enough to seal the legacy
/// producer's canonical payload wire.
struct PrivateCognitiveProjectionV1 {
    revision: u64,
    turn_id: Id128,
    turn_binding: Digest,
    envelope: CognitiveEnvelopeV1,
}

#[derive(Clone, Copy)]
struct PrivateProjectionWireValidationV1 {
    revision: u64,
    epistemic_revision: u64,
    turn_id: Id128,
    epistemic_turn_id: Id128,
    turn_binding: Digest,
    projection_digest: Digest,
    identity_digest: Digest,
    epistemic_identity_digest: Digest,
    source_state_digest: Digest,
    soma_state_digest: Digest,
    epistemic_state_digest: Digest,
    action_contract_digest: Digest,
    certificate_action_contract_digest: Digest,
    action_realization_digest: Digest,
    certificate_action_realization_digest: Digest,
    epistemic_projection_digest: Digest,
    certificate_epistemic_projection_digest: Digest,
}

fn validate_private_projection_for_wire_v1(
    validation: &PrivateProjectionWireValidationV1,
) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
    require_nonzero_id(&validation.turn_id, "turn_id")?;
    for (field, digest) in [
        ("turn_binding", &validation.turn_binding),
        ("projection_digest", &validation.projection_digest),
        ("identity_digest", &validation.identity_digest),
        ("source_state_digest", &validation.source_state_digest),
        ("soma_state_digest", &validation.soma_state_digest),
        ("action_contract_digest", &validation.action_contract_digest),
        (
            "action_realization_digest",
            &validation.action_realization_digest,
        ),
        (
            "epistemic_projection_digest",
            &validation.epistemic_projection_digest,
        ),
    ] {
        require_nonzero_digest(digest, field)?;
    }
    if validation.revision != validation.epistemic_revision {
        return Err(
            PrivateProjectionPayloadWireErrorV1::ProjectionBindingMismatch { field: "revision" },
        );
    }
    if validation.turn_id != validation.epistemic_turn_id {
        return Err(
            PrivateProjectionPayloadWireErrorV1::ProjectionBindingMismatch { field: "turn_id" },
        );
    }
    if validation.identity_digest != validation.epistemic_identity_digest {
        return Err(
            PrivateProjectionPayloadWireErrorV1::ProjectionBindingMismatch {
                field: "identity_digest",
            },
        );
    }
    // SOMA owns a distinct digest. Its explicit source-state binding is
    // validated by `ae-cognitive-envelope`; this transport layer must never
    // collapse the SOMA digest into the organism semantic-state digest.
    if validation.source_state_digest != validation.epistemic_state_digest {
        return Err(
            PrivateProjectionPayloadWireErrorV1::ProjectionBindingMismatch {
                field: "source_state_digest",
            },
        );
    }
    if validation.action_contract_digest != validation.certificate_action_contract_digest {
        return Err(
            PrivateProjectionPayloadWireErrorV1::ProjectionBindingMismatch {
                field: "action_contract_digest",
            },
        );
    }
    if validation.action_realization_digest != validation.certificate_action_realization_digest {
        return Err(
            PrivateProjectionPayloadWireErrorV1::ProjectionBindingMismatch {
                field: "action_realization_digest",
            },
        );
    }
    if validation.epistemic_projection_digest != validation.certificate_epistemic_projection_digest
    {
        return Err(
            PrivateProjectionPayloadWireErrorV1::ProjectionBindingMismatch {
                field: "epistemic_projection_digest",
            },
        );
    }
    Ok(())
}

fn require_nonzero_digest(
    digest: &Digest,
    field: &'static str,
) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
    if digest.iter().all(|byte| *byte == 0) {
        return Err(PrivateProjectionPayloadWireErrorV1::ZeroIdentity { field });
    }
    Ok(())
}

fn require_nonzero_id(
    id: &Id128,
    field: &'static str,
) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
    if id.iter().all(|byte| *byte == 0) {
        return Err(PrivateProjectionPayloadWireErrorV1::ZeroIdentity { field });
    }
    Ok(())
}

/// A complete, caller-provided typed ingress for one private projection
/// payload. The KV value is an opaque reference only: it must exactly match
/// the digest already bound by the typed cognitive-KV source capsule.
pub(crate) struct NativeProjectionPayloadProducerInputV1 {
    update: NativeProjectionUpdateV1,
    kv_snapshot_digest: Digest,
    morph_affordance_catalog: MorphAffordanceCatalogV1,
}

impl NativeProjectionPayloadProducerInputV1 {
    pub(crate) fn new(
        update: NativeProjectionUpdateV1,
        kv_snapshot_digest: Digest,
        morph_affordance_catalog: MorphAffordanceCatalogV1,
    ) -> Self {
        Self {
            update,
            kv_snapshot_digest,
            morph_affordance_catalog,
        }
    }
}

/// The producer accepts no fallback or default input. Callers that cannot
/// supply every typed source must state that condition explicitly.
pub(crate) enum NativeProjectionPayloadIngressV1 {
    Unavailable,
    Ready(Box<NativeProjectionPayloadProducerInputV1>),
}

impl NativeProjectionPayloadIngressV1 {
    pub(crate) fn unavailable() -> Self {
        Self::Unavailable
    }

    pub(crate) fn ready(input: NativeProjectionPayloadProducerInputV1) -> Self {
        Self::Ready(Box::new(input))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub(crate) enum NativeProjectionPayloadProducerErrorV1 {
    #[error("fully-bound native projection input is unavailable")]
    InputUnavailable,
    #[error("opaque KV snapshot digest does not match the typed cognitive-KV source")]
    KvSnapshotDigestMismatch,
    #[error("typed MORPH catalog does not bind to the update: {field}")]
    MorphBindingMismatch { field: &'static str },
    #[error("native projection validation or compilation failed: {0}")]
    Runtime(#[from] OrganismRuntimeErrorV1),
    #[error("private projection payload sealing failed: {0}")]
    Wire(#[from] PrivateProjectionPayloadWireErrorV1),
}

/// Legacy compatibility producer. Its watermark is independent of
/// `AstrRuntime`'s R7 semantic field/revision and advances only after a
/// canonical wire is successfully sealed.
pub(crate) struct NativeProjectionPayloadProducerV1 {
    runtime: OrganismProjectionRuntimeV1,
    last_issued_revision: Option<u64>,
}

impl NativeProjectionPayloadProducerV1 {
    pub(crate) fn new(
        identity: SourceCapsuleV1<IdentityConstitutionV1>,
    ) -> Result<Self, OrganismRuntimeErrorV1> {
        Ok(Self {
            runtime: OrganismProjectionRuntimeV1::new(identity)?,
            last_issued_revision: None,
        })
    }

    pub(crate) fn current_revision(&self) -> Option<u64> {
        self.last_issued_revision
    }

    pub(crate) fn produce(
        &mut self,
        ingress: NativeProjectionPayloadIngressV1,
    ) -> Result<PrivateProjectionPayloadWireV1, NativeProjectionPayloadProducerErrorV1> {
        let (incoming_revision, projection) = self.prepare_projection_for_issue_v1(ingress)?;
        let wire = seal_private_projection_payload_wire_v1(projection)?;
        self.last_issued_revision = Some(incoming_revision);
        Ok(wire)
    }

    /// Test-only transaction probe. It runs the same fully-bound ingress and
    /// projection preparation as `produce`, then fails at the private sealing
    /// boundary before a legacy issuance watermark can advance.
    #[cfg(test)]
    pub(crate) fn produce_with_test_only_sealing_failure_v1(
        &mut self,
        ingress: NativeProjectionPayloadIngressV1,
    ) -> Result<PrivateProjectionPayloadWireV1, NativeProjectionPayloadProducerErrorV1> {
        let (incoming_revision, projection) = self.prepare_projection_for_issue_v1(ingress)?;
        let wire = fail_test_only_legacy_projection_seal_v1(projection)?;
        self.last_issued_revision = Some(incoming_revision);
        Ok(wire)
    }

    fn prepare_projection_for_issue_v1(
        &self,
        ingress: NativeProjectionPayloadIngressV1,
    ) -> Result<(u64, PrivateCognitiveProjectionV1), NativeProjectionPayloadProducerErrorV1> {
        let input = match ingress {
            NativeProjectionPayloadIngressV1::Unavailable => {
                return Err(NativeProjectionPayloadProducerErrorV1::InputUnavailable)
            }
            NativeProjectionPayloadIngressV1::Ready(input) => *input,
        };
        self.validate_ingress(&input)?;
        let incoming_revision = input.update.revision;
        if let Some(current) = self.last_issued_revision {
            if incoming_revision <= current {
                return Err(NativeProjectionPayloadProducerErrorV1::Runtime(
                    OrganismRuntimeErrorV1::StaleOrReplayedRevision {
                        current,
                        incoming: incoming_revision,
                    },
                ));
            }
        }
        let projection = self.runtime.compile_uncommitted(input.update)?;
        Ok((incoming_revision, projection))
    }

    fn validate_ingress(
        &self,
        input: &NativeProjectionPayloadProducerInputV1,
    ) -> Result<(), NativeProjectionPayloadProducerErrorV1> {
        if input.kv_snapshot_digest
            != *input
                .update
                .references
                .cognitive_kv_view
                .value()
                .kv_snapshot_digest()
        {
            return Err(NativeProjectionPayloadProducerErrorV1::KvSnapshotDigestMismatch);
        }

        let morph = &input.morph_affordance_catalog;
        if morph.revision() != input.update.revision {
            return Err(
                NativeProjectionPayloadProducerErrorV1::MorphBindingMismatch { field: "revision" },
            );
        }
        if morph.identity_constitution_digest()
            != self.runtime.identity.value().constitution_digest()
        {
            return Err(
                NativeProjectionPayloadProducerErrorV1::MorphBindingMismatch {
                    field: "identity_constitution_digest",
                },
            );
        }
        if input.update.soma_state.value().source_state_digest()
            != Some(morph.source_state_digest())
        {
            return Err(
                NativeProjectionPayloadProducerErrorV1::MorphBindingMismatch {
                    field: "source_state_digest",
                },
            );
        }
        Ok(())
    }
}

#[cfg(test)]
fn fail_test_only_legacy_projection_seal_v1(
    _projection: PrivateCognitiveProjectionV1,
) -> Result<PrivateProjectionPayloadWireV1, PrivateProjectionPayloadWireErrorV1> {
    Err(PrivateProjectionPayloadWireErrorV1::PayloadEncodingInvalid)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PreOutputProjectionWireValidationV1 {
    revision: u64,
    epistemic_revision: u64,
    turn_id: Id128,
    epistemic_turn_id: Id128,
    turn_binding: Digest,
    projection_digest: Digest,
    identity_digest: Digest,
    epistemic_identity_digest: Digest,
    source_state_digest: Digest,
    soma_state_digest: Digest,
    epistemic_state_digest: Digest,
    action_contract_digest: Digest,
    certificate_action_contract_digest: Digest,
    epistemic_projection_digest: Digest,
    certificate_epistemic_projection_digest: Digest,
}

fn validate_pre_output_projection_for_wire_v1(
    validation: &PreOutputProjectionWireValidationV1,
) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
    for (field, digest) in [
        ("turn_binding", &validation.turn_binding),
        ("projection_digest", &validation.projection_digest),
        ("identity_digest", &validation.identity_digest),
        (
            "epistemic_identity_digest",
            &validation.epistemic_identity_digest,
        ),
        ("source_state_digest", &validation.source_state_digest),
        ("soma_state_digest", &validation.soma_state_digest),
        ("epistemic_state_digest", &validation.epistemic_state_digest),
        ("action_contract_digest", &validation.action_contract_digest),
        (
            "certificate_action_contract_digest",
            &validation.certificate_action_contract_digest,
        ),
        (
            "epistemic_projection_digest",
            &validation.epistemic_projection_digest,
        ),
        (
            "certificate_epistemic_projection_digest",
            &validation.certificate_epistemic_projection_digest,
        ),
    ] {
        require_payload_digest(digest, field)?;
    }
    if validation.turn_id.iter().all(|byte| *byte == 0)
        || validation.epistemic_turn_id.iter().all(|byte| *byte == 0)
    {
        return Err(PrivateProjectionPayloadWireErrorV1::ZeroIdentity { field: "turn_id" });
    }
    for (field, actual, expected) in [(
        "revision",
        validation.epistemic_revision,
        validation.revision,
    )] {
        if actual != expected {
            return Err(PrivateProjectionPayloadWireErrorV1::ProjectionBindingMismatch { field });
        }
    }
    for (field, actual, expected) in [
        (
            "turn_id",
            &validation.epistemic_turn_id[..],
            &validation.turn_id[..],
        ),
        (
            "identity_digest",
            &validation.epistemic_identity_digest[..],
            &validation.identity_digest[..],
        ),
        (
            "source_state_digest",
            &validation.epistemic_state_digest[..],
            &validation.source_state_digest[..],
        ),
        (
            "action_contract_digest",
            &validation.certificate_action_contract_digest[..],
            &validation.action_contract_digest[..],
        ),
        (
            "epistemic_projection_digest",
            &validation.certificate_epistemic_projection_digest[..],
            &validation.epistemic_projection_digest[..],
        ),
    ] {
        if actual != expected {
            return Err(PrivateProjectionPayloadWireErrorV1::ProjectionBindingMismatch { field });
        }
    }
    Ok(())
}

/// The only crate-private typed source bundle accepted by the atomic R7
/// transition. It contains no candidate, sealer, callback, wire, or raw
/// payload input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct R7PreOutputProjectionInputV1 {
    identity: SourceCapsuleV1<IdentityConstitutionV1>,
    update: PreOutputProjectionUpdateV1,
}

impl R7PreOutputProjectionInputV1 {
    pub(crate) fn new(
        identity: SourceCapsuleV1<IdentityConstitutionV1>,
        update: PreOutputProjectionUpdateV1,
    ) -> Result<Self, OrganismRuntimeErrorV1> {
        validate_r7_identity_v1(&identity)?;
        Ok(Self { identity, update })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub(crate) enum R7AtomicProjectionErrorV1 {
    #[error("typed pre-output input was rejected: {0}")]
    Runtime(#[from] OrganismRuntimeErrorV1),
    #[error("pre-output envelope compilation failed: {0}")]
    Projection(#[from] ProjectionErrorV1),
    #[error("private pre-output wire sealing failed: {0}")]
    Wire(#[from] PrivateProjectionPayloadWireErrorV1),
}

/// Runtime-private semantic facts required to bind a pre-output envelope to
/// the exact uncommitted field transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct R7SemanticProjectionBindingV1 {
    next_revision: u64,
    state_after: Digest,
    turn_id: Id128,
    scope_digest: Digest,
    _event_digest: Digest,
    _authority_digest: Digest,
    turn_binding: Digest,
}

impl R7SemanticProjectionBindingV1 {
    pub(crate) fn new(
        next_revision: u64,
        state_after: Digest,
        turn_id: Id128,
        scope_digest: Digest,
        event_digest: Digest,
        authority_digest: Digest,
        turn_binding: Digest,
    ) -> Self {
        Self {
            next_revision,
            state_after,
            turn_id,
            scope_digest,
            _event_digest: event_digest,
            _authority_digest: authority_digest,
            turn_binding,
        }
    }
}

pub(crate) fn compile_atomic_pre_output_wire_v1(
    binding: &R7SemanticProjectionBindingV1,
    input: &R7PreOutputProjectionInputV1,
) -> Result<PrivateProjectionPayloadWireV1, R7AtomicProjectionErrorV1> {
    validate_r7_identity_v1(&input.identity)?;
    validate_atomic_pre_output_update_v1(binding, &input.update, input.identity.value())?;
    let projection =
        compile_pre_output_projection_for_atomic_v1(input.identity.clone(), input.update.clone())?;
    seal_pre_output_private_projection_payload_wire_v1(projection).map_err(Into::into)
}

fn validate_r7_identity_v1(
    identity: &SourceCapsuleV1<IdentityConstitutionV1>,
) -> Result<(), OrganismRuntimeErrorV1> {
    let actual = identity.provenance().source_kind();
    if actual != ProjectionSourceKindV1::IdentityConstitution {
        return Err(OrganismRuntimeErrorV1::WrongIdentitySourceKind { actual });
    }
    if identity.content_digest() != identity.value().constitution_digest() {
        return Err(OrganismRuntimeErrorV1::IdentityConstitutionDigestMismatch);
    }
    Ok(())
}

fn validate_atomic_pre_output_update_v1(
    binding: &R7SemanticProjectionBindingV1,
    update: &PreOutputProjectionUpdateV1,
    identity: &IdentityConstitutionV1,
) -> Result<(), OrganismRuntimeErrorV1> {
    if update.revision != binding.next_revision {
        return Err(OrganismRuntimeErrorV1::PreparedTransitionRevisionMismatch);
    }
    if update.references.organism_snapshot.value().state_digest() != &binding.state_after {
        return Err(OrganismRuntimeErrorV1::PreparedTransitionStateBindingMismatch);
    }
    if update.references.organism_snapshot.value().turn_id() != &binding.turn_id {
        return Err(OrganismRuntimeErrorV1::PreparedTransitionTurnBindingMismatch);
    }
    for (field, actual) in [
        (
            "organism_snapshot.turn_binding",
            update.references.organism_snapshot.value().turn_binding(),
        ),
        (
            "action_contract.turn_binding",
            update.action_contract.value().turn_binding(),
        ),
    ] {
        if actual != &binding.turn_binding {
            return Err(OrganismRuntimeErrorV1::PreparedTransitionCausalBindingMismatch { field });
        }
    }
    let expected_scope_ref = projection_scope_ref_v1(&binding.scope_digest);
    for (field, actual) in [
        (
            "organism_snapshot.turn.scope_ref",
            update
                .references
                .organism_snapshot
                .value()
                .turn()
                .scope_ref(),
        ),
        (
            "relation_scope.scope_ref",
            update.references.relation_scope.value().scope_ref(),
        ),
        (
            "affordance_catalog.scope_ref",
            update.references.affordance_catalog.value().scope_ref(),
        ),
    ] {
        if actual != expected_scope_ref {
            return Err(OrganismRuntimeErrorV1::PreparedTransitionScopeBindingMismatch { field });
        }
    }
    validate_pre_output_update_bindings_v1(update, identity.constitution_digest())
}

fn projection_scope_ref_v1(scope_digest: &Digest) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity("native-scope-v1:".len() + 64);
    value.push_str("native-scope-v1:");
    for byte in scope_digest {
        value.push(char::from(HEX[usize::from(byte >> 4)]));
        value.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    value
}

fn compile_pre_output_projection_for_atomic_v1(
    identity: SourceCapsuleV1<IdentityConstitutionV1>,
    update: PreOutputProjectionUpdateV1,
) -> Result<PrivatePreOutputCognitiveProjectionV1, R7AtomicProjectionErrorV1> {
    let revision = update.revision;
    let PreOutputProjectionUpdateV1 {
        references,
        action_contract,
        soma_state,
        soma_classification_ingress,
        epistemic_projection,
        preconditions,
        ..
    } = update;
    let BoundedProjectionReferencesV1 {
        organism_snapshot,
        cognitive_kv_view,
        exact_turn_anchors,
        relation_scope,
        affordance_catalog,
        provider_profile,
    } = references;
    let turn_id = *organism_snapshot.value().turn_id();
    let turn_binding = *organism_snapshot.value().turn_binding();
    let input = PreOutputProjectionInputV1::new(
        organism_snapshot,
        cognitive_kv_view,
        exact_turn_anchors,
        identity,
        relation_scope,
        action_contract,
        affordance_catalog,
        provider_profile,
        soma_state,
        soma_classification_ingress,
        epistemic_projection,
    );
    let (envelope, _) = compile_pre_output_projection_v1(&input, &preconditions)?;
    Ok(PrivatePreOutputCognitiveProjectionV1 {
        revision,
        turn_id,
        turn_binding,
        envelope,
    })
}

fn validate_pre_output_update_bindings_v1(
    update: &PreOutputProjectionUpdateV1,
    identity_digest: &Digest,
) -> Result<(), OrganismRuntimeErrorV1> {
    for (field, actual) in [
        (
            "organism_snapshot",
            update
                .references
                .organism_snapshot
                .provenance()
                .source_revision(),
        ),
        (
            "cognitive_kv_view",
            update
                .references
                .cognitive_kv_view
                .provenance()
                .source_revision(),
        ),
        (
            "exact_turn_anchors",
            update
                .references
                .exact_turn_anchors
                .provenance()
                .source_revision(),
        ),
        (
            "relation_scope",
            update
                .references
                .relation_scope
                .provenance()
                .source_revision(),
        ),
        (
            "affordance_catalog",
            update
                .references
                .affordance_catalog
                .provenance()
                .source_revision(),
        ),
        (
            "provider_profile",
            update
                .references
                .provider_profile
                .provenance()
                .source_revision(),
        ),
        (
            "action_contract",
            update.action_contract.provenance().source_revision(),
        ),
        (
            "soma_state",
            update.soma_state.provenance().source_revision(),
        ),
        ("soma_state.value", update.soma_state.value().revision()),
        (
            "epistemic_projection",
            update.epistemic_projection.revision(),
        ),
    ] {
        if actual != update.revision {
            return Err(OrganismRuntimeErrorV1::RevisionBindingMismatch {
                field,
                expected: update.revision,
                actual,
            });
        }
    }
    if update.soma_state.value().identity_constitution_digest() != identity_digest {
        return Err(OrganismRuntimeErrorV1::IdentityBindingMismatch {
            field: "soma_state",
        });
    }
    if update.epistemic_projection.identity_digest() != identity_digest {
        return Err(OrganismRuntimeErrorV1::IdentityBindingMismatch {
            field: "epistemic_projection",
        });
    }
    if update
        .action_contract
        .value()
        .identity_constitution_digest()
        != identity_digest
    {
        return Err(OrganismRuntimeErrorV1::IdentityBindingMismatch {
            field: "action_contract",
        });
    }
    Ok(())
}

/// Seals a runtime-private legacy projection into its canonical payload wire.
/// No caller JSON or text participates in this operation.
fn seal_private_projection_payload_wire_v1(
    projection: PrivateCognitiveProjectionV1,
) -> Result<PrivateProjectionPayloadWireV1, PrivateProjectionPayloadWireErrorV1> {
    let envelope = &projection.envelope;
    let certificate = envelope.projection_certificate();
    let validation = PrivateProjectionWireValidationV1 {
        revision: projection.revision,
        epistemic_revision: envelope.epistemics().revision(),
        turn_id: projection.turn_id,
        epistemic_turn_id: *envelope.epistemics().turn_id(),
        turn_binding: projection.turn_binding,
        projection_digest: *envelope.envelope_digest(),
        identity_digest: *envelope.identity().constitution_digest(),
        epistemic_identity_digest: *envelope.epistemics().identity_digest(),
        source_state_digest: *certificate.source_state_digest(),
        soma_state_digest: *certificate.soma_state_digest(),
        epistemic_state_digest: *envelope.epistemics().state_digest(),
        action_contract_digest: *envelope.action_contract().contract_digest(),
        certificate_action_contract_digest: *certificate.action_contract_digest(),
        action_realization_digest: *envelope.realization().realization_digest(),
        certificate_action_realization_digest: *certificate.action_realization_digest(),
        epistemic_projection_digest: *envelope.epistemics().projection_digest(),
        certificate_epistemic_projection_digest: *certificate.epistemic_projection_digest(),
    };
    validate_private_projection_for_wire_v1(&validation)?;

    let efference_copy_digest = *certificate.efference_copy_digest();
    require_payload_digest(&efference_copy_digest, "efference_copy_digest")?;
    let mut body = serde_json::to_value(envelope)
        .map_err(|_| PrivateProjectionPayloadWireErrorV1::PayloadEncodingInvalid)?;
    normalize_payload_digests_v1(&mut body, None)?;
    validate_closed_payload_shape_v1(&body)?;
    validate_safe_payload_values_v1(&body, None)?;
    validate_payload_bindings_v1(&body, &validation, &efference_copy_digest)?;
    let binding_digest = projection_payload_binding_digest_v1(&validation, &efference_copy_digest);
    let metadata = PrivateProjectionPayloadWireBindingMetadataV1::new(
        projection.revision,
        projection.turn_id,
        projection.turn_binding,
        validation.projection_digest,
        validation.source_state_digest,
    )?;
    seal_cognitive_envelope_v1(metadata, envelope, binding_digest)
}

/// Seals only the typed state available before provider output. The shared
/// `AER7PPW1` transport framing remains one-shot; the closed payload schema
/// explicitly identifies the pre-output phase and carries no realization or
/// efference-copy placeholders.
fn seal_pre_output_private_projection_payload_wire_v1(
    projection: PrivatePreOutputCognitiveProjectionV1,
) -> Result<PrivateProjectionPayloadWireV1, PrivateProjectionPayloadWireErrorV1> {
    let envelope = &projection.envelope;
    let certificate = envelope.projection_certificate();
    let validation = PreOutputProjectionWireValidationV1 {
        revision: projection.revision,
        epistemic_revision: envelope.epistemics().revision(),
        turn_id: projection.turn_id,
        epistemic_turn_id: *envelope.epistemics().turn_id(),
        turn_binding: projection.turn_binding,
        projection_digest: *envelope.envelope_digest(),
        identity_digest: *envelope.identity().constitution_digest(),
        epistemic_identity_digest: *envelope.epistemics().identity_digest(),
        source_state_digest: *certificate.source_state_digest(),
        soma_state_digest: *certificate.soma_state_digest(),
        epistemic_state_digest: *envelope.epistemics().state_digest(),
        action_contract_digest: *envelope.action_contract().contract_digest(),
        certificate_action_contract_digest: *certificate.action_contract_digest(),
        epistemic_projection_digest: *envelope.epistemics().projection_digest(),
        certificate_epistemic_projection_digest: *certificate.epistemic_projection_digest(),
    };
    validate_pre_output_projection_for_wire_v1(&validation)?;

    let mut body = serde_json::to_value(envelope)
        .map_err(|_| PrivateProjectionPayloadWireErrorV1::PayloadEncodingInvalid)?;
    normalize_payload_digests_v1(&mut body, None)?;
    validate_pre_output_payload_shape_v1(&body)?;
    validate_safe_payload_values_v1(&body, None)?;
    validate_pre_output_payload_bindings_v1(&body, &validation)?;
    let binding_digest = pre_output_projection_payload_binding_digest_v1(&validation);
    let metadata = PrivateProjectionPayloadWireBindingMetadataV1::new(
        projection.revision,
        projection.turn_id,
        projection.turn_binding,
        validation.projection_digest,
        validation.source_state_digest,
    )?;
    seal_pre_output_cognitive_envelope_v1(metadata, envelope, binding_digest)
}

fn pre_output_projection_payload_binding_digest_v1(
    validation: &PreOutputProjectionWireValidationV1,
) -> Digest {
    let revision = validation.revision.to_be_bytes();
    wire::domain_hash(
        b"astr-embodiment/r7/private-projection-payload/pre-output-binding-v1",
        &[
            &revision,
            &validation.turn_id,
            &validation.turn_binding,
            &validation.projection_digest,
            &validation.identity_digest,
            &validation.source_state_digest,
            &validation.action_contract_digest,
            &validation.epistemic_projection_digest,
        ],
    )
}

fn projection_payload_binding_digest_v1(
    validation: &PrivateProjectionWireValidationV1,
    efference_copy_digest: &Digest,
) -> Digest {
    let revision = validation.revision.to_be_bytes();
    wire::domain_hash(
        PRIVATE_PROJECTION_PAYLOAD_BINDING_DOMAIN_V1,
        &[
            &revision,
            &validation.turn_id,
            &validation.turn_binding,
            &validation.projection_digest,
            &validation.identity_digest,
            &validation.source_state_digest,
            &validation.action_contract_digest,
            &validation.action_realization_digest,
            efference_copy_digest,
        ],
    )
}

fn normalize_payload_digests_v1(
    value: &mut Value,
    field: Option<&str>,
) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
    if field == Some("included_capsule_digests") {
        let values = value
            .as_array_mut()
            .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadShape)?;
        if values.len() != ProjectionInput::FIELD_COUNT {
            return Err(PrivateProjectionPayloadWireErrorV1::UnsafePayloadShape);
        }
        for digest in values {
            normalize_one_digest_v1(digest)?;
        }
        return Ok(());
    }
    if field.is_some_and(|name| name.ends_with("_digest")) {
        return normalize_one_digest_v1(value);
    }
    match value {
        Value::Object(object) => {
            for (name, nested) in object {
                normalize_payload_digests_v1(nested, Some(name))?;
            }
        }
        Value::Array(values) => {
            for nested in values {
                normalize_payload_digests_v1(nested, field)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn normalize_one_digest_v1(value: &mut Value) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
    if let Value::String(encoded) = value {
        decode_hex_v1::<32>(encoded)
            .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue)?;
        return Ok(());
    }
    let bytes = value
        .as_array()
        .filter(|values| values.len() == 32)
        .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue)?
        .iter()
        .map(|item| item.as_u64().and_then(|part| u8::try_from(part).ok()))
        .collect::<Option<Vec<_>>>()
        .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue)?;
    *value = Value::String(encode_hex_v1(&bytes));
    Ok(())
}

fn validate_closed_payload_shape_v1(
    value: &Value,
) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
    let root = require_exact_object_v1(
        value,
        &[
            "affordances",
            "agency",
            "epistemics",
            "exact_anchors",
            "identity",
            "praxis",
            "projection_certificate",
            "realization",
            "relation",
            "schema",
            "subjective_present",
            "turn",
        ],
    )?;
    require_exact_string_v1(root.get("schema"), "astrembodiment.cognitive-envelope.v1")?;
    require_exact_object_v1(
        required_field_v1(root, "turn")?,
        &["incarnation_ref", "scope_ref", "turn_ref"],
    )?;
    require_exact_object_v1(
        required_field_v1(root, "identity")?,
        &[
            "anti_goals",
            "constitution_digest",
            "correction_boundary_constitution",
            "expression_basis",
            "incarnation_digest",
            "operational_commitments",
            "persona_revision_digest",
            "relational_play_limits",
        ],
    )?;

    for item in require_array_v1(required_field_v1(root, "subjective_present")?, Some(32))? {
        let subjective = require_exact_object_v1(
            item,
            &[
                "axis",
                "band",
                "behavioral_effect",
                "cause_ref",
                "confidence",
                "disclosure",
                "trend",
            ],
        )?;
        require_one_of_v1(
            subjective.get("band"),
            &["very_low", "low", "moderate", "high", "very_high"],
        )?;
        require_one_of_v1(
            subjective.get("trend"),
            &["falling_fast", "falling", "stable", "rising", "rising_fast"],
        )?;
        require_one_of_v1(
            subjective.get("disclosure"),
            &[
                "PRIVATE_CONTROL",
                "BEHAVIORAL_ONLY",
                "IMPLICIT_ALLOWED",
                "EXPLICIT_ALLOWED",
                "REQUIRED",
            ],
        )?;
        require_one_of_v1(subjective.get("confidence"), &["low", "moderate", "high"])?;
    }

    validate_field_object_v1(required_field_v1(root, "relation")?)?;
    validate_field_object_v1(required_field_v1(root, "praxis")?)?;
    require_exact_object_v1(
        required_field_v1(root, "epistemics")?,
        &[
            "claim_under_challenge",
            "classification_is_caller_provided",
            "identity_digest",
            "projection_digest",
            "revision",
            "source_estimate_digest",
            "state_digest",
            "turn_id",
        ],
    )?;

    for item in require_array_v1(required_field_v1(root, "affordances")?, Some(64))? {
        require_exact_object_v1(
            item,
            &[
                "affordance_id",
                "authority_evidence_digest",
                "description",
                "policy_evidence_digest",
            ],
        )?;
    }

    let agency = require_exact_object_v1(
        required_field_v1(root, "agency")?,
        &[
            "action_contract",
            "action_contract_digest",
            "efference_copy_digest",
        ],
    )?;
    let action_contract = require_exact_object_v1(
        required_field_v1(agency, "action_contract")?,
        &[
            "action_id",
            "allowed_disclosures",
            "allowed_tools",
            "base_revision",
            "confidence_ceiling",
            "contract_digest",
            "disposition",
            "expires_at_ms",
            "identity_constitution_digest",
            "requirements",
            "schema",
            "source_state_digest",
            "speech_act",
            "turn_binding",
        ],
    )?;
    require_exact_string_v1(
        action_contract.get("schema"),
        "astrembodiment.action-contract.v1",
    )?;
    require_one_of_v1(
        action_contract.get("disposition"),
        &["silence", "speech", "tool_plan", "speech_and_tool_plan"],
    )?;
    require_exact_object_v1(
        required_field_v1(action_contract, "requirements")?,
        &["may", "must", "must_not", "should"],
    )?;

    let realization = require_exact_object_v1(
        required_field_v1(root, "realization")?,
        &[
            "action_id",
            "contract_digest",
            "disclosures_used",
            "manifest_confidence",
            "owned_claims",
            "proposed_tools",
            "schema",
            "speech_act",
        ],
    )?;
    require_exact_string_v1(
        realization.get("schema"),
        "astrembodiment.action-realization.v1",
    )?;
    for item in require_array_v1(required_field_v1(realization, "owned_claims")?, Some(64))? {
        require_exact_object_v1(
            item,
            &[
                "assertiveness",
                "claim_digest",
                "confidence",
                "span_ref",
                "stakes",
                "verifiable",
            ],
        )?;
    }
    for item in require_array_v1(required_field_v1(realization, "proposed_tools")?, Some(32))? {
        require_exact_object_v1(item, &["arguments_digest", "tool_id"])?;
    }
    for item in require_array_v1(
        required_field_v1(realization, "disclosures_used")?,
        Some(32),
    )? {
        require_exact_object_v1(item, &["disclosure_id", "source_digest"])?;
    }

    for item in require_array_v1(required_field_v1(root, "exact_anchors")?, Some(64))? {
        let anchor = require_exact_object_v1(
            item,
            &["anchor_ref", "exact_content", "kind", "source_digest"],
        )?;
        require_one_of_v1(
            anchor.get("kind"),
            &[
                "confirmed_error_or_corrected_fact",
                "explicit_user_boundary_or_consent",
                "active_safety_requirement",
                "active_commitment_or_completion_obligation",
                "challenged_claim_or_action",
                "required_tool_or_delivery_fact",
                "must_state",
                "must_not_state",
                "incarnation_or_persona_binding",
            ],
        )?;
    }

    require_exact_object_v1(
        required_field_v1(root, "projection_certificate")?,
        &[
            "action_contract_digest",
            "action_realization_digest",
            "action_sensitivity_bound",
            "action_sensitivity_residual",
            "disclosure_residual",
            "efference_copy_digest",
            "epistemic_projection_digest",
            "exact_anchor_residual",
            "included_capsule_digests",
            "kv_snapshot_digest",
            "provider_profile_digest",
            "scope_residual",
            "soma_state_digest",
            "source_state_digest",
            "subjective_present_digest",
            "token_budget_used",
        ],
    )?;
    Ok(())
}

fn validate_pre_output_payload_shape_v1(
    value: &Value,
) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
    let root = require_exact_object_v1(
        value,
        &[
            "affordances",
            "agency",
            "epistemics",
            "exact_anchors",
            "identity",
            "praxis",
            "projection_certificate",
            "relation",
            "schema",
            "subjective_present",
            "turn",
        ],
    )?;
    require_exact_string_v1(
        root.get("schema"),
        "astrembodiment.cognitive-envelope.pre-output.v1",
    )?;
    require_exact_object_v1(
        required_field_v1(root, "turn")?,
        &["incarnation_ref", "scope_ref", "turn_ref"],
    )?;
    require_exact_object_v1(
        required_field_v1(root, "identity")?,
        &[
            "anti_goals",
            "constitution_digest",
            "correction_boundary_constitution",
            "expression_basis",
            "incarnation_digest",
            "operational_commitments",
            "persona_revision_digest",
            "relational_play_limits",
        ],
    )?;
    for item in require_array_v1(required_field_v1(root, "subjective_present")?, Some(32))? {
        require_exact_object_v1(
            item,
            &[
                "axis",
                "band",
                "behavioral_effect",
                "cause_ref",
                "confidence",
                "disclosure",
                "trend",
            ],
        )?;
    }
    validate_field_object_v1(required_field_v1(root, "relation")?)?;
    validate_field_object_v1(required_field_v1(root, "praxis")?)?;
    require_exact_object_v1(
        required_field_v1(root, "epistemics")?,
        &[
            "claim_under_challenge",
            "classification_is_caller_provided",
            "identity_digest",
            "projection_digest",
            "revision",
            "source_estimate_digest",
            "state_digest",
            "turn_id",
        ],
    )?;
    for item in require_array_v1(required_field_v1(root, "affordances")?, Some(64))? {
        require_exact_object_v1(
            item,
            &[
                "affordance_id",
                "authority_evidence_digest",
                "description",
                "policy_evidence_digest",
            ],
        )?;
    }
    let agency = require_exact_object_v1(
        required_field_v1(root, "agency")?,
        &["action_contract", "action_contract_digest"],
    )?;
    let action_contract = require_exact_object_v1(
        required_field_v1(agency, "action_contract")?,
        &[
            "action_id",
            "allowed_disclosures",
            "allowed_tools",
            "base_revision",
            "confidence_ceiling",
            "contract_digest",
            "disposition",
            "expires_at_ms",
            "identity_constitution_digest",
            "requirements",
            "schema",
            "source_state_digest",
            "speech_act",
            "turn_binding",
        ],
    )?;
    require_exact_string_v1(
        action_contract.get("schema"),
        "astrembodiment.action-contract.v1",
    )?;
    require_exact_object_v1(
        required_field_v1(action_contract, "requirements")?,
        &["may", "must", "must_not", "should"],
    )?;
    for item in require_array_v1(required_field_v1(root, "exact_anchors")?, Some(64))? {
        require_exact_object_v1(
            item,
            &["anchor_ref", "exact_content", "kind", "source_digest"],
        )?;
    }
    require_exact_object_v1(
        required_field_v1(root, "projection_certificate")?,
        &[
            "action_contract_digest",
            "action_sensitivity_bound",
            "action_sensitivity_residual",
            "disclosure_residual",
            "epistemic_projection_digest",
            "exact_anchor_residual",
            "included_capsule_digests",
            "kv_snapshot_digest",
            "provider_profile_digest",
            "scope_residual",
            "soma_state_digest",
            "source_state_digest",
            "subjective_present_digest",
            "token_budget_used",
        ],
    )?;
    Ok(())
}

fn validate_field_object_v1(value: &Value) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
    let object = require_exact_object_v1(value, &["fields"])?;
    for item in require_array_v1(required_field_v1(object, "fields")?, None)? {
        require_exact_object_v1(item, &["name", "source_digest", "value"])?;
    }
    Ok(())
}

fn require_exact_object_v1<'a>(
    value: &'a Value,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, PrivateProjectionPayloadWireErrorV1> {
    let object = value
        .as_object()
        .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadShape)?;
    if object.len() != keys.len()
        || keys.iter().any(|key| !object.contains_key(*key))
        || object.keys().any(|key| !keys.contains(&key.as_str()))
    {
        return Err(PrivateProjectionPayloadWireErrorV1::UnsafePayloadShape);
    }
    Ok(object)
}

fn required_field_v1<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a Value, PrivateProjectionPayloadWireErrorV1> {
    object
        .get(key)
        .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadShape)
}

fn require_array_v1(
    value: &Value,
    max_items: Option<usize>,
) -> Result<&[Value], PrivateProjectionPayloadWireErrorV1> {
    let values = value
        .as_array()
        .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadShape)?;
    if max_items.is_some_and(|max| values.len() > max) {
        return Err(PrivateProjectionPayloadWireErrorV1::UnsafePayloadShape);
    }
    Ok(values)
}

fn require_exact_string_v1(
    value: Option<&Value>,
    expected: &str,
) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
    if value.and_then(Value::as_str) != Some(expected) {
        return Err(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue);
    }
    Ok(())
}

fn require_one_of_v1(
    value: Option<&Value>,
    allowed: &[&str],
) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
    let value = value
        .and_then(Value::as_str)
        .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue)?;
    if !allowed.contains(&value) {
        return Err(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue);
    }
    Ok(())
}

fn validate_safe_payload_values_v1(
    value: &Value,
    field: Option<&str>,
) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
    match value {
        Value::Null => {
            if !matches!(
                field,
                Some("cause_ref" | "claim_under_challenge" | "span_ref")
            ) {
                return Err(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue);
            }
        }
        Value::Bool(_) => {
            if !matches!(
                field,
                Some("classification_is_caller_provided" | "verifiable")
            ) {
                return Err(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue);
            }
        }
        Value::Number(number) => {
            if matches!(
                field,
                Some(
                    "assertiveness"
                        | "confidence"
                        | "confidence_ceiling"
                        | "manifest_confidence"
                        | "stakes"
                )
            ) {
                let value = number
                    .as_f64()
                    .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
                    .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue)?;
                let _ = value;
            } else if number.as_u64().is_none() {
                return Err(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue);
            }
        }
        Value::String(text) => {
            if field.is_some_and(|name| name.ends_with("_digest"))
                || field == Some("included_capsule_digests")
            {
                let digest = decode_hex_v1::<32>(text)
                    .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue)?;
                require_payload_digest(&digest, "payload_digest_value")?;
            } else if matches!(
                field,
                Some("action_id" | "claim_under_challenge" | "turn_id")
            ) {
                let id = decode_hex_v1::<16>(text)
                    .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue)?;
                if id.iter().all(|byte| *byte == 0) {
                    return Err(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue);
                }
            } else if field == Some("turn_binding") {
                decode_hex_v1::<32>(text)
                    .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue)?;
            } else if !is_safe_semantic_token_v1(text) {
                return Err(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue);
            }
            if contains_prohibited_semantic_fragment_v1(text) {
                return Err(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue);
            }
        }
        Value::Array(values) => {
            if field == Some("included_capsule_digests")
                && values.len() != ProjectionInput::FIELD_COUNT
            {
                return Err(PrivateProjectionPayloadWireErrorV1::UnsafePayloadShape);
            }
            for nested in values {
                if nested.is_number() {
                    return Err(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue);
                }
                validate_safe_payload_values_v1(nested, field)?;
            }
        }
        Value::Object(object) => {
            for (name, nested) in object {
                validate_safe_payload_values_v1(nested, Some(name))?;
            }
        }
    }
    Ok(())
}

fn is_safe_semantic_token_v1(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    if matches!(
        value,
        "PRIVATE_CONTROL"
            | "BEHAVIORAL_ONLY"
            | "IMPLICIT_ALLOWED"
            | "EXPLICIT_ALLOWED"
            | "REQUIRED"
    ) {
        return true;
    }
    let mut previous_was_separator = true;
    for byte in value.bytes() {
        if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
            previous_was_separator = false;
        } else if matches!(byte, b'_' | b'-' | b'.' | b':') && !previous_was_separator {
            previous_was_separator = true;
        } else {
            return false;
        }
    }
    !previous_was_separator
}

fn contains_prohibited_semantic_fragment_v1(value: &str) -> bool {
    let normalized = value.to_ascii_lowercase();
    [
        "raw_persona",
        "persona_prompt",
        "raw_user",
        "user_conversation",
        "raw_provider",
        "provider_payload",
        "raw_evidence",
        "continuum_kv",
        "neural_array",
        "kv_array",
        "effect_payload",
        "public_text",
        "visible_text",
        "error_string",
    ]
    .iter()
    .any(|fragment| normalized.contains(fragment))
}

fn validate_payload_bindings_v1(
    value: &Value,
    validation: &PrivateProjectionWireValidationV1,
    expected_efference_copy_digest: &Digest,
) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
    let root = value
        .as_object()
        .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadShape)?;
    let identity = object_field_v1(root, "identity")?;
    let epistemics = object_field_v1(root, "epistemics")?;
    let agency = object_field_v1(root, "agency")?;
    let action_contract = object_field_v1(agency, "action_contract")?;
    let realization = object_field_v1(root, "realization")?;
    let certificate = object_field_v1(root, "projection_certificate")?;

    require_equal_digest_v1(
        digest_field_v1(identity, "constitution_digest")?,
        validation.identity_digest,
        "identity_digest",
    )?;
    require_equal_digest_v1(
        digest_field_v1(epistemics, "identity_digest")?,
        validation.epistemic_identity_digest,
        "epistemic_identity_digest",
    )?;
    require_equal_digest_v1(
        digest_field_v1(epistemics, "state_digest")?,
        validation.epistemic_state_digest,
        "epistemic_state_digest",
    )?;
    require_equal_digest_v1(
        digest_field_v1(epistemics, "projection_digest")?,
        validation.epistemic_projection_digest,
        "epistemic_projection_digest",
    )?;
    require_equal_id_v1(
        id_field_v1(epistemics, "turn_id")?,
        validation.epistemic_turn_id,
        "turn_id",
    )?;
    require_equal_u64_v1(
        u64_field_v1(epistemics, "revision")?,
        validation.epistemic_revision,
        "revision",
    )?;

    require_equal_digest_v1(
        digest_field_v1(action_contract, "turn_binding")?,
        validation.turn_binding,
        "turn_binding",
    )?;
    require_equal_u64_v1(
        u64_field_v1(action_contract, "base_revision")?,
        validation.revision,
        "base_revision",
    )?;
    require_equal_digest_v1(
        digest_field_v1(action_contract, "source_state_digest")?,
        validation.source_state_digest,
        "source_state_digest",
    )?;
    require_equal_digest_v1(
        digest_field_v1(action_contract, "identity_constitution_digest")?,
        validation.identity_digest,
        "identity_constitution_digest",
    )?;
    require_equal_digest_v1(
        digest_field_v1(action_contract, "contract_digest")?,
        validation.action_contract_digest,
        "action_contract_digest",
    )?;
    require_equal_digest_v1(
        digest_field_v1(agency, "action_contract_digest")?,
        validation.action_contract_digest,
        "agency.action_contract_digest",
    )?;
    require_equal_digest_v1(
        digest_field_v1(agency, "efference_copy_digest")?,
        *expected_efference_copy_digest,
        "agency.efference_copy_digest",
    )?;

    if string_field_v1(realization, "action_id")? != string_field_v1(action_contract, "action_id")?
    {
        return Err(
            PrivateProjectionPayloadWireErrorV1::ProjectionBindingMismatch {
                field: "realization.action_id",
            },
        );
    }
    require_equal_digest_v1(
        digest_field_v1(realization, "contract_digest")?,
        validation.action_contract_digest,
        "realization.contract_digest",
    )?;

    require_equal_digest_v1(
        digest_field_v1(certificate, "source_state_digest")?,
        validation.source_state_digest,
        "certificate.source_state_digest",
    )?;
    require_equal_digest_v1(
        digest_field_v1(certificate, "soma_state_digest")?,
        validation.soma_state_digest,
        "certificate.soma_state_digest",
    )?;
    require_equal_digest_v1(
        digest_field_v1(certificate, "epistemic_projection_digest")?,
        validation.epistemic_projection_digest,
        "certificate.epistemic_projection_digest",
    )?;
    require_equal_digest_v1(
        digest_field_v1(certificate, "action_contract_digest")?,
        validation.action_contract_digest,
        "certificate.action_contract_digest",
    )?;
    require_equal_digest_v1(
        digest_field_v1(certificate, "action_realization_digest")?,
        validation.action_realization_digest,
        "certificate.action_realization_digest",
    )?;
    require_equal_digest_v1(
        digest_field_v1(certificate, "efference_copy_digest")?,
        *expected_efference_copy_digest,
        "certificate.efference_copy_digest",
    )?;

    for field in [
        "subjective_present_digest",
        "kv_snapshot_digest",
        "provider_profile_digest",
    ] {
        let _ = digest_field_v1(certificate, field)?;
    }
    let included = certificate
        .get("included_capsule_digests")
        .and_then(Value::as_array)
        .filter(|digests| digests.len() == ProjectionInput::FIELD_COUNT)
        .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadShape)?;
    for digest in included {
        let encoded = digest
            .as_str()
            .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue)?;
        require_payload_digest(
            &decode_hex_v1::<32>(encoded)
                .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue)?,
            "included_capsule_digest",
        )?;
    }
    let exact_anchor_residual = u64_field_v1(certificate, "exact_anchor_residual")?;
    let scope_residual = u64_field_v1(certificate, "scope_residual")?;
    let action_sensitivity_residual = u64_field_v1(certificate, "action_sensitivity_residual")?;
    let action_sensitivity_bound = u64_field_v1(certificate, "action_sensitivity_bound")?;
    let disclosure_residual = u64_field_v1(certificate, "disclosure_residual")?;
    let token_budget_used = u64_field_v1(certificate, "token_budget_used")?;
    if exact_anchor_residual != 0
        || scope_residual != 0
        || disclosure_residual != 0
        || action_sensitivity_residual > action_sensitivity_bound
        || token_budget_used > 3_200
    {
        return Err(
            PrivateProjectionPayloadWireErrorV1::ProjectionBindingMismatch {
                field: "projection_certificate",
            },
        );
    }
    Ok(())
}

fn validate_pre_output_payload_bindings_v1(
    value: &Value,
    validation: &PreOutputProjectionWireValidationV1,
) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
    let root = value
        .as_object()
        .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadShape)?;
    let identity = object_field_v1(root, "identity")?;
    let epistemics = object_field_v1(root, "epistemics")?;
    let agency = object_field_v1(root, "agency")?;
    let action_contract = object_field_v1(agency, "action_contract")?;
    let certificate = object_field_v1(root, "projection_certificate")?;
    for (field, actual, expected) in [
        (
            "identity_digest",
            digest_field_v1(identity, "constitution_digest")?,
            validation.identity_digest,
        ),
        (
            "epistemic_identity_digest",
            digest_field_v1(epistemics, "identity_digest")?,
            validation.epistemic_identity_digest,
        ),
        (
            "epistemic_state_digest",
            digest_field_v1(epistemics, "state_digest")?,
            validation.epistemic_state_digest,
        ),
        (
            "epistemic_projection_digest",
            digest_field_v1(epistemics, "projection_digest")?,
            validation.epistemic_projection_digest,
        ),
        (
            "source_state_digest",
            digest_field_v1(action_contract, "source_state_digest")?,
            validation.source_state_digest,
        ),
        (
            "action_contract_digest",
            digest_field_v1(action_contract, "contract_digest")?,
            validation.action_contract_digest,
        ),
        (
            "agency.action_contract_digest",
            digest_field_v1(agency, "action_contract_digest")?,
            validation.action_contract_digest,
        ),
        (
            "certificate.source_state_digest",
            digest_field_v1(certificate, "source_state_digest")?,
            validation.source_state_digest,
        ),
        (
            "certificate.soma_state_digest",
            digest_field_v1(certificate, "soma_state_digest")?,
            validation.soma_state_digest,
        ),
        (
            "certificate.epistemic_projection_digest",
            digest_field_v1(certificate, "epistemic_projection_digest")?,
            validation.epistemic_projection_digest,
        ),
        (
            "certificate.action_contract_digest",
            digest_field_v1(certificate, "action_contract_digest")?,
            validation.action_contract_digest,
        ),
    ] {
        require_equal_digest_v1(actual, expected, field)?;
    }
    require_equal_id_v1(
        id_field_v1(epistemics, "turn_id")?,
        validation.epistemic_turn_id,
        "turn_id",
    )?;
    require_equal_u64_v1(
        u64_field_v1(epistemics, "revision")?,
        validation.epistemic_revision,
        "revision",
    )?;
    require_equal_digest_v1(
        digest_field_v1(action_contract, "turn_binding")?,
        validation.turn_binding,
        "turn_binding",
    )?;
    require_equal_u64_v1(
        u64_field_v1(action_contract, "base_revision")?,
        validation.revision,
        "base_revision",
    )?;
    require_equal_digest_v1(
        digest_field_v1(action_contract, "identity_constitution_digest")?,
        validation.identity_digest,
        "identity_constitution_digest",
    )?;
    let included = certificate
        .get("included_capsule_digests")
        .and_then(Value::as_array)
        .filter(|digests| digests.len() == PreOutputProjectionInputV1::FIELD_COUNT)
        .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadShape)?;
    for digest in included {
        require_payload_digest(
            &decode_hex_v1::<32>(
                digest
                    .as_str()
                    .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue)?,
            )
            .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue)?,
            "included_capsule_digest",
        )?;
    }
    let exact_anchor_residual = u64_field_v1(certificate, "exact_anchor_residual")?;
    let scope_residual = u64_field_v1(certificate, "scope_residual")?;
    let action_sensitivity_residual = u64_field_v1(certificate, "action_sensitivity_residual")?;
    let action_sensitivity_bound = u64_field_v1(certificate, "action_sensitivity_bound")?;
    let disclosure_residual = u64_field_v1(certificate, "disclosure_residual")?;
    let token_budget_used = u64_field_v1(certificate, "token_budget_used")?;
    if exact_anchor_residual != 0
        || scope_residual != 0
        || disclosure_residual != 0
        || action_sensitivity_residual > action_sensitivity_bound
        || token_budget_used > 3_200
    {
        return Err(
            PrivateProjectionPayloadWireErrorV1::ProjectionBindingMismatch {
                field: "projection_certificate",
            },
        );
    }
    Ok(())
}

fn object_field_v1<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a Map<String, Value>, PrivateProjectionPayloadWireErrorV1> {
    object
        .get(field)
        .and_then(Value::as_object)
        .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadShape)
}

fn string_field_v1<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a str, PrivateProjectionPayloadWireErrorV1> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue)
}

fn digest_field_v1(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Digest, PrivateProjectionPayloadWireErrorV1> {
    let digest = decode_hex_v1::<32>(string_field_v1(object, field)?)
        .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue)?;
    require_payload_digest(&digest, field)?;
    Ok(digest)
}

fn id_field_v1(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Id128, PrivateProjectionPayloadWireErrorV1> {
    let id = decode_hex_v1::<16>(string_field_v1(object, field)?)
        .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue)?;
    if id.iter().all(|byte| *byte == 0) {
        return Err(PrivateProjectionPayloadWireErrorV1::ZeroIdentity { field });
    }
    Ok(id)
}

fn u64_field_v1(
    object: &Map<String, Value>,
    field: &str,
) -> Result<u64, PrivateProjectionPayloadWireErrorV1> {
    object
        .get(field)
        .and_then(Value::as_u64)
        .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue)
}

fn require_equal_digest_v1(
    actual: Digest,
    expected: Digest,
    field: &'static str,
) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
    if actual != expected {
        return Err(PrivateProjectionPayloadWireErrorV1::ProjectionBindingMismatch { field });
    }
    Ok(())
}

fn require_equal_id_v1(
    actual: Id128,
    expected: Id128,
    field: &'static str,
) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
    if actual != expected {
        return Err(PrivateProjectionPayloadWireErrorV1::ProjectionBindingMismatch { field });
    }
    Ok(())
}

fn require_equal_u64_v1(
    actual: u64,
    expected: u64,
    field: &'static str,
) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
    if actual != expected {
        return Err(PrivateProjectionPayloadWireErrorV1::ProjectionBindingMismatch { field });
    }
    Ok(())
}

fn require_payload_digest(
    digest: &Digest,
    field: &'static str,
) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
    if digest.iter().all(|byte| *byte == 0) {
        return Err(PrivateProjectionPayloadWireErrorV1::ZeroIdentity { field });
    }
    Ok(())
}

#[cfg(test)]
fn validate_private_projection_payload_wire_bytes_v1(
    bytes: &[u8],
) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
    let minimum_len = PRIVATE_PROJECTION_PAYLOAD_WIRE_HEADER_LEN_V1 + 32;
    if bytes.len() < minimum_len
        || bytes.get(..8) != Some(PRIVATE_PROJECTION_PAYLOAD_WIRE_MAGIC_V1.as_slice())
        || read_u16_v1(bytes, 8) != Some(PRIVATE_PROJECTION_PAYLOAD_WIRE_SCHEMA_VERSION_V1)
        || read_u16_v1(bytes, 10).map(usize::from)
            != Some(PRIVATE_PROJECTION_PAYLOAD_WIRE_HEADER_LEN_V1)
    {
        return Err(PrivateProjectionPayloadWireErrorV1::MalformedWire);
    }
    let payload_len = read_u32_v1(bytes, 12)
        .and_then(|length| usize::try_from(length).ok())
        .ok_or(PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    if payload_len > PRIVATE_PROJECTION_PAYLOAD_MAX_BYTES_V1 {
        return Err(PrivateProjectionPayloadWireErrorV1::PayloadTooLarge);
    }
    let payload_end = PRIVATE_PROJECTION_PAYLOAD_WIRE_HEADER_LEN_V1
        .checked_add(payload_len)
        .ok_or(PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    let expected_len = payload_end
        .checked_add(32)
        .ok_or(PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    if bytes.len() != expected_len {
        return Err(PrivateProjectionPayloadWireErrorV1::MalformedWire);
    }

    let revision =
        read_u64_v1(bytes, 16).ok_or(PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    let turn_id =
        read_id_v1(bytes, 24).ok_or(PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    let turn_binding =
        read_digest_v1(bytes, 40).ok_or(PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    let projection_digest =
        read_digest_v1(bytes, 72).ok_or(PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    let certificate_digest =
        read_digest_v1(bytes, 104).ok_or(PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    let binding_digest =
        read_digest_v1(bytes, 136).ok_or(PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    let payload_digest =
        read_digest_v1(bytes, 168).ok_or(PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    for (field, digest) in [
        ("turn_binding", &turn_binding),
        ("projection_digest", &projection_digest),
        ("certificate_digest", &certificate_digest),
        ("binding_digest", &binding_digest),
        ("payload_digest", &payload_digest),
    ] {
        require_payload_digest(digest, field)?;
    }
    if turn_id.iter().all(|byte| *byte == 0) {
        return Err(PrivateProjectionPayloadWireErrorV1::ZeroIdentity { field: "turn_id" });
    }

    let payload = bytes
        .get(PRIVATE_PROJECTION_PAYLOAD_WIRE_HEADER_LEN_V1..payload_end)
        .ok_or(PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    let value: Value = serde_json::from_slice(payload)
        .map_err(|_| PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    let canonical = serde_json::to_vec(&value)
        .map_err(|_| PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    if canonical != payload {
        return Err(PrivateProjectionPayloadWireErrorV1::NonCanonicalWire);
    }
    validate_closed_payload_shape_v1(&value)?;
    validate_safe_payload_values_v1(&value, None)?;

    let root = value
        .as_object()
        .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadShape)?;
    let identity = object_field_v1(root, "identity")?;
    let epistemics = object_field_v1(root, "epistemics")?;
    let agency = object_field_v1(root, "agency")?;
    let certificate = object_field_v1(root, "projection_certificate")?;
    let action_realization_digest = digest_field_v1(certificate, "action_realization_digest")?;
    let validation = PrivateProjectionWireValidationV1 {
        revision,
        epistemic_revision: u64_field_v1(epistemics, "revision")?,
        turn_id,
        epistemic_turn_id: id_field_v1(epistemics, "turn_id")?,
        turn_binding,
        projection_digest,
        identity_digest: digest_field_v1(identity, "constitution_digest")?,
        epistemic_identity_digest: digest_field_v1(epistemics, "identity_digest")?,
        source_state_digest: digest_field_v1(certificate, "source_state_digest")?,
        soma_state_digest: digest_field_v1(certificate, "soma_state_digest")?,
        epistemic_state_digest: digest_field_v1(epistemics, "state_digest")?,
        action_contract_digest: digest_field_v1(agency, "action_contract_digest")?,
        certificate_action_contract_digest: digest_field_v1(certificate, "action_contract_digest")?,
        action_realization_digest,
        certificate_action_realization_digest: action_realization_digest,
        epistemic_projection_digest: digest_field_v1(epistemics, "projection_digest")?,
        certificate_epistemic_projection_digest: digest_field_v1(
            certificate,
            "epistemic_projection_digest",
        )?,
    };
    validate_private_projection_for_wire_v1(&validation)?;
    let efference_copy_digest = digest_field_v1(certificate, "efference_copy_digest")?;
    validate_payload_bindings_v1(&value, &validation, &efference_copy_digest)?;

    let expected_payload_digest = payload_digest_v1(payload);
    if payload_digest != expected_payload_digest {
        return Err(PrivateProjectionPayloadWireErrorV1::DigestMismatch {
            field: "payload_digest",
        });
    }
    let certificate_bytes = serde_json::to_vec(
        root.get("projection_certificate")
            .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadShape)?,
    )
    .map_err(|_| PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    let expected_certificate_digest = certificate_digest_v1(&certificate_bytes);
    if certificate_digest != expected_certificate_digest {
        return Err(PrivateProjectionPayloadWireErrorV1::DigestMismatch {
            field: "certificate_digest",
        });
    }
    let expected_binding_digest =
        projection_payload_binding_digest_v1(&validation, &efference_copy_digest);
    if binding_digest != expected_binding_digest {
        return Err(PrivateProjectionPayloadWireErrorV1::DigestMismatch {
            field: "binding_digest",
        });
    }
    let expected_wire_digest = wire_digest_v1(&bytes[..payload_end]);
    let actual_wire_digest = read_digest_v1(bytes, payload_end)
        .ok_or(PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    if actual_wire_digest != expected_wire_digest {
        return Err(PrivateProjectionPayloadWireErrorV1::DigestMismatch {
            field: "wire_digest",
        });
    }
    Ok(())
}

#[cfg(test)]
fn read_u16_v1(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

#[cfg(test)]
fn read_u32_v1(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

#[cfg(test)]
fn read_u64_v1(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_be_bytes(
        bytes.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

#[cfg(test)]
fn read_id_v1(bytes: &[u8], offset: usize) -> Option<Id128> {
    bytes.get(offset..offset + 16)?.try_into().ok()
}

#[cfg(test)]
fn read_digest_v1(bytes: &[u8], offset: usize) -> Option<Digest> {
    bytes.get(offset..offset + 32)?.try_into().ok()
}

fn decode_hex_v1<const N: usize>(value: &str) -> Option<[u8; N]> {
    if value.len() != N * 2
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    let mut output = [0; N];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = (hex_nibble_v1(value.as_bytes()[index * 2])? << 4)
            | hex_nibble_v1(value.as_bytes()[index * 2 + 1])?;
    }
    Some(output)
}

fn hex_nibble_v1(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod legacy_producer_sealing_transaction_tests {
    use super::*;
    use ae_morph::{
        MorphAffordanceCatalogV1, MorphAvailabilityV1, MorphClassificationVocabularyInputV1,
        MorphClassificationVocabularyV1, MorphConfirmationRequirementV1, MorphEffectorInputV1,
        MorphEffectorV1, MorphStateBindingV1, MorphVocabularyBoundsV1,
        MORPH_AFFORDANCE_MAX_ITEMS_V1,
    };

    include!("../../tests/support/native_projection_runtime.rs");

    fn morph_catalog(
        revision: u64,
        identity_digest: Digest,
        state_digest: Digest,
    ) -> MorphAffordanceCatalogV1 {
        let binding = MorphStateBindingV1::new(revision, identity_digest, state_digest)
            .expect("typed morph binding");
        let vocabulary = MorphClassificationVocabularyV1::new(
            MorphClassificationVocabularyInputV1 {
                capability_classes: vec!["capability_a".into()],
                safety_classes: vec!["safety_a".into()],
                reliability_classes: vec!["reliability_a".into()],
                side_effect_classes: vec!["side_effect_a".into()],
                latency_classes: vec!["latency_a".into()],
                cost_classes: vec!["cost_a".into()],
                reversibility_classes: vec!["reversibility_a".into()],
            },
            MorphVocabularyBoundsV1::new(4, 32).expect("typed morph vocabulary bounds"),
        )
        .expect("typed morph vocabulary");
        let effector = MorphEffectorV1::new(
            MorphEffectorInputV1 {
                effector_id: "effector.alpha".into(),
                capability_class: "capability_a".into(),
                availability: MorphAvailabilityV1::Available,
                safety_class: "safety_a".into(),
                reliability_class: "reliability_a".into(),
                side_effect_class: "side_effect_a".into(),
                confirmation_requirement: MorphConfirmationRequirementV1::Required,
                latency_class: "latency_a".into(),
                cost_class: "cost_a".into(),
                reversibility_class: "reversibility_a".into(),
            },
            32,
            &vocabulary,
            &binding,
        )
        .expect("typed morph effector");
        MorphAffordanceCatalogV1::new(
            "morph.catalog.v1".into(),
            32,
            binding,
            vocabulary,
            vec![effector],
            MORPH_AFFORDANCE_MAX_ITEMS_V1,
        )
        .expect("typed morph catalog")
    }

    fn producer_input(
        identity: IdentityConstitutionV1,
        revision: u64,
        kv_snapshot_digest: Digest,
        identity_digest: Digest,
        state_digest: Digest,
    ) -> NativeProjectionPayloadIngressV1 {
        NativeProjectionPayloadIngressV1::ready(NativeProjectionPayloadProducerInputV1::new(
            update(identity, revision, revision),
            kv_snapshot_digest,
            morph_catalog(revision, identity_digest, state_digest),
        ))
    }

    #[test]
    fn forced_legacy_sealing_failure_does_not_advance_watermark_or_consume_revision() {
        let identity = identity(41);
        let identity_digest = *identity.constitution_digest();
        let state_digest = semantic_state_digest(9);
        let mut producer =
            NativeProjectionPayloadProducerV1::new(identity_capsule(identity.clone()))
                .expect("immutable typed identity");

        assert!(matches!(
            producer.produce_with_test_only_sealing_failure_v1(producer_input(
                identity.clone(),
                9,
                digest(21),
                identity_digest,
                state_digest,
            )),
            Err(NativeProjectionPayloadProducerErrorV1::Wire(
                PrivateProjectionPayloadWireErrorV1::PayloadEncodingInvalid
            ))
        ));
        assert_eq!(producer.current_revision(), None);

        producer
            .produce(producer_input(
                identity,
                9,
                digest(21),
                identity_digest,
                state_digest,
            ))
            .expect("the failed sealing attempt left this revision retryable");
        assert_eq!(producer.current_revision(), Some(9));
    }
}

fn encode_hex_v1(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 15) as usize] as char);
    }
    output
}

/// Internal legacy projection compiler with one immutable identity.
struct OrganismProjectionRuntimeV1 {
    identity: SourceCapsuleV1<IdentityConstitutionV1>,
}

impl OrganismProjectionRuntimeV1 {
    fn new(
        identity: SourceCapsuleV1<IdentityConstitutionV1>,
    ) -> Result<Self, OrganismRuntimeErrorV1> {
        let actual = identity.provenance().source_kind();
        if actual != ProjectionSourceKindV1::IdentityConstitution {
            return Err(OrganismRuntimeErrorV1::WrongIdentitySourceKind { actual });
        }
        if identity.content_digest() != identity.value().constitution_digest() {
            return Err(OrganismRuntimeErrorV1::IdentityConstitutionDigestMismatch);
        }
        Ok(Self { identity })
    }

    fn compile_uncommitted(
        &self,
        update: NativeProjectionUpdateV1,
    ) -> Result<PrivateCognitiveProjectionV1, OrganismRuntimeErrorV1> {
        update.validate_bindings(self.identity.value().constitution_digest())?;
        let revision = update.revision;
        let NativeProjectionUpdateV1 {
            references,
            action_contract,
            soma_state,
            soma_classification_ingress,
            epistemic_projection,
            action_realization,
            efference_copy,
            preconditions,
            ..
        } = update;
        let BoundedProjectionReferencesV1 {
            organism_snapshot,
            cognitive_kv_view,
            exact_turn_anchors,
            relation_scope,
            affordance_catalog,
            provider_profile,
        } = references;
        let turn_id = *organism_snapshot.value().turn_id();
        let turn_binding = *organism_snapshot.value().turn_binding();
        let input = ProjectionInput::new(
            organism_snapshot,
            cognitive_kv_view,
            exact_turn_anchors,
            self.identity.clone(),
            relation_scope,
            action_contract,
            affordance_catalog,
            provider_profile,
            soma_state,
            soma_classification_ingress,
            epistemic_projection,
            action_realization,
            efference_copy,
        );
        let (envelope, _) = compile_projection_v1(&input, &preconditions)?;
        Ok(PrivateCognitiveProjectionV1 {
            revision,
            turn_id,
            turn_binding,
            envelope,
        })
    }
}

impl NativeProjectionUpdateV1 {
    fn validate_bindings(&self, identity_digest: &Digest) -> Result<(), OrganismRuntimeErrorV1> {
        for (field, actual) in [
            (
                "organism_snapshot",
                self.references
                    .organism_snapshot
                    .provenance()
                    .source_revision(),
            ),
            (
                "cognitive_kv_view",
                self.references
                    .cognitive_kv_view
                    .provenance()
                    .source_revision(),
            ),
            (
                "exact_turn_anchors",
                self.references
                    .exact_turn_anchors
                    .provenance()
                    .source_revision(),
            ),
            (
                "relation_scope",
                self.references
                    .relation_scope
                    .provenance()
                    .source_revision(),
            ),
            (
                "affordance_catalog",
                self.references
                    .affordance_catalog
                    .provenance()
                    .source_revision(),
            ),
            (
                "provider_profile",
                self.references
                    .provider_profile
                    .provenance()
                    .source_revision(),
            ),
            (
                "action_contract",
                self.action_contract.provenance().source_revision(),
            ),
            ("soma_state", self.soma_state.provenance().source_revision()),
            ("soma_state.value", self.soma_state.value().revision()),
            ("epistemic_projection", self.epistemic_projection.revision()),
        ] {
            if actual != self.revision {
                return Err(OrganismRuntimeErrorV1::RevisionBindingMismatch {
                    field,
                    expected: self.revision,
                    actual,
                });
            }
        }
        if self.soma_state.value().identity_constitution_digest() != identity_digest {
            return Err(OrganismRuntimeErrorV1::IdentityBindingMismatch {
                field: "soma_state",
            });
        }
        if self.epistemic_projection.identity_digest() != identity_digest {
            return Err(OrganismRuntimeErrorV1::IdentityBindingMismatch {
                field: "epistemic_projection",
            });
        }
        Ok(())
    }
}
