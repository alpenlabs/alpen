//! Account output types that get applied to the ledger.

use strata_acct_types::{AcctId, MsgPayload};

/// Outputs from a snark account update.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateOutputs {
    transfers: Vec<OutputTransfer>,
    messages: Vec<OutputMessage>,
}

impl UpdateOutputs {
    pub fn new(transfers: Vec<OutputTransfer>, messages: Vec<OutputMessage>) -> Self {
        Self {
            transfers,
            messages,
        }
    }

    pub fn transfers(&self) -> &[OutputTransfer] {
        &self.transfers
    }

    pub fn messages(&self) -> &[OutputMessage] {
        &self.messages
    }
}

/// Transfer from one account to another.
///
/// This IS NOT a message and DOES NOT carry data.  This is NOT intended to be
/// for sending funds between subjects.  This is primarily intendede for future
/// functionality related to sequencer accounts.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct OutputTransfer {
    dest: AcctId,
    value: u64,
}

impl OutputTransfer {
    pub fn new(dest: AcctId, value: u64) -> Self {
        Self { dest, value }
    }

    pub fn dest(&self) -> AcctId {
        self.dest
    }

    pub fn value(&self) -> u64 {
        self.value
    }
}

/// Message from one account to another.
///
/// This DOES carry data and value.  This is how many EE-related features are
/// implemented, including
///
/// Drawing a parallel with EVM, this is like a contract call if it was
/// asynchronous.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputMessage {
    dest: AcctId,
    payload: MsgPayload,
}

impl OutputMessage {
    pub fn new(dest: AcctId, payload: MsgPayload) -> Self {
        Self { dest, payload }
    }

    pub fn dest(&self) -> AcctId {
        self.dest
    }

    pub fn payload(&self) -> &MsgPayload {
        &self.payload
    }
}
