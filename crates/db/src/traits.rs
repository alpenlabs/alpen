//! Trait definitions for low level database interfaces.  This borrows some of
//! its naming conventions from reth.

use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::{
    batch::EpochSummary,
    l1::{L1Tx, *},
    prelude::*,
    proof::{ProofContext, ProofKey},
};
use strata_state::{
    block::L2BlockBundle, chain_state::Chainstate, client_state::L1ClientState, operation::*,
    state_op::WriteBatchEntry, sync_event::SyncEvent,
};
use zkaleido::ProofReceipt;

use crate::{
    entities::bridge_tx_state::BridgeTxState,
    types::{BundledPayloadEntry, CheckpointEntry, IntentEntry, L1TxEntry},
    DbResult,
};

/// Common database interface that we can parameterize worker tasks over if
/// parameterizing them over each individual trait gets cumbersome or if we need
/// to use behavior that crosses different interfaces.
pub trait Database {
    type L1DB: L1Database + Send + Sync;
    type L2DB: L2BlockDatabase + Send + Sync;
    type SyncEventDB: SyncEventDatabase + Send + Sync;
    type ClientStateDB: ClientStateDatabase + Send + Sync;
    type ChainstateDB: ChainstateDatabase + Send + Sync;
    type CheckpointDB: CheckpointDatabase + Send + Sync;

    fn l1_db(&self) -> &Arc<Self::L1DB>;
    fn l2_db(&self) -> &Arc<Self::L2DB>;
    fn sync_event_db(&self) -> &Arc<Self::SyncEventDB>;
    fn client_state_db(&self) -> &Arc<Self::ClientStateDB>;
    fn chain_state_db(&self) -> &Arc<Self::ChainstateDB>;
    fn checkpoint_db(&self) -> &Arc<Self::CheckpointDB>;
}

/// Database interface to control our view of L1 data.
/// Operations are NOT VALIDATED at this level.
/// Ensure all operations are done through `L1BlockManager`
pub trait L1Database {
    /// Atomically extends the chain with a new block, providing the manifest
    /// and a list of transactions we find relevant.  Returns error if
    /// provided out-of-order.
    #[deprecated]
    fn put_block_data(&self, mf: L1BlockManifest) -> DbResult<()>;

    /// Store new block as pending validation
    fn put_block_pending_validation(&self, block: L1Block) -> DbResult<()>;

    /// mark new block as valid and store accumulated pow
    fn mark_block_valid(&self, block_id: L1BlockId, height: u64, pow: [u8; 32]) -> DbResult<()>;

    /// remove invalid block from db
    fn remove_invalid_block(&self, block_id: L1BlockId) -> DbResult<()>;

    /// Set a specific height, blockid in canonical chain records.
    #[deprecated]
    fn set_canonical_chain_entry(&self, height: u64, blockid: L1BlockId) -> DbResult<()>;

    /// remove canonical chain records in given range (inclusive)
    #[deprecated]
    fn remove_canonical_chain_entries(&self, start_height: u64, end_height: u64) -> DbResult<()>;

    /// Prune earliest blocks till height
    fn prune_to_height(&self, height: u64) -> DbResult<()>;

    // TODO DA scraping storage

    // Gets current chain tip height, blockid
    #[deprecated]
    fn get_canonical_chain_tip(&self) -> DbResult<Option<(u64, L1BlockId)>>;

    fn get_block(&self, blockid: L1BlockId) -> DbResult<Option<L1Block>>;

    fn get_block_pow(&self, blockid: L1BlockId) -> DbResult<Option<[u8; 32]>>;

    fn get_best_valid_block_height(&self) -> DbResult<Option<u64>>;

    fn get_blocks_at_height_range(&self, start: u64, end: u64) -> DbResult<Vec<L1BlockId>>;

    /// Gets the block manifest for a blockid.
    fn get_block_manifest(&self, blockid: L1BlockId) -> DbResult<Option<L1BlockManifest>>;

    /// Gets the blockid at height for the current chain.
    #[deprecated]
    fn get_canonical_blockid_at_height(&self, height: u64) -> DbResult<Option<L1BlockId>>;

