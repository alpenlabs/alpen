//! Account output types that get applied to the ledger.

use strata_acct_types::{AcctId, MsgPayload};

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
