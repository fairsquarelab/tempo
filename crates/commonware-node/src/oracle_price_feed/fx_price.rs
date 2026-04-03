//! `FxPrice` — fixed-point 6-decimal price representation.
//!
//! Every FX price flowing through the oracle system is expressed as an `FxPrice`:
//! *"how many units of the non-base currency per 1 unit of the base currency (USD)"*,
//! encoded as `value / 10^6`.
//!
//! Example: 1 USD = 1 450.23 KRW → `FxPrice(1_450_230_000)`.
//!
//! Source-specific raw formats (Pyth `(price, expo)`, Frankfurter `f64` rate, etc.)
//! are converted into `FxPrice` via the `from_*` constructors defined here.

use alloy_primitives::U256;
use std::fmt;

/// Number of decimal places in the fixed-point encoding.
pub const DECIMALS: u32 = 6;

/// Scale factor: `10^DECIMALS`.
pub const SCALE: u128 = 1_000_000;

/// Fixed-point 6-decimal FX scalar (newtype over `U256`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FxPrice(pub U256);

impl FxPrice {
    /// Create an `FxPrice` from a raw `U256` that is already in 6-decimal encoding.
    pub const fn from_raw(v: U256) -> Self {
        Self(v)
    }

    /// Underlying `U256` value.
    pub const fn inner(self) -> U256 {
        self.0
    }

    pub fn is_zero(&self) -> bool {
        self.0.is_zero()
    }

    /// Compute the reciprocal: `SCALE^2 / self`, maintaining 6-decimal precision.
    ///
    /// Used when a Pyth feed reports the rate in the inverse direction
    /// (e.g. EUR/USD gives USD-per-EUR; we need EUR-per-USD).
    /// Returns `None` if `self` is zero.
    pub fn reciprocal(self) -> Option<Self> {
        if self.0.is_zero() {
            return None;
        }
        let scale_sq = U256::from(SCALE) * U256::from(SCALE);
        Some(Self(scale_sq / self.0))
    }

    /// Convert an `f64` exchange rate (e.g. from Frankfurter / ExchangeRate-API) to `FxPrice`.
    ///
    /// `rate` is interpreted as *"1 base-currency unit = `rate` quote-currency units"*.
    /// Example: `1450.23` → `FxPrice(1_450_230_000)`.
    pub fn from_f64_rate(rate: f64) -> Self {
        let scaled = (rate * SCALE as f64) as u128;
        Self(U256::from(scaled))
    }

    /// Convert a Pyth Hermes `(price, expo)` pair to `FxPrice`.
    ///
    /// Returns `None` if the price string is unparseable or negative.
    ///
    /// Pyth prices have a negative exponent: `real_price = price × 10^expo`.
    /// We normalise to 6 decimals: `result = price × 10^(6 + expo)`.
    pub fn from_pyth(price_str: &str, expo: i32) -> Option<Self> {
        let raw: i128 = price_str.parse().ok()?;
        if raw < 0 {
            return None;
        }
        let raw = raw as u128;

        let adjustment = DECIMALS as i32 + expo;
        let result = if adjustment >= 0 {
            raw.checked_mul(10u128.pow(adjustment as u32))?
        } else {
            let divisor = 10u128.pow((-adjustment) as u32);
            raw / divisor
        };

        Some(Self(U256::from(result)))
    }
}

impl fmt::Display for FxPrice {
    /// Human-readable form with 6 decimal places.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Ok(v) = u128::try_from(self.0) else {
            return write!(f, "{}", self.0);
        };
        write!(f, "{:.6}", v as f64 / SCALE as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_f64_rate_typical() {
        let s = FxPrice::from_f64_rate(1450.23);
        assert_eq!(s.inner(), U256::from(1_450_230_000u128));
    }

    #[test]
    fn from_f64_rate_one() {
        assert_eq!(FxPrice::from_f64_rate(1.0).inner(), U256::from(SCALE));
    }

    #[test]
    fn from_f64_rate_zero() {
        assert!(FxPrice::from_f64_rate(0.0).is_zero());
    }

    #[test]
    fn from_pyth_expo_minus_5() {
        // price=145023000, expo=-5 → 145023000 * 10^1 = 1_450_230_000
        let s = FxPrice::from_pyth("145023000", -5).unwrap();
        assert_eq!(s.inner(), U256::from(1_450_230_000u128));
    }

    #[test]
    fn from_pyth_expo_minus_8() {
        // price=145023000, expo=-8 → 145023000 / 10^2 = 1_450_230
        let s = FxPrice::from_pyth("145023000", -8).unwrap();
        assert_eq!(s.inner(), U256::from(1_450_230u128));
    }

    #[test]
    fn from_pyth_expo_zero() {
        let s = FxPrice::from_pyth("1", 0).unwrap();
        assert_eq!(s.inner(), U256::from(1_000_000u128));
    }

    #[test]
    fn from_pyth_expo_matches_target() {
        let s = FxPrice::from_pyth("1000000", -6).unwrap();
        assert_eq!(s.inner(), U256::from(1_000_000u128));
    }

    #[test]
    fn from_pyth_negative_price() {
        assert!(FxPrice::from_pyth("-1", -6).is_none());
    }

    #[test]
    fn from_pyth_invalid_string() {
        assert!(FxPrice::from_pyth("not_a_number", -6).is_none());
    }

    #[test]
    fn display_typical() {
        let s = FxPrice::from_f64_rate(1450.23);
        assert_eq!(s.to_string(), "1450.230000");
    }

    #[test]
    fn display_one() {
        let s = FxPrice::from_f64_rate(1.0);
        assert_eq!(s.to_string(), "1.000000");
    }
}
