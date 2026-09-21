use crate::SemanticCoreError;
use ae_neurofield::{NeuralField, NEURON_SLOTS, REGION_LAYOUT};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PotentialRegionProjectionV1 {
    pub previous_mean_fxp6: [i64; REGION_LAYOUT.len()],
    pub current_mean_fxp6: [i64; REGION_LAYOUT.len()],
    pub delta_mean_fxp6: [i64; REGION_LAYOUT.len()],
}

fn mean_fxp6(sum: i128, count: usize) -> Result<i64, SemanticCoreError> {
    let divisor = i128::try_from(count)
        .ok()
        .filter(|value| *value > 0)
        .ok_or(SemanticCoreError::FieldStateInvalid)?;
    i64::try_from(sum / divisor).map_err(|_| SemanticCoreError::DynamicsInvalid)
}

/// Reduce the immutable predecessor and current 16K potential fields into the
/// frozen nine-region layout. No node value or node identifier crosses this
/// boundary.
pub fn potential_region_projection_v1(
    previous: &NeuralField,
    current: &NeuralField,
) -> Result<PotentialRegionProjectionV1, SemanticCoreError> {
    if !previous.validate() || !current.validate() {
        return Err(SemanticCoreError::FieldStateInvalid);
    }
    let mut previous_mean_fxp6 = [0_i64; REGION_LAYOUT.len()];
    let mut current_mean_fxp6 = [0_i64; REGION_LAYOUT.len()];
    let mut delta_mean_fxp6 = [0_i64; REGION_LAYOUT.len()];
    let mut expected_start = 0_usize;
    for (region, &(start, count)) in REGION_LAYOUT.iter().enumerate() {
        let end = start
            .checked_add(count)
            .filter(|end| start == expected_start && *end <= NEURON_SLOTS)
            .ok_or(SemanticCoreError::FieldStateInvalid)?;
        expected_start = end;
        let mut previous_sum = 0_i128;
        let mut current_sum = 0_i128;
        let mut delta_sum = 0_i128;
        for node in start..end {
            let before = i128::from(previous.potential[node].raw());
            let after = i128::from(current.potential[node].raw());
            previous_sum = previous_sum
                .checked_add(before)
                .ok_or(SemanticCoreError::DynamicsInvalid)?;
            current_sum = current_sum
                .checked_add(after)
                .ok_or(SemanticCoreError::DynamicsInvalid)?;
            delta_sum = delta_sum
                .checked_add(
                    after
                        .checked_sub(before)
                        .ok_or(SemanticCoreError::DynamicsInvalid)?,
                )
                .ok_or(SemanticCoreError::DynamicsInvalid)?;
        }
        previous_mean_fxp6[region] = mean_fxp6(previous_sum, count)?;
        current_mean_fxp6[region] = mean_fxp6(current_sum, count)?;
        delta_mean_fxp6[region] = mean_fxp6(delta_sum, count)?;
    }
    if expected_start != NEURON_SLOTS {
        return Err(SemanticCoreError::FieldStateInvalid);
    }
    Ok(PotentialRegionProjectionV1 {
        previous_mean_fxp6,
        current_mean_fxp6,
        delta_mean_fxp6,
    })
}
