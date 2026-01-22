//! Account output types that get applied to the ledger.

use ssz_types::VariableList;
use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload};

use crate::{
    error::OutputsError,
    ssz_generated::ssz::outputs::{
        MAX_MESSAGES, MAX_TRANSFERS, OutputMessage, OutputTransfer, UpdateOutputs,
    },
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
    pub fn try_extend_transfers<I>(&mut self, iter: I) -> Result<(), OutputsError>
    where
        I: IntoIterator<Item = OutputTransfer>,
        I::IntoIter: ExactSizeIterator,
    {
        let iter = iter.into_iter();
        let needed = self.transfers.len() + iter.len();

        if needed > MAX_TRANSFERS as usize {
            return Err(OutputsError::TransfersCapacityExceeded);
        }

        for item in iter {
            self.transfers.push(item).expect("capacity already checked");
        }

        Ok(())
    }

    /// Tries to extend messages with items from an iterator.
    ///
    /// Returns an error if adding all items would exceed capacity.
    /// Does not modify the list if capacity would be exceeded.
    pub fn try_extend_messages<I>(&mut self, iter: I) -> Result<(), OutputsError>
    where
        I: IntoIterator<Item = OutputMessage>,
        I::IntoIter: ExactSizeIterator,
    {
        let iter = iter.into_iter();
        let needed = self.messages.len() + iter.len();

        if needed > MAX_MESSAGES as usize {
            return Err(OutputsError::MessagesCapacityExceeded);
        }

        for item in iter {
            self.messages.push(item).expect("capacity already checked");
        }

        Ok(())
    }

    /// Computes the total value across all transfers and messages.
    /// Returns None if overflow occurs.
    pub fn compute_total_value(&self) -> Option<BitcoinAmount> {
        let mut total = BitcoinAmount::zero();
        for transfer in self.transfers() {
            total = total.checked_add(transfer.value())?;
        }
        for msg in self.messages() {
            total = total.checked_add(msg.payload().value())?;
        }
        Some(total)
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
