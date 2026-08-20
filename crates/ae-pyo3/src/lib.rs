#![forbid(unsafe_code)]

use ae_runtime::{
    AstrBotPublicSignalV1, AstrBotToolDispositionV1, AstrBotToolIngressV1, AstrBotToolOutcomeV1,
    DeliveryKnowledgeV1, HostEffectDispositionV1, HostEffectV1, HostIngressKindV1, HostIngressV1,
    HostSettlementStatusV1, HostSettlementV1, NativeProjectionPayloadIngressV1,
    NativeProjectionPayloadProducerV1, PrivateProjectionPayloadWireV1, PublicTextV1,
};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

#[pyfunction]
fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[pyfunction]
fn health() -> String {
    r#"{"status":"scaffold","formula":"aster-ccn-v1","neuron_slots":16384}"#.to_owned()
}

/// Rust-created semantic payload bridge. Python cannot construct or inspect
/// this value and may consume its native bytes only once.
#[pyclass(
    name = "_PrivateProjectionPayloadWireV1",
    module = "astrembodiment_core._native"
)]
struct PyPrivateProjectionPayloadWireV1 {
    inner: Option<PrivateProjectionPayloadWireV1>,
}

impl PyPrivateProjectionPayloadWireV1 {
    fn from_native(wire: PrivateProjectionPayloadWireV1) -> Self {
        Self { inner: Some(wire) }
    }
}

impl PyPrivateProjectionPayloadWireV1 {
    fn consume_exact<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let mut wire = self
            .inner
            .take()
            .ok_or_else(private_projection_unavailable_error)?;
        let private_bytes = wire
            .consume_once()
            .map_err(|_| private_projection_unavailable_error())?;
        Ok(PyBytes::new(py, &private_bytes))
    }
}

#[pyfunction]
fn _consume_private_projection_payload_wire_v1<'py>(
    py: Python<'py>,
    wire: &mut PyPrivateProjectionPayloadWireV1,
) -> PyResult<Bound<'py, PyBytes>> {
    wire.consume_exact(py)
}

#[pyfunction]
#[pyo3(pass_module)]
fn _astrbot_host_private_projection_wire_capability_v1<'py>(
    module: &Bound<'py, PyModule>,
) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyAny>)> {
    Ok((
        module.getattr("_PrivateProjectionPayloadWireV1")?,
        module.getattr("_consume_private_projection_payload_wire_v1")?,
    ))
}

/// Rust-created producer wrapper. It owns only the native transaction state;
/// Python gets no constructor, source input, revision, or inspection surface.
#[pyclass(name = "_PrivateProjectionPayloadProducerV1", unsendable)]
struct PyPrivateProjectionPayloadProducerV1 {
    inner: NativeProjectionPayloadProducerV1,
}

impl PyPrivateProjectionPayloadProducerV1 {
    #[allow(dead_code)]
    fn from_native(inner: NativeProjectionPayloadProducerV1) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyPrivateProjectionPayloadProducerV1 {
    #[pyo3(name = "_produce_private_projection_payload_wire_v1")]
    fn produce_private_projection_payload_wire_v1(
        &mut self,
        ingress: &mut PyPrivateProjectionPayloadIngressV1,
    ) -> PyResult<Option<PyPrivateProjectionPayloadWireV1>> {
        let ingress = ingress
            .inner
            .take()
            .ok_or_else(private_projection_unavailable_error)?;
        if matches!(&ingress, NativeProjectionPayloadIngressV1::Unavailable) {
            return Ok(None);
        }
        self.inner
            .produce(ingress)
            .map(PyPrivateProjectionPayloadWireV1::from_native)
            .map(Some)
            .map_err(|_| private_projection_unavailable_error())
    }
}

/// Rust-created one-shot native ingress. `None` is deliberately distinct
/// from native `Unavailable`: it represents an absent or replayed wrapper.
#[pyclass(name = "_PrivateProjectionPayloadIngressV1", unsendable)]
struct PyPrivateProjectionPayloadIngressV1 {
    inner: Option<NativeProjectionPayloadIngressV1>,
}

impl PyPrivateProjectionPayloadIngressV1 {
    #[allow(dead_code)]
    fn from_native(inner: NativeProjectionPayloadIngressV1) -> Self {
        Self { inner: Some(inner) }
    }
}

#[pyclass(name = "HostSettlementV1", frozen)]
struct PyHostSettlementV1 {
    inner: HostSettlementV1,
}

#[pymethods]
impl PyHostSettlementV1 {
    #[new]
    #[allow(clippy::too_many_arguments)]
    fn new(
        schema_version: u16,
        settlement_id: String,
        effect_id: String,
        process_epoch_id: String,
        adapter_type: String,
        adapter_id_binding: String,
        scope_binding: String,
        session_binding: String,
        turn_binding: String,
        action_id: String,
        status: String,
        delivery: String,
        observed_at_ms: u64,
    ) -> PyResult<Self> {
        Ok(Self {
            inner: HostSettlementV1 {
                schema_version,
                settlement_id: decode_digest(&settlement_id)?,
                effect_id: decode_digest(&effect_id)?,
                process_epoch_id: decode_id128(&process_epoch_id)?,
                adapter_type,
                adapter_id_binding: decode_digest(&adapter_id_binding)?,
                scope_binding: decode_digest(&scope_binding)?,
                session_binding: decode_digest(&session_binding)?,
                turn_binding: decode_digest(&turn_binding)?,
                action_id: decode_digest(&action_id)?,
                status: parse_status(&status)?,
                delivery: parse_delivery(&delivery)?,
                observed_at_ms,
            },
        })
    }

