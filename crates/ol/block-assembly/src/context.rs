//! Block assembly context trait.

use std::sync::Arc;

use strata_asm_common::AsmLogEntry;
use strata_identifiers::{L1BlockCommitment, OLTxId};
use strata_ol_mempool::{
    BestTransactions, BestTransactionsIterator, MempoolHandle, OLMempoolError,
};
use strata_params::Params;
use strata_storage::NodeStorage;

use crate::error::BlockAssemblyError;

/// Context interface for block assembly.
///
/// Provides access to external resources needed for OL block assembly:
/// - ASM logs for L1 updates (checkpoints, deposits)
/// - Mempool for pending transactions
///
/// # Design Notes
///
/// This trait provides access to external data sources needed during block assembly:
/// - **ASM logs**: Read-only access to validated L1 data (checkpoints, deposits)
/// - **Mempool**: Read and write access to pending transactions
///
/// The context provides access to external resources
/// without owning or mutating them directly.
///
/// # Transaction Flow
///
/// Block assembly retrieves `OLMempoolTransaction` entries (without accumulator proofs),
/// generates accumulator proofs during block assembly, then converts them to `OLTransaction`
/// (with proofs) for inclusion in blocks.
///
/// Transactions are retrieved via `get_mempool_transactions()` which returns an iterator. During
/// iteration, block assembly can mark transactions as invalid using `mark_invalid()`. After
/// iteration completes, call `remove_mempool_transactions()` to remove all marked transactions
/// from the mempool.
pub trait BlockAssemblyContext {
    /// Gets ASM's latest processed L1 block commitment (tip).
    ///
    /// Returns `Ok(None)` if no L1 blocks processed yet, `Err` with [`BlockAssemblyError`] if error
    /// occurs.
    fn get_latest_l1_block(&self) -> Result<Option<L1BlockCommitment>, BlockAssemblyError>;

    /// Fetches ASM logs in the [`L1BlockCommitment`] range `[from_block, to_block]` (inclusive).
    ///
    /// Returns entries in ascending order by L1 block height.
    /// Logs are used for scanning checkpoints and creating ASM manifest entries.
    /// Returns empty [`Vec`] if no logs available in range.
    ///
    /// # Errors
    ///
    /// - [`BlockAssemblyError::InvalidRange`] - if `from_block` height > `to_block` height
    /// - [`BlockAssemblyError::Database`] - if database error occurs
    fn get_asm_logs_range(
        &self,
        from_block: L1BlockCommitment,
        to_block: L1BlockCommitment,
    ) -> Result<Vec<(L1BlockCommitment, Vec<AsmLogEntry>)>, BlockAssemblyError>;

    /// Gets an iterator over pending [`OLMempoolTransaction`] entries from mempool.
    ///
    /// Returns an iterator that yields transactions in priority order with their [`OLTxId`] values.
    /// Block assembly can mark transactions as invalid during iteration, then call
    /// [`remove_mempool_transactions`] to remove them after iteration completes.
    ///
    /// # Usage
    ///
    /// ```rust,no_run
    /// let mut iter = ctx.get_mempool_transactions()?;
    /// while let Some((txid, tx)) = iter.next() {
    ///     match validate_and_execute(tx) {
    ///         Ok(_) => include_in_block(tx),
    ///         Err(_) => iter.mark_invalid(&txid),
    ///     }
    /// }
    /// let invalid_txids = iter.marked_invalid();
    /// ctx.remove_mempool_transactions(&invalid_txids)?;
    /// ```
    ///
    /// # Errors
    ///
    /// - [`BlockAssemblyError::Mempool`] - if mempool error occurs
    fn get_mempool_transactions(
        &self,
    ) -> Result<Box<dyn BestTransactions + Send>, BlockAssemblyError>;

    /// Removes transactions from mempool by their IDs.
    ///
    /// Returns the list of transaction IDs that were successfully removed.
    /// Transaction IDs that don't exist in the mempool are silently ignored.
    ///
    /// # Errors
    ///
    /// - [`BlockAssemblyError::Mempool`] - if mempool error occurs
    fn remove_mempool_transactions(
        &self,
        txids: &[OLTxId],
    ) -> Result<Vec<OLTxId>, BlockAssemblyError>;
}

