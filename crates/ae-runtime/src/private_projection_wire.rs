//! Canonical opaque `AER7PPW1` transport framing.
//!
//! This private runtime module owns the fixed payload-wire bytes, framing
//! metadata, and one-shot capability. No external crate can mint a wire or
//! inspect its binding/header state.

use ae_cognitive_envelope::{CognitiveEnvelopeV1, PreOutputCognitiveEnvelopeV1};
use ae_contracts::{wire, Digest, Id128};
use serde_json::Value;
use thiserror::Error;

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
pub enum PrivateProjectionPayloadWireErrorV1 {
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
/// body, text, JSON, envelope, or reusable byte getter. The sole transfer is
/// `consume_once`, which moves the capability to its exact native bridge.
///
/// ```compile_fail
/// use ae_runtime::PrivateProjectionPayloadWireV1;
/// use std::fmt::Debug;
/// fn require_debug<T: Debug>() {}
/// require_debug::<PrivateProjectionPayloadWireV1>();
/// ```
pub struct PrivateProjectionPayloadWireV1 {
    private_bytes: Option<Box<[u8]>>,
    metadata: PrivateProjectionPayloadWireBindingMetadataV1,
    wire_digest: Digest,
}

impl PrivateProjectionPayloadWireV1 {
    /// Confirms that this capability is both unconsumed and still encoded with
    /// the exact canonical `AER7PPW1` framing. It reveals no payload bytes.
    pub(crate) fn validate_live_canonical_v1(
        &self,
    ) -> Result<(), PrivateProjectionPayloadWireErrorV1> {
        let bytes = self
            .private_bytes
            .as_deref()
            .ok_or(PrivateProjectionPayloadWireErrorV1::AlreadyConsumed)?;
        validate_canonical_framing_v1(bytes)
    }

    pub(crate) fn binding_metadata(&self) -> &PrivateProjectionPayloadWireBindingMetadataV1 {
        &self.metadata
    }

    pub fn wire_digest(&self) -> &Digest {
        &self.wire_digest
    }

