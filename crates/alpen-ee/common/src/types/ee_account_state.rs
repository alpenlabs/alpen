use strata_acct_types::Hash;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::{Epoch, EpochCommitment, OLBlockId};

/// EE account internal state corresponding to OL block.
#[derive(Debug, Clone)]
pub struct EeAccountStateAtEpoch {
    commitment: EpochCommitment,
    state: EeAccountState,
}

impl EeAccountStateAtEpoch {
    /// Creates a new EE account state at a specific OL block.
    pub fn new(ol_block: EpochCommitment, state: EeAccountState) -> Self {
        Self {
            commitment: ol_block,
            state,
        }
    }

    /// Returns the OL block commitment this EEAccountState corresponds to.
    pub fn epoch_commitment(&self) -> &EpochCommitment {
        &self.commitment
    }

    /// Returns the EE account state.
    pub fn ee_state(&self) -> &EeAccountState {
        &self.state
    }

    /// Returns the OL slot number this EEAccountState corresponds to.
    pub fn ol_slot(&self) -> u64 {
        self.commitment.last_slot()
    }

    /// Returns the OL block ID this EEAccountState corresponds to.
    pub fn ol_blockid(&self) -> &OLBlockId {
        self.commitment.last_blkid()
    }

    pub fn ol_epoch(&self) -> Epoch {
        self.commitment.epoch()
    }

    /// Returns the last execution block ID from the account state.
    /// This is the blockhash of the execution block.
    pub fn last_exec_blkid(&self) -> Hash {
        self.state.last_exec_blkid()
    }

    pub fn into_parts(self) -> (EpochCommitment, EeAccountState) {
        (self.commitment, self.state)
    }
}
