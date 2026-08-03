//! Coin abstraction to ensure we don't accidentally create or destroy funds.

use std::{mem, ops::Drop};

use strata_acct_types::BitcoinAmount;

/// Error arising from value-preserving [`Coin`] operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CoinError {
    /// Tried to split off more value than the coin holds.
    #[error("insufficient coin value (have {have} sats, want {want} sats)")]
    InsufficientValue { have: u64, want: u64 },
}

/// A linear coin that must be explicitly created and destroyed.  This allows us
/// to more safely reason about flow of funds between components and reduce the
/// complexity of bookkeeping by making it more likely that accounting bugs are
/// turned into panics.
///
/// Triggers a panic when dropped without [`Self::safely_consume_unchecked`] being
/// called.
#[derive(Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Coin(BitcoinAmount);

impl Coin {
    /// Creates a coin with some value.
    ///
    /// Care must be used to ensure that this does not create new value.
    pub fn new_unchecked(amt: BitcoinAmount) -> Self {
        Self(amt)
    }

    /// Creates a coin with zero value.
    ///
    /// This is always safe since it creates no value.
    pub fn zero() -> Self {
        Self(BitcoinAmount::zero())
    }

    /// Gets the amount of value this coin represents.
    pub fn amt(&self) -> BitcoinAmount {
        self.0
    }

    /// Splits `amt` off of this coin, reducing it by `amt` and returning a new
    /// coin holding `amt`.
    ///
    /// This is value-preserving: the returned coin plus what remains in `self`
    /// together hold exactly what `self` did before.  Returns an error, leaving
    /// `self` untouched, if `amt` exceeds this coin's value.
    pub fn split_out(&mut self, amt: BitcoinAmount) -> Result<Self, CoinError> {
        let Some(remainder) = self.0.checked_sub(amt) else {
            return Err(CoinError::InsufficientValue {
                have: self.0.to_sat(),
                want: amt.to_sat(),
            });
        };

        self.0 = remainder;
        Ok(Self(amt))
    }

    /// Consumes the coin without panicking.
    ///
    /// Care must be used to ensure that this does not destroy value.
    pub fn safely_consume_unchecked(self) {
        // Since this is just flat bytes, we can do this to safely destroy
        // ourselves without calling `Drop::drop`.
        mem::forget(self);
    }

    /// Consumes a coin that must hold zero value.
    ///
    /// # Panics
    ///
    /// Panics if the coin still holds value.  Reaching this with a nonzero
    /// coin means value went unaccounted for, which is a bug rather than a
    /// recoverable condition.
    pub fn consume_zero(self) {
        // Consume first so a failed assert doesn't also trip the `Drop` panic
        // and turn a clean panic into an abort.
        let amt = self.0;
        self.safely_consume_unchecked();
        assert!(
            amt.is_zero(),
            "coin: consumed nonzero coin as zero ({} sats)",
            amt.to_sat()
        );
    }
}

impl Drop for Coin {
    fn drop(&mut self) {
        let amt: u64 = self.amt().into();
        panic!("coin: accidentally destroyed value ({amt} sats)");
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::BitcoinAmount;

    use super::{Coin, CoinError};

    #[test]
    fn test_coin_create_destroy() {
        let coin = Coin::new_unchecked(123.into());
        coin.safely_consume_unchecked();
    }

    #[test]
    #[should_panic]
    fn test_coin_create_panic() {
        let _coin = Coin::new_unchecked(123.into());
        // should panic
    }

    #[test]
    fn test_coin_zero_consume_zero() {
        let coin = Coin::zero();
        assert!(coin.amt().is_zero());
        coin.consume_zero();
    }

    #[test]
    fn test_coin_split_out_preserves_total() {
        let mut coin = Coin::new_unchecked(BitcoinAmount::from_sat(100));
        let taken = coin
            .split_out(BitcoinAmount::from_sat(30))
            .expect("split within value");
        assert_eq!(taken.amt(), BitcoinAmount::from_sat(30));
        assert_eq!(coin.amt(), BitcoinAmount::from_sat(70));
        taken.safely_consume_unchecked();
        coin.safely_consume_unchecked();
    }

    #[test]
    fn test_coin_split_out_exact_leaves_zero() {
        let mut coin = Coin::new_unchecked(BitcoinAmount::from_sat(42));
        let taken = coin
            .split_out(BitcoinAmount::from_sat(42))
            .expect("split within value");
        taken.safely_consume_unchecked();
        coin.consume_zero();
    }

    #[test]
    fn test_coin_split_out_insufficient_errors() {
        let mut coin = Coin::new_unchecked(BitcoinAmount::from_sat(10));
        let err = coin
            .split_out(BitcoinAmount::from_sat(11))
            .expect_err("split beyond value must fail");
        assert_eq!(err, CoinError::InsufficientValue { have: 10, want: 11 });
        // `coin` is untouched by the failed split, so it still must be consumed.
        assert_eq!(coin.amt(), BitcoinAmount::from_sat(10));
        coin.safely_consume_unchecked();
    }

    #[test]
    #[should_panic]
    fn test_coin_consume_zero_nonzero_panics() {
        let coin = Coin::new_unchecked(BitcoinAmount::from_sat(1));
        coin.consume_zero();
    }
}
