//! Transaction effects.

use crate::{
    AccountId, BitcoinAmount, MsgPayload, MsgPayloadError, SentMessage, SentTransfer, TxEffects,
};

impl TxEffects {
    /// Attempts to add a transfer.
    ///
    /// Returns false if full.
    pub fn add_transfer(&mut self, xfr: SentTransfer) -> bool {
        match &mut self.transfers {
            ssz_types::Optional::Some(list) => list.push(xfr).is_ok(),
            none => {
                *none = ssz_types::Optional::Some(
                    vec![xfr]
                        .try_into()
                        .expect("transfer list must fit within SSZ max length"),
                );
                true
            }
        }
    }

    /// Adds a transfer to the given destination with the specified satoshi amount.
    ///
    /// Constructs a [`SentTransfer`] internally and appends it.  Returns false
    /// if the transfer list is full.
    pub fn push_transfer(&mut self, dest: AccountId, sats: u64) -> bool {
        self.add_transfer(SentTransfer::new(dest, BitcoinAmount::from_sat(sats)))
    }

    /// Returns an iterator over the transfers.
    pub fn transfers_iter(&self) -> impl Iterator<Item = &SentTransfer> {
        match &self.transfers {
            ssz_types::Optional::Some(list) => list.iter(),
            ssz_types::Optional::None => [].iter(),
        }
    }

    /// Attempts to add a message.
    ///
    /// Returns false if full.
    pub fn add_message(&mut self, msg: SentMessage) -> bool {
        match &mut self.messages {
            ssz_types::Optional::Some(list) => list.push(msg).is_ok(),
            none => {
                *none = ssz_types::Optional::Some(
                    vec![msg]
                        .try_into()
                        .expect("message list must fit within SSZ max length"),
                );
                true
            }
        }
    }

    /// Adds a message to the given destination with the specified value and data.
    ///
    /// Constructs a [`SentMessage`] (with [`MsgPayload`]) internally and appends
    /// it.  Returns false if the message list is full.
    pub fn push_message(
        &mut self,
        dest: AccountId,
        sats: u64,
        data: Vec<u8>,
    ) -> Result<bool, MsgPayloadError> {
        let payload = MsgPayload::from_bytes(BitcoinAmount::from_sat(sats), data)?;
        Ok(self.add_message(SentMessage::new(dest, payload)))
    }

    /// Returns an iterator over the messages.
    pub fn messages_iter(&self) -> impl Iterator<Item = &SentMessage> {
        match &self.messages {
            ssz_types::Optional::Some(list) => list.iter(),
            ssz_types::Optional::None => [].iter(),
        }
    }

    /// Gets the total value sent from the bundle of effects, or `None` if it's
    /// overflowing.
    pub fn get_total_value_sent(&self) -> Option<BitcoinAmount> {
        // Absolutely beautiful iterator combinator chain.
        self.transfers_iter()
            .map(|t| t.value())
            .chain(self.messages_iter().map(|m| m.payload().value()))
            .try_fold(BitcoinAmount::zero(), |acc, e| acc.checked_add(e))
    }
}

impl ITxEffects for &TxEffects {
    fn num_transfers(&self) -> usize {
        self.transfers_iter().count()
    }

    fn get_transfer(&self, idx: usize) -> Option<SentTransfer> {
        self.transfers_iter()
            .nth(idx)
            .map(|t| SentTransfer::new(t.dest(), t.value()))
    }

    fn num_messages(&self) -> usize {
        self.messages_iter().count()
    }

    fn get_message(&self, idx: usize) -> Option<SentMessage> {
        self.messages_iter()
            .nth(idx)
            .map(|m| SentMessage::new(m.dest(), m.payload().clone()))
    }
}

/// Describes outputs from a transaction abstractly.
pub trait ITxEffects {
    /// Gets the number of transfers being sent.
    fn num_transfers(&self) -> usize;

    /// Gets the transfer data by idx.
    fn get_transfer(&self, idx: usize) -> Option<SentTransfer>;

    /// Returns an iterator over the transfers.
    fn transfers_iter(&self) -> impl Iterator<Item = SentTransfer> {
        (0..self.num_transfers()).map(|i| {
            self.get_transfer(i)
                .expect("acct-types: incorrect ITxEffects impl")
        })
    }

    /// Gets the number of messages being sent.
    fn num_messages(&self) -> usize;

    /// Gets the message data by idx.
    fn get_message(&self, idx: usize) -> Option<SentMessage>;

    /// Returns an iterator over the transfers.
    fn messages_iter(&self) -> impl Iterator<Item = SentMessage> {
        (0..self.num_messages()).map(|i| {
            self.get_message(i)
                .expect("acct-types: incorrectt ITxEffects impl")
        })
    }
}
