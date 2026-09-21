use ae_continuum::{
    key_digest, value_digest, ContinuumKey, ContinuumObjectKind, NativeContinuumKv,
    VerifiedKvReferenceV1, VersionedValue,
};

fn key() -> ContinuumKey {
    ContinuumKey {
        scope_digest: [7; 32],
        kind: ContinuumObjectKind::Snapshot,
        logical_id: b"canonical-state".to_vec(),
    }
}

fn value() -> VersionedValue {
    let bytes = b"canonical-state-bytes".to_vec();
    VersionedValue {
        revision: 4,
        value_digest: value_digest(&bytes).unwrap(),
        canonical_bytes: bytes,
    }
}

#[test]
fn verifier_recomputes_digest_and_preserves_only_kv_revision() {
    let verified: VerifiedKvReferenceV1 =
        NativeContinuumKv::verify_native_kv_reference_v1(&key(), &value()).unwrap();
    assert_eq!(verified.key_digest, key_digest(&key()).unwrap());
    assert_eq!(verified.value_digest, value().value_digest);
    assert_eq!(verified.canonical_value_digest, value().value_digest);
    assert_eq!(
        verified.canonical_value_len,
        value().canonical_bytes.len() as u64
    );
    assert_eq!(verified.kv_stream_revision, 4);
}

#[test]
fn verifier_rejects_forged_value_digest_without_an_n1_revision_argument() {
    let mut forged = value();
    forged.value_digest = [0; 32];
    assert!(NativeContinuumKv::verify_native_kv_reference_v1(&key(), &forged).is_err());
}
