//! L1 broadcast database interface and its transaction-entry record types.

use std::fmt;

use arbitrary::Arbitrary;
use bitcoin::consensus::{self, deserialize, serialize};
use bitcoin::{Amount, FeeRate, Transaction};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
#[cfg(feature = "proxies")]
use strata_db_macros::gen_proxy;
use strata_identifiers::Buf32;
use strata_primitives::L1Height;

use crate::common::L1TxId;
use crate::fee_bump::{TxNodeId, TxNodeRecord};
#[cfg(feature = "proxies")]
use crate::DbError;
use crate::DbResult;

/// This is the entry that gets saved to the database corresponding to a bitcoin transaction that
/// the broadcaster will publish and watches for until finalization
#[derive(Debug, Clone, PartialEq, Arbitrary, Serialize, Deserialize)]
pub struct L1TxEntry {
    /// Raw serialized transaction. This is basically `consensus::serialize()` of [`Transaction`]
    tx_raw: Vec<u8>,

    /// The status of the transaction in bitcoin
    pub status: L1TxStatus,

    /// Metadata used by writer-side RBF replacement logic.
    ///
    /// Only set for writer-owned entries published while fee bumping is enabled; `None` for every
    /// other entry, including ones written before fee bumping existed.
    pub rbf: Option<L1TxRbfInfo>,
}

impl L1TxEntry {
    /// Create a new [`L1TxEntry`] from a [`Transaction`].
    pub fn from_tx(tx: &Transaction) -> Self {
        Self {
            tx_raw: serialize(tx),
            status: L1TxStatus::Unpublished,
            rbf: None,
        }
    }

    /// Creates a writer-owned [`L1TxEntry`] carrying the RBF metadata for `fee_rate` and `fee`.
    pub fn from_tx_with_fee(tx: &Transaction, fee_rate: FeeRate, fee: Amount) -> Self {
        Self {
            tx_raw: serialize(tx),
            status: L1TxStatus::Unpublished,
            rbf: Some(L1TxRbfInfo {
                fee_rate_sat_vb: fee_rate.to_sat_per_vb_ceil(),
                fee_sats: fee.to_sat(),
                replaces: None,
            }),
        }
    }

    /// Records that this entry replaced `original_txid`.
    ///
    /// A no-op for an entry with no RBF metadata, which is never part of a replacement chain.
    pub fn set_replaces(&mut self, original_txid: L1TxId) {
        if let Some(rbf) = self.rbf.as_mut() {
            rbf.replaces = Some(original_txid);
        }
    }

    /// Creates an entry from persisted raw transaction bytes and metadata.
    pub fn from_raw_parts(tx_raw: Vec<u8>, status: L1TxStatus, rbf: Option<L1TxRbfInfo>) -> Self {
        Self {
            tx_raw,
            status,
            rbf,
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

    /// Reports whether the broadcaster should keep polling this entry under its own txid.
    ///
    /// `InvalidInputs` is dead: the inputs are gone and nothing can bring the transaction back.
    /// `Replaced` is not dead, but it is no longer the chain's head, so it is followed through
    /// the replacement rather than polled directly. A superseded transaction that a miner
    /// includes anyway is recovered by `adopt_live_ancestor`, which reverses the link and
    /// re-admits the winner.
    pub fn is_trackable(&self) -> bool {
        !matches!(
            self.status,
            L1TxStatus::InvalidInputs | L1TxStatus::Replaced { .. }
        )
    }

    pub fn is_finalized(&self) -> bool {
        matches!(self.status, L1TxStatus::Finalized { .. })
    }
}

/// RBF metadata for one concrete broadcast transaction.
///
/// Replacement bookkeeping (attempt history, publication height, terminal errors) lives on the
/// [`TxNodeRecord`] for the logical transaction. This entry-level record only carries what the
/// broadcast row itself needs: the fee rate the transaction was built at, which seeds the
/// tx-node's first attempt.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    BorshSerialize,
    BorshDeserialize,
    Arbitrary,
    Serialize,
    Deserialize,
)]
pub struct L1TxRbfInfo {
    /// Fee rate used to construct this transaction in sat/vB.
    pub fee_rate_sat_vb: u64,

    /// Absolute fee this transaction pays, in satoshis.
    ///
    /// Recorded rather than derived: for chunked commits the builder absorbs sub-dust change into
    /// the fee, so `fee_rate_sat_vb * vsize` under-reports it, and the BIP-125 absolute-fee floor
    /// for a replacement is computed from this value.
    pub fee_sats: u64,

