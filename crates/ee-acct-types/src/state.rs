//! EE account internal state.

use strata_acct_types::SubjectId;

type Hash = [u8; 32];

#[derive(Clone, Debug)]
pub struct EeAccountState {
    /// ID of the last execution block that we've processed.
    last_exec_blkid: Hash,

    /// Tracked balance bridged into execution env, according to processed
    /// messages.
    tracked_balance: u64,

    /// Pending deposits that haven't been accepted into a block.
    pending_deposits: Vec<PendingDepositEntry>,

    /// Pending forced inclusions that haven't been included in a block.
    pending_fincls: Vec<PendingFinclEntry>,
}

impl EeAccountState {
    pub fn last_exec_blkid(&self) -> Hash {
        self.last_exec_blkid
    }

    pub fn set_last_exec_blkid(&mut self, blkid: Hash) {
        self.last_exec_blkid = blkid;
    }

    pub fn tracked_balance(&self) -> u64 {
        self.tracked_balance
    }

    pub fn pending_deposits(&self) -> &[PendingDepositEntry] {
        &self.pending_deposits
    }

    pub fn pending_fincls(&self) -> &[PendingFinclEntry] {
        &self.pending_fincls
    }
}

/// A pending deposit that's been accepted by the EE account but not processed
/// by it.
#[derive(Clone, Debug)]
pub struct PendingDepositEntry {
    epoch: u32,
    dest: SubjectId, // TODO need to figure out if we unify this with subj transfers
}

/// A pending forced inclusion that's been accepted by the EE account but not
/// included in a block yet.
#[derive(Clone, Debug)]
pub struct PendingFinclEntry {
    epoch: u32,
    raw_tx_hash: Hash,
}
