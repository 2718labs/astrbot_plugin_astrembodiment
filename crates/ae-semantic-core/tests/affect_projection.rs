use ae_fixed::Fixed;
use ae_neurofield::{NeuralField, REGION_LAYOUT};
use ae_semantic_core::potential_region_projection_v1;

#[test]
fn potential_projection_reduces_exact_fixed_regions_without_node_output() {
    let mut previous = NeuralField::zeroed();
    let mut current = NeuralField::zeroed();
    for (region, (start, count)) in REGION_LAYOUT.into_iter().enumerate() {
        for node in start..start + count {
            previous.potential[node] = Fixed::from_raw(100_000 + region as i64);
            current.potential[node] = Fixed::from_raw(200_000 + region as i64 * 2);
        }
    }

    let projection = potential_region_projection_v1(&previous, &current).unwrap();
    assert_eq!(
        projection.previous_mean_fxp6,
        std::array::from_fn(|i| 100_000 + i as i64)
    );
    assert_eq!(
        projection.current_mean_fxp6,
        std::array::from_fn(|i| 200_000 + i as i64 * 2)
    );
    assert_eq!(
        projection.delta_mean_fxp6,
        std::array::from_fn(|i| 100_000 + i as i64)
    );

    let mut rounding_previous = NeuralField::zeroed();
    let mut rounding_current = NeuralField::zeroed();
    let (start, count) = REGION_LAYOUT[0];
    for node in start..start + count {
        rounding_previous.potential[node] = Fixed::from_raw(1);
        rounding_current.potential[node] = Fixed::from_raw(1);
    }
    rounding_previous.potential[start] = Fixed::ZERO;
    let rounding = potential_region_projection_v1(&rounding_previous, &rounding_current).unwrap();
    assert_eq!(rounding.previous_mean_fxp6[0], 0);
    assert_eq!(rounding.current_mean_fxp6[0], 1);
    assert_eq!(rounding.delta_mean_fxp6[0], 0);
}
