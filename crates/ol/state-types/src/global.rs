//! Global state variables that are always accessible.

use strata_codec::Codec;

#[derive(Clone, Debug, Codec)]
pub struct GlobalState {
    cur_slot: u64,
}

impl GlobalState {
    /// Create a new global state.
    pub fn new(cur_slot: u64) -> Self {
        Self { cur_slot }
    }

    /// Get the current slot (immutable).
    pub fn get_cur_slot(&self) -> u64 {
        self.cur_slot
    }

    /// Set the current slot.
    pub fn set_cur_slot(&mut self, slot: u64) {
        self.cur_slot = slot;
    }
}
