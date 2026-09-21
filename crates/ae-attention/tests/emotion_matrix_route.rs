use ae_attention::emotion_matrix::{assemble_full_vector_load, FullVectorLoadError};
use ae_contracts::emotion_matrix::{
    evidence_vector_from_values, phase0_semantic_route_digest_v1, PHASE0_SEMANTIC_ROUTE_RULES_V1,
};
use ae_contracts::EvidenceVector;
use ae_fixed::Fixed;
use std::collections::BTreeSet;

#[test]
fn all_fifteen_dimensions_route_through_one_frozen_contract() {
    let values = [
        50_003, 110_009, 170_021, 230_033, 290_047, 350_059, 410_071, 470_087, 530_101, 590_119,
        650_137, 710_159, 770_183, 830_211, 890_243,
    ]
    .map(Fixed::from_raw);
    assert_eq!(
        values
            .iter()
            .map(|value| value.raw())
            .collect::<BTreeSet<_>>()
            .len(),
        15
    );

    let load = assemble_full_vector_load(&evidence_vector_from_values(values))
        .expect("all fifteen bounded FXP6 dimensions must route");
    assert_eq!(load.evaluated_dimension_count, 15);
    assert_eq!(load.injected_dimension_count, 15);
    assert_eq!(load.route_digest, phase0_semantic_route_digest_v1());
    // Hand-computed from the fifteen frozen primary/secondary routes. Odd raw
    // values deliberately expose FXP6 truncation instead of permitting a
    // global-average or `1 - evidence_mean` shortcut.
    assert_eq!(
        load.evidence_means,
        [530_132, 80_006, 410_072, 470_088, 612_638, 455_092, 410_071, 620_134, 422_095,]
            .map(Fixed::from_raw)
    );
    assert_eq!(
        load.neutral_means,
        [469_868, 919_994, 589_927, 529_911, 387_361, 544_906, 589_929, 379_864, 577_903,]
            .map(Fixed::from_raw)
    );
    assert_ne!(
        load.neutral_means,
        load.evidence_means
            .map(|value| Fixed::ONE.saturating_sub(value))
    );

    let mut covered_regions = [false; 9];
    for rule in PHASE0_SEMANTIC_ROUTE_RULES_V1 {
        covered_regions[usize::from(rule.primary)] = true;
        if let Some(secondary) = rule.secondary {
            covered_regions[usize::from(secondary)] = true;
        }
    }
    assert!(covered_regions.into_iter().all(|covered| covered));

    let neutral = assemble_full_vector_load(&EvidenceVector::default())
        .expect("literal zero is valid neutral evidence");
    assert_eq!(neutral.evaluated_dimension_count, 15);
    assert_eq!(neutral.injected_dimension_count, 15);
    assert_eq!(neutral.evidence_means, [Fixed::ZERO; 9]);
    assert_eq!(neutral.neutral_means, [Fixed::ONE; 9]);

    let mut invalid = EvidenceVector::default();
    invalid.hostility = Fixed::from_raw(1_000_001);
    assert_eq!(
        assemble_full_vector_load(&invalid),
        Err(FullVectorLoadError::InvalidDimension)
    );
}