    /// The transaction this one replaced, if it is itself a replacement.
    ///
    /// [`L1TxStatus::Replaced`] only links forward, which is all the live-entry lookup needs. This
    /// is the reverse link, and it exists for one case: a superseded ancestor that gets mined
    /// anyway. Bitcoin Core then reports negative confirmations for the replacement, and without a
    /// way back through the chain there is no way to find out which transaction actually won.
    #[serde(default)]
    pub replaces: Option<L1TxId>,
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

    /// The transaction has been superseded by an RBF replacement.
    Replaced {
        /// Replacement transaction id.
        by: L1TxId,
    },
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
            Self::Replaced { by } => write!(f, "replaced_by({by})"),
        }
    }
}

/// A trait encapsulating the provider and store traits for interacting with the broadcast
/// transactions([`L1TxEntry`]), their indices and ids
#[cfg_attr(
    feature = "proxies",
    gen_proxy(error = DbError, tracing_component = "storage:l1_broadcast")
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

    /// Atomically inserts `replacement` and marks `original_txid` as superseded by it.
    ///
    /// Both rows live in the broadcast tree, so the insert and the transition happen in one
    /// transaction. Doing them separately leaves a window where the replacement exists, and can be
    /// broadcast, with nothing linking it back to the transaction it supersedes.
    ///
    /// The transition follows the same eligibility rule as
    /// [`try_mark_tx_entry_replaced`](Self::try_mark_tx_entry_replaced). Returns the replacement's
    /// broadcast index when the swap applied, or `None` when the original was no longer
    /// replaceable, in which case nothing is written.
    fn put_replacement_tx_entry(
        &self,
        original_txid: Buf32,
        replacement_txid: Buf32,
        replacement: L1TxEntry,
    ) -> DbResult<Option<u64>>;

    /// Atomically marks `txid` as superseded by `replacement_txid`.
    ///
    /// The transition only applies to an entry that is still awaiting inclusion
    /// ([`L1TxStatus::Unpublished`] or [`L1TxStatus::Published`]); a transaction that already
    /// confirmed has won and is left untouched. Read and write happen in one transaction so a
    /// concurrent confirmation cannot be clobbered.
    ///
    /// Returns whether the entry was transitioned.
    fn try_mark_tx_entry_replaced(&self, txid: Buf32, replacement_txid: L1TxId) -> DbResult<bool>;

    /// Atomically reverses a replacement after the superseded transaction won on-chain.
    ///
    /// A miner can include an original after the local node accepted its replacement. The
    /// replacement can then never confirm, and the ancestor that did is still marked
    /// [`L1TxStatus::Replaced`], so the live-entry lookup walks straight past it.
    ///
    /// This repoints the chain at the winner: `winner_txid` takes `winner_status`, and `loser_txid`
    /// becomes `Replaced { by: winner_txid }`. Both writes commit together because the intermediate
    /// state is a cycle, which would make the lookup exhaust its hop budget and fail.
    ///
    /// The winner may sit anywhere in the loser's ancestry, not just one hop back: a chain bumped
    /// several times has intermediate attempts between them and a miner can include any of them.
    /// Intermediates keep their forward links, which still resolve to the winner through the
    /// reversed one.
    ///
    /// Returns whether the reversal applied; `false` when either row is missing, when the winner's
    /// replacement chain does not reach the loser, or when the loser has already left
    /// `Unpublished`/`Published`, so nothing is written. That last case means a concurrent
    /// replacement superseded the loser while this adoption was deciding, and reversing over it
    /// would cut the replacement out of the chain while it stays indexed and broadcastable.
    fn adopt_confirmed_ancestor(
        &self,
        loser_txid: Buf32,
        winner_txid: Buf32,
        winner_status: L1TxStatus,
    ) -> DbResult<bool>;

    /// Stores a logical transaction replacement-chain record.
    fn put_tx_node(&self, node_id: TxNodeId, record: TxNodeRecord) -> DbResult<()>;

    /// Fetches a logical transaction replacement-chain record by id.
    fn get_tx_node(&self, node_id: TxNodeId) -> DbResult<Option<TxNodeRecord>>;

    /// Fetches all logical transaction replacement-chain records.
    fn get_all_tx_nodes(&self) -> DbResult<Vec<TxNodeRecord>>;
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
            (
                L1TxStatus::Replaced { by: L1TxId::zero() },
                r#"{"status":"Replaced","by":"0000000000000000000000000000000000000000000000000000000000000000"}"#,
            ),
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
