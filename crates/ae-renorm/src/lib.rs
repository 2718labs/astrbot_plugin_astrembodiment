#![forbid(unsafe_code)]

use ae_contracts::{wire, Digest};
use ae_fixed::Fixed;
use ae_neurofield::{NeuralField, NEURON_SLOTS};
use serde::{Deserialize, Serialize};

pub const LEVELS: [usize; 4] = [16_384, 2_048, 256, 32];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub tokens: Vec<[Fixed; 8]>,
    pub consistency_residual: Fixed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenormPyramid {
    pub levels: [Vec<[Fixed; 8]>; 4],
    pub mapping_digest: Digest,
    pub consistency_residual: Fixed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceWinnerV1 {
    pub token_index: u8,
    pub score: Fixed,
    pub token: [Fixed; 8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenormError {
    InvalidField,
    InvalidLevelShape,
    InvalidWinner,
}

pub fn empty_workspace() -> Workspace {
    Workspace {
        tokens: vec![[Fixed::ZERO; 8]; LEVELS[3]],
        consistency_residual: Fixed::ZERO,
    }
}

fn l0_tokens(field: &NeuralField) -> Vec<[Fixed; 8]> {
    (0..NEURON_SLOTS)
        .map(|index| {
            [
                field.potential[index],
                field.excitation[index],
                field.inhibition[index],
                field.adaptation[index],
                field.precision[index],
                field.prediction_error[index],
                field.eligibility[index],
                field.metabolic_reserve[index],
            ]
        })
        .collect()
}

fn restrict_once(children: &[[Fixed; 8]]) -> Result<Vec<[Fixed; 8]>, RenormError> {
    if children.is_empty() || !children.len().is_multiple_of(8) {
        return Err(RenormError::InvalidLevelShape);
    }
    Ok(children
        .chunks_exact(8)
        .map(|group| {
            let mut token = [Fixed::ZERO; 8];
            for component in 0..8 {
                let sum = group
                    .iter()
                    .map(|child| i128::from(child[component].raw()))
                    .sum::<i128>();
                let mean = sum / 8;
                token[component] = Fixed::from_raw(i64::try_from(mean).unwrap_or(if mean < 0 {
                    i64::MIN
                } else {
                    i64::MAX
                }));
            }
            token
        })
        .collect())
}

fn level_residual(children: &[[Fixed; 8]], parents: &[[Fixed; 8]]) -> i64 {
    let mut maximum = 0i128;
    for (index, child) in children.iter().enumerate() {
        let parent = &parents[index / 8];
        for component in 0..8 {
            let difference =
                (i128::from(child[component].raw()) - i128::from(parent[component].raw())).abs();
            maximum = maximum.max(difference);
        }
    }
    i64::try_from(maximum).unwrap_or(i64::MAX)
}

pub fn restrict(
    field: &NeuralField,
    formula_digest: &Digest,
) -> Result<RenormPyramid, RenormError> {
    if !field.validate() {
        return Err(RenormError::InvalidField);
    }
    let l0 = l0_tokens(field);
    let l1 = restrict_once(&l0)?;
    let l2 = restrict_once(&l1)?;
    let l3 = restrict_once(&l2)?;
    if [l0.len(), l1.len(), l2.len(), l3.len()] != LEVELS {
        return Err(RenormError::InvalidLevelShape);
    }
    let residual = level_residual(&l0, &l1)
        .max(level_residual(&l1, &l2))
        .max(level_residual(&l2, &l3));
    let mut layout = Vec::with_capacity(LEVELS.len() * 8);
    for size in LEVELS {
        layout.extend_from_slice(&(size as u64).to_le_bytes());
    }
    Ok(RenormPyramid {
        levels: [l0, l1, l2, l3],
        mapping_digest: wire::domain_hash(b"ae.renorm.mapping.v1", &[formula_digest, &layout]),
        consistency_residual: Fixed::from_raw(residual),
    })
}

fn score(token: &[Fixed; 8]) -> Fixed {
    let positive = i128::from(token[1].raw())
        + i128::from(token[4].raw())
        + i128::from(token[5].raw())
        + i128::from(token[6].raw());
    let negative = i128::from(token[2].raw()) + i128::from(token[3].raw());
    let raw = (positive - negative) / 4;
    Fixed::from_raw(i64::try_from(raw).unwrap_or(if raw < 0 { i64::MIN } else { i64::MAX }))
}

pub fn compete(pyramid: &RenormPyramid, threshold: Fixed) -> Option<WorkspaceWinnerV1> {
    pyramid.levels[3]
        .iter()
        .enumerate()
        .map(|(index, token)| WorkspaceWinnerV1 {
            token_index: index as u8,
            score: score(token),
            token: *token,
        })
        .filter(|winner| winner.score >= threshold)
        .max_by(|left, right| {
            left.score
                .cmp(&right.score)
                .then_with(|| right.token_index.cmp(&left.token_index))
        })
}

pub fn prolong_transient(
    winner: &WorkspaceWinnerV1,
    field: &mut NeuralField,
) -> Result<(), RenormError> {
    if !field.validate() || usize::from(winner.token_index) >= LEVELS[3] {
        return Err(RenormError::InvalidWinner);
    }
    let receptive_width = LEVELS[0] / LEVELS[3];
    let start = usize::from(winner.token_index) * receptive_width;
    let end = start + receptive_width;
    let excitation_boost = winner.score.clamp(Fixed::ZERO, Fixed::ONE);
    let eligibility_boost = winner.token[6].clamp(Fixed::ZERO, Fixed::ONE);
    for index in start..end {
        field.excitation[index] = field.excitation[index].saturating_add(excitation_boost);
        field.eligibility[index] = field.eligibility[index].saturating_add(eligibility_boost);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ae_neurofield::{NeuralField, NEURON_SLOTS};

    fn patterned_field() -> NeuralField {
        let mut field = NeuralField::zeroed();
        for index in 0..NEURON_SLOTS {
            field.potential[index] = Fixed::from_raw((index % 97) as i64 * 1_000);
            field.excitation[index] = Fixed::from_raw((index % 31) as i64 * 500);
            field.eligibility[index] = Fixed::from_raw((index % 17) as i64 * 250);
        }
        field
    }

    #[test]
    fn restriction_has_exact_level_sizes() {
        let pyramid = restrict(&patterned_field(), &[9; 32]).unwrap();
        assert_eq!(pyramid.levels.each_ref().map(Vec::len), LEVELS);
    }

    #[test]
    fn mapping_digest_and_rebuild_are_deterministic() {
        let field = patterned_field();
        let first = restrict(&field, &[9; 32]).unwrap();
        let second = restrict(&field, &[9; 32]).unwrap();
        assert_eq!(first.mapping_digest, second.mapping_digest);
        assert_eq!(first.levels, second.levels);
        assert_eq!(first.consistency_residual, second.consistency_residual);
        assert_ne!(
            first.mapping_digest,
            restrict(&field, &[8; 32]).unwrap().mapping_digest
        );
    }

    #[test]
    fn prolongation_changes_transient_fields_only() {
        let mut field = patterned_field();
        let original = field.clone();
        let pyramid = restrict(&field, &[9; 32]).unwrap();
        let winner = compete(&pyramid, Fixed::ZERO).unwrap();
        prolong_transient(&winner, &mut field).unwrap();
        assert_ne!(field.excitation, original.excitation);
        assert_ne!(field.eligibility, original.eligibility);
        assert_eq!(field.potential, original.potential);
        assert_eq!(field.inhibition, original.inhibition);
        assert_eq!(field.adaptation, original.adaptation);
        assert_eq!(field.precision, original.precision);
        assert_eq!(field.prediction_error, original.prediction_error);
        assert_eq!(field.metabolic_reserve, original.metabolic_reserve);
    }
}
