use strata_primitives::prelude::*;

#[derive(Copy, Clone, Debug)]
pub struct TipState {
    /// Current tip block.
    cur_tip: L2BlockCommitment,

    /// Previous epoch we're building on top of.
    prev_epoch: EpochCommitment,
}

impl TipState {
    fn new(cur_tip: L2BlockCommitment, prev_epoch: EpochCommitment) -> Self {
        Self {
            cur_tip,
            prev_epoch,
        }
    }

    fn cur_tip(&self) -> L2BlockCommitment {
        self.cur_tip
    }

    fn prev_epoch(&self) -> EpochCommitment {
        self.prev_epoch
    }

    /// Returns the current epoch of the `cur_tip` block.  This is always the
    /// one after the `prev_epoch`.
    fn cur_epoch(&self) -> u64 {
        self.prev_epoch.epoch() + 1
    }
}
