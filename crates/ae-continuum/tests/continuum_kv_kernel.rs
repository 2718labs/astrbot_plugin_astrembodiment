#![allow(non_snake_case)]

use ae_continuum::r7::{
    key_digest, validate_canonical_value_len, validate_key, validate_scan_limit, validate_value,
    value_digest, ContinuumKey, ContinuumObjectKind, ContinuumValidationError, VersionedValue,
    KEY_DOMAIN, MAX_CANONICAL_VALUE_BYTES, VALUE_DOMAIN,
};
use ae_contracts::r7::{wire::domain_hash, Digest};

fn digest(hex: &str) -> Digest {
    assert_eq!(hex.len(), 64);
    let mut output = [0_u8; 32];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).unwrap();
    }
    output
}

fn key(scope_digest: Digest, kind: ContinuumObjectKind, logical_id: Vec<u8>) -> ContinuumKey {
    ContinuumKey {
        scope_digest,
        kind,
        logical_id,
    }
}

#[test]
fn K01_domain_hash_zero_fields_vector() {
    assert_eq!(
        domain_hash(VALUE_DOMAIN, &[]),
        digest("bedf2529bc4c9ca14ec40b9f34eef0fef5a826c5bc451333a28ea3e76679d8c7")
    );
}

#[test]
fn K02_empty_field_is_not_zero_fields() {
    let zero_fields = domain_hash(VALUE_DOMAIN, &[]);
    let one_empty_field = domain_hash(VALUE_DOMAIN, &[&[]]);
    assert_eq!(
        one_empty_field,
        digest("92e73ee3672f2ecc2b691aaaf5a161dc4b3d5a3aa81033d58b98505a65184081")
    );
    assert_ne!(zero_fields, one_empty_field);
}

#[test]
fn K03_empty_and_single_byte_value_vectors() {
    assert_eq!(
        value_digest(&[]).unwrap(),
        digest("92e73ee3672f2ecc2b691aaaf5a161dc4b3d5a3aa81033d58b98505a65184081")
    );
    assert_eq!(
        value_digest(&[0]).unwrap(),
        digest("7248d90b1e7a1033d4456e3d8965efa6272eb9ce2b5b69b3809fd08f8195d0bc")
    );
}

#[test]
fn K04_key_vectors_and_u16_little_endian_kind() {
    let snapshot = key([0; 32], ContinuumObjectKind::Snapshot, vec![0]);
    assert_eq!(
        key_digest(&snapshot).unwrap(),
        digest("c462b2e625b0e5a854fb8825118fabffd50f7865f522728a84dca9f2926956f3")
    );

    let delta = key(
        core::array::from_fn(|index| index as u8),
        ContinuumObjectKind::Delta,
        b"id".to_vec(),
    );
    assert_eq!(ContinuumObjectKind::Snapshot as u16, 1);
    assert_eq!(ContinuumObjectKind::Delta as u16, 2);
    assert_eq!(
        key_digest(&delta).unwrap(),
        digest("4e12a27e38f3cbc49aa864adb271a071cdcef33a9ef427bc61e4874db759f78c")
    );
}

#[test]
fn K05_logical_id_is_binary_and_exact() {
    let scope = [7; 32];
    let logical_id = vec![0xff, 0, b'I', b'D'];
    let binary_key = key(scope, ContinuumObjectKind::ActiveHead, logical_id.clone());
    let kind = (ContinuumObjectKind::ActiveHead as u16).to_le_bytes();
    assert_eq!(
        key_digest(&binary_key).unwrap(),
        domain_hash(KEY_DOMAIN, &[&scope, &kind, &logical_id])
    );
    assert_ne!(
        key_digest(&binary_key).unwrap(),
        key_digest(&key(scope, ContinuumObjectKind::ActiveHead, b"ID".to_vec())).unwrap()
    );
}

#[test]
fn K06_empty_logical_id_rejected() {
    let empty = key([0; 32], ContinuumObjectKind::Snapshot, vec![]);
    assert_eq!(
        validate_key(&empty),
        Err(ContinuumValidationError::EmptyLogicalId)
    );
    assert_eq!(
        key_digest(&empty),
        Err(ContinuumValidationError::EmptyLogicalId)
    );
}

#[test]
fn K07_257_byte_logical_id_rejected() {
    let too_long = key([0; 32], ContinuumObjectKind::Snapshot, vec![0; 257]);
    assert_eq!(
        validate_key(&too_long),
        Err(ContinuumValidationError::LogicalIdTooLong { actual: 257 })
    );
}

#[test]
fn K08_empty_value_is_valid() {
    let value = VersionedValue {
        revision: 1,
        value_digest: value_digest(&[]).unwrap(),
        canonical_bytes: vec![],
    };
    assert_eq!(validate_value(&value), Ok(()));
}

#[test]
fn K09_value_limit_and_limit_plus_one() {
    assert_eq!(
        validate_canonical_value_len(MAX_CANONICAL_VALUE_BYTES),
        Ok(())
    );
    assert_eq!(
        validate_canonical_value_len(MAX_CANONICAL_VALUE_BYTES + 1),
        Err(ContinuumValidationError::CanonicalValueTooLong {
            actual: MAX_CANONICAL_VALUE_BYTES + 1,
        })
    );
}

#[test]
fn K10_declared_value_digest_mismatch_rejected() {
    let value = VersionedValue {
        revision: 1,
        value_digest: [0; 32],
        canonical_bytes: b"canonical".to_vec(),
    };
    assert_eq!(
        validate_value(&value),
        Err(ContinuumValidationError::ValueDigestMismatch)
    );
}

#[test]
fn K11_scan_limit_zero_and_4097_rejected() {
    assert_eq!(
        validate_scan_limit(0),
        Err(ContinuumValidationError::InvalidScanLimit { actual: 0 })
    );
    assert_eq!(
        validate_scan_limit(4097),
        Err(ContinuumValidationError::InvalidScanLimit { actual: 4097 })
    );
}

#[test]
fn K12_scan_limit_1_and_4096_accepted() {
    assert_eq!(validate_scan_limit(1), Ok(()));
    assert_eq!(validate_scan_limit(4096), Ok(()));
}
