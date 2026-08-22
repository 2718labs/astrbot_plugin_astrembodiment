//! Canonical opaque `AER7PPW1` transport framing.
//!
//! This private runtime module owns the fixed payload-wire bytes, framing
//! metadata, and one-shot capability. No external crate can mint a wire or
//! inspect its binding/header state.

use ae_cognitive_envelope::{CognitiveEnvelopeV1, PreOutputCognitiveEnvelopeV1};
use ae_contracts::r7::{wire, Digest, Id128};
use serde_json::Value;
use thiserror::Error;
use zeroize::Zeroize;

#[cfg(test)]
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

pub(crate) const PRIVATE_PROJECTION_PAYLOAD_WIRE_SCHEMA_VERSION_V1: u16 = 1;
pub(crate) const PRIVATE_PROJECTION_PAYLOAD_WIRE_MAGIC_V1: &[u8; 8] = b"AER7PPW1";
pub(crate) const PRIVATE_PROJECTION_PAYLOAD_WIRE_HEADER_LEN_V1: usize = 200;
pub(crate) const PRIVATE_PROJECTION_PAYLOAD_MAX_BYTES_V1: usize = 65_536;

const PRIVATE_PROJECTION_PAYLOAD_DOMAIN_V1: &[u8] =
    b"astr-embodiment/r7/private-projection-payload-v1";
const PRIVATE_PROJECTION_PAYLOAD_CERTIFICATE_DOMAIN_V1: &[u8] =
    b"astr-embodiment/r7/private-projection-payload-certificate-v1";
