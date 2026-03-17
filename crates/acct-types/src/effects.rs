//! Transaction effects.

use crate::{SentMessage, SentTransfer, TxEffects};

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

    /// Returns an iterator over the transfers.
    pub fn transfers_iter(&self) -> impl Iterator<Item = &SentTransfer> {
        match &self.transfers {
            ssz_types::Optional::Some(list) => list.iter(),
            ssz_types::Optional::None => [].iter(),
        }
    }

    /// Attempts to add a transfer.
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

    /// Returns an iterator over the messages.
    pub fn messages_iter(&self) -> impl Iterator<Item = &SentMessage> {
        match &self.messages {
            ssz_types::Optional::Some(list) => list.iter(),
            ssz_types::Optional::None => [].iter(),
        }
    }
}
