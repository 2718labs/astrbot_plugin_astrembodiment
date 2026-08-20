#![forbid(unsafe_code)]

use ae_fixed::Fixed;
use serde::{Deserialize, Serialize};

pub const LEVELS: [usize; 4] = [16_384, 2_048, 256, 32];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub tokens: Vec<[Fixed; 8]>,
    pub consistency_residual: Fixed,
}

pub fn empty_workspace() -> Workspace {
    Workspace {
        tokens: vec![[Fixed::ZERO; 8]; LEVELS[3]],
        consistency_residual: Fixed::ZERO,
    }
}
