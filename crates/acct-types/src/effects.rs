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
    /// if the transfer list is full, and errors if `sats` exceeds the Bitcoin
    /// money supply.
    pub fn push_transfer(&mut self, dest: AccountId, sats: u64) -> Result<bool, MsgPayloadError> {
        let value =
            BitcoinAmount::try_from(sats).map_err(|_| MsgPayloadError::ValueTooLarge { sats })?;
        Ok(self.add_transfer(SentTransfer::new(dest, value)))
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
    /// it.  Returns false if the message list is full, and errors if `sats`
    /// exceeds the Bitcoin money supply or the data exceeds the SSZ maximum
    /// length.
    pub fn push_message(
        &mut self,
        dest: AccountId,
        sats: u64,
        data: Vec<u8>,
    ) -> Result<bool, MsgPayloadError> {
        let value =
            BitcoinAmount::try_from(sats).map_err(|_| MsgPayloadError::ValueTooLarge { sats })?;
        let payload = MsgPayload::from_bytes(value, data)?;
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
            .try_fold(0u64, |acc, amount| acc.checked_add(amount.to_sat()))
            .and_then(|sats| BitcoinAmount::try_from(sats).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX_BITCOIN_MONEY_SATS: u64 = 21_000_000 * 100_000_000;

    fn test_account_id() -> AccountId {
        AccountId::from([1u8; 32])
    }

    #[test]
    fn push_transfer_accepts_max_money() {
        let mut effects = TxEffects::default();

        assert!(
            effects
                .push_transfer(test_account_id(), MAX_BITCOIN_MONEY_SATS)
                .expect("max-money transfer amount must succeed")
        );
        assert_eq!(effects.transfers_iter().count(), 1);
    }

    #[test]
    fn push_transfer_rejects_oversized_amount() {
        let mut effects = TxEffects::default();
        let sats = MAX_BITCOIN_MONEY_SATS + 1;

        let err = effects
            .push_transfer(test_account_id(), sats)
            .expect_err("oversized transfer amount must fail");

        assert_eq!(err, MsgPayloadError::ValueTooLarge { sats });
        assert_eq!(effects.transfers_iter().count(), 0);
    }

    #[test]
    fn push_message_rejects_oversized_value() {
        let mut effects = TxEffects::default();
        let sats = MAX_BITCOIN_MONEY_SATS + 1;

        let err = effects
            .push_message(test_account_id(), sats, vec![])
            .expect_err("oversized message value must fail");

        assert_eq!(err, MsgPayloadError::ValueTooLarge { sats });
        assert_eq!(effects.messages_iter().count(), 0);
    }
}
