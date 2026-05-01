//! Global state variables that are always accessible.

use strata_acct_types::{AccountSerial, BitcoinAmount};
use strata_identifiers::Slot;
use strata_ledger_types::Coin;

use crate::ssz_generated::ssz::state::GlobalState;

impl GlobalState {
    /// Create a new global state.
    pub fn new(cur_slot: Slot, next_avail_serial: AccountSerial) -> Self {
        Self {
            cur_slot,
            // FIXME(STR-3227): fix this conversion
            next_avail_serial: next_avail_serial.into_inner() as u64,
            limbo_funds_sats: 0,
        }
    }

    /// Get the current slot (immutable).
    pub fn get_cur_slot(&self) -> Slot {
        self.cur_slot
    }

    /// Set the current slot.
    pub fn set_cur_slot(&mut self, slot: Slot) {
        self.cur_slot = slot;
    }

    /// Gets the next available serial to be assigned to an account.
    pub fn get_next_avail_serial(&self) -> AccountSerial {
        // FIXME(STR-3227): fix this conversion
        AccountSerial::from(self.next_avail_serial as u32)
    }

    /// Gets the next available serial to be assigned to an account.
    pub fn set_next_avail_serial(&mut self, serial: AccountSerial) {
        // FIXME(STR-3227): fix this conversion
        self.next_avail_serial = serial.into_inner() as u64;
    }

    /// Gets the amount of funds in limbo.
    pub fn limbo_funds(&self) -> BitcoinAmount {
        BitcoinAmount::from_sat(self.limbo_funds_sats)
    }

    /// Attempts to add limbo funds.
    pub fn add_limbo_funds(&mut self, amt: BitcoinAmount) -> bool {
        let Some(new_lf) = self.limbo_funds().checked_add(amt) else {
            return false;
        };
        self.limbo_funds_sats = new_lf.to_sat();
        true
    }

    /// Adds a [`Coin`] to limbo funds, consuming it.
    ///
    /// # Panics
    ///
    /// If there's balance overflow.
    pub fn add_limbo_funds_coin(&mut self, coin: Coin) {
        assert!(
            self.add_limbo_funds(coin.amt()),
            "ol/state: limbo funds overflow"
        );
        coin.safely_consume_unchecked();
    }

    /// Takes some limbo funds as a [`Coin`], if possible.
    pub fn take_limbo_funds_coin(&mut self, amt: BitcoinAmount) -> Option<Coin> {
        let lf = self.limbo_funds();

        let new_lf = lf.checked_sub(amt)?;

        // This sanity check should be optimized out.
        assert_eq!(
            new_lf.checked_add(amt),
            Some(lf),
            "ol/state: inconsistent limbo funds change"
        );

        let coin = Coin::new_unchecked(amt);
        self.limbo_funds_sats = new_lf.to_sat();
        Some(coin)
    }
}

#[cfg(test)]
mod tests {
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;
    use crate::test_utils::global_state_strategy;

    ssz_proptest!(GlobalState, global_state_strategy());
}