    pub fn consume_once(&mut self) -> Result<Box<[u8]>, PrivateProjectionPayloadWireErrorV1> {
        self.private_bytes
            .take()
            .ok_or(PrivateProjectionPayloadWireErrorV1::AlreadyConsumed)
    }
}

/// Canonically seals a typed full cognitive envelope. Callers cannot supply
/// raw payload bytes, header fields, or a direct output constructor.
pub(crate) fn seal_cognitive_envelope_v1(
    metadata: PrivateProjectionPayloadWireBindingMetadataV1,
    envelope: &CognitiveEnvelopeV1,
    binding_digest: Digest,
) -> Result<PrivateProjectionPayloadWireV1, PrivateProjectionPayloadWireErrorV1> {
    let (payload, certificate_payload) = canonical_payload_from_envelope_v1(envelope)?;
    seal_canonical_payload_v1(metadata, payload, certificate_payload, binding_digest)
}

/// Canonically seals a typed pre-output cognitive envelope. It uses the same
/// stable `AER7PPW1` header bytes as the full envelope path.
pub(crate) fn seal_pre_output_cognitive_envelope_v1(
    metadata: PrivateProjectionPayloadWireBindingMetadataV1,
    envelope: &PreOutputCognitiveEnvelopeV1,
    binding_digest: Digest,
) -> Result<PrivateProjectionPayloadWireV1, PrivateProjectionPayloadWireErrorV1> {
    let (payload, certificate_payload) = canonical_payload_from_envelope_v1(envelope)?;
    seal_canonical_payload_v1(metadata, payload, certificate_payload, binding_digest)
}

fn seal_canonical_payload_v1(
    metadata: PrivateProjectionPayloadWireBindingMetadataV1,
    payload: Vec<u8>,
    certificate_payload: Vec<u8>,
    binding_digest: Digest,
) -> Result<PrivateProjectionPayloadWireV1, PrivateProjectionPayloadWireErrorV1> {
    if payload.len() > PRIVATE_PROJECTION_PAYLOAD_MAX_BYTES_V1 {
        return Err(PrivateProjectionPayloadWireErrorV1::PayloadTooLarge);
    }
    require_nonzero_digest(&binding_digest, "binding_digest")?;
    let payload_len = u32::try_from(payload.len())
        .map_err(|_| PrivateProjectionPayloadWireErrorV1::PayloadTooLarge)?;
    let payload_digest = payload_digest_v1(&payload);
    let certificate_digest = certificate_digest_v1(&certificate_payload);
    let mut bytes =
        Vec::with_capacity(PRIVATE_PROJECTION_PAYLOAD_WIRE_HEADER_LEN_V1 + payload.len() + 32);
    bytes.extend_from_slice(PRIVATE_PROJECTION_PAYLOAD_WIRE_MAGIC_V1);
    bytes.extend_from_slice(&PRIVATE_PROJECTION_PAYLOAD_WIRE_SCHEMA_VERSION_V1.to_be_bytes());
    bytes.extend_from_slice(
        &u16::try_from(PRIVATE_PROJECTION_PAYLOAD_WIRE_HEADER_LEN_V1)
            .expect("fixed payload header length fits u16")
            .to_be_bytes(),
    );
    bytes.extend_from_slice(&payload_len.to_be_bytes());
    bytes.extend_from_slice(&metadata.revision.to_be_bytes());
    bytes.extend_from_slice(&metadata.turn_id);
    bytes.extend_from_slice(&metadata.turn_binding);
    bytes.extend_from_slice(&metadata.projection_digest);
    bytes.extend_from_slice(&certificate_digest);
    bytes.extend_from_slice(&binding_digest);
    bytes.extend_from_slice(&payload_digest);
    if bytes.len() != PRIVATE_PROJECTION_PAYLOAD_WIRE_HEADER_LEN_V1 {
        return Err(PrivateProjectionPayloadWireErrorV1::MalformedWire);
    }
    bytes.extend_from_slice(&payload);
    let wire_digest = wire_digest_v1(&bytes);
    bytes.extend_from_slice(&wire_digest);
    validate_canonical_framing_v1(&bytes)?;
    Ok(PrivateProjectionPayloadWireV1 {
        private_bytes: Some(bytes.into_boxed_slice()),
        metadata,
        wire_digest,
    })
}

/// Test-only fixture constructor for an internal binding-correct capability.
#[cfg(test)]
pub(crate) fn test_only_wire_for_metadata_v1(
    metadata: PrivateProjectionPayloadWireBindingMetadataV1,
) -> Result<PrivateProjectionPayloadWireV1, PrivateProjectionPayloadWireErrorV1> {
    seal_canonical_payload_v1(metadata, b"{}".to_vec(), b"{}".to_vec(), [1; 32])
}

/// Internal regression fixture only. The capability preserves its private
/// metadata but has noncanonical framing, so runtime validation must reject it
/// before any semantic field/revision commit.
#[cfg(test)]
pub(crate) fn test_only_tampered_wire_for_metadata_v1(
    metadata: PrivateProjectionPayloadWireBindingMetadataV1,
) -> Result<PrivateProjectionPayloadWireV1, PrivateProjectionPayloadWireErrorV1> {
    let mut wire = test_only_wire_for_metadata_v1(metadata)?;
    let bytes = wire
        .private_bytes
        .as_deref_mut()
        .expect("test fixture starts live");
    bytes[168] ^= 1;
    Ok(wire)
}

fn canonical_payload_from_envelope_v1(
    envelope: &impl serde::Serialize,
) -> Result<(Vec<u8>, Vec<u8>), PrivateProjectionPayloadWireErrorV1> {
    let mut body = serde_json::to_value(envelope)
        .map_err(|_| PrivateProjectionPayloadWireErrorV1::PayloadEncodingInvalid)?;
    normalize_payload_digests_v1(&mut body, None)?;
    let payload = serde_json::to_vec(&body)
        .map_err(|_| PrivateProjectionPayloadWireErrorV1::PayloadEncodingInvalid)?;
    if payload.len() > PRIVATE_PROJECTION_PAYLOAD_MAX_BYTES_V1 {
        return Err(PrivateProjectionPayloadWireErrorV1::PayloadTooLarge);
    }
    let certificate = body
        .get("projection_certificate")
        .ok_or(PrivateProjectionPayloadWireErrorV1::UnsafePayloadShape)?;
    let certificate_payload = serde_json::to_vec(certificate)
        .map_err(|_| PrivateProjectionPayloadWireErrorV1::PayloadEncodingInvalid)?;
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
    fn golden_header_is_stable_and_one_shot() {
        let metadata = PrivateProjectionPayloadWireBindingMetadataV1::new(
            7, [1; 16], [2; 32], [3; 32], [4; 32],
        )
        .expect("nonzero metadata");
        let mut wire = seal_canonical_payload_v1(metadata, b"{}".to_vec(), b"{}".to_vec(), [5; 32])
            .expect("fixed canonical framing");
        let bytes = wire.consume_once().expect("first consumption");
        assert_eq!(&bytes[..8], PRIVATE_PROJECTION_PAYLOAD_WIRE_MAGIC_V1);
        assert_eq!(u16::from_be_bytes(bytes[8..10].try_into().unwrap()), 1);
        assert_eq!(u16::from_be_bytes(bytes[10..12].try_into().unwrap()), 200);
        assert_eq!(u64::from_be_bytes(bytes[16..24].try_into().unwrap()), 7);
        validate_canonical_framing_v1(&bytes).expect("stable golden header validates");
        assert!(matches!(
            wire.consume_once(),
            Err(PrivateProjectionPayloadWireErrorV1::AlreadyConsumed)
        ));
    }

    #[test]
    fn rejects_digest_tampering() {
        let metadata = PrivateProjectionPayloadWireBindingMetadataV1::new(
            7, [1; 16], [2; 32], [3; 32], [4; 32],
        )
        .expect("nonzero metadata");
        let mut wire = seal_canonical_payload_v1(metadata, b"{}".to_vec(), b"{}".to_vec(), [5; 32])
            .expect("fixed canonical framing");
        let mut bytes = wire.consume_once().expect("wire bytes").into_vec();
        bytes[168] ^= 1;
        assert!(matches!(
            validate_canonical_framing_v1(&bytes),
            Err(PrivateProjectionPayloadWireErrorV1::DigestMismatch { .. })
        ));
    }

    #[test]
    fn live_canonical_validation_rejects_a_preconsumed_wire() {
        let metadata = PrivateProjectionPayloadWireBindingMetadataV1::new(
            7, [1; 16], [2; 32], [3; 32], [4; 32],
        )
        .expect("nonzero metadata");
        let mut wire = seal_canonical_payload_v1(metadata, b"{}".to_vec(), b"{}".to_vec(), [5; 32])
            .expect("fixed canonical framing");
        wire.validate_live_canonical_v1()
            .expect("unconsumed canonical wire is live");
        let _ = wire.consume_once().expect("consume the exact capability");
        assert!(matches!(
            wire.validate_live_canonical_v1(),
            Err(PrivateProjectionPayloadWireErrorV1::AlreadyConsumed)
        ));
    }
}
