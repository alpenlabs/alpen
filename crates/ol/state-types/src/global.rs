//! Global state variables that are always accessible.

use strata_identifiers::Slot;

use crate::ssz_generated::ssz::state::GlobalState;

impl GlobalState {
    /// Create a new global state.
    pub fn new(cur_slot: Slot) -> Self {
        Self { cur_slot }
    }

    /// Get the current slot (immutable).
    pub fn get_cur_slot(&self) -> Slot {
        self.cur_slot
    }

    /// Set the current slot.
    pub fn set_cur_slot(&mut self, slot: Slot) {
        self.cur_slot = slot;
    }
}

#[cfg(test)]
mod tests {
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;
    use crate::test_utils::global_state_strategy;

    ssz_proptest!(GlobalState, global_state_strategy());
}
