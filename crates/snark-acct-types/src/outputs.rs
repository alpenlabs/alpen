//! Account output types that get applied to the ledger.

use ssz_types::VariableList;
use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload};

use crate::ssz_generated::ssz::outputs::{
    MAX_MESSAGES, MAX_TRANSFERS, OutputMessage, OutputTransfer, UpdateOutputs,
};

impl UpdateOutputs {
    /// Creates new update outputs.
    pub fn new(transfers: Vec<OutputTransfer>, messages: Vec<OutputMessage>) -> Self {
        Self {
            // FIXME does this panic if the vecs are too large?
            transfers: transfers.into(),
            messages: messages.into(),
        }
    }

    /// Creates empty update outputs.
    pub fn new_empty() -> Self {
        Self::new(Vec::new(), Vec::new())
    }

    /// Gets the transfers.
    pub fn transfers(&self) -> &[OutputTransfer] {
        self.transfers.as_ref()
    }

    /// Gets mutable transfers.
    pub fn transfers_mut(
        &mut self,
    ) -> &mut VariableList<OutputTransfer, { MAX_TRANSFERS as usize }> {
        &mut self.transfers
    }

    /// Gets the messages.
    pub fn messages(&self) -> &[OutputMessage] {
        self.messages.as_ref()
    }

    /// Gets mutable messages.
    pub fn messages_mut(&mut self) -> &mut VariableList<OutputMessage, { MAX_MESSAGES as usize }> {
        &mut self.messages
    }

    /// Tries to extend transfers with items from an iterator.
    ///
    /// Returns an error if adding all items would exceed capacity.
    /// Does not modify the list if capacity would be exceeded.
    pub fn try_extend_transfers<I>(&mut self, iter: I) -> Result<(), &'static str>
    where
        I: IntoIterator<Item = OutputTransfer>,
    {
        let items: Vec<_> = iter.into_iter().collect();
        let needed = self.transfers.len() + items.len();

        if needed > MAX_TRANSFERS as usize {
            return Err("transfers capacity would be exceeded");
        }

        for item in items {
            self.transfers.push(item).expect("capacity already checked");
        }

        Ok(())
    }

    /// Tries to extend messages with items from an iterator.
    ///
    /// Returns an error if adding all items would exceed capacity.
    /// Does not modify the list if capacity would be exceeded.
    pub fn try_extend_messages<I>(&mut self, iter: I) -> Result<(), &'static str>
    where
        I: IntoIterator<Item = OutputMessage>,
    {
        let items: Vec<_> = iter.into_iter().collect();
        let needed = self.messages.len() + items.len();

        if needed > MAX_MESSAGES as usize {
            return Err("messages capacity would be exceeded");
        }

        for item in items {
            self.messages.push(item).expect("capacity already checked");
        }

        Ok(())
    }
}

impl OutputTransfer {
    /// Creates a new output transfer.
    pub fn new(dest: AccountId, value: BitcoinAmount) -> Self {
        Self { dest, value }
    }

    /// Gets the destination account ID.
    pub fn dest(&self) -> AccountId {
        self.dest
    }

    /// Gets the transfer value.
    pub fn value(&self) -> BitcoinAmount {
        self.value
    }
}

impl OutputMessage {
    /// Creates a new output message.
    pub fn new(dest: AccountId, payload: MsgPayload) -> Self {
        Self { dest, payload }
    }

    /// Gets the destination account ID.
    pub fn dest(&self) -> AccountId {
        self.dest
    }

    /// Gets the message payload.
    pub fn payload(&self) -> &MsgPayload {
        &self.payload
    }
}
