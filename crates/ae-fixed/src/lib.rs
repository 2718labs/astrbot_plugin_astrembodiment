#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::fmt;

pub const SCALE: i64 = 1_000_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Fixed(i64);

impl Fixed {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(SCALE);

    pub const fn from_raw(raw: i64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> i64 {
        self.0
    }

    pub fn from_ratio(numerator: i64, denominator: i64) -> Option<Self> {
        if denominator == 0 {
            return None;
        }
        let scaled = i128::from(numerator) * i128::from(SCALE);
        let value = scaled / i128::from(denominator);
        i64::try_from(value).ok().map(Self)
    }

    pub fn saturating_add(self, rhs: Self) -> Self {
        Self(self.0.saturating_add(rhs.0))
    }

    pub fn saturating_sub(self, rhs: Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }

    pub fn checked_mul(self, rhs: Self) -> Option<Self> {
        let product = i128::from(self.0) * i128::from(rhs.0);
        let scaled = product / i128::from(SCALE);
        i64::try_from(scaled).ok().map(Self)
    }

    pub fn clamp(self, min: Self, max: Self) -> Self {
        Self(self.0.clamp(min.0, max.0))
    }
}

impl fmt::Display for Fixed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let absolute = self.0.unsigned_abs();
        write!(
            f,
            "{}{}.{:06}",
            sign,
            absolute / SCALE as u64,
            absolute % SCALE as u64
        )
    }
}
