//! Bitcoin encoding for broadcast transaction entries.
//!
//! [`strata_db_types::l1_broadcast::L1TxEntry`] stores the transaction as opaque bytes so the
//! database crate stays free of a Bitcoin dependency. This extension trait supplies the
//! `Transaction` conversions for the code that actually deals in Bitcoin transactions.

use bitcoin::{
    consensus::{self, deserialize, serialize},
    Amount, FeeRate, Transaction,
};
use strata_db_types::l1_broadcast::{L1TxEntry, L1TxRbfInfo, L1TxStatus};

/// Bitcoin conversions for [`L1TxEntry`].
pub trait L1TxEntryExt: Sized {
    /// Creates an unpublished entry from a [`Transaction`].
    fn from_tx(tx: &Transaction) -> Self;

    /// Creates a writer-owned unpublished entry carrying the RBF metadata for `fee_rate` and
    /// `fee`.
    fn from_tx_with_fee(tx: &Transaction, fee_rate: FeeRate, fee: Amount) -> Self;

    /// Deserializes the stored bytes back into a [`Transaction`].
    fn try_to_tx(&self) -> Result<Transaction, consensus::encode::Error>;
}

impl L1TxEntryExt for L1TxEntry {
    fn from_tx(tx: &Transaction) -> Self {
        Self::new_unpublished(serialize(tx))
    }

    fn from_tx_with_fee(tx: &Transaction, fee_rate: FeeRate, fee: Amount) -> Self {
        Self::from_raw_parts(
            serialize(tx),
            L1TxStatus::Unpublished,
            Some(L1TxRbfInfo {
                fee_rate_sat_vb: fee_rate.to_sat_per_vb_ceil(),
                fee_sats: fee.to_sat(),
                replaces: None,
            }),
        )
    }

    fn try_to_tx(&self) -> Result<Transaction, consensus::encode::Error> {
        deserialize(self.tx_raw())
    }
}