    // TODO: This should not exist in database level and should be handled by downstream manager.
    /// Returns a half-open interval of block hashes, if we have all of them
    /// present.  Otherwise, returns error.
    #[deprecated]
    fn get_canonical_blockid_range(&self, start_idx: u64, end_idx: u64)
        -> DbResult<Vec<L1BlockId>>;

    /// Gets the relevant txs we stored in a block.
    #[deprecated]
    fn get_block_txs(&self, blockid: L1BlockId) -> DbResult<Option<Vec<L1TxRef>>>;

    /// Gets the tx with proof given a tx ref, if present.
    #[deprecated]
    fn get_tx(&self, tx_ref: L1TxRef) -> DbResult<Option<L1Tx>>;

    // TODO DA queries
}

/// Provider and store to write and query sync events.  This does not provide notifications, that
/// should be handled at a higher level.
pub trait SyncEventDatabase {
    /// Atomically writes a new sync event, returning its index.
    fn write_sync_event(&self, ev: SyncEvent) -> DbResult<u64>;

    /// Atomically clears sync events in a range, defined as a half-open
    /// interval.  This should only be used for deeply buried events where we'll
    /// never need to look at them again.
    fn clear_sync_event_range(&self, start_idx: u64, end_idx: u64) -> DbResult<()>;

    /// Returns the index of the most recently written sync event.
    fn get_last_idx(&self) -> DbResult<Option<u64>>;

    /// Gets the sync event with some index, if it exists.
    fn get_sync_event(&self, idx: u64) -> DbResult<Option<SyncEvent>>;

    /// Gets the unix millis timestamp that a sync event was inserted.
    fn get_event_timestamp(&self, idx: u64) -> DbResult<Option<u64>>;
}

/// Db for client state updates and checkpoints.
pub trait ClientStateDatabase {
    /// Writes a new consensus output for a given input index.  These input
    /// indexes correspond to indexes in [``SyncEventDatabase``] and
    /// [``SyncEventDatabase``].  Will error if `idx - 1` does not exist (unless
    /// `idx` is 0) or if trying to overwrite a state, as this is almost
    /// certainly a bug.
    fn put_client_update(&self, idx: u64, output: ClientUpdateOutput) -> DbResult<()>;

    /// Gets the output client state writes for some input index.
    fn get_client_update(&self, idx: u64) -> DbResult<Option<ClientUpdateOutput>>;

    /// Gets the idx of the last written state.  Or returns error if a bootstrap
    /// state has not been written yet.
    fn get_last_state_idx(&self) -> DbResult<u64>;

    fn put_client_state(&self, block_id: L1BlockId, state: L1ClientState) -> DbResult<()>;

    fn get_client_state(&self, block_id: L1BlockId) -> DbResult<Option<L1ClientState>>;
}

/// L2 data store for CL blocks.  Does not store anything about what we think
/// the L2 chain tip is, that's controlled by the consensus state.
pub trait L2BlockDatabase {
    /// Stores an L2 block, does not care about the block height of the L2
    /// block.  Also sets the block's status to "unchecked".
    fn put_block_data(&self, block: L2BlockBundle) -> DbResult<()>;

    /// Tries to delete an L2 block from the store, returning if it really
    /// existed or not.  This should only be used for blocks well before some
    /// buried L1 finalization horizon.
    fn del_block_data(&self, id: L2BlockId) -> DbResult<bool>;

    /// Sets the block's validity status.
    fn set_block_status(&self, id: L2BlockId, status: BlockStatus) -> DbResult<()>;

    /// Gets the L2 block by its ID, if we have it.
    fn get_block_data(&self, id: L2BlockId) -> DbResult<Option<L2BlockBundle>>;

    /// Gets the L2 block IDs that we have at some height, in case there's more
    /// than one on competing forks.
    // TODO do we even want to permit this as being a possible thing?
    fn get_blocks_at_height(&self, idx: u64) -> DbResult<Vec<L2BlockId>>;

    /// Gets the validity status of a block.
    fn get_block_status(&self, id: L2BlockId) -> DbResult<Option<BlockStatus>>;
}

