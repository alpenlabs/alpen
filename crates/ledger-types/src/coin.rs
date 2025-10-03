//! Coin abstraction to ensure we don't accidentally create or destroy funds.

use std::{mem, ops::Drop};

use strata_acct_types::BitcoinAmount;

/// A linear coin that must be explicitly created and destroyed.  This allows us
/// to more safely reason about flow of funds between components and reduce the
/// complexity of bookkeeping by making it more likely that accounting bugs are
/// turned into panics.
///
/// Triggers a panic when dropped without [`safely_consume_unchecked`] being
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

    /// Gets the amount of value this coin represents.
    pub fn amt(&self) -> BitcoinAmount {
        self.0
    }

    /// Consumes the coin without panicking.
    ///
    /// Care must be used to ensure that this does not destroy value.
    pub fn safely_consume_unchecked(self) {
        // Since this is just flat bytes, we can do this to safely destroy
        // ourselves without calling `Drop::drop`.
        mem::forget(self);
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
    use super::Coin;

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
}
