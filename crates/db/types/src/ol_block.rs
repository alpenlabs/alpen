//! OL block database interface and its block-status type.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::Serialize;
#[cfg(feature = "proxies")]
use strata_db_macros::gen_proxy;
use strata_identifiers::{EpochCommitment, OLBlockCommitment, OLBlockId, Slot};
use strata_ol_chain_types::{OLBlock, OLBlockHeader};

#[cfg(feature = "proxies")]
use crate::DbError;
use crate::DbResult;

/// Gets the status of a block.
#[derive(
    Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, BorshSerialize, BorshDeserialize, Serialize,
)]
pub enum BlockStatus {
    /// Block's validity hasn't been checked yet.
    Unchecked,

    /// Block is valid, although this doesn't mean it's in the canonical chain.
    Valid,

    /// Block is invalid, for no particular reason.  We'd have to look somewhere
    /// else for that.
    Invalid,
}

/// Describes whether a full OL block body is locally readable at a known commitment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BlockAvailability {
    /// The full OL block is locally available.
    Available(Box<OLBlock>),

    /// The body is not retained locally because the commitment is at or below the history base
    /// (pruned or bootstrapped history).
    Pruned,

    /// The commitment is outside known pruned history and has no local block record.
    Missing,
}

/// OL data store for OL blocks. Does not store anything about what we think
/// the OL chain tip is, that's controlled by the consensus state.
///
/// This stores OL blocks (header + body) keyed by block commitment (slot + block ID).
#[cfg_attr(
    feature = "proxies",
    gen_proxy(error = DbError, tracing_component = "storage:ol")
)]
pub trait OLBlockDatabase: Send + Sync + 'static {
    /// Stores an OL block. The slot is extracted from the block header. Also sets the block's
    /// status to "unchecked" if this is a new block.
    fn put_block_data(&self, block: OLBlock) -> DbResult<()>;

    /// Returns the latest OL block committed through the high-watermark path, if any.
    ///
    /// This is not the highest block in the OL block database. Plain
    /// [`Self::put_block_data`] does not read or update it.
    fn get_block_high_watermark(&self) -> DbResult<Option<OLBlockCommitment>>;

    /// Stores an OL block and advances the block high-watermark atomically.
    ///
    /// Block persistence semantics match [`Self::put_block_data`]. If the block's slot is not
    /// strictly greater than the current high-watermark slot, this writes nothing and returns
    /// `DbError::BlockHighWatermarkConflict`.
    fn put_block_data_with_high_watermark(&self, block: OLBlock) -> DbResult<OLBlockCommitment>;

    /// Clears the block high-watermark if it currently equals `expected`.
    ///
    /// This does not delete block data, block status, or height-index entries.
    /// Returns `true` when the high-watermark was cleared.
    fn clear_block_high_watermark(&self, expected: OLBlockCommitment) -> DbResult<bool>;

    /// Rolls the block high-watermark back to an existing target block.
    ///
    /// This is for explicit recovery paths that revert OL state. If the current high-watermark is
    /// already at or below `target`, this is a no-op and returns `false`. Otherwise, the
    /// high-watermark is set to `target` and this returns `true`.
    fn rollback_block_high_watermark(&self, target: OLBlockCommitment) -> DbResult<bool>;

    /// Retrieves an OL block for a given block ID.
    fn get_block_data(&self, id: OLBlockId) -> DbResult<Option<OLBlock>>;

    /// Stores an unsigned checkpoint terminal [`OLBlockHeader`].
    ///
    /// Returns [`DbError::OLTerminalHeaderIdMismatch`] when the header does not compute to `id`.
    fn put_terminal_header(&self, id: OLBlockId, header: OLBlockHeader) -> DbResult<()>;

    /// Retrieves an unsigned checkpoint terminal [`OLBlockHeader`].
    fn get_terminal_header(&self, id: OLBlockId) -> DbResult<Option<OLBlockHeader>>;

    /// Retrieves an [`OLBlockHeader`] from a full block before consulting terminal headers.
    fn get_ol_header(&self, id: OLBlockId) -> DbResult<Option<OLBlockHeader>>;

    /// Returns the immutable base of locally available full OL block history.
    ///
    /// An absent marker means the database retains full history from genesis.
    fn get_history_base(&self) -> DbResult<Option<EpochCommitment>>;

    /// Classifies full-block availability using the commitment's slot and the history base.
    fn get_block_at(&self, commitment: OLBlockCommitment) -> DbResult<BlockAvailability>;

    /// Atomically promotes a checkpoint terminal into the canonical OL history anchor.
    ///
    /// The operation replaces the canonical suffix from the anchor slot with exactly the anchor
    /// and writes the history-base marker in the same transaction. Reapplying the same anchor is
    /// idempotent; a different existing marker returns [`DbError::OLHistoryBaseConflict`].
    fn promote_to_history_anchor(&self, anchor: EpochCommitment) -> DbResult<()>;

    /// Tries to delete an OL block from the store, returning if it really
    /// existed or not.
    fn del_block_data(&self, id: OLBlockId) -> DbResult<bool>;

    /// Sets the block's validity status.
    ///
    /// Returns `true` if the status was updated.
    fn set_block_status(&self, id: OLBlockId, status: BlockStatus) -> DbResult<bool>;

    /// Gets the OL block IDs that we have at some slot, in case there's more
    /// than one on competing forks.
    fn get_blocks_at_height(&self, slot: u64) -> DbResult<Vec<OLBlockId>>;

    /// Gets the validity status of a block.
    fn get_block_status(&self, id: OLBlockId) -> DbResult<Option<BlockStatus>>;

    /// Returns the highest slot recorded in the canonical OL block index.
    fn get_tip_slot(&self) -> DbResult<Slot>;

    /// Gets the canonical OL block id at a slot, as recorded by fork choice.
    ///
    /// Returns `None` for slots above the current canonical tip or never written.
    fn get_canonical_block(&self, slot: Slot) -> DbResult<Option<OLBlockId>>;

    /// Replaces the canonical suffix from `start_slot`.
    ///
    /// Atomically removes every canonical entry for slots greater than or equal to `start_slot`,
    /// then writes each block ID into a contiguous suffix starting at `start_slot`.
    ///
    /// Single-writer contract: callers must not invoke this concurrently with another canonical
    /// write; the atomicity guarantee covers the remove-then-insert against readers, not against a
    /// competing writer.
    fn replace_canonical_suffix_from(
        &self,
        start_slot: Slot,
        block_ids: Vec<OLBlockId>,
    ) -> DbResult<()>;
}