const PRIVATE_PROJECTION_PAYLOAD_WIRE_DOMAIN_V1: &[u8] =
    b"astr-embodiment/r7/private-projection-payload-wire-v1";

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub(crate) enum PrivateProjectionPayloadWireErrorV1 {
    #[error("private projection payload wire has already been consumed")]
    AlreadyConsumed,
    #[error("private projection identity is zero: {field}")]
    ZeroIdentity { field: &'static str },
    #[error("private projection binding mismatch: {field}")]
    ProjectionBindingMismatch { field: &'static str },
    #[error("private projection payload encoding failed")]
    PayloadEncodingInvalid,
    #[error("private projection payload is not closed or safe")]
    UnsafePayloadShape,
    #[error("private projection payload has an unsafe value")]
    UnsafePayloadValue,
    #[error("private projection payload exceeds its wire bound")]
    PayloadTooLarge,
    #[error("private projection payload wire is malformed")]
    MalformedWire,
    #[error("private projection payload wire is not canonical")]
    NonCanonicalWire,
    #[error("private projection payload wire digest mismatch: {field}")]
    DigestMismatch { field: &'static str },
}

/// The only owner for N3H-A encoded payload, certificate, and frame bytes.
/// It deliberately exposes no public byte conversion or serialization seam.
struct SecretBufferV1 {
    bytes: Vec<u8>,
    #[cfg(test)]
    zeroization_probe: Option<TestOnlyZeroizationProbeV1>,
}

impl SecretBufferV1 {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            #[cfg(test)]
            zeroization_probe: None,
        }
    }

    #[cfg(test)]
    fn with_probe(bytes: Vec<u8>, probe: TestOnlyZeroizationProbeV1) -> Self {
        Self {
            bytes,
            zeroization_probe: Some(probe),
        }
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
}

impl Drop for SecretBufferV1 {
    fn drop(&mut self) {
        self.bytes.as_mut_slice().zeroize();
        #[cfg(test)]
        if let Some(probe) = &self.zeroization_probe {
            probe.observe_zeroized(&self.bytes);
        }
    }
}

#[cfg(test)]
#[derive(Clone, Default)]
pub(crate) struct TestOnlyZeroizationProbeV1 {
    zeroized_observations: Arc<AtomicUsize>,
}

#[cfg(test)]
impl TestOnlyZeroizationProbeV1 {
    fn observe_zeroized(&self, bytes: &[u8]) {
        assert!(
            bytes.iter().all(|byte| *byte == 0),
            "N3H-A owned storage must be zero before release"
        );
        self.zeroized_observations.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn assert_zeroized_observations(&self, expected: usize) {
        assert_eq!(
            self.zeroized_observations.load(Ordering::SeqCst),
            expected,
            "unexpected count of zero-before-release observations"
        );
    }
}

/// Digest-only binding metadata carried by the fixed `AER7PPW1` header.
/// No raw payload/body is exposed by this type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PrivateProjectionPayloadWireBindingMetadataV1 {
    revision: u64,
    turn_id: Id128,
    turn_binding: Digest,
    projection_digest: Digest,
    source_state_digest: Digest,
}

impl PrivateProjectionPayloadWireBindingMetadataV1 {
    pub(crate) fn new(
        revision: u64,
        turn_id: Id128,
        turn_binding: Digest,
        projection_digest: Digest,
        source_state_digest: Digest,
    ) -> Result<Self, PrivateProjectionPayloadWireErrorV1> {
        require_nonzero_id(&turn_id, "turn_id")?;
        for (field, digest) in [
            ("turn_binding", &turn_binding),
            ("projection_digest", &projection_digest),
            ("source_state_digest", &source_state_digest),
        ] {
            require_nonzero_digest(digest, field)?;
        }
        Ok(Self {
            revision,
            turn_id,
            turn_binding,
            projection_digest,
            source_state_digest,
        })
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn turn_id(&self) -> Id128 {
        self.turn_id
    }

    pub(crate) fn turn_binding(&self) -> &Digest {
        &self.turn_binding
    }

    pub(crate) fn source_state_digest(&self) -> &Digest {
        &self.source_state_digest
    }
}

/// One-shot opaque semantic payload for the private Host boundary.
///
/// It implements neither `Clone`, `Debug`, nor serialization and exposes no
/// body, text, JSON, envelope, reusable byte getter, or callback materializer.
pub(crate) struct PrivateProjectionPayloadWireV1 {
    private_bytes: Option<SecretBufferV1>,
    metadata: PrivateProjectionPayloadWireBindingMetadataV1,
    wire_digest: Digest,
}

impl Drop for PrivateProjectionPayloadWireV1 {
    fn drop(&mut self) {
        drop(self.private_bytes.take());
    }
}

/// Crate-private one-shot handoff object. It can only be consumed by the
/// trusted native terminal below; neither it nor that terminal returns bytes.
pub(crate) struct PrivateProjectionTransferV1 {
    private_bytes: Option<SecretBufferV1>,
}

impl Drop for PrivateProjectionTransferV1 {
    fn drop(&mut self) {
        drop(self.private_bytes.take());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrivateProjectionTransferReceiptV1 {
    Discarded,
    Cancelled,
}

impl PrivateProjectionPayloadWireV1 {
    /// Confirms that this capability is both unconsumed and still encoded with
    /// the exact canonical `AER7PPW1` framing. It reveals no payload bytes.
    pub(crate) fn validate_live_canonical_v1(
        &self,
    ) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
        let bytes = self
            .private_bytes
            .as_ref()
            .ok_or(PrivateProjectionPayloadWireErrorV1::AlreadyConsumed)?;
        validate_canonical_framing_v1(bytes.as_slice())
    }

    pub(crate) fn binding_metadata(&self) -> &PrivateProjectionPayloadWireBindingMetadataV1 {
        &self.metadata
    }

    pub(crate) fn wire_digest(&self) -> &Digest {
        &self.wire_digest
    }

    pub(crate) fn begin_transfer_once_v1(
        &mut self,
    ) -> Result<PrivateProjectionTransferV1, PrivateProjectionPayloadWireErrorV1> {
        let private_bytes = self
            .private_bytes
            .take()
            .ok_or(PrivateProjectionPayloadWireErrorV1::AlreadyConsumed)?;
        Ok(PrivateProjectionTransferV1 {
            private_bytes: Some(private_bytes),
        })
    }

    pub(crate) fn cancel_v1(mut self) -> PrivateProjectionTransferReceiptV1 {
        drop(self.private_bytes.take());
        PrivateProjectionTransferReceiptV1::Cancelled
    }
}

/// Current N3H-A terminal. N3H-C may later replace this with an isolated
/// gateway, but this task performs no provider, IPC, or Host delivery.
pub(crate) fn discard_private_projection_transfer_v1(
    mut transfer: PrivateProjectionTransferV1,
) -> PrivateProjectionTransferReceiptV1 {
    drop(transfer.private_bytes.take());
    PrivateProjectionTransferReceiptV1::Discarded
}

/// Canonically seals a typed full cognitive envelope. Callers cannot supply
/// raw payload bytes, header fields, or a direct output constructor.
pub(crate) fn seal_cognitive_envelope_v1(
    metadata: PrivateProjectionPayloadWireBindingMetadataV1,
    envelope: &CognitiveEnvelopeV1,
    binding_digest: Digest,
) -> Result<PrivateProjectionPayloadWireV1, PrivateProjectionPayloadWireErrorV1> {
    let (payload, certificate_payload) = canonical_payload_from_envelope_v1(envelope)?;
    seal_owned_payload_v1(metadata, payload, certificate_payload, binding_digest)
}

/// Canonically seals a typed pre-output cognitive envelope. It uses the same
/// stable `AER7PPW1` header bytes as the full envelope path.
pub(crate) fn seal_pre_output_cognitive_envelope_v1(
    metadata: PrivateProjectionPayloadWireBindingMetadataV1,
    envelope: &PreOutputCognitiveEnvelopeV1,
    binding_digest: Digest,
) -> Result<PrivateProjectionPayloadWireV1, PrivateProjectionPayloadWireErrorV1> {
    let (payload, certificate_payload) = canonical_payload_from_envelope_v1(envelope)?;
    seal_owned_payload_v1(metadata, payload, certificate_payload, binding_digest)
}

fn seal_owned_payload_v1(
    metadata: PrivateProjectionPayloadWireBindingMetadataV1,
    payload: SecretBufferV1,
    certificate_payload: SecretBufferV1,
    binding_digest: Digest,
) -> Result<PrivateProjectionPayloadWireV1, PrivateProjectionPayloadWireErrorV1> {
    seal_owned_payload_with_frame_v1(
        metadata,
        payload,
        certificate_payload,
        binding_digest,
        SecretBufferV1::new,
    )
}

fn seal_owned_payload_with_frame_v1(
    metadata: PrivateProjectionPayloadWireBindingMetadataV1,
    payload: SecretBufferV1,
    certificate_payload: SecretBufferV1,
    binding_digest: Digest,
    frame_owner: impl FnOnce(Vec<u8>) -> SecretBufferV1,
) -> Result<PrivateProjectionPayloadWireV1, PrivateProjectionPayloadWireErrorV1> {
    if payload.as_slice().len() > PRIVATE_PROJECTION_PAYLOAD_MAX_BYTES_V1 {
        return Err(PrivateProjectionPayloadWireErrorV1::PayloadTooLarge);
    }
    require_nonzero_digest(&binding_digest, "binding_digest")?;
    let payload_len = u32::try_from(payload.as_slice().len())
        .map_err(|_| PrivateProjectionPayloadWireErrorV1::PayloadTooLarge)?;
    let payload_digest = payload_digest_v1(payload.as_slice());
    let certificate_digest = certificate_digest_v1(certificate_payload.as_slice());
    let mut bytes = frame_owner(Vec::with_capacity(
        PRIVATE_PROJECTION_PAYLOAD_WIRE_HEADER_LEN_V1 + payload.as_slice().len() + 32,
    ));
    bytes
        .bytes
        .extend_from_slice(PRIVATE_PROJECTION_PAYLOAD_WIRE_MAGIC_V1);
    bytes
        .bytes
        .extend_from_slice(&PRIVATE_PROJECTION_PAYLOAD_WIRE_SCHEMA_VERSION_V1.to_be_bytes());
    bytes.bytes.extend_from_slice(
        &u16::try_from(PRIVATE_PROJECTION_PAYLOAD_WIRE_HEADER_LEN_V1)
            .expect("fixed payload header length fits u16")
            .to_be_bytes(),
    );
    bytes.bytes.extend_from_slice(&payload_len.to_be_bytes());
    bytes
        .bytes
        .extend_from_slice(&metadata.revision.to_be_bytes());
    bytes.bytes.extend_from_slice(&metadata.turn_id);
    bytes.bytes.extend_from_slice(&metadata.turn_binding);
    bytes.bytes.extend_from_slice(&metadata.projection_digest);
    bytes.bytes.extend_from_slice(&certificate_digest);
    bytes.bytes.extend_from_slice(&binding_digest);
    bytes.bytes.extend_from_slice(&payload_digest);
    if bytes.as_slice().len() != PRIVATE_PROJECTION_PAYLOAD_WIRE_HEADER_LEN_V1 {
        return Err(PrivateProjectionPayloadWireErrorV1::MalformedWire);
    }
    bytes.bytes.extend_from_slice(payload.as_slice());
    let wire_digest = wire_digest_v1(bytes.as_slice());
    bytes.bytes.extend_from_slice(&wire_digest);
    validate_canonical_framing_v1(bytes.as_slice())?;
    Ok(PrivateProjectionPayloadWireV1 {
        private_bytes: Some(bytes),
        metadata,
        wire_digest,
    })
}

/// Test-only fixture constructor for an internal binding-correct capability.
#[cfg(test)]
pub(crate) fn test_only_wire_for_metadata_v1(
    metadata: PrivateProjectionPayloadWireBindingMetadataV1,
) -> Result<PrivateProjectionPayloadWireV1, PrivateProjectionPayloadWireErrorV1> {
    seal_owned_payload_v1(
        metadata,
        SecretBufferV1::new(b"{}".to_vec()),
        SecretBufferV1::new(b"{}".to_vec()),
        [1; 32],
    )
}

#[cfg(test)]
pub(crate) fn test_only_wire_for_metadata_with_probe_v1(
    metadata: PrivateProjectionPayloadWireBindingMetadataV1,
    probe: TestOnlyZeroizationProbeV1,
) -> Result<PrivateProjectionPayloadWireV1, PrivateProjectionPayloadWireErrorV1> {
    let payload = SecretBufferV1::with_probe(b"{}".to_vec(), probe.clone());
    let certificate = SecretBufferV1::with_probe(b"{}".to_vec(), probe.clone());
    seal_owned_payload_with_frame_v1(metadata, payload, certificate, [1; 32], |bytes| {
        SecretBufferV1::with_probe(bytes, probe)
    })
}

#[cfg(test)]
pub(crate) fn test_only_tampered_wire_for_metadata_with_probe_v1(
    metadata: PrivateProjectionPayloadWireBindingMetadataV1,
    probe: TestOnlyZeroizationProbeV1,
) -> Result<PrivateProjectionPayloadWireV1, PrivateProjectionPayloadWireErrorV1> {
    let mut wire = test_only_wire_for_metadata_with_probe_v1(metadata, probe)?;
    wire.private_bytes
        .as_mut()
        .expect("test fixture starts live")
        .as_mut_slice()[168] ^= 1;
    Ok(wire)
}

#[cfg(test)]
pub(crate) fn test_only_post_allocation_seal_error_v1(
    metadata: PrivateProjectionPayloadWireBindingMetadataV1,
    probe: TestOnlyZeroizationProbeV1,
) -> Result<PrivateProjectionPayloadWireV1, PrivateProjectionPayloadWireErrorV1> {
    let mut wire = test_only_wire_for_metadata_with_probe_v1(metadata, probe)?;
    wire.private_bytes
        .as_mut()
        .expect("post-allocation fixture starts live")
        .as_mut_slice()[0] ^= 1;
    let error = wire
        .validate_live_canonical_v1()
        .expect_err("tampered allocated frame must fail validation");
    drop(wire);
    Err(error)
}

fn canonical_payload_from_envelope_v1(
    envelope: &impl serde::Serialize,
) -> Result<(SecretBufferV1, SecretBufferV1), PrivateProjectionPayloadWireErrorV1> {
    let mut body = serde_json::to_value(envelope)
        .map_err(|_| PrivateProjectionPayloadWireErrorV1::PayloadEncodingInvalid)?;
    normalize_payload_digests_v1(&mut body, None)?;
    let payload = SecretBufferV1::new(
        serde_json::to_vec(&body)
            .map_err(|_| PrivateProjectionPayloadWireErrorV1::PayloadEncodingInvalid)?,
    );
    if payload.as_slice().len() > PRIVATE_PROJECTION_PAYLOAD_MAX_BYTES_V1 {
        return Err(PrivateProjectionPayloadWireErrorV1::PayloadTooLarge);
    }
    let certificate = body
        .get("projection_certificate")
        .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadShape)?;
    let certificate_payload = SecretBufferV1::new(
        serde_json::to_vec(certificate)
            .map_err(|_| PrivateProjectionPayloadWireErrorV1::PayloadEncodingInvalid)?,
    );
    Ok((payload, certificate_payload))
}

fn normalize_payload_digests_v1(
    value: &mut Value,
    field: Option<&str>,
) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
    if field == Some("included_capsule_digests") {
        let values = value
            .as_array_mut()
            .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadShape)?;
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
        if decode_hex_v1::<32>(encoded).is_none() {
            return Err(PrivateProjectionPayloadWireErrorV1::UnsafePayloadValue);
        }
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

fn encode_hex_v1(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 15) as usize] as char);
    }
    output
}