    #[getter]
    fn schema_version(&self) -> u16 {
        self.inner.schema_version
    }
    #[getter]
    fn settlement_id(&self) -> String {
        encode_digest(self.inner.settlement_id)
    }
    #[getter]
    fn effect_id(&self) -> String {
        encode_digest(self.inner.effect_id)
    }
    #[getter]
    fn process_epoch_id(&self) -> String {
        encode_id128(self.inner.process_epoch_id)
    }
    #[getter]
    fn adapter_type(&self) -> String {
        self.inner.adapter_type.clone()
    }
    #[getter]
    fn adapter_id_binding(&self) -> String {
        encode_digest(self.inner.adapter_id_binding)
    }
    #[getter]
    fn scope_binding(&self) -> String {
        encode_digest(self.inner.scope_binding)
    }
    #[getter]
    fn session_binding(&self) -> String {
        encode_digest(self.inner.session_binding)
    }
    #[getter]
    fn turn_binding(&self) -> String {
        encode_digest(self.inner.turn_binding)
    }
    #[getter]
    fn action_id(&self) -> String {
        encode_digest(self.inner.action_id)
    }
    #[getter]
    fn status(&self) -> String {
        status_name(self.inner.status).to_owned()
    }
    #[getter]
    fn delivery(&self) -> String {
        delivery_name(self.inner.delivery).to_owned()
    }
    #[getter]
    fn observed_at_ms(&self) -> u64 {
        self.inner.observed_at_ms
    }
}

#[pyclass(name = "HostIngressV1", frozen)]
struct PyHostIngressV1 {
    inner: HostIngressV1,
}

#[pymethods]
impl PyHostIngressV1 {
    #[new]
    #[allow(clippy::too_many_arguments)]
    fn new(
        py: Python<'_>,
        schema_version: u16,
        kind: String,
        ingress_id: String,
        process_epoch_id: String,
        adapter_type: String,
        adapter_id_binding: String,
        scope_binding: String,
        session_binding: String,
        turn_binding: String,
        event_id: String,
        observed_at_ms: u64,
        base_revision: u64,
        current_event_text: Option<String>,
        settlement: Option<Py<PyHostSettlementV1>>,
    ) -> PyResult<Self> {
        let settlement = settlement.map(|item| item.borrow(py).inner.clone());
        Ok(Self {
            inner: HostIngressV1 {
                schema_version,
                kind: parse_ingress_kind(&kind)?,
                ingress_id: decode_digest(&ingress_id)?,
                process_epoch_id: decode_id128(&process_epoch_id)?,
                adapter_type,
                adapter_id_binding: decode_digest(&adapter_id_binding)?,
                scope_binding: decode_digest(&scope_binding)?,
                session_binding: decode_digest(&session_binding)?,
                turn_binding: decode_digest(&turn_binding)?,
                event_id: decode_digest(&event_id)?,
                observed_at_ms,
                base_revision,
                current_event_text,
                settlement,
            },
        })
    }

