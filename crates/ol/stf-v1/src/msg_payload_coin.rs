//! In-flight message payload carrying linear-typed value.

use strata_acct_types::{BitcoinAmount, MsgPayload, MsgPayloadData};
use strata_ledger_types::Coin;

/// A message payload whose value is carried as a linear [`Coin`] rather than a
/// plain [`BitcoinAmount`].
///
/// This is the execution-time counterpart to [`MsgPayload`]: it is used while
/// value is in flight between accounts so that Rust enforces the value is
/// eventually credited, limboed, or otherwise consumed exactly once.  It is not
/// serialized or stored; [`MsgPayload`] remains the on-ledger record type.
pub(crate) struct MsgPayloadCoin {
    coin: Coin,
    data: MsgPayloadData,
}

impl MsgPayloadCoin {
    /// Creates a new payload pairing a coin with message data.
    pub(crate) fn new(coin: Coin, data: MsgPayloadData) -> Self {
        Self { coin, data }
    }

    /// Gets the value carried by the coin.
    pub(crate) fn coin_amt(&self) -> BitcoinAmount {
        self.coin.amt()
    }

    /// Gets the message data buffer.
    pub(crate) fn data(&self) -> &[u8] {
        &self.data
    }

    /// Consumes the payload, discarding the data and returning the live coin.
    pub(crate) fn into_coin(self) -> Coin {
        self.coin
    }

    /// Splits the payload into its live coin and a reconstructed [`MsgPayload`]
    /// record capturing the same value and data.
    ///
    /// The record is a copy for storage (e.g. a snark account inbox); the value
    /// it names is not linear and does not track the returned coin.
    pub(crate) fn into_coin_and_record(self) -> (Coin, MsgPayload) {
        let record = MsgPayload::new(self.coin.amt(), self.data);
        (self.coin, record)
    }
}
