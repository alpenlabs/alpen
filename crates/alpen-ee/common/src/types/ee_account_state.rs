use strata_acct_types::Hash;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::{OLBlockCommitment, OLBlockId};

#[derive(Debug, Clone)]
/// EE account internal state corresponding to ol Block
pub struct EeAccountStateAtBlock {
    ol_block: OLBlockCommitment,
    state: EeAccountState,
}

impl EeAccountStateAtBlock {
    pub fn new(ol_block: OLBlockCommitment, state: EeAccountState) -> Self {
        Self { ol_block, state }
    }

    pub fn ol_block(&self) -> &OLBlockCommitment {
        &self.ol_block
    }
    pub fn ee_state(&self) -> &EeAccountState {
        &self.state
    }

    pub fn ol_slot(&self) -> u64 {
        self.ol_block.slot()
    }

    pub fn ol_blockid(&self) -> &OLBlockId {
        self.ol_block.blkid()
    }

    pub fn last_exec_blkid(&self) -> Hash {
        self.state.last_exec_blkid()
    }
}