    #[getter]
    fn schema_version(&self) -> u16 {
        self.inner.schema_version
    }
    #[getter]
    fn kind(&self) -> String {
        ingress_kind_name(self.inner.kind).to_owned()
    }
    #[getter]
    fn ingress_id(&self) -> String {
        encode_digest(self.inner.ingress_id)
    }
    #[getter]
    fn process_epoch_id(&self) -> String {
        encode_id128(self.inner.process_epoch_id)
    }
    #[getter]
    fn adapter_type(&self) -> String {
        self.inner.adapter_type.clone()
    }
    #[getter]
    fn adapter_id_binding(&self) -> String {
        encode_digest(self.inner.adapter_id_binding)
    }
    #[getter]
    fn scope_binding(&self) -> String {
        encode_digest(self.inner.scope_binding)
    }
    #[getter]
    fn session_binding(&self) -> String {
        encode_digest(self.inner.session_binding)
    }
    #[getter]
    fn turn_binding(&self) -> String {
        encode_digest(self.inner.turn_binding)
    }
    #[getter]
    fn event_id(&self) -> String {
        encode_digest(self.inner.event_id)
    }
    #[getter]
    fn observed_at_ms(&self) -> u64 {
        self.inner.observed_at_ms
    }
    #[getter]
    fn base_revision(&self) -> u64 {
        self.inner.base_revision
    }
    #[getter]
    fn current_event_text(&self) -> Option<String> {
        self.inner.current_event_text.clone()
    }
    #[getter]
    fn settlement(&self, py: Python<'_>) -> PyResult<Option<Py<PyHostSettlementV1>>> {
        self.inner
            .settlement
            .clone()
            .map(|inner| Py::new(py, PyHostSettlementV1 { inner }))
            .transpose()
    }
}

#[pyclass(name = "HostEffectV1", frozen)]
struct PyHostEffectV1 {
    inner: HostEffectV1,
}

#[pymethods]
impl PyHostEffectV1 {
    #[new]
    #[allow(clippy::too_many_arguments)]
    fn new(
        schema_version: u16,
        disposition: String,
        effect_id: String,
        process_epoch_id: String,
        adapter_type: String,
        adapter_id_binding: String,
        scope_binding: String,
        session_binding: String,
        turn_binding: String,
        action_id: String,
        capability_id: String,
        authority_evidence_digest: String,
        policy_evidence_digest: String,
        authority_granted: bool,
        policy_granted: bool,
        payload_class: String,
        public_text: Option<String>,
        expires_at_ms: u64,
    ) -> PyResult<Self> {
        let public_payload = public_text
            .map(PublicTextV1::new)
            .transpose()
            .map_err(|_| fixed_error())?;
        Ok(Self {
            inner: HostEffectV1 {
                schema_version,
                disposition: parse_disposition(&disposition)?,
                effect_id: decode_digest(&effect_id)?,
                process_epoch_id: decode_id128(&process_epoch_id)?,
                adapter_type,
                adapter_id_binding: decode_digest(&adapter_id_binding)?,
                scope_binding: decode_digest(&scope_binding)?,
                session_binding: decode_digest(&session_binding)?,
                turn_binding: decode_digest(&turn_binding)?,
                action_id: decode_digest(&action_id)?,
                capability_id,
                authority_evidence_digest: decode_digest(&authority_evidence_digest)?,
                policy_evidence_digest: decode_digest(&policy_evidence_digest)?,
                authority_granted,
                policy_granted,
                payload_class,
                public_payload,
                expires_at_ms,
            },
        })
    }

    #[getter]
    fn schema_version(&self) -> u16 {
        self.inner.schema_version
    }
    #[getter]
    fn disposition(&self) -> String {
        disposition_name(self.inner.disposition).to_owned()
    }
    #[getter]
    fn effect_id(&self) -> String {
        encode_digest(self.inner.effect_id)
    }
    #[getter]
    fn process_epoch_id(&self) -> String {
        encode_id128(self.inner.process_epoch_id)
    }
    #[getter]
    fn adapter_type(&self) -> String {
        self.inner.adapter_type.clone()
    }
    #[getter]
    fn adapter_id_binding(&self) -> String {
        encode_digest(self.inner.adapter_id_binding)
    }
    #[getter]
    fn scope_binding(&self) -> String {
        encode_digest(self.inner.scope_binding)
    }
    #[getter]
    fn session_binding(&self) -> String {
        encode_digest(self.inner.session_binding)
    }
    #[getter]
    fn turn_binding(&self) -> String {
        encode_digest(self.inner.turn_binding)
    }
    #[getter]
    fn action_id(&self) -> String {
        encode_digest(self.inner.action_id)
    }
    #[getter]
    fn capability_id(&self) -> String {
        self.inner.capability_id.clone()
    }
    #[getter]
    fn authority_evidence_digest(&self) -> String {
        encode_digest(self.inner.authority_evidence_digest)
    }
    #[getter]
    fn policy_evidence_digest(&self) -> String {
        encode_digest(self.inner.policy_evidence_digest)
    }
    #[getter]
    fn authority_granted(&self) -> bool {
        self.inner.authority_granted
    }
    #[getter]
    fn policy_granted(&self) -> bool {
        self.inner.policy_granted
    }
    #[getter]
    fn payload_class(&self) -> String {
        self.inner.payload_class.clone()
    }
    #[getter]
    fn public_text(&self) -> Option<String> {
        self.inner
            .public_payload
            .as_ref()
            .map(|payload| payload.text.clone())
    }
    #[getter]
    fn expires_at_ms(&self) -> u64 {
        self.inner.expires_at_ms
    }
}

#[pyclass(name = "AstrBotToolIngressV1", frozen)]
struct PyAstrBotToolIngressV1 {
    inner: AstrBotToolIngressV1,
}

#[pymethods]
impl PyAstrBotToolIngressV1 {
    #[new]
    #[allow(clippy::too_many_arguments)]
    fn new(
        schema_version: u16,
        invocation_id: String,
        process_epoch_id: String,
        adapter_binding: String,
        session_binding: String,
        turn_binding: String,
        event_binding: String,
        observed_at_ms: u64,
        base_revision: u64,
        current_event_text: String,
    ) -> PyResult<Self> {
        let inner = AstrBotToolIngressV1 {
            schema_version,
            invocation_id: decode_digest(&invocation_id)?,
            process_epoch_id: decode_digest(&process_epoch_id)?,
            adapter_binding: decode_digest(&adapter_binding)?,
            session_binding: decode_digest(&session_binding)?,
            turn_binding: decode_digest(&turn_binding)?,
            event_binding: decode_digest(&event_binding)?,
            observed_at_ms,
            base_revision,
            current_event_text,
        };
        inner.validate_shape().map_err(|_| astrbot_tool_error())?;
        Ok(Self { inner })
    }

