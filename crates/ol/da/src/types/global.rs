//! Global state diff types.

use strata_da_framework::{
    DaCounter,
    counter_schemes::{CtrU64BySignedVarInt, CtrU64ByU16},
    make_compound_impl,
};

/// Diff of global state fields covered by DA.
#[derive(Debug)]
pub struct GlobalStateDiff {
    /// Slot counter diff.
    pub cur_slot: DaCounter<CtrU64ByU16>,

    /// Limbo funds counter diff.
    pub limbo_funds_sats: DaCounter<CtrU64BySignedVarInt>,
}

impl Default for GlobalStateDiff {
    fn default() -> Self {
        Self {
            cur_slot: DaCounter::new_unchanged(),
            limbo_funds_sats: DaCounter::default(),
        }
    }
}

impl GlobalStateDiff {
    /// Creates a new [`GlobalStateDiff`] from a slot counter.
    pub fn new(
        cur_slot: DaCounter<CtrU64ByU16>,
        limbo_funds_sats: DaCounter<CtrU64BySignedVarInt>,
    ) -> Self {
        Self {
            cur_slot,
            limbo_funds_sats,
        }
    }
}

make_compound_impl! {
    GlobalStateDiff < (), crate::DaError > u8 => GlobalStateTarget {
        cur_slot: counter (CtrU64ByU16),
        limbo_funds_sats: counter (CtrU64BySignedVarInt)
    }
}

/// Target for applying a global state diff.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GlobalStateTarget {
    /// Current slot value.
    pub cur_slot: u64,

    /// Limbo funds value.
    pub limbo_funds_sats: u64,
}
