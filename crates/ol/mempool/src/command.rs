//! Command types for mempool service.

use strata_identifiers::{OLBlockCommitment, OLTxId};
use strata_service::CommandCompletionSender;

use crate::{
    OLMempoolResult,
    types::{OLMempoolStats, OLMempoolTransaction},
};

/// Type alias for transaction submission result.
type SubmitTransactionResult = OLMempoolResult<OLTxId>;

/// Type alias for best transactions retrieval result.
type BestTransactionsResult = OLMempoolResult<Vec<(OLTxId, OLMempoolTransaction)>>;

/// Type alias for transaction removal result (returns IDs of removed transactions).
type RemoveTransactionsResult = OLMempoolResult<Vec<OLTxId>>;

/// Commands that can be sent to the mempool service.
#[derive(Debug)]
pub enum MempoolCommand {
    /// Submit a new transaction to the mempool.
    SubmitTransaction {
        /// Raw transaction bytes (opaque blob).
        tx_bytes: Vec<u8>,
        /// Completion sender for transaction ID.
        completion: CommandCompletionSender<SubmitTransactionResult>,
    },

    /// Get all best transactions for block assembly (iterator pattern).
    ///
    /// Returns all transactions in priority order for use with `BestTransactionsIterator`.
    /// This command is designed for the Reth-style iterator pattern where block assembly
    /// iterates over transactions and marks invalid ones for removal.
    ///
    /// For limited queries, callers can take the first N items from the returned iterator.
    BestTransactions {
        /// Completion sender for all transactions (txid, parsed transaction) in priority order.
        completion: CommandCompletionSender<BestTransactionsResult>,
    },

    /// Remove transactions from the mempool (after inclusion in block).
    RemoveTransactions {
        /// Transaction IDs to remove.
        ids: Vec<OLTxId>,
        /// Completion sender for IDs of removed transactions.
        completion: CommandCompletionSender<RemoveTransactionsResult>,
    },

    /// Check if a transaction exists in the mempool.
    Contains {
        /// Transaction ID to check.
        id: OLTxId,
        /// Completion sender for existence check.
        completion: CommandCompletionSender<bool>,
    },

    /// Get mempool statistics.
    Stats {
        /// Completion sender for statistics.
        completion: CommandCompletionSender<OLMempoolStats>,
    },

    /// Chain tip update (notification from fork-choice manager).
    ///
    /// Updates the mempool's view of the chain tip and revalidates all pending transactions.
    /// Returns the count of transactions removed due to invalidity.
    ///
    /// NOTE: This command does not include StateAccessor - the mempool creates it internally
    /// from storage and the new tip.
    ChainUpdate {
        /// New chain tip (slot + block ID).
        new_tip: OLBlockCommitment,
        /// Completion sender for count of removed transactions.
        completion: CommandCompletionSender<OLMempoolResult<usize>>,
    },
}
