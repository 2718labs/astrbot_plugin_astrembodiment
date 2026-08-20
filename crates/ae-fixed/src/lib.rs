#![forbid(unsafe_code)]

//! Fixed-point scalar, 1e-6 scale ("fxp6-i64"), the only numeric ABI used by
//! the ASTER-CCN kernel.
//!
//! Wire form is the raw little-endian i64. JSON round-trips use the raw
//! integer only; decimal floats are rejected at the Python boundary and must
//! be converted with explicit rounding before entering Rust.

use serde::{Deserialize, Serialize};
use std::fmt;

pub const SCALE: i64 = 1_000_000;

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
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

    pub fn checked_div(self, rhs: Self) -> Option<Self> {
        if rhs.0 == 0 {
            return None;
        }
        let numerator = i128::from(self.0) * i128::from(SCALE);
        let quotient = numerator / i128::from(rhs.0);
        i64::try_from(quotient).ok().map(Self)
    }

    pub fn checked_neg(self) -> Option<Self> {
        self.0.checked_neg().map(Self)
    }

    pub fn clamp(self, min: Self, max: Self) -> Self {
        Self(self.0.clamp(min.0, max.0))
    }

    /// Canonical wire encoding: raw value, little-endian.
    pub fn encode(self) -> [u8; 8] {
        self.0.to_le_bytes()
    }

    /// Canonical wire decoding: raw value, little-endian.
    pub fn decode(bytes: [u8; 8]) -> Self {
        Self(i64::from_le_bytes(bytes))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_round_trip() {
        for raw in [0, 1, -1, SCALE, -SCALE, i64::MIN, i64::MAX, 123_456_789] {
            let value = Fixed::from_raw(raw);
            assert_eq!(Fixed::decode(value.encode()), value);
        }
    }

    #[test]
    fn wire_is_little_endian() {
        assert_eq!(
            Fixed::from_raw(0x0102_0304_0506_0708).encode()[..4],
            [0x08, 0x07, 0x06, 0x05]
        );
    }

    #[test]
    fn arithmetic_saturates_and_checks() {
        assert_eq!(Fixed::from_ratio(1, 2).unwrap(), Fixed::from_raw(500_000));
        assert_eq!(Fixed::from_ratio(1, 3).unwrap(), Fixed::from_raw(333_333));
        assert_eq!(Fixed::ONE.checked_mul(Fixed::ONE).unwrap(), Fixed::ONE);
        assert_eq!(
            Fixed::from_raw(SCALE)
                .checked_div(Fixed::from_raw(4 * SCALE))
                .unwrap(),
            Fixed::from_raw(250_000)
        );
        assert_eq!(Fixed::ZERO.checked_div(Fixed::ZERO), None);
        assert_eq!(Fixed::from_ratio(1, 0), None);
        assert_eq!(Fixed::from_raw(5).checked_neg(), Some(Fixed::from_raw(-5)));
        assert_eq!(
            Fixed::from_raw(i64::MAX).saturating_add(Fixed::ONE),
            Fixed::from_raw(i64::MAX)
        );
    }

    #[test]
    fn clamp_and_display() {
        let value = Fixed::from_raw(1_200_000).clamp(Fixed::ZERO, Fixed::ONE);
        assert_eq!(value, Fixed::ONE);
        assert_eq!(Fixed::from_raw(-1_000_000).to_string(), "-1.000000");
        assert_eq!(Fixed::from_raw(500_000).to_string(), "0.500000");
    }
}
