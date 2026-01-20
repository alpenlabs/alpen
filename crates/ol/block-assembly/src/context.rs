//! Block assembly context trait.

use strata_asm_common::AsmLogEntry;
use strata_identifiers::{OLTxId, Slot};
use strata_ol_chain_types_new::OLTransaction;
use strata_primitives::l1::L1BlockCommitment;

use crate::error::BlockAssemblyError;

/// Context interface for block assembly.
///
/// Provides access to external resources needed for OL block assembly:
/// - ASM logs for L1 updates (checkpoints, deposits)
/// - Mempool for pending transactions
/// - Epoch sealing policy for terminal block decisions
///
/// # Design Notes
///
/// This trait provides access to external data sources needed during block assembly:
/// - **ASM logs**: Read-only access to validated L1 data (checkpoints, deposits)
/// - **Mempool**: Read and write access to pending transactions
/// - **Sealing policy**: Determines when to create terminal blocks
///
/// The context provides access to external resources
/// without owning or mutating them directly.
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

    /// Gets pending [`OLTransaction`] entries from mempool (up to `limit`).
    ///
    /// Returns transactions with their [`OLTxId`] values.
    /// Returns empty [`Vec`] if no transactions available (not an error).
    ///
    /// # Errors
    ///
    /// - [`BlockAssemblyError::Mempool`] - if mempool error occurs
    fn get_mempool_transactions(
        &self,
        limit: u64,
    ) -> Result<Vec<(OLTxId, OLTransaction)>, BlockAssemblyError>;

    /// Removes included transactions from mempool.
    ///
    /// Attempts to clear each transaction with the given [`OLTxId`].
    /// Returns the list of transaction IDs that were successfully removed.
    /// Returns empty [`Vec`] if no transactions were removed.
    ///
    /// This operation is idempotent - already-removed transactions will simply
    /// not appear in the returned list.
    ///
    /// # Errors
    ///
    /// - [`BlockAssemblyError::Mempool`] - if mempool error occurs (connectivity, internal error)
    fn clear_mempool_transactions(
        &self,
        txids: &[OLTxId],
    ) -> Result<Vec<OLTxId>, BlockAssemblyError>;

    /// Determines whether an epoch should be sealed at the given slot.
    ///
    /// Returns `true` if a terminal block should be created at this slot,
    /// `false` for a common block.
    fn should_seal_epoch(&self, slot: Slot) -> bool;
}
