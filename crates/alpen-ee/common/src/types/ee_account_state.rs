use strata_acct_types::Hash;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::{OLBlockCommitment, OLBlockId};

/// EE account internal state corresponding to OL block.
#[derive(Debug, Clone)]
pub struct EeAccountStateAtBlock {
    ol_block: OLBlockCommitment,
    state: EeAccountState,
}

impl EeAccountStateAtBlock {
    /// Creates a new EE account state at a specific OL block.
    pub fn new(ol_block: OLBlockCommitment, state: EeAccountState) -> Self {
        Self { ol_block, state }
    }

    /// Returns the OL block commitment this EEAccountState corresponds to.
    pub fn ol_block(&self) -> &OLBlockCommitment {
        &self.ol_block
    }

    /// Returns the EE account state.
    pub fn ee_state(&self) -> &EeAccountState {
        &self.state
    }

    /// Returns the OL slot number this EEAccountState corresponds to.
    pub fn ol_slot(&self) -> u64 {
        self.ol_block.slot()
    }

    /// Returns the OL block ID this EEAccountState corresponds to.
    pub fn ol_blockid(&self) -> &OLBlockId {
        self.ol_block.blkid()
    }

    /// Returns the last execution block ID from the account state.
    /// This is the blockhash of the execution block.
    pub fn last_exec_blkid(&self) -> Hash {
        self.state.last_exec_blkid()
    }
}
