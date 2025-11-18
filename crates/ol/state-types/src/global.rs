//! Global state variables that are always accessible.

use strata_ledger_types::IGlobalState;

#[derive(Clone, Debug)]
pub struct GlobalState {
    cur_slot: u64,
}

impl IGlobalState for GlobalState {
    fn cur_slot(&mut self) -> u64 {
        self.cur_slot
    }

    fn set_cur_slot(&mut self, slot: u64) {
        self.cur_slot = slot;
    }
}
