use std::fmt;

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

/// A wrapper for bitcoin amount in sats similar to the implementation in [`bitcoin::Amount`].
///
/// NOTE: This wrapper has been created so that we can implement `Borsh*` traits on it.
#[derive(
    Copy,
    Clone,
    Debug,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Arbitrary,
    BorshDeserialize,
    BorshSerialize,
    Deserialize,
    Serialize,
)]
pub struct BitcoinAmount(u64);

impl fmt::Display for BitcoinAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(feature = "fullbtc")]
impl From<bitcoin::Amount> for BitcoinAmount {
    fn from(value: bitcoin::Amount) -> Self {
        Self::from_sat(value.to_sat())
    }
}

#[cfg(feature = "fullbtc")]
impl From<BitcoinAmount> for bitcoin::Amount {
    fn from(value: BitcoinAmount) -> Self {
        Self::from_sat(value.to_sat())
    }
}

impl BitcoinAmount {
    /// The zero amount.
    pub const ZERO: BitcoinAmount = Self(0);

    /// The maximum value allowed as an amount. Useful for sanity checking.
    pub const MAX_MONEY: BitcoinAmount = Self::from_int_btc(21_000_000);

    /// The minimum value of an amount.
    pub const MIN: BitcoinAmount = Self::ZERO;

    /// The maximum value of an amount.
    pub const MAX: BitcoinAmount = Self(u64::MAX);

    /// The number of bytes that an amount contributes to the size of a transaction.
    /// Serialized length of a u64.
    pub const SIZE: usize = 8;

    /// The number of sats in 1 bitcoin.
    pub const SATS_FACTOR: u64 = 100_000_000;

    /// Get the number of sats in this [`BitcoinAmount`].
    pub fn to_sat(&self) -> u64 {
        self.0
    }

    /// Create a [`BitcoinAmount`] with sats precision and the given number of sats.
    pub const fn from_sat(value: u64) -> Self {
        Self(value)
    }

    /// Convert from a value strataing integer values of bitcoins to a [`BitcoinAmount`]
    /// in const context.
    ///
    /// ## Panics
    ///
    /// The function panics if the argument multiplied by the number of sats
    /// per bitcoin overflows a u64 type, or is greater than [`BitcoinAmount::MAX_MONEY`].
    pub const fn from_int_btc(btc: u64) -> Self {
        match btc.checked_mul(Self::SATS_FACTOR) {
            Some(amount) => Self::from_sat(amount),
            None => {
                panic!("number of sats greater than u64::MAX");
            }
        }
    }

    /// Checked addition. Returns [`None`] if overflow occurred.
    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self::from_sat)
    }

    /// Checked subtraction. Returns [`None`] if overflow occurred.
    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.0.checked_sub(rhs.0).map(Self::from_sat)
    }

    /// Checked multiplication. Returns [`None`] if overflow occurred.
    pub fn checked_mul(self, rhs: u64) -> Option<Self> {
        self.0.checked_mul(rhs).map(Self::from_sat)
    }

    /// Checked division. Returns [`None`] if `rhs == 0`.
    pub fn checked_div(self, rhs: u64) -> Option<Self> {
        self.0.checked_div(rhs).map(Self::from_sat)
    }

    /// Saturating subtraction. Computes `self - rhs`, returning [`Self::ZERO`] if overflow
    /// occurred.
    pub fn saturating_sub(self, rhs: Self) -> Self {
        Self::from_sat(self.to_sat().saturating_sub(rhs.to_sat()))
    }

    /// Saturating addition. Computes `self + rhs`, saturating at the numeric bounds.
    pub fn saturating_add(self, rhs: Self) -> Self {
        Self::from_sat(self.to_sat().saturating_add(rhs.to_sat()))
    }
}
