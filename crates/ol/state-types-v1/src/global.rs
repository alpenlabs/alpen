//! Global state variables that are always accessible.

use ssz::DecodeError;
use ssz_types::Optional;
use ssz_types::view::ToOwnedSsz;
use strata_acct_types::{AccountSerial, BitcoinAmount};
use strata_identifiers::{Slot, SszDelegate, impl_ssz_via_delegate};
use strata_ol_state_types::Coin;

use crate::required_fields::require_present;
use crate::ssz_generated::ssz::state::GlobalStateV1Ssz;

/// Global OL state with all mandatory V1 fields present.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobalStateV1 {
    cur_slot: Slot,
    next_avail_serial: u64,
    limbo_funds_sats: u64,
}

impl SszDelegate for GlobalStateV1 {
    type Delegate = GlobalStateV1Ssz;

    fn into_delegate(self) -> Self::Delegate {
        GlobalStateV1Ssz {
            cur_slot: Optional::Some(self.cur_slot),
            next_avail_serial: Optional::Some(self.next_avail_serial),
            limbo_funds_sats: Optional::Some(self.limbo_funds_sats),
        }
    }

    fn from_delegate(delegate: Self::Delegate) -> Result<Self, DecodeError> {
        Ok(Self {
            cur_slot: require_present(delegate.cur_slot, "global.cur_slot")?,
            next_avail_serial: require_present(
                delegate.next_avail_serial,
                "global.next_avail_serial",
            )?,
            limbo_funds_sats: require_present(
                delegate.limbo_funds_sats,
                "global.limbo_funds_sats",
            )?,
        })
    }
}

impl_ssz_via_delegate!(GlobalStateV1);

impl ToOwnedSsz<GlobalStateV1> for GlobalStateV1 {
    fn to_owned(&self) -> GlobalStateV1 {
        self.clone()
    }
}

impl GlobalStateV1 {
    pub(crate) fn set_limbo_funds_sats(&mut self, sats: u64) {
        self.limbo_funds_sats = sats;
    }

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
        BitcoinAmount::try_from(self.limbo_funds_sats)
            .expect("amount must not exceed the Bitcoin money supply")
    }

    /// Attempts to add limbo funds.
    pub fn add_limbo_funds(&mut self, amt: BitcoinAmount) -> bool {
        let Some(new_lf_sats) = self.limbo_funds_sats.checked_add(amt.to_sat()) else {
            return false;
        };
        if BitcoinAmount::try_from(new_lf_sats).is_err() {
            return false;
        }
        self.limbo_funds_sats = new_lf_sats;
        true
    }

    /// Adds a [`Coin`] to limbo funds, consuming it.
    ///
    /// # Panics
    ///
    /// Panics if there is balance overflow.
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

        let new_lf_sats = lf.to_sat().checked_sub(amt.to_sat())?;
        let new_lf = BitcoinAmount::try_from(new_lf_sats)
            .expect("subtracting from valid limbo funds must remain valid");

        // This sanity check should be optimized out.
        assert_eq!(
            new_lf.to_sat().checked_add(amt.to_sat()),
            Some(lf.to_sat()),
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

    ssz_proptest!(GlobalStateV1, global_state_strategy());
}
