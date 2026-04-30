//! Global state variables that are always accessible.

use strata_acct_types::AccountSerial;
use strata_identifiers::Slot;

use crate::ssz_generated::ssz::state::GlobalState;

impl GlobalState {
    /// Create a new global state.
    pub fn new(cur_slot: Slot, next_avail_serial: AccountSerial) -> Self {
        Self {
            cur_slot,
            // FIXME(STR-3227): fix this conversion
            next_avail_serial: next_avail_serial.into_inner() as u64,
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
}

#[cfg(test)]
mod tests {
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;
    use crate::test_utils::global_state_strategy;

    ssz_proptest!(GlobalState, global_state_strategy());
}