/// Concrete implementation of [`BlockAssemblyContext`] using [`NodeStorage`] and [`MempoolHandle`].
#[expect(
    missing_debug_implementations,
    reason = "Inner types don't have Debug implementation"
)]
pub struct BlockAssemblyContextImpl {
    params: Arc<Params>,
    storage: Arc<NodeStorage>,
    mempool_handle: Arc<MempoolHandle>,
}

impl BlockAssemblyContextImpl {
    /// Create a new block assembly context.
    pub fn new(
        params: Arc<Params>,
        storage: Arc<NodeStorage>,
        mempool_handle: Arc<MempoolHandle>,
    ) -> Self {
        Self {
            params,
            storage,
            mempool_handle,
        }
    }

    /// Get a reference to params.
    pub fn params(&self) -> &Params {
        &self.params
    }

    /// Get a reference to storage.
    pub fn storage(&self) -> &NodeStorage {
        &self.storage
    }

    /// Get a reference to mempool handle.
    pub fn mempool_handle(&self) -> &MempoolHandle {
        &self.mempool_handle
    }
}

impl BlockAssemblyContext for BlockAssemblyContextImpl {
    fn get_latest_l1_block(&self) -> Result<Option<L1BlockCommitment>, BlockAssemblyError> {
        let asm_manager = self.storage.asm();
        let result = asm_manager
            .fetch_most_recent_state()
            .map_err(BlockAssemblyError::Database)?;
        Ok(result.map(|(commitment, _)| commitment))
    }

    fn get_asm_logs_range(
        &self,
        from_block: L1BlockCommitment,
        to_block: L1BlockCommitment,
    ) -> Result<Vec<(L1BlockCommitment, Vec<AsmLogEntry>)>, BlockAssemblyError> {
        // Validate range
        if from_block.height_u64() > to_block.height_u64() {
            return Err(BlockAssemblyError::InvalidRange {
                from_height: from_block.height_u64(),
                to_height: to_block.height_u64(),
            });
        }

        let asm_manager = self.storage.asm();

        // Calculate how many blocks we need (inclusive range)
        let block_count = (to_block.height_u64() - from_block.height_u64() + 1) as usize;

        // Get ASM states starting from from_block
        let states = asm_manager
            .get_states_from(from_block, block_count)
            .map_err(BlockAssemblyError::Database)?;

        // Filter to only include blocks in the range and extract logs
        let mut result = Vec::new();
        for (commitment, asm_state) in states {
            if commitment.height_u64() > to_block.height_u64() {
                break;
            }
            let logs = asm_state.logs().clone();
            result.push((commitment, logs));
        }

        Ok(result)
    }

    fn get_mempool_transactions(
        &self,
    ) -> Result<Box<dyn BestTransactions + Send>, BlockAssemblyError> {
        // Note: This is a blocking call, but get_mempool_transactions() is synchronous.
        // In a real async context, we'd need to handle this differently.
        // For now, we'll use tokio::runtime::Handle::current() to block on the async call.
        let handle = tokio::runtime::Handle::try_current().map_err(|_| {
            BlockAssemblyError::Mempool(OLMempoolError::ServiceClosed(
                "No tokio runtime available".to_string(),
            ))
        })?;

        let transactions = handle
            .block_on(self.mempool_handle.best_transactions())
            .map_err(BlockAssemblyError::Mempool)?;

        // Create iterator from transactions
        let iterator = BestTransactionsIterator::new(transactions);
        Ok(Box::new(iterator))
    }

    fn remove_mempool_transactions(
        &self,
        txids: &[OLTxId],
    ) -> Result<Vec<OLTxId>, BlockAssemblyError> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| {
            BlockAssemblyError::Mempool(OLMempoolError::ServiceClosed(
                "No tokio runtime available".to_string(),
            ))
        })?;

        let removed = handle
            .block_on(self.mempool_handle.remove_transactions(txids.to_vec()))
            .map_err(BlockAssemblyError::Mempool)?;

        Ok(removed)
    }
}
