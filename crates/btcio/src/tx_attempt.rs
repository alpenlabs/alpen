//! Bitcoin encoding for fee-bump attempt records.
//!
//! [`strata_db_types::fee_bump`] stores attempt transactions as opaque bytes so the database
//! crate stays free of a Bitcoin dependency. This module supplies the [`Transaction`]
//! conversions for the code that actually deals in Bitcoin transactions.

use bitcoin::{
    consensus::{self, deserialize, serialize},
    hashes::Hash,
    Amount, FeeRate, Transaction,
};
use strata_db_types::{
    common::{L1TxId, L1WtxId},
    fee_bump::{TxAttempt, TxAttemptParts},
};

/// Builds the persistent attempt material for `tx`.
pub fn attempt_parts(tx: &Transaction, fee_rate: FeeRate, fee: Amount) -> TxAttemptParts {
    TxAttemptParts {
        raw_tx: serialize(tx),
        txid: L1TxId::from(tx.compute_txid().to_byte_array()),
        wtxid: L1WtxId::from(tx.compute_wtxid().to_byte_array()),
        fee_rate_sat_vb: fee_rate.to_sat_per_vb_ceil(),
        fee_sats: fee.to_sat(),
    }
}

/// Bitcoin conversions for [`TxAttempt`].
pub trait TxAttemptExt {
    /// Deserializes the raw transaction for this attempt.
    fn try_to_tx(&self) -> Result<Transaction, consensus::encode::Error>;

    /// Returns the fee rate the attempt was built at.
    fn fee_rate(&self) -> Option<FeeRate>;

    /// Returns the absolute fee the attempt pays.
    fn fee(&self) -> Amount;
}

impl TxAttemptExt for TxAttempt {
    fn try_to_tx(&self) -> Result<Transaction, consensus::encode::Error> {
        deserialize(&self.raw_tx)
    }

    fn fee_rate(&self) -> Option<FeeRate> {
        FeeRate::from_sat_per_vb(self.fee_rate_sat_vb)
    }

    fn fee(&self) -> Amount {
        Amount::from_sat(self.fee_sats)
    }
}