    #[getter]
    fn schema_version(&self) -> u16 {
        self.inner.schema_version
    }
    #[getter]
    fn invocation_id(&self) -> String {
        encode_digest(self.inner.invocation_id)
    }
    #[getter]
    fn process_epoch_id(&self) -> String {
        encode_digest(self.inner.process_epoch_id)
    }
    #[getter]
    fn adapter_binding(&self) -> String {
        encode_digest(self.inner.adapter_binding)
    }
    #[getter]
    fn session_binding(&self) -> String {
        encode_digest(self.inner.session_binding)
    }
    #[getter]
    fn turn_binding(&self) -> String {
        encode_digest(self.inner.turn_binding)
    }
    #[getter]
    fn event_binding(&self) -> String {
        encode_digest(self.inner.event_binding)
    }
    #[getter]
    fn observed_at_ms(&self) -> u64 {
        self.inner.observed_at_ms
    }
    #[getter]
    fn base_revision(&self) -> u64 {
        self.inner.base_revision
    }
    #[getter]
    fn current_event_text(&self) -> String {
        self.inner.current_event_text.clone()
    }

    fn __repr__(&self) -> String {
        format!(
            "AstrBotToolIngressV1(schema_version={}, current_event_text=<private>)",
            self.inner.schema_version
        )
    }
}

#[pyclass(name = "AstrBotToolOutcomeV1", frozen)]
struct PyAstrBotToolOutcomeV1 {
    inner: AstrBotToolOutcomeV1,
}

#[pymethods]
impl PyAstrBotToolOutcomeV1 {
    #[getter]
    fn schema_version(&self) -> u16 {
        self.inner.schema_version
    }
    #[getter]
    fn outcome_id(&self) -> String {
        encode_digest(self.inner.outcome_id)
    }
    #[getter]
    fn invocation_id(&self) -> String {
        encode_digest(self.inner.invocation_id)
    }
    #[getter]
    fn process_epoch_id(&self) -> String {
        encode_digest(self.inner.process_epoch_id)
    }
    #[getter]
    fn adapter_binding(&self) -> String {
        encode_digest(self.inner.adapter_binding)
    }
    #[getter]
    fn session_binding(&self) -> String {
        encode_digest(self.inner.session_binding)
    }
    #[getter]
    fn turn_binding(&self) -> String {
        encode_digest(self.inner.turn_binding)
    }
    #[getter]
    fn event_binding(&self) -> String {
        encode_digest(self.inner.event_binding)
    }
    #[getter]
    fn revision(&self) -> u64 {
        self.inner.revision
    }
    #[getter]
    fn disposition(&self) -> &'static str {
        match self.inner.disposition {
            AstrBotToolDispositionV1::Silence => "SILENCE",
            AstrBotToolDispositionV1::PublicSignal => "PUBLIC_SIGNAL",
        }
    }
    #[getter]
    fn public_signal(&self) -> Option<&'static str> {
        self.inner.public_signal.map(|signal| match signal {
            AstrBotPublicSignalV1::Observed => "OBSERVED",
        })
    }

    fn __repr__(&self) -> String {
        format!(
            "AstrBotToolOutcomeV1(schema_version={}, disposition={}, public_signal={})",
            self.inner.schema_version,
            self.disposition(),
            self.public_signal().unwrap_or("NONE")
        )
    }
}

#[pyclass(name = "Route1Runtime", unsendable)]
struct PyRoute1Runtime {
    inner: ae_runtime::AstrRuntime,
}

#[pymethods]
impl PyRoute1Runtime {
    #[new]
    fn new() -> Self {
        Self {
            inner: ae_runtime::AstrRuntime::scaffold(),
        }
    }

    fn current_revision(&self) -> u64 {
        self.inner.current_revision()
    }

    fn apply_host_ingress_v1(
        &mut self,
        ingress: &PyHostIngressV1,
    ) -> PyResult<Option<PyHostEffectV1>> {
        self.inner
            .apply_host_ingress_v1(ingress.inner.clone())
            .map(|effect| effect.map(|inner| PyHostEffectV1 { inner }))
            .map_err(|_| fixed_error())
    }