/// Gets the status of a block.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, BorshSerialize, BorshDeserialize)]
pub enum BlockStatus {
    /// Block's validity hasn't been checked yet.
    Unchecked,

    /// Block is valid, although this doesn't mean it's in the canonical chain.
    Valid,

    /// Block is invalid, for no particular reason.  We'd have to look somewhere
    /// else for that.
    Invalid,
}

/// Low-level Strata chainstate database.  This provides the basic interface for
/// storing and fetching write batches and toplevel states on disk.
///
/// Currently we do not have a "bulk" state that we would want to avoid storing
/// in memory all at once.  In the future, we expect that this interface would
/// be extended to expose a "finalized" state that's fully materialized, along
/// with functions to walk the finalized state forwards and backwards.  We can
/// use the unmerged write batches to construct a view of more recent states
/// than the fully materialized state in-memory.
///
/// For now, the full state is just the "toplevel" state that can always be
/// expected to be of moderate size in memory.
// TODO maybe rewrite this around storing write batches according to blkid?
pub trait ChainstateDatabase {
    /// Writes the genesis chainstate at index 0.
    fn write_genesis_state(&self, toplevel: Chainstate, blockid: L2BlockId) -> DbResult<()>;

    /// Stores a write batch in the database, possibly computing that state
    /// under the hood from the writes.  Will not overwrite existing data,
    /// previous writes must be purged first in order to be replaced.
    fn put_write_batch(&self, idx: u64, batch: WriteBatchEntry) -> DbResult<()>;

    /// Gets the write batch stored to compute a height.
    fn get_write_batch(&self, idx: u64) -> DbResult<Option<WriteBatchEntry>>;

    /// Tells the database to purge state before a certain index.
    fn purge_entries_before(&self, before_idx: u64) -> DbResult<()>;

    /// Rolls back any writes after a specified index.
    fn rollback_writes_to(&self, new_tip_idx: u64) -> DbResult<()>;

    /// Gets the last written state.
    fn get_last_write_idx(&self) -> DbResult<u64>;

    /// Gets the earliest written state.  This corresponds to calls to
    /// `purge_entries_before`.
    fn get_earliest_write_idx(&self) -> DbResult<u64>;
}

/// Database for checkpoint data.
pub trait CheckpointDatabase {
    /// Inserts an epoch summary retrievable by its epoch commitment.
    ///
    /// Fails if there's already an entry there.
    fn insert_epoch_summary(&self, epoch: EpochSummary) -> DbResult<()>;

    /// Gets an epoch summary given an epoch commitment.
    fn get_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<Option<EpochSummary>>;

    /// Gets all commitments for an epoch.  This makes no guarantees about ordering.
    fn get_epoch_commitments_at(&self, epoch: u64) -> DbResult<Vec<EpochCommitment>>;

    /// Gets the index of the last epoch that we have a summary for, if any.
    fn get_last_summarized_epoch(&self) -> DbResult<Option<u64>>;

    /// Store a [`CheckpointEntry`]
    ///
    /// `batchidx` for the Checkpoint is expected to increase monotonically and
    /// correspond to the value of `cur_epoch` in
    /// [`strata_state::chain_state::Chainstate`].
    fn put_checkpoint(&self, epoch: u64, entry: CheckpointEntry) -> DbResult<()>;

    /// Get a [`CheckpointEntry`] by its index.
    fn get_checkpoint(&self, epoch: u64) -> DbResult<Option<CheckpointEntry>>;

    /// Get last written checkpoint index.
    fn get_last_checkpoint_idx(&self) -> DbResult<Option<u64>>;
}

/// Encapsulates provider and store traits to create/update [`BundledPayloadEntry`] in the
/// database and to fetch [`BundledPayloadEntry`] and indices from the database
pub trait L1WriterDatabase {
    /// Store the [`BundledPayloadEntry`].
    fn put_payload_entry(&self, idx: u64, payloadentry: BundledPayloadEntry) -> DbResult<()>;

    /// Get a [`BundledPayloadEntry`] by its index.
    fn get_payload_entry_by_idx(&self, idx: u64) -> DbResult<Option<BundledPayloadEntry>>;

