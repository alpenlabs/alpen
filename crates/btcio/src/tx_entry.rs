//! Bitcoin encoding for broadcast transaction entries.
//!
//! [`strata_db_types::l1_broadcast::L1TxEntry`] stores the transaction as opaque bytes so the
//! database crate stays free of a Bitcoin dependency. This extension trait supplies the
//! `Transaction` conversions for the code that actually deals in Bitcoin transactions.

use bitcoin::{
    consensus::{self, deserialize, serialize},
    Transaction,
};
use strata_db_types::l1_broadcast::L1TxEntry;

/// Bitcoin conversions for [`L1TxEntry`].
pub trait L1TxEntryExt: Sized {
    /// Creates an unpublished entry from a [`Transaction`].
    fn from_tx(tx: &Transaction) -> Self;

    /// Deserializes the stored bytes back into a [`Transaction`].
    fn try_to_tx(&self) -> Result<Transaction, consensus::encode::Error>;
}

impl L1TxEntryExt for L1TxEntry {
    fn from_tx(tx: &Transaction) -> Self {
        Self::new_unpublished(serialize(tx))
    }

    fn try_to_tx(&self) -> Result<Transaction, consensus::encode::Error> {
        deserialize(self.tx_raw())
    }
}
