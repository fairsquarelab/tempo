//! `Currency` enum — ISO 4217 numeric codes for type-safe currency identification.
//!
//! The enum is intentionally small and fixed: every on-chain `currencyId` registered at genesis
//! must have a matching variant here (same release). Source-specific identifiers (Pyth feed IDs,
//! API symbol strings, etc.) are **not** stored on `Currency`; each `PriceSource` implementation
//! maps `Currency` to its own identifiers internally.

use std::fmt;
use std::str::FromStr;

/// ISO 4217 numeric currency codes used as on-chain `currencyId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Currency {
    /// US Dollar (base currency — never fetched/submitted as a price feed).
    USD = 840,
    /// South Korean Won.
    KRW = 410,
    /// Japanese Yen.
    JPY = 392,
    /// Euro.
    EUR = 978,
    /// British Pound Sterling.
    GBP = 826,
    /// Singapore Dollar.
    SGD = 702,
    /// Swiss Franc.
    CHF = 756,
}

/// The single base currency against which all FX scalars are quoted.
pub const BASE_CURRENCY: Currency = Currency::USD;

impl Currency {
    /// Returns the ISO 4217 numeric code (matches the on-chain `uint32 currencyId`).
    pub const fn iso_numeric(self) -> u32 {
        self as u32
    }

    /// Whether this currency is the base currency (USD).
    pub const fn is_base(self) -> bool {
        self.iso_numeric() == BASE_CURRENCY.iso_numeric()
    }
}

impl TryFrom<u32> for Currency {
    type Error = UnknownCurrencyId;

    fn try_from(id: u32) -> Result<Self, Self::Error> {
        match id {
            840 => Ok(Currency::USD),
            410 => Ok(Currency::KRW),
            392 => Ok(Currency::JPY),
            978 => Ok(Currency::EUR),
            826 => Ok(Currency::GBP),
            702 => Ok(Currency::SGD),
            756 => Ok(Currency::CHF),
            _ => Err(UnknownCurrencyId(id)),
        }
    }
}

/// Error returned when a `u32` does not map to any known `Currency` variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownCurrencyId(pub u32);

impl fmt::Display for UnknownCurrencyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown currency id: {}", self.0)
    }
}

impl std::error::Error for UnknownCurrencyId {}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Currency::USD => "USD",
            Currency::KRW => "KRW",
            Currency::JPY => "JPY",
            Currency::EUR => "EUR",
            Currency::GBP => "GBP",
            Currency::SGD => "SGD",
            Currency::CHF => "CHF",
        };
        f.write_str(s)
    }
}

impl FromStr for Currency {
    type Err = UnknownCurrencyId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "USD" => Ok(Currency::USD),
            "KRW" => Ok(Currency::KRW),
            "JPY" => Ok(Currency::JPY),
            "EUR" => Ok(Currency::EUR),
            "GBP" => Ok(Currency::GBP),
            "SGD" => Ok(Currency::SGD),
            "CHF" => Ok(Currency::CHF),
            _ => Err(UnknownCurrencyId(0)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_numeric_roundtrip() {
        for c in [
            Currency::USD,
            Currency::KRW,
            Currency::JPY,
            Currency::EUR,
            Currency::GBP,
            Currency::SGD,
            Currency::CHF,
        ] {
            assert_eq!(Currency::try_from(c.iso_numeric()), Ok(c));
        }
    }

    #[test]
    fn unknown_id_errors() {
        assert!(Currency::try_from(999).is_err());
    }

    #[test]
    fn base_currency_is_usd() {
        assert!(Currency::USD.is_base());
        assert!(!Currency::KRW.is_base());
    }

    #[test]
    fn display_and_parse() {
        for c in [
            Currency::USD,
            Currency::KRW,
            Currency::JPY,
            Currency::EUR,
            Currency::GBP,
            Currency::SGD,
            Currency::CHF,
        ] {
            let s = c.to_string();
            assert_eq!(s.parse::<Currency>(), Ok(c));
        }
    }
}
