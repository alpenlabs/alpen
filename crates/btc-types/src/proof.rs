use arbitrary::Arbitrary;
use bitcoin::Transaction;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_identifiers::Buf32;

/// A trait for computing some kind of transaction ID (e.g., [`Txid`](bitcoin::Txid) or
/// [`Wtxid`](bitcoin::Wtxid)) from a [`Transaction`].
///
/// This trait is designed to be implemented by "marker" types that define how a transaction ID
/// should be computed. For example, [`TxIdMarker`] invokes [`Transaction::compute_txid`], and
/// [`WtxIdMarker`] invokes [`Transaction::compute_wtxid`]. This approach avoids duplicating
/// inclusion-proof or serialization logic across multiple ID computations.
pub trait TxIdComputable {
    /// Computes the transaction ID for the given transaction.
    ///
    /// The `idx` parameter allows marker types to handle special cases such as the coinbase
    /// transaction (which has a zero [`Wtxid`](bitcoin::Wtxid)) by looking up the transaction
    /// index.
    fn compute_id(tx: &Transaction, idx: usize) -> Buf32;
}

/// Marker type for computing the [`Txid`](bitcoin::Txid).
#[derive(
    Clone, Debug, PartialEq, Eq, Arbitrary, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct TxIdMarker;

/// Marker type for computing the [`Wtxid`](bitcoin::Wtxid).
#[derive(
    Clone, Debug, PartialEq, Eq, Arbitrary, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct WtxIdMarker;

impl TxIdComputable for TxIdMarker {
    fn compute_id(tx: &Transaction, _idx: usize) -> Buf32 {
        tx.compute_txid().into()
    }
}

impl TxIdComputable for WtxIdMarker {
    fn compute_id(tx: &Transaction, idx: usize) -> Buf32 {
        // Coinbase transaction wtxid is hash with zeroes
        if idx == 0 {
            return Buf32::zero();
        }
        tx.compute_wtxid().into()
    }
}
