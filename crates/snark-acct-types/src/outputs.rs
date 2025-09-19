//! Account output types that get applied to the ledger.

use strata_acct_types::{AcctId, MessageData};

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputMessage {
    dest: AcctId,
    data: MessageData,
}
