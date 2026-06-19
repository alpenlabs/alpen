//! L1 broadcast database interface and its transaction-entry record types.

use std::fmt;

use arbitrary::Arbitrary;
use bitcoin::consensus::{self, deserialize, serialize};
use bitcoin::Transaction;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_identifiers::Buf32;
use strata_primitives::L1Height;

use crate::DbResult;

/// This is the entry that gets saved to the database corresponding to a bitcoin transaction that
/// the broadcaster will publish and watches for until finalization
#[derive(
    Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary, Serialize, Deserialize,
)]
pub struct L1TxEntry {
    /// Raw serialized transaction. This is basically `consensus::serialize()` of [`Transaction`]
    tx_raw: Vec<u8>,

    /// The status of the transaction in bitcoin
    pub status: L1TxStatus,
}

impl L1TxEntry {
    /// Create a new [`L1TxEntry`] from a [`Transaction`].
    pub fn from_tx(tx: &Transaction) -> Self {
        Self {
            tx_raw: serialize(tx),
            status: L1TxStatus::Unpublished,
        }
    }

    /// Returns the raw serialized transaction.
    ///
    /// # Note
    ///
    /// Whenever possible use [`try_to_tx()`](L1TxEntry::try_to_tx) to deserialize the transaction.
    /// This imposes more strict type checks.
    pub fn tx_raw(&self) -> &[u8] {
        &self.tx_raw
    }

    /// Deserializes the raw transaction into a [`Transaction`].
    pub fn try_to_tx(&self) -> Result<Transaction, consensus::encode::Error> {
        deserialize(&self.tx_raw)
    }

    pub fn is_valid(&self) -> bool {
        !matches!(self.status, L1TxStatus::InvalidInputs)
    }

    pub fn is_finalized(&self) -> bool {
        matches!(self.status, L1TxStatus::Finalized { .. })
    }
}

/// The possible statuses of a publishable transaction
#[derive(
    Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary, Serialize, Deserialize,
)]
#[serde(tag = "status")]
pub enum L1TxStatus {
    /// The transaction is waiting to be published
    Unpublished,

    /// The transaction is published
    Published,

    /// The transaction is included in L1 with the given number of confirmations.
    ///
    /// `block_hash` and `block_height` identify the L1 block the transaction was included in.
    Confirmed {
        confirmations: u64,
        block_hash: Buf32,
        block_height: L1Height,
    },

    /// The transaction is finalized in L1 with the given number of confirmations.
    ///
    /// `block_hash` and `block_height` identify the L1 block the transaction was included in.
    Finalized {
        confirmations: u64,
        block_hash: Buf32,
        block_height: L1Height,
    },

    /// The transaction is not included in L1 because it's inputs were invalid
    InvalidInputs,
}

impl fmt::Display for L1TxStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unpublished => f.write_str("unpublished"),
            Self::Published => f.write_str("published"),
            Self::Confirmed {
                confirmations,
                block_hash,
                block_height,
            } => {
                write!(
                    f,
                    "confirmed@{block_height}/{block_hash} ({confirmations} confs)"
                )
            }
            Self::Finalized {
                confirmations,
                block_hash,
                block_height,
            } => {
                write!(
                    f,
                    "finalized@{block_height}/{block_hash} ({confirmations} confs)"
                )
            }
            Self::InvalidInputs => f.write_str("invalid_inputs"),
        }
    }
}

/// A trait encapsulating the provider and store traits for interacting with the broadcast
/// transactions([`L1TxEntry`]), their indices and ids
#[cfg_attr(
    feature = "proxies",
    strata_db_macros::gen_proxy(error = crate::DbError, tracing_component = "storage:l1_broadcast")
)]
pub trait L1BroadcastDatabase: Send + Sync + 'static {
    /// Updates/Inserts a txentry to database. Returns Some(idx) if newly inserted else None
    fn put_tx_entry(&self, txid: Buf32, txentry: L1TxEntry) -> DbResult<Option<u64>>;

    /// Updates an existing txentry
    fn put_tx_entry_by_idx(&self, idx: u64, txentry: L1TxEntry) -> DbResult<()>;

    /// Delete a specific tx entry by its ID.
    ///
    /// Returns true if the tx entry existed and was deleted, false otherwise.
    fn del_tx_entry(&self, txid: Buf32) -> DbResult<bool>;

    /// Delete tx entries from the specified index onwards (inclusive).
    ///
    /// This method deletes all tx entries with index >= start_idx.
    /// Returns a vector of deleted tx indices.
    fn del_tx_entries_from_idx(&self, start_idx: u64) -> DbResult<Vec<u64>>;

    /// Fetch [`L1TxEntry`] from db
    fn get_tx_entry_by_id(&self, txid: Buf32) -> DbResult<Option<L1TxEntry>>;

    /// Get next index to be inserted to
    fn get_next_tx_idx(&self) -> DbResult<u64>;

    /// Get transaction id for index
    fn get_txid(&self, idx: u64) -> DbResult<Option<Buf32>>;

    /// get txentry by idx
    fn get_tx_entry(&self, idx: u64) -> DbResult<Option<L1TxEntry>>;

    /// Get last broadcast entry
    fn get_last_tx_entry(&self) -> DbResult<Option<L1TxEntry>>;
}

#[cfg(test)]
mod tests {
    use serde_json;

    use super::*;

    #[test]
    fn check_serde_of_l1txstatus() {
        let test_cases: Vec<(L1TxStatus, &str)> = vec![
            (L1TxStatus::Unpublished, r#"{"status":"Unpublished"}"#),
            (L1TxStatus::Published, r#"{"status":"Published"}"#),
            (
                L1TxStatus::Confirmed {
                    confirmations: 10,
                    block_hash: Buf32::zero(),
                    block_height: 42,
                },
                r#"{"status":"Confirmed","confirmations":10,"block_hash":"0000000000000000000000000000000000000000000000000000000000000000","block_height":42}"#,
            ),
            (
                L1TxStatus::Finalized {
                    confirmations: 100,
                    block_hash: Buf32::zero(),
                    block_height: 42,
                },
                r#"{"status":"Finalized","confirmations":100,"block_hash":"0000000000000000000000000000000000000000000000000000000000000000","block_height":42}"#,
            ),
            (L1TxStatus::InvalidInputs, r#"{"status":"InvalidInputs"}"#),
        ];

        // check serialization and deserialization
        for (l1_tx_status, serialized) in test_cases {
            let actual = serde_json::to_string(&l1_tx_status).unwrap();
            assert_eq!(actual, serialized);

            let actual: L1TxStatus = serde_json::from_str(serialized).unwrap();
            assert_eq!(actual, l1_tx_status);
        }
    }

    #[test]
    fn display_l1txstatus_uses_log_friendly_format() {
        let status = L1TxStatus::Confirmed {
            confirmations: 12,
            block_hash: Buf32::zero(),
            block_height: 42,
        };

        assert_eq!(status.to_string(), "confirmed@42/000000..000000 (12 confs)");
    }
}
