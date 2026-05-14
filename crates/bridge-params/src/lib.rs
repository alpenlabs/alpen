//! Bridge denomination and cap parameters.

#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use thiserror::Error;

/// Bridge denomination and optional maximum withdrawal amount, in satoshis.
///
/// Constructed via [`BridgeParams::new`] which validates the invariants:
/// - `denomination` must be non-zero
/// - If `max_withdrawal_amount` is set, it must be `>= denomination` and a multiple of it
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct BridgeParams {
    denomination: u64,
    max_withdrawal_amount: Option<u64>,
}

impl BridgeParams {
    /// Creates a new [`BridgeParams`] after validating invariants.
    pub fn new(
        denomination: u64,
        max_withdrawal_amount: Option<u64>,
    ) -> Result<Self, BridgeParamsError> {
        if denomination == 0 {
            return Err(BridgeParamsError::ZeroDenomination);
        }
        if let Some(max) = max_withdrawal_amount {
            if max < denomination {
                return Err(BridgeParamsError::MaxBelowDenomination { denomination, max });
            }
            if max % denomination != 0 {
                return Err(BridgeParamsError::MaxNotMultiple { denomination, max });
            }
        }
        Ok(Self {
            denomination,
            max_withdrawal_amount,
        })
    }

    pub fn denomination(&self) -> u64 {
        self.denomination
    }

    pub fn max_withdrawal_amount(&self) -> Option<u64> {
        self.max_withdrawal_amount
    }

    /// Returns whether a withdrawal amount (in sats) is valid.
    pub fn validate_withdrawal_amount(&self, amount_sats: u64) -> bool {
        amount_sats > 0
            && amount_sats.is_multiple_of(self.denomination)
            && self
                .max_withdrawal_amount
                .is_none_or(|cap| amount_sats <= cap)
    }
}

/// Default bridge denomination: 1 BTC in satoshis.
pub const DEFAULT_DENOMINATION_SATS: u64 = 100_000_000;

/// Default maximum withdrawal amount: 10 BTC in satoshis.
pub const DEFAULT_MAX_WITHDRAWAL_SATS: u64 = 1_000_000_000;

impl Default for BridgeParams {
    fn default() -> Self {
        Self {
            denomination: DEFAULT_DENOMINATION_SATS,
            max_withdrawal_amount: Some(DEFAULT_MAX_WITHDRAWAL_SATS),
        }
    }
}

#[derive(Debug, Error)]
pub enum BridgeParamsError {
    #[error("denomination must not be zero")]
    ZeroDenomination,

    #[error("max_withdrawal_amount ({max}) is below denomination ({denomination})")]
    MaxBelowDenomination { denomination: u64, max: u64 },

    #[error("max_withdrawal_amount ({max}) is not a multiple of denomination ({denomination})")]
    MaxNotMultiple { denomination: u64, max: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_no_cap() {
        let w = BridgeParams::new(100_000_000, None).unwrap();
        assert!(w.validate_withdrawal_amount(100_000_000));
        assert!(w.validate_withdrawal_amount(300_000_000));
        assert!(!w.validate_withdrawal_amount(0));
        assert!(!w.validate_withdrawal_amount(150_000_000));
    }

    #[test]
    fn valid_with_cap() {
        let w = BridgeParams::new(100_000_000, Some(1_000_000_000)).unwrap();
        assert!(w.validate_withdrawal_amount(100_000_000));
        assert!(w.validate_withdrawal_amount(1_000_000_000));
        assert!(!w.validate_withdrawal_amount(1_100_000_000));
        assert!(!w.validate_withdrawal_amount(0));
        assert!(!w.validate_withdrawal_amount(150_000_000));
    }

    #[test]
    fn zero_denomination_rejected() {
        assert!(BridgeParams::new(0, None).is_err());
    }

    #[test]
    fn max_below_denomination_rejected() {
        assert!(BridgeParams::new(100_000_000, Some(50_000_000)).is_err());
    }

    #[test]
    fn max_not_multiple_rejected() {
        assert!(BridgeParams::new(100_000_000, Some(150_000_000)).is_err());
    }

    #[test]
    fn max_equals_denomination_accepted() {
        let w = BridgeParams::new(100_000_000, Some(100_000_000)).unwrap();
        assert!(w.validate_withdrawal_amount(100_000_000));
        assert!(!w.validate_withdrawal_amount(200_000_000));
    }
}