    fn apply_astrbot_tool_v1(
        &mut self,
        ingress: &PyAstrBotToolIngressV1,
    ) -> PyResult<PyAstrBotToolOutcomeV1> {
        self.inner
            .apply_astrbot_tool_v1(ingress.inner.clone())
            .map(|inner| PyAstrBotToolOutcomeV1 { inner })
            .map_err(|_| astrbot_tool_error())
    }
}

fn fixed_error() -> PyErr {
    PyValueError::new_err("route1_contract_error")
}

fn astrbot_tool_error() -> PyErr {
    PyValueError::new_err("astrbot_tool_v1_unavailable")
}

fn private_projection_unavailable_error() -> PyErr {
    PyValueError::new_err("private_projection_unavailable")
}

fn decode_id128(value: &str) -> PyResult<[u8; 16]> {
    decode_hex(value).ok_or_else(fixed_error)
}

fn decode_digest(value: &str) -> PyResult<[u8; 32]> {
    decode_hex(value).ok_or_else(fixed_error)
}

fn decode_hex<const N: usize>(value: &str) -> Option<[u8; N]> {
    if value.len() != N * 2
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    let mut output = [0; N];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = (hex_nibble(value.as_bytes()[index * 2])? << 4)
            | hex_nibble(value.as_bytes()[index * 2 + 1])?;
    }
    Some(output)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn encode_id128(value: [u8; 16]) -> String {
    encode_hex(&value)
}
fn encode_digest(value: [u8; 32]) -> String {
    encode_hex(&value)
}

fn encode_hex(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 15) as usize] as char);
    }
    output
}

fn parse_ingress_kind(value: &str) -> PyResult<HostIngressKindV1> {
    match value {
        "current_event" => Ok(HostIngressKindV1::CurrentEvent),
        "effect_settlement" => Ok(HostIngressKindV1::EffectSettlement),
        _ => Err(fixed_error()),
    }
}
fn ingress_kind_name(value: HostIngressKindV1) -> &'static str {
    match value {
        HostIngressKindV1::CurrentEvent => "current_event",
        HostIngressKindV1::EffectSettlement => "effect_settlement",
    }
}
fn parse_disposition(value: &str) -> PyResult<HostEffectDispositionV1> {
    match value {
        "silence" => Ok(HostEffectDispositionV1::Silence),
        "public_effect" => Ok(HostEffectDispositionV1::PublicEffect),
        _ => Err(fixed_error()),
    }
}
fn disposition_name(value: HostEffectDispositionV1) -> &'static str {
    match value {
        HostEffectDispositionV1::Silence => "silence",
        HostEffectDispositionV1::PublicEffect => "public_effect",
    }
}
fn parse_delivery(value: &str) -> PyResult<DeliveryKnowledgeV1> {
    match value {
        "not_dispatched" => Ok(DeliveryKnowledgeV1::NotDispatched),
        "unknown" => Ok(DeliveryKnowledgeV1::Unknown),
        _ => Err(fixed_error()),
    }
}
fn delivery_name(value: DeliveryKnowledgeV1) -> &'static str {
    match value {
        DeliveryKnowledgeV1::NotDispatched => "not_dispatched",
        DeliveryKnowledgeV1::Unknown => "unknown",
    }
}
fn parse_status(value: &str) -> PyResult<HostSettlementStatusV1> {
    match value {
        "silenced" => Ok(HostSettlementStatusV1::Silenced),
        "rejected_schema" => Ok(HostSettlementStatusV1::RejectedSchema),
        "rejected_ingress_kind" => Ok(HostSettlementStatusV1::RejectedIngressKind),
        "rejected_platform" => Ok(HostSettlementStatusV1::RejectedPlatform),
        "rejected_adapter_identity" => Ok(HostSettlementStatusV1::RejectedAdapterIdentity),
        "rejected_scope" => Ok(HostSettlementStatusV1::RejectedScope),
        "rejected_session" => Ok(HostSettlementStatusV1::RejectedSession),
        "rejected_turn" => Ok(HostSettlementStatusV1::RejectedTurn),
        "rejected_action" => Ok(HostSettlementStatusV1::RejectedAction),
        "rejected_process_epoch" => Ok(HostSettlementStatusV1::RejectedProcessEpoch),
        "rejected_capability" => Ok(HostSettlementStatusV1::RejectedCapability),
        "rejected_authority" => Ok(HostSettlementStatusV1::RejectedAuthority),
        "rejected_policy" => Ok(HostSettlementStatusV1::RejectedPolicy),
        "rejected_expired" => Ok(HostSettlementStatusV1::RejectedExpired),
        "rejected_payload_class" => Ok(HostSettlementStatusV1::RejectedPayloadClass),
        "rejected_payload_shape" => Ok(HostSettlementStatusV1::RejectedPayloadShape),
        "idempotency_conflict" => Ok(HostSettlementStatusV1::IdempotencyConflict),
        "duplicate_suppressed" => Ok(HostSettlementStatusV1::DuplicateSuppressed),
        "failed_before_dispatch" => Ok(HostSettlementStatusV1::FailedBeforeDispatch),
        "dispatch_returned_no_typed_receipt" => {
            Ok(HostSettlementStatusV1::DispatchReturnedNoTypedReceipt)
        }
        "delivery_unknown" => Ok(HostSettlementStatusV1::DeliveryUnknown),
        _ => Err(fixed_error()),
    }
}
fn status_name(value: HostSettlementStatusV1) -> &'static str {
    match value {
        HostSettlementStatusV1::Silenced => "silenced",
        HostSettlementStatusV1::RejectedSchema => "rejected_schema",
        HostSettlementStatusV1::RejectedIngressKind => "rejected_ingress_kind",
        HostSettlementStatusV1::RejectedPlatform => "rejected_platform",
        HostSettlementStatusV1::RejectedAdapterIdentity => "rejected_adapter_identity",
        HostSettlementStatusV1::RejectedScope => "rejected_scope",
        HostSettlementStatusV1::RejectedSession => "rejected_session",
        HostSettlementStatusV1::RejectedTurn => "rejected_turn",
        HostSettlementStatusV1::RejectedAction => "rejected_action",
        HostSettlementStatusV1::RejectedProcessEpoch => "rejected_process_epoch",
        HostSettlementStatusV1::RejectedCapability => "rejected_capability",
        HostSettlementStatusV1::RejectedAuthority => "rejected_authority",
        HostSettlementStatusV1::RejectedPolicy => "rejected_policy",
        HostSettlementStatusV1::RejectedExpired => "rejected_expired",
        HostSettlementStatusV1::RejectedPayloadClass => "rejected_payload_class",
        HostSettlementStatusV1::RejectedPayloadShape => "rejected_payload_shape",
        HostSettlementStatusV1::IdempotencyConflict => "idempotency_conflict",
        HostSettlementStatusV1::DuplicateSuppressed => "duplicate_suppressed",
        HostSettlementStatusV1::FailedBeforeDispatch => "failed_before_dispatch",
        HostSettlementStatusV1::DispatchReturnedNoTypedReceipt => {
            "dispatch_returned_no_typed_receipt"
        }
        HostSettlementStatusV1::DeliveryUnknown => "delivery_unknown",
    }
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(version, module)?)?;
    module.add_function(wrap_pyfunction!(health, module)?)?;
    module.add_function(wrap_pyfunction!(
        _consume_private_projection_payload_wire_v1,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        _astrbot_host_private_projection_wire_capability_v1,
        module
    )?)?;
    module.add_class::<PyPrivateProjectionPayloadWireV1>()?;
    module.add_class::<PyPrivateProjectionPayloadProducerV1>()?;
    module.add_class::<PyPrivateProjectionPayloadIngressV1>()?;
    module.add_class::<PyHostSettlementV1>()?;
    module.add_class::<PyHostIngressV1>()?;
    module.add_class::<PyHostEffectV1>()?;
    module.add_class::<PyAstrBotToolIngressV1>()?;
    module.add_class::<PyAstrBotToolOutcomeV1>()?;
    module.add_class::<PyRoute1Runtime>()?;
    Ok(())
}

