//! Command types for mempool service.

use strata_identifiers::OLTxId;
use strata_ol_tx_types_v1::OLTransactionV1;
use strata_service::CommandCompletionSender;
use tokio::sync::oneshot;

use crate::{MempoolCandidates, MempoolTxInvalidReason, OLMempoolResult};

/// Type alias for transaction submission result.
type SubmitTransactionResult = OLMempoolResult<OLTxId>;

/// Commands that can be sent to the mempool service.
#[derive(Debug)]
pub enum MempoolCommand {
    /// Submit a transaction to the mempool.
    ///
    /// Validates and adds the transaction if it passes all checks.
    /// Returns the transaction ID on success.
    SubmitTransaction {
        /// Transaction to submit (boxed to reduce enum size).
        tx: Box<OLTransactionV1>,
        /// Completion sender for the transaction ID.
        completion: CommandCompletionSender<SubmitTransactionResult>,
    },

    /// Snapshot shared transaction bodies and ordering metadata for one selection attempt.
    GetCandidates {
        /// Completion sender for the snapshot.
        completion: CommandCompletionSender<MempoolCandidates>,
    },

    /// Report invalid transactions to the mempool.
    ReportInvalidTransactions {
        /// Transactions IDs with reasons for being invalid.
        txs: Vec<(OLTxId, MempoolTxInvalidReason)>,
        /// Completion sender.
        completion: CommandCompletionSender<()>,
    },
}

/// Helper function to create a completion channel pair.
///
/// Returns (CommandCompletionSender, Receiver) for command-response pattern.
pub(crate) fn create_completion<T>() -> (CommandCompletionSender<T>, oneshot::Receiver<T>) {
    let (tx, rx) = oneshot::channel();
    (CommandCompletionSender::new(tx), rx)
}
