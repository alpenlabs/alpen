//! Account output types that get applied to the ledger.

use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload};

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

    pub fn new_empty() -> Self {
        Self::new(Vec::new(), Vec::new())
    }

    pub fn transfers(&self) -> &[OutputTransfer] {
        &self.transfers
    }

    pub fn transfers_mut(&mut self) -> &mut Vec<OutputTransfer> {
        &mut self.transfers
    }

    pub fn messages(&self) -> &[OutputMessage] {
        &self.messages
    }

    pub fn messages_mut(&mut self) -> &mut Vec<OutputMessage> {
        &mut self.messages
    }

    pub fn total_output_value(&self) -> Option<BitcoinAmount> {
        let mut total_sent = BitcoinAmount::zero();

        for t in self.transfers() {
            total_sent = total_sent.checked_add(t.value())?;
        }

        for m in self.messages() {
            total_sent = total_sent.checked_add(m.payload().value())?;
        }

        Some(total_sent)
    }
}

/// Transfer from one account to another.
///
/// This IS NOT a message and DOES NOT carry data.  This is NOT intended to be
/// for sending funds between subjects.  This is primarily intendede for future
/// functionality related to sequencer accounts.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct OutputTransfer {
    dest: AccountId,
    value: BitcoinAmount,
}

impl OutputTransfer {
    pub fn new(dest: AccountId, value: BitcoinAmount) -> Self {
        Self { dest, value }
    }

    pub fn dest(&self) -> AccountId {
        self.dest
    }

    pub fn value(&self) -> BitcoinAmount {
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
    dest: AccountId,
    payload: MsgPayload,
}

impl OutputMessage {
    pub fn new(dest: AccountId, payload: MsgPayload) -> Self {
        Self { dest, payload }
    }

    pub fn dest(&self) -> AccountId {
        self.dest
    }

    pub fn payload(&self) -> &MsgPayload {
        &self.payload
    }
}