#[cfg(test)]
mod private_projection_wire_tests {
    use super::*;
    use ae_morph::{
        MorphAffordanceCatalogV1, MorphAvailabilityV1, MorphClassificationVocabularyInputV1,
        MorphClassificationVocabularyV1, MorphConfirmationRequirementV1, MorphEffectorInputV1,
        MorphEffectorV1, MorphStateBindingV1, MorphVocabularyBoundsV1,
        MORPH_AFFORDANCE_MAX_ITEMS_V1,
    };
    use ae_runtime::NativeProjectionPayloadProducerInputV1;
    use pyo3::exceptions::PyTypeError;
    use pyo3::types::PyTuple;

    // This is a Rust-only fixture import. It never reaches the Python module;
    // it supplies only existing fully-bound typed native source values.
    include!("../../ae-organism-runtime/tests/private_projection_runtime.rs");

    #[test]
    fn python_private_payload_wire_has_only_its_private_one_shot_surface() {
        Python::initialize();
        Python::attach(|py| {
            let module = PyModule::new(py, "_native").expect("test module");
            _native(&module).expect("module registration");
            let class = module
                .getattr("_PrivateProjectionPayloadWireV1")
                .expect("private payload class");
            let construction_error = class.call0().expect_err("no Python constructor");
            assert!(construction_error.is_instance_of::<PyTypeError>(py));

            let class_dict = class.getattr("__dict__").expect("class dictionary");
            for forbidden in [
                "__repr__",
                "__bytes__",
                "to_json",
                "serialize",
                "envelope",
                "payload",
                "payload_digest",
                "wire_bytes",
            ] {
                let present: bool = class_dict
                    .call_method1("__contains__", (forbidden,))
                    .expect("dictionary lookup")
                    .extract()
                    .expect("boolean result");
                assert!(!present, "unexpected Python surface: {forbidden}");
            }
        });
    }

