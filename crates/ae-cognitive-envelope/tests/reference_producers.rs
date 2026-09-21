use ae_cognitive_envelope::{
    BoundedTextV1, ProjectionSourceKindV1, ProviderProfileV1, SourceCapsuleV1, SourceProvenanceV1,
};

fn digest(seed: u8) -> [u8; 32] {
    [seed; 32]
}

#[test]
fn reference_capsule_replay_preserves_canonical_digest() {
    let value =
        ProviderProfileV1::new(BoundedTextV1::new("r7".to_string(), 32).unwrap(), 128).unwrap();
    let provenance = SourceProvenanceV1::new(
        ProjectionSourceKindV1::ProviderProfile,
        BoundedTextV1::new("native.session.turn".to_string(), 128).unwrap(),
        41,
        digest(7),
    )
    .unwrap();
    let first = SourceCapsuleV1::new(provenance.clone(), digest(8), value.clone()).unwrap();
    let replay = SourceCapsuleV1::new(provenance, digest(8), value).unwrap();
    assert_ne!(*first.content_digest(), [0; 32]);
    assert_eq!(first.capsule_digest(), replay.capsule_digest());
}

#[test]
fn reference_producer_rejects_zero_source_digest() {
    let value =
        ProviderProfileV1::new(BoundedTextV1::new("r7".to_string(), 32).unwrap(), 128).unwrap();
    let provenance = SourceProvenanceV1::new(
        ProjectionSourceKindV1::ProviderProfile,
        BoundedTextV1::new("native.session.turn".to_string(), 128).unwrap(),
        41,
        digest(7),
    )
    .unwrap();
    assert!(SourceCapsuleV1::new(provenance, [0; 32], value).is_err());
}
