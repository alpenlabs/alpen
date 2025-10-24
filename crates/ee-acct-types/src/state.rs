//! EE account internal state.

use strata_acct_types::BitcoinAmount;
use strata_ee_chain_types::SubjectDepositData;

type Hash = [u8; 32];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EeAccountState {
    /// ID of the last execution block that we've processed.
    last_exec_blkid: Hash,

    /// Tracked balance bridged into execution env, according to processed
    /// messages.
    tracked_balance: BitcoinAmount,

    /// Pending inputs that haven't been accepted into a block.
    ///
    /// These must be processed in order.
    pending_inputs: Vec<PendingInputEntry>,

    /// Pending forced inclusions that haven't been included in a block.
    ///
    /// These are separate from pending inputs because they're not really an
    /// input but a requirement we have to check about the blocks.
    pending_fincls: Vec<PendingFinclEntry>,
}

impl EeAccountState {
    pub fn new(
        last_exec_blkid: Hash,
        tracked_balance: BitcoinAmount,
        pending_inputs: Vec<PendingInputEntry>,
        pending_fincls: Vec<PendingFinclEntry>,
    ) -> Self {
        Self {
            last_exec_blkid,
            tracked_balance,
            pending_inputs,
            pending_fincls,
        }
    }

    pub fn last_exec_blkid(&self) -> Hash {
        self.last_exec_blkid
    }

    pub fn set_last_exec_blkid(&mut self, blkid: Hash) {
        self.last_exec_blkid = blkid;
    }

    pub fn tracked_balance(&self) -> BitcoinAmount {
        self.tracked_balance
    }

    /// Adds to the tracked balance, panicking on overflow.
    pub fn add_tracked_balance(&mut self, amt: BitcoinAmount) {
        self.tracked_balance = self
            .tracked_balance
            .checked_add(amt)
            .expect("snarkacct: overflowing balance");
    }

    pub fn pending_inputs(&self) -> &[PendingInputEntry] {
        &self.pending_inputs
    }

    pub fn add_pending_input(&mut self, inp: PendingInputEntry) {
        self.pending_inputs.push(inp);
    }

    /// Removing some number of pending inputs.
    pub fn remove_pending_inputs(&mut self, n: usize) -> bool {
        if self.pending_inputs.len() < n {
            false
        } else {
            self.pending_inputs.drain(..n);
            true
        }
    }

    pub fn pending_fincls(&self) -> &[PendingFinclEntry] {
        &self.pending_fincls
    }

    pub fn add_pending_fincl(&mut self, inp: PendingFinclEntry) {
        self.pending_fincls.push(inp);
    }

    /// Removing some number of pending forced inclusions.
    pub fn remove_pending_fincls(&mut self, n: usize) -> bool {
        if self.pending_fincls.len() < n {
            false
        } else {
            self.pending_fincls.drain(..n);
            true
        }
    }
}

/// Pending input we expect to see in a block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PendingInputEntry {
    Deposit(SubjectDepositData),
}

impl PendingInputEntry {
    pub fn ty(&self) -> PendingInputType {
        match self {
            PendingInputEntry::Deposit(_) => PendingInputType::Deposit,
        }
    }
}

/// Pending input type.
#[repr(u8)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum PendingInputType {
    Deposit,
}

/// A pending forced inclusion that's been accepted by the EE account but not
/// included in a block yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingFinclEntry {
    epoch: u32,
    raw_tx_hash: Hash,
}