    #[test]
    fn python_module_does_not_register_the_legacy_raw_private_host_wire_class() {
        Python::initialize();
        Python::attach(|py| {
            let module = PyModule::new(py, "astrembodiment_core._native").expect("test module");
            _native(&module).expect("module registration");
            assert!(
                module.getattr("_PrivateHostWireV1").is_err(),
                "the runtime-owned opaque payload wire is the only private wire class"
            );
        });
    }

    #[test]
    fn host_wire_capability_returns_only_the_native_wire_type_and_module_consumer() {
        Python::initialize();
        Python::attach(|py| {
            let module = PyModule::new(py, "astrembodiment_core._native").expect("test module");
            _native(&module).expect("module registration");
            let capability = module
                .getattr("_astrbot_host_private_projection_wire_capability_v1")
                .expect("private Host capability");
            let pair = capability.call0().expect("capability pair");
            let pair = pair.cast::<PyTuple>().expect("capability tuple");
            assert_eq!(pair.len(), 2);
            let wire_type = pair.get_item(0).expect("wire type");
            let exact_consumer = pair.get_item(1).expect("wire consumer");
            assert!(wire_type.is(module
                .getattr("_PrivateProjectionPayloadWireV1")
                .expect("native wire type")));
            let wire_module: String = wire_type
                .getattr("__module__")
                .expect("native wire module")
                .extract()
                .expect("native wire module string");
            assert_eq!(wire_module, "astrembodiment_core._native");
            assert_ne!(wire_module, "builtins");
            assert!(exact_consumer.is_callable());
            let non_native_error = match exact_consumer.call1((py.None(),)) {
                Err(error) => error,
                Ok(_) => panic!("non-native wire must fail nominal extraction"),
            };
            assert!(non_native_error.is_instance_of::<PyTypeError>(py));

            let wire_dict = module
                .getattr("_PrivateProjectionPayloadWireV1")
                .expect("native wire type")
                .getattr("__dict__")
                .expect("wire class dictionary");
            let direct_consumer: bool = wire_dict
                .call_method1(
                    "__contains__",
                    ("_consume_private_projection_payload_wire_v1",),
                )
                .expect("dictionary lookup")
                .extract()
                .expect("boolean result");
            assert!(!direct_consumer);
        });
    }

    #[test]
    fn python_private_producer_and_ingress_have_no_constructor_or_public_surface() {
        Python::initialize();
        Python::attach(|py| {
            let module = PyModule::new(py, "_native").expect("test module");
            _native(&module).expect("module registration");
            for class_name in [
                "_PrivateProjectionPayloadProducerV1",
                "_PrivateProjectionPayloadIngressV1",
            ] {
                let class = module
                    .getattr(class_name)
                    .expect("private bridge class is registered");
                let construction_error = class.call0().expect_err("no Python constructor");
                assert!(construction_error.is_instance_of::<PyTypeError>(py));
                let class_dict = class.getattr("__dict__").expect("class dictionary");
                for forbidden in [
                    "__repr__",
                    "__str__",
                    "__bytes__",
                    "to_json",
                    "serialize",
                    "current_revision",
                    "new",
                ] {
                    let present: bool = class_dict
                        .call_method1("__contains__", (forbidden,))
                        .expect("dictionary lookup")
                        .extract()
                        .expect("boolean result");
                    assert!(!present, "unexpected Python surface: {forbidden}");
                }
            }

            let producer_class = module
                .getattr("_PrivateProjectionPayloadProducerV1")
                .expect("private producer class");
            let producer_dict = producer_class
                .getattr("__dict__")
                .expect("producer class dictionary");
            let producer_consumer: bool = producer_dict
                .call_method1(
                    "__contains__",
                    ("_produce_private_projection_payload_wire_v1",),
                )
                .expect("dictionary lookup")
                .extract()
                .expect("boolean result");
            assert!(producer_consumer);
        });
    }

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

    fn ready_ingress(
        identity: IdentityConstitutionV1,
        revision: u64,
        anchors_revision: u64,
    ) -> NativeProjectionPayloadIngressV1 {
        let identity_digest = *identity.constitution_digest();
        let state_digest = semantic_state_digest(revision);
        NativeProjectionPayloadIngressV1::ready(NativeProjectionPayloadProducerInputV1::new(
            update(identity, revision, anchors_revision),
            digest(21),
            morph_catalog(revision, identity_digest, state_digest),
        ))
    }

