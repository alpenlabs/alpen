//! Epoch-level state that is changed during sealing/checkin.
//!
//! This can be completely omitted from DA.

use strata_acct_types::BitcoinAmount;
use strata_codec::Codec;
use strata_identifiers::L1BlockCommitment;
use strata_ledger_types::*;

#[derive(Clone, Debug, Codec)]
pub struct EpochalState {
    total_ledger_funds: BitcoinAmount,
    cur_epoch: u32,
    last_l1_block: L1BlockCommitment,
    checkpointed_epoch: EpochCommitment,
}

impl EpochalState {
    /// Create a new epochal state for testing.
    pub fn new(
        total_ledger_funds: BitcoinAmount,
        cur_epoch: u32,
        last_l1_block: L1BlockCommitment,
        checkpointed_epoch: EpochCommitment,
    ) -> Self {
        Self {
            total_ledger_funds,
            cur_epoch,
            last_l1_block,
            checkpointed_epoch,
        }
    }
}

impl IL1ViewState for EpochalState {
    fn cur_epoch(&self) -> u32 {
        self.cur_epoch
    }

    fn set_cur_epoch(&mut self, epoch: u32) {
        self.cur_epoch = epoch;
    }

    fn last_l1_blkid(&self) -> &L1BlockId {
        self.last_l1_block.blkid()
    }

    fn last_l1_height(&self) -> L1Height {
        // FIXME this conversion is weird
        self.last_l1_block.height_u64() as u32
    }

    fn append_manifest(&mut self, height: L1Height, mf: AsmManifest) {
        // TODO actually insert into mmr
        // FIXME make this conversion less weird
        self.last_l1_block = L1BlockCommitment::from_height_u64(height as u64, *mf.blkid())
            .expect("state: weird conversion")
    }

    fn asm_recorded_epoch(&self) -> &EpochCommitment {
        &self.checkpointed_epoch
    }

    fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment) {
        self.checkpointed_epoch = epoch;
    }

    fn total_ledger_balance(&self) -> BitcoinAmount {
        self.total_ledger_funds
    }

    fn set_total_ledger_balance(&mut self, amt: BitcoinAmount) {
        self.total_ledger_funds = amt;
    }
}
