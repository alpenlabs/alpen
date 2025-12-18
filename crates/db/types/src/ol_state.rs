//! Database trait for OL state management.
//!
//! This module provides the database interface for storing and retrieving
//! OL execution state, write batches, ASM manifests, and snark account inbox messages.

use strata_acct_types::AccountId;
use strata_ol_state_types::{NativeAccountState, OLState, WriteBatch};
use strata_primitives::{buf::Buf32, l1::L1Height, l2::OLBlockId};
use strata_snark_acct_types::MessageEntry;

use crate::DbResult;

/// Type alias for OL write batch with native account state.
pub type OLWriteBatch = WriteBatch<NativeAccountState>;

/// Type alias for finalized OL state.
///
/// This represents a committed state at an epoch terminal that serves as a
/// base for reconstructing subsequent states.
pub type OLFinalizedState = OLState;

/// Database interface for OL state storage.
///
/// This trait provides methods for storing and retrieving:
///
/// - Write batches for each block (for state reconstruction)
/// - Finalized state snapshots (at epoch terminals)
/// - ASM manifest MMR entries (per L1 height)
/// - Snark account inbox messages
pub trait OLStateDatabase: Send + Sync + 'static {
    // ===== Write batch storage =====

    /// Stores a write batch for a given block.
    ///
    /// Write batches are keyed by block ID and used to reconstruct state
    /// from a finalized base state.
    fn put_slot_write_batch(&self, slot_blkid: OLBlockId, wb: OLWriteBatch) -> DbResult<()>;

    /// Retrieves a write batch for a given block.
    ///
    /// Returns `None` if no write batch exists for the given block ID.
    fn get_slot_write_batch(&self, slot_blkid: OLBlockId) -> DbResult<Option<OLWriteBatch>>;

    // ===== Finalized state management =====

    /// Stores a finalized state snapshot.
    ///
    /// This typically happens at epoch terminals and provides a base state
    /// for reconstructing subsequent states from write batches.
    fn put_finalized_state(&self, state: OLFinalizedState) -> DbResult<()>;

    /// Retrieves the current finalized state.
    ///
    /// Returns `None` if no finalized state has been stored yet.
    fn get_finalized_state(&self) -> DbResult<Option<OLFinalizedState>>;

    // ===== ASM manifest MMR entries =====

    /// Appends a manifest entry to the MMR for a given L1 height.
    ///
    /// This is used to track ASM manifests as they are processed during
    /// terminal block execution.
    fn append_manifest_entry(&self, height: L1Height, manifest_hash: Buf32) -> DbResult<()>;

    /// Retrieves the current manifest MMR root.
    ///
    /// This root represents the commitment to all manifest entries
    /// that have been appended so far.
    fn get_manifest_mmr_root(&self) -> DbResult<Buf32>;

    // ===== Snark account inbox messages =====

    /// Stores an inbox message for a snark account.
    ///
    /// Messages are stored per account with a sequential index.
    fn put_inbox_message(
        &self,
        acct_id: AccountId,
        msg_idx: u64,
        entry: MessageEntry,
    ) -> DbResult<()>;

    /// Retrieves inbox messages for a snark account.
    ///
    /// Returns up to `count` messages starting from `from_idx`.
    /// If fewer messages are available, returns only the available ones.
    fn get_inbox_messages(
        &self,
        acct_id: AccountId,
        from_idx: u64,
        count: u32,
    ) -> DbResult<Vec<MessageEntry>>;
}
