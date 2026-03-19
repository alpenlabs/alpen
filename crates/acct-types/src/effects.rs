//! Transaction effects.

use crate::{AccountId, BitcoinAmount, MsgPayload, SentMessage, SentTransfer, TxEffects};

impl TxEffects {
    /// Attempts to add a transfer.
    ///
    /// Returns false if full.
    pub fn add_transfer(&mut self, xfr: SentTransfer) -> bool {
        match &mut self.transfers {
            ssz_types::Optional::Some(list) => list.push(xfr).is_ok(),
            none => {
                *none = ssz_types::Optional::Some(vec![xfr].into());
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
                *none = ssz_types::Optional::Some(vec![msg].into());
                true
            }
        }
    }

    /// Adds a message to the given destination with the specified value and data.
    ///
    /// Constructs a [`SentMessage`] (with [`MsgPayload`]) internally and appends
    /// it.  Returns false if the message list is full.
    pub fn push_message(&mut self, dest: AccountId, sats: u64, data: Vec<u8>) -> bool {
        let payload = MsgPayload::new(BitcoinAmount::from_sat(sats), data);
        self.add_message(SentMessage::new(dest, payload))
    }

    /// Returns an iterator over the messages.
    pub fn messages_iter(&self) -> impl Iterator<Item = &SentMessage> {
        match &self.messages {
            ssz_types::Optional::Some(list) => list.iter(),
            ssz_types::Optional::None => [].iter(),
        }
    }
}
