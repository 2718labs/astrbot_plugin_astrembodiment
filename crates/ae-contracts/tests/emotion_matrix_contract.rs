use ae_contracts::emotion_matrix::{
    evidence_vector_from_values, perception_dimension_values, phase0_semantic_route_digest_v1,
    PerceptionProposalErrorV1, PerceptionProposalV1,
    PHASE0_SEMANTIC_ROUTE_PRIMARY_COEFFICIENT_FXP6, PHASE0_SEMANTIC_ROUTE_RULES_V1,
    PHASE0_SEMANTIC_ROUTE_SECONDARY_COEFFICIENT_FXP6,
};
use ae_fixed::Fixed;

#[test]
fn all_fifteen_dimensions_have_frozen_order_and_routes() {
    let values = [
        Fixed::from_raw(1),
        Fixed::from_raw(2),
        Fixed::from_raw(3),
        Fixed::from_raw(4),
        Fixed::from_raw(5),
        Fixed::from_raw(6),
        Fixed::from_raw(7),
        Fixed::from_raw(8),
        Fixed::from_raw(9),
        Fixed::from_raw(10),
        Fixed::from_raw(11),
        Fixed::from_raw(12),
        Fixed::from_raw(13),
        Fixed::from_raw(14),
        Fixed::from_raw(15),
    ];
    let evidence = evidence_vector_from_values(values);
    assert_eq!(perception_dimension_values(&evidence), values);

    let routes = PHASE0_SEMANTIC_ROUTE_RULES_V1.map(|rule| (rule.primary, rule.secondary));
    assert_eq!(
        routes,
        [
            (1, Some(8)),
            (1, Some(8)),
            (0, Some(5)),
            (4, Some(5)),
            (3, Some(8)),
            (2, Some(7)),
            (6, Some(2)),
            (2, Some(3)),
            (3, Some(7)),
            (3, Some(7)),
            (4, Some(7)),
            (5, Some(4)),
            (4, Some(7)),
            (8, Some(7)),
            (0, Some(4)),
        ]
    );
    assert_eq!(
        PHASE0_SEMANTIC_ROUTE_PRIMARY_COEFFICIENT_FXP6.raw(),
        1_000_000
    );
    assert_eq!(
        PHASE0_SEMANTIC_ROUTE_SECONDARY_COEFFICIENT_FXP6.raw(),
        500_000
    );
    assert_eq!(
        phase0_semantic_route_digest_v1(),
        [
            0x1e, 0x88, 0x67, 0x06, 0x35, 0x54, 0xb2, 0xfa, 0xac, 0xa9, 0x80, 0xa9, 0xcd, 0x80,
            0x4c, 0xe8, 0x95, 0x01, 0xfc, 0x6c, 0x87, 0x0a, 0x77, 0x07, 0xfe, 0xb2, 0x90, 0xc2,
            0x65, 0xcb, 0xa7, 0x65,
        ]
    );

    let valid = PerceptionProposalV1 {
        schema_version: PerceptionProposalV1::SCHEMA_VERSION,
        origin_digest: [1; 32],
        dimensions: evidence,
        estimator_confidence: Fixed::from_raw(500_000),
        protocol_version: PerceptionProposalV1::PROTOCOL_VERSION,
        request_nonce_digest: [3; 32],
    };
    assert_eq!(valid.validate_v1(), Ok(()));

    for dimension in 0..values.len() {
        for boundary in [Fixed::ZERO, Fixed::ONE] {
            let mut bounded_values = values;
            bounded_values[dimension] = boundary;
            let mut bounded = valid.clone();
            bounded.dimensions = evidence_vector_from_values(bounded_values);
            assert_eq!(
                bounded.validate_v1(),
                Ok(()),
                "dimension {dimension} must include bound {}",
                boundary.raw()
            );
        }

        let mut below_values = values;
        below_values[dimension] = Fixed::from_raw(-1);
        let mut below = valid.clone();
        below.dimensions = evidence_vector_from_values(below_values);
        assert_eq!(
            below.validate_v1(),
            Err(PerceptionProposalErrorV1::InvalidDimensions),
            "dimension {dimension} must reject values below zero"
        );

        let mut above_values = values;
        above_values[dimension] = Fixed::from_raw(1_000_001);
        let mut above = valid.clone();
        above.dimensions = evidence_vector_from_values(above_values);
        assert_eq!(
            above.validate_v1(),
            Err(PerceptionProposalErrorV1::InvalidDimensions),
            "dimension {dimension} must reject values above one"
        );
    }
}