/// Validates the fixed `AER7PPW1` header and all framing digests without
/// exposing a body or accepting a caller-provided output object.
pub(crate) fn validate_canonical_framing_v1(
    bytes: &[u8],
) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
    let minimum_len = PRIVATE_PROJECTION_PAYLOAD_WIRE_HEADER_LEN_V1 + 32;
    if bytes.len() < minimum_len
        || bytes.get(..8) != Some(PRIVATE_PROJECTION_PAYLOAD_WIRE_MAGIC_V1.as_slice())
        || read_u16(bytes, 8) != Some(PRIVATE_PROJECTION_PAYLOAD_WIRE_SCHEMA_VERSION_V1)
        || read_u16(bytes, 10).map(usize::from)
            != Some(PRIVATE_PROJECTION_PAYLOAD_WIRE_HEADER_LEN_V1)
    {
        return Err(PrivateProjectionPayloadWireErrorV1::MalformedWire);
    }
    let payload_len = read_u32(bytes, 12)
        .and_then(|length| usize::try_from(length).ok())
        .ok_or(PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    if payload_len > PRIVATE_PROJECTION_PAYLOAD_MAX_BYTES_V1 {
        return Err(PrivateProjectionPayloadWireErrorV1::PayloadTooLarge);
    }
    let payload_end = PRIVATE_PROJECTION_PAYLOAD_WIRE_HEADER_LEN_V1
        .checked_add(payload_len)
        .ok_or(PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    if bytes.len()
        != payload_end
            .checked_add(32)
            .ok_or(PrivateProjectionPayloadWireErrorV1::MalformedWire)?
    {
        return Err(PrivateProjectionPayloadWireErrorV1::MalformedWire);
    }
    let turn_id = read_id(bytes, 24).ok_or(PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    require_nonzero_id(&turn_id, "turn_id")?;
    let turn_binding =
        read_digest(bytes, 40).ok_or(PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    let projection_digest =
        read_digest(bytes, 72).ok_or(PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    let certificate_digest =
        read_digest(bytes, 104).ok_or(PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    let binding_digest =
        read_digest(bytes, 136).ok_or(PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    let payload_digest =
        read_digest(bytes, 168).ok_or(PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    for (field, digest) in [
        ("turn_binding", &turn_binding),
        ("projection_digest", &projection_digest),
        ("certificate_digest", &certificate_digest),
        ("binding_digest", &binding_digest),
        ("payload_digest", &payload_digest),
    ] {
        require_nonzero_digest(digest, field)?;
    }
    let payload = bytes
        .get(PRIVATE_PROJECTION_PAYLOAD_WIRE_HEADER_LEN_V1..payload_end)
        .ok_or(PrivateProjectionPayloadWireErrorV1::MalformedWire)?;
    if payload_digest != payload_digest_v1(payload) {
        return Err(PrivateProjectionPayloadWireErrorV1::DigestMismatch {
            field: "payload_digest",
        });
    }
    let expected_wire_digest = wire_digest_v1(&bytes[..payload_end]);
    if read_digest(bytes, payload_end) != Some(expected_wire_digest) {
        return Err(PrivateProjectionPayloadWireErrorV1::DigestMismatch {
            field: "wire_digest",
        });
    }
    Ok(())
}

pub(crate) fn payload_digest_v1(payload: &[u8]) -> Digest {
    wire::domain_hash(PRIVATE_PROJECTION_PAYLOAD_DOMAIN_V1, &[payload])
}

pub(crate) fn certificate_digest_v1(certificate_payload: &[u8]) -> Digest {
    wire::domain_hash(
        PRIVATE_PROJECTION_PAYLOAD_CERTIFICATE_DOMAIN_V1,
        &[certificate_payload],
    )
}

pub(crate) fn wire_digest_v1(framed_without_trailer: &[u8]) -> Digest {
    wire::domain_hash(
        PRIVATE_PROJECTION_PAYLOAD_WIRE_DOMAIN_V1,
        &[framed_without_trailer],
    )
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

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn read_id(bytes: &[u8], offset: usize) -> Option<Id128> {
    bytes.get(offset..offset + 16)?.try_into().ok()
}

fn read_digest(bytes: &[u8], offset: usize) -> Option<Digest> {
    bytes.get(offset..offset + 32)?.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_header_is_stable_and_native_discard_is_one_shot() {
        let metadata = PrivateProjectionPayloadWireBindingMetadataV1::new(
            7, [1; 16], [2; 32], [3; 32], [4; 32],
        )
        .expect("nonzero metadata");
        let probe = TestOnlyZeroizationProbeV1::default();
        let mut wire = test_only_wire_for_metadata_with_probe_v1(metadata, probe.clone())
            .expect("fixed canonical framing");
        let bytes: &[u8] = wire
            .private_bytes
            .as_ref()
            .expect("live private buffer")
            .as_slice();
        assert_eq!(&bytes[..8], PRIVATE_PROJECTION_PAYLOAD_WIRE_MAGIC_V1);
        assert_eq!(u16::from_be_bytes(bytes[8..10].try_into().unwrap()), 1);
        assert_eq!(u16::from_be_bytes(bytes[10..12].try_into().unwrap()), 200);
        assert_eq!(u64::from_be_bytes(bytes[16..24].try_into().unwrap()), 7);
        validate_canonical_framing_v1(bytes).expect("stable golden header validates");
        let transfer = wire
            .begin_transfer_once_v1()
            .expect("begin the exact native transfer once");
        assert!(matches!(
            wire.begin_transfer_once_v1(),
            Err(PrivateProjectionPayloadWireErrorV1::AlreadyConsumed)
        ));
        assert_eq!(
            discard_private_projection_transfer_v1(transfer),
            PrivateProjectionTransferReceiptV1::Discarded
        );
        probe.assert_zeroized_observations(3);
    }

    #[test]
    fn rejects_digest_tampering() {
        let metadata = PrivateProjectionPayloadWireBindingMetadataV1::new(
            7, [1; 16], [2; 32], [3; 32], [4; 32],
        )
        .expect("nonzero metadata");
        let probe = TestOnlyZeroizationProbeV1::default();
        let mut wire = test_only_wire_for_metadata_with_probe_v1(metadata, probe.clone())
            .expect("fixed canonical framing");
        wire.private_bytes
            .as_mut()
            .expect("live private buffer")
            .as_mut_slice()[168] ^= 1;
        assert!(matches!(
            wire.validate_live_canonical_v1(),
            Err(PrivateProjectionPayloadWireErrorV1::DigestMismatch { .. })
        ));
        drop(wire);
        probe.assert_zeroized_observations(3);
    }

    #[test]
    fn live_canonical_validation_rejects_a_preconsumed_wire() {
        let metadata = PrivateProjectionPayloadWireBindingMetadataV1::new(
            7, [1; 16], [2; 32], [3; 32], [4; 32],
        )
        .expect("nonzero metadata");
        let probe = TestOnlyZeroizationProbeV1::default();
        let mut wire = test_only_wire_for_metadata_with_probe_v1(metadata, probe.clone())
            .expect("fixed canonical framing");
        wire.validate_live_canonical_v1()
            .expect("unconsumed canonical wire is live");
        let transfer = wire
            .begin_transfer_once_v1()
            .expect("move the exact capability into its native transfer");
        assert!(matches!(
            wire.validate_live_canonical_v1(),
            Err(PrivateProjectionPayloadWireErrorV1::AlreadyConsumed)
        ));
        drop(transfer);
        probe.assert_zeroized_observations(3);
    }

    #[test]
    fn cancel_drop_and_post_allocation_error_zero_owned_buffers_before_release() {
        let metadata = PrivateProjectionPayloadWireBindingMetadataV1::new(
            7, [1; 16], [2; 32], [3; 32], [4; 32],
        )
        .expect("nonzero metadata");

        let cancel_probe = TestOnlyZeroizationProbeV1::default();
        let wire = test_only_wire_for_metadata_with_probe_v1(metadata, cancel_probe.clone())
            .expect("cancel fixture");
        assert_eq!(
            wire.cancel_v1(),
            PrivateProjectionTransferReceiptV1::Cancelled
        );
        cancel_probe.assert_zeroized_observations(3);

        let drop_probe = TestOnlyZeroizationProbeV1::default();
        let wire = test_only_wire_for_metadata_with_probe_v1(metadata, drop_probe.clone())
            .expect("drop fixture");
        drop(wire);
        drop_probe.assert_zeroized_observations(3);

        let error_probe = TestOnlyZeroizationProbeV1::default();
        assert!(matches!(
            test_only_post_allocation_seal_error_v1(metadata, error_probe.clone()),
            Err(PrivateProjectionPayloadWireErrorV1::MalformedWire)
        ));
        error_probe.assert_zeroized_observations(3);
    }
}
