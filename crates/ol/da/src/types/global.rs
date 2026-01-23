//! Global state diff types.

use strata_da_framework::{
    DaCounter,
    counter_schemes::{self, CtrU64ByU16},
    make_compound_impl,
};

/// Diff of global state fields covered by DA.
#[derive(Debug)]
pub struct GlobalStateDiff {
    /// Slot counter diff.
    pub cur_slot: DaCounter<CtrU64ByU16>,
}

impl Default for GlobalStateDiff {
    fn default() -> Self {
        Self {
            cur_slot: DaCounter::new_unchanged(),
        }
    }
}

impl GlobalStateDiff {
    /// Creates a new [`GlobalStateDiff`] from a slot counter.
    pub fn new(cur_slot: DaCounter<counter_schemes::CtrU64ByU16>) -> Self {
        Self { cur_slot }
    }
}

make_compound_impl! {
    GlobalStateDiff u8 => GlobalStateTarget {
        cur_slot: counter (counter_schemes::CtrU64ByU16),
    }
}

/// Target for applying a global state diff.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GlobalStateTarget {
    /// Current slot value.
    pub cur_slot: u64,
}