    /// Get the next payload index
    fn get_next_payload_idx(&self) -> DbResult<u64>;

    /// Store the [`IntentEntry`].
    fn put_intent_entry(&self, payloadid: Buf32, payloadentry: IntentEntry) -> DbResult<()>;

    /// Get a [`IntentEntry`] by its hash
    fn get_intent_by_id(&self, id: Buf32) -> DbResult<Option<IntentEntry>>;

    /// Get a [`IntentEntry`] by its idx
    fn get_intent_by_idx(&self, idx: u64) -> DbResult<Option<IntentEntry>>;

    /// Get  the next intent index
    fn get_next_intent_idx(&self) -> DbResult<u64>;
}

pub trait ProofDatabase {
    /// Inserts a proof into the database.
    ///
    /// Returns `Ok(())` on success, or an error on failure.
    fn put_proof(&self, proof_key: ProofKey, proof: ProofReceipt) -> DbResult<()>;

    /// Retrieves a proof by its key.
    ///
    /// Returns `Some(proof)` if found, or `None` if not.
    fn get_proof(&self, proof_key: &ProofKey) -> DbResult<Option<ProofReceipt>>;

    /// Deletes a proof by its key.
    ///
    /// Tries to delete a proof by its key, returning if it really
    /// existed or not.
    fn del_proof(&self, proof_key: ProofKey) -> DbResult<bool>;

    /// Inserts dependencies for a given [`ProofContext`] into the database.
    ///
    /// Returns `Ok(())` on success, or an error on failure.
    fn put_proof_deps(&self, proof_context: ProofContext, deps: Vec<ProofContext>) -> DbResult<()>;

    /// Retrieves proof dependencies by it's [`ProofContext`].
    ///
    /// Returns `Some(dependencies)` if found, or `None` if not.
    fn get_proof_deps(&self, proof_context: ProofContext) -> DbResult<Option<Vec<ProofContext>>>;

    /// Deletes dependencies for a given [`ProofContext`].
    ///
    /// Tries to delete dependencies of by its context, returning if it really
    /// existed or not.
    fn del_proof_deps(&self, proof_context: ProofContext) -> DbResult<bool>;
}

// TODO remove this trait, just like the high level `Database` trait
pub trait BroadcastDatabase {
    type L1BroadcastDB: L1BroadcastDatabase + Sync + Send;

    /// Return a reference to the L1 broadcast db implementation
    fn l1_broadcast_db(&self) -> &Arc<Self::L1BroadcastDB>;
}

/// A trait encapsulating the provider and store traits for interacting with the broadcast
/// transactions([`L1TxEntry`]), their indices and ids
pub trait L1BroadcastDatabase {
    /// Updates/Inserts a txentry to database. Returns Some(idx) if newly inserted else None
    fn put_tx_entry(&self, txid: Buf32, txentry: L1TxEntry) -> DbResult<Option<u64>>;

    /// Updates an existing txentry
    fn put_tx_entry_by_idx(&self, idx: u64, txentry: L1TxEntry) -> DbResult<()>;

    // TODO: possibly add delete as well

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

/// Provides access to the implementers of provider and store traits for interacting with the
/// transaction state database of the bridge client.
///
/// This trait assumes that the [`Txid`](bitcoin::Txid) is always unique.
pub trait BridgeTxDatabase {
    /// Add [`BridgeTxState`] to the database replacing the existing one if present.
    fn put_tx_state(&self, txid: Buf32, tx_state: BridgeTxState) -> DbResult<()>;

    /// Delete the stored [`BridgeTxState`] from the database and return it. This can be invoked,
    /// for example, when a fully signed Deposit Transaction has been broadcasted. If the `txid`
    /// did not exist, `None` is returned.
    ///
    /// *WARNING*: This can have detrimental effects if used at the wrong time.
    fn delete_tx_state(&self, txid: Buf32) -> DbResult<Option<BridgeTxState>>;

    /// Fetch [`BridgeTxState`] from db.
    fn get_tx_state(&self, txid: Buf32) -> DbResult<Option<BridgeTxState>>;
}
