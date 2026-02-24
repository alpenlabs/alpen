//! Epoch-level state that is changed during sealing/checkin.
//!
//! This can be completely omitted from DA.

use strata_acct_types::{BitcoinAmount, Mmr64, StrataHasher, tree_hash::TreeHash};
use strata_asm_manifest_types::AsmManifest;
use strata_identifiers::{EpochCommitment, L1BlockCommitment, L1BlockId, L1Height};
use strata_merkle::Mmr;

use crate::ssz_generated::ssz::state::EpochalState;

impl EpochalState {
    /// Create a new epochal state for testing.
    pub fn new(
        total_ledger_funds: BitcoinAmount,
        cur_epoch: u32,
        last_l1_block: L1BlockCommitment,
        checkpointed_epoch: EpochCommitment,
        manifests_mmr: Mmr64,
        manifests_mmr_offset: u64,
    ) -> Self {
        Self {
            total_ledger_funds,
            cur_epoch,
            last_l1_block,
            checkpointed_epoch,
            manifests_mmr,
            manifests_mmr_offset,
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
        // FIXME this conversion is weird
        self.last_l1_block.height_u64() as u32
    }

    /// Appends a new ASM manifest to the accumulator, also updating the last L1
    /// block height and other fields.
    pub fn append_manifest(&mut self, height: L1Height, mf: AsmManifest) {
        let manifest_hash = <AsmManifest as TreeHash>::tree_hash_root(&mf);

        Mmr::<StrataHasher>::add_leaf(&mut self.manifests_mmr, manifest_hash.into_inner())
            .expect("MMR capacity exceeded");
        // FIXME make this conversion less weird
        self.last_l1_block = L1BlockCommitment::from_height_u64(height as u64, *mf.blkid())
            .expect("state: weird conversion")
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

    /// Gets the ASM manifests MMR.
    pub fn asm_manifests_mmr(&self) -> &Mmr64 {
        &self.manifests_mmr
    }

    /// Gets the offset for mapping L1 block heights to MMR leaf indices.
    pub fn manifests_mmr_offset(&self) -> u64 {
        self.manifests_mmr_offset
    }
}

#[cfg(test)]
mod tests {
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;
    use crate::test_utils::epochal_state_strategy;

    ssz_proptest!(EpochalState, epochal_state_strategy());
}