    #[test]
    fn native_ready_ingress_seals_an_actual_wire_and_is_one_shot_at_both_boundaries() {
        Python::initialize();
        Python::attach(|py| {
            let identity = identity(41);
            let mut producer = PyPrivateProjectionPayloadProducerV1::from_native(
                NativeProjectionPayloadProducerV1::new(identity_capsule(identity.clone()))
                    .expect("immutable typed identity"),
            );
            let mut ingress =
                PyPrivateProjectionPayloadIngressV1::from_native(ready_ingress(identity, 9, 9));

            let wire = producer
                .produce_private_projection_payload_wire_v1(&mut ingress)
                .expect("fully-bound native ingress")
                .expect("ready ingress emits an opaque wire");
            assert_eq!(producer.inner.current_revision(), Some(9));
            let wire = Py::new(py, wire).expect("opaque Python wire object");
            let module = PyModule::new(py, "_native").expect("test module");
            _native(&module).expect("module registration");
            let capability = module
                .getattr("_astrbot_host_private_projection_wire_capability_v1")
                .expect("private Host capability");
            let pair = capability.call0().expect("capability pair");
            let pair = pair.cast::<PyTuple>().expect("capability tuple");
            let exact_consumer = pair.get_item(1).expect("wire consumer");
            let bytes = exact_consumer
                .call1((wire.bind(py),))
                .expect("one native wire consumption")
                .cast_into::<PyBytes>()
                .expect("native wire bytes");
            assert!(!bytes.as_bytes().is_empty(), "native opaque wire bytes");
            let wire_replay = match exact_consumer.call1((wire.bind(py),)) {
                Err(error) => error,
                Ok(_) => panic!("wire replay must fail closed"),
            };
            assert_eq!(
                wire_replay.value(py).to_string(),
                "private_projection_unavailable"
            );
            let ingress_replay =
                match producer.produce_private_projection_payload_wire_v1(&mut ingress) {
                    Err(error) => error,
                    Ok(_) => panic!("ingress replay must fail closed"),
                };
            assert_eq!(
                ingress_replay.value(py).to_string(),
                "private_projection_unavailable"
            );
        });
    }

    #[test]
    fn unavailable_ingress_returns_none_without_advancing_or_becoming_reusable() {
        Python::initialize();
        Python::attach(|py| {
            let identity = identity(41);
            let mut producer = PyPrivateProjectionPayloadProducerV1::from_native(
                NativeProjectionPayloadProducerV1::new(identity_capsule(identity))
                    .expect("immutable typed identity"),
            );
            let mut ingress = PyPrivateProjectionPayloadIngressV1::from_native(
                NativeProjectionPayloadIngressV1::unavailable(),
            );

            assert!(producer
                .produce_private_projection_payload_wire_v1(&mut ingress)
                .expect("native unavailable maps to none")
                .is_none());
            assert_eq!(producer.inner.current_revision(), None);
            let replay = match producer.produce_private_projection_payload_wire_v1(&mut ingress) {
                Err(error) => error,
                Ok(_) => panic!("unavailable ingress wrapper must be one-shot"),
            };
            assert_eq!(
                replay.value(py).to_string(),
                "private_projection_unavailable"
            );
        });
    }

    #[test]
    fn invalid_native_ready_ingress_is_redacted_and_does_not_advance_revision() {
        Python::initialize();
        Python::attach(|py| {
            let identity = identity(41);
            let mut producer = PyPrivateProjectionPayloadProducerV1::from_native(
                NativeProjectionPayloadProducerV1::new(identity_capsule(identity.clone()))
                    .expect("immutable typed identity"),
            );
            let mut ingress =
                PyPrivateProjectionPayloadIngressV1::from_native(ready_ingress(identity, 9, 8));

            let error = match producer.produce_private_projection_payload_wire_v1(&mut ingress) {
                Err(error) => error,
                Ok(_) => panic!("mismatched native source must fail closed"),
            };
            assert_eq!(
                error.value(py).to_string(),
                "private_projection_unavailable"
            );
            assert_eq!(producer.inner.current_revision(), None);
        });
    }

    #[test]
    fn fake_python_ingress_is_rejected_by_nominal_pyo3_extraction() {
        Python::initialize();
        Python::attach(|py| {
            let identity = identity(41);
            let producer = Py::new(
                py,
                PyPrivateProjectionPayloadProducerV1::from_native(
                    NativeProjectionPayloadProducerV1::new(identity_capsule(identity))
                        .expect("immutable typed identity"),
                ),
            )
            .expect("producer Python object");
            let error = match producer
                .bind(py)
                .call_method1("_produce_private_projection_payload_wire_v1", (py.None(),))
            {
                Err(error) => error,
                Ok(_) => panic!("non-native Python object must not extract as private ingress"),
            };
            assert!(error.is_instance_of::<PyTypeError>(py));
        });
    }

    #[test]
    fn python_private_payload_wire_fails_closed_without_a_native_wire() {
        Python::initialize();
        Python::attach(|py| {
            let mut wrapper = PyPrivateProjectionPayloadWireV1 { inner: None };
            let error = wrapper
                .consume_exact(py)
                .expect_err("missing native wire must fail closed");
            assert!(error.is_instance_of::<PyValueError>(py));
            assert_eq!(
                error.value(py).to_string(),
                "private_projection_unavailable"
            );
        });
    }
}
