//! Epoch-level state that is changed during sealing/checkin.
//!
//! This can be completely omitted from DA.

use strata_acct_types::{BitcoinAmount, L1BlockRecord, Mmr64, append_l1_block_rec_to_mmr};
use strata_identifiers::{Buf32, EpochCommitment, L1BlockCommitment, L1BlockId, L1Height};

use crate::ssz_generated::ssz::state::EpochalState;

impl EpochalState {
    /// Create a new epochal state for testing.
    pub fn new(
        total_ledger_funds: BitcoinAmount,
        cur_epoch: u32,
        last_l1_block: L1BlockCommitment,
        checkpointed_epoch: EpochCommitment,
        l1_block_refs_mmr: Mmr64,
    ) -> Self {
        Self {
            total_ledger_funds,
            cur_epoch,
            last_l1_block,
            checkpointed_epoch,
            l1_block_refs_mmr,
        }
    }

    /// Gets the current epoch.
    pub fn cur_epoch(&self) -> u32 {
        self.cur_epoch
    }

    /// Sets the current epoch.
    pub fn set_cur_epoch(&mut self, epoch: u32) {
        self.cur_epoch = epoch;
    }

    /// Last L1 block ID.
    pub fn last_l1_blkid(&self) -> &L1BlockId {
        self.last_l1_block.blkid()
    }

    /// Last L1 block height.
    pub fn last_l1_height(&self) -> L1Height {
        self.last_l1_block.height()
    }

    /// Appends an accepted [`L1BlockRecord`] to the accumulator.
    ///
    /// This also updates the last L1 block height and ID.
    ///
    /// The MMR is height-indexed: the leaf for an L1 block at height `h` lives
    /// at MMR index `h`. The MMR is prefilled with dummy-hash entries up to
    /// `genesis_l1_height` at genesis, so callers must append accepted records
    /// with strictly contiguous heights matching the next available MMR index.
    pub fn append_l1_block_rec(&mut self, height: L1Height, rec: L1BlockRecord) {
        debug_assert_eq!(
            self.l1_block_refs_mmr.num_entries(),
            height as u64,
            "ol/state: L1 height must equal next MMR index"
        );

        append_l1_block_rec_to_mmr(&mut self.l1_block_refs_mmr, &rec);

        let blkid = L1BlockId::from(Buf32::from(rec.block_hash()));
        self.last_l1_block = L1BlockCommitment::new(height, blkid);
    }

    /// Gets the field for the epoch that the ASM considers to be valid.
    ///
    /// This is our perspective of the perspective of the last block's ASM
    /// manifest we've accepted.
    pub fn asm_recorded_epoch(&self) -> &EpochCommitment {
        &self.checkpointed_epoch
    }

    /// Sets the field for the epoch that the ASM considers to be finalized.
    ///
    /// This is our perspective of the perspective of the last block's ASM
    /// manifest we've accepted.
    pub fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment) {
        self.checkpointed_epoch = epoch;
    }

    /// Gets the total OL ledger balance.
    pub fn total_ledger_balance(&self) -> BitcoinAmount {
        self.total_ledger_funds
    }

    /// Sets the total OL ledger balance.
    pub fn set_total_ledger_balance(&mut self, amt: BitcoinAmount) {
        self.total_ledger_funds = amt;
    }

    /// Gets the OL L1 block refs MMR.
    ///
    /// Indices into this MMR are L1 block heights.
    pub fn l1_block_refs_mmr(&self) -> &Mmr64 {
        &self.l1_block_refs_mmr
    }
}

#[cfg(test)]
mod tests {
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;
    use crate::test_utils::epochal_state_strategy;

    ssz_proptest!(EpochalState, epochal_state_strategy());
}
