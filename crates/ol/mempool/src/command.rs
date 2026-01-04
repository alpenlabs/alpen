//! Command types for mempool service.

use strata_identifiers::OLTxId;
use strata_service::CommandCompletionSender;
use tokio::sync::oneshot;

use crate::{MempoolTxRemovalReason, OLMempoolResult, types::OLMempoolTransaction};

/// Type alias for transaction submission result.
type SubmitTransactionResult = OLMempoolResult<OLTxId>;

/// Type alias for get transactions result.
type GetTransactionsResult = OLMempoolResult<Vec<(OLTxId, OLMempoolTransaction)>>;

/// Type alias for transaction removal result (returns IDs of removed transactions).
type RemoveTransactionsResult = OLMempoolResult<Vec<OLTxId>>;

/// Commands that can be sent to the mempool service.
#[derive(Debug)]
pub enum MempoolCommand {
    /// Submit a transaction to the mempool.
    ///
    /// Validates and adds the transaction if it passes all checks.
    /// Returns the transaction ID on success.
    SubmitTransaction {
        /// Transaction to submit (boxed to reduce enum size).
        tx: Box<OLMempoolTransaction>,
        /// Completion sender for the transaction ID.
        completion: CommandCompletionSender<SubmitTransactionResult>,
    },

    /// Get transactions from the mempool in priority order.
    ///
    /// Returns up to `limit` transactions.
    GetTransactions {
        /// Maximum number of transactions to return.
        limit: usize,
        /// Completion sender for the result.
        completion: CommandCompletionSender<GetTransactionsResult>,
    },

    /// Remove transactions from the mempool.
    ///
    /// Each transaction specifies a removal reason that determines cascade behavior:
    /// - `Included`: Transaction was included in block
    /// - `Failed`: Transaction execution or validation failed
    RemoveTransactions {
        /// Transactions to remove with their removal reasons.
        txs: Vec<(OLTxId, MempoolTxRemovalReason)>,
        /// Completion sender for the result.
        completion: CommandCompletionSender<RemoveTransactionsResult>,
    },
}

/// Helper function to create a completion channel pair.
///
/// Returns (CommandCompletionSender, Receiver) for command-response pattern.
pub(crate) fn create_completion<T>() -> (CommandCompletionSender<T>, oneshot::Receiver<T>) {
    let (tx, rx) = oneshot::channel();
    (CommandCompletionSender::new(tx), rx)
}
