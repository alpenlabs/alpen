//! Trait definitions for low level database interfaces.  This borrows some of
//! its naming conventions from reth.

use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use strata_mmr::CompactMmr;
use strata_primitives::{l1::*, prelude::*};
use strata_state::{
    block::L2BlockBundle, bridge_duties::BridgeDutyStatus, chain_state::ChainState,
    client_state::ClientState, l1::L1Tx, operation::*, prelude::*, state_op::WriteBatch,
    sync_event::SyncEvent,
};

use crate::{
    entities::bridge_tx_state::BridgeTxState,
    types::{BlobEntry, CheckpointEntry, L1TxEntry},
    DbResult,
};

/// Common database interface that we can parameterize worker tasks over if
/// parameterizing them over each individual trait gets cumbersome or if we need
/// to use behavior that crosses different interfaces.
pub trait Database {
    type L1DataStore: L1DataStore + Send + Sync;
    type L1Provider: L1DataProvider + Send + Sync;
    type L2DataStore: L2DataStore + Send + Sync;
    type L2DataProv: L2DataProvider + Send + Sync;
    type SyncEventStore: SyncEventStore + Send + Sync;
    type SyncEventProvider: SyncEventProvider + Send + Sync;
    type ClientStateStore: ClientStateStore + Send + Sync;
    type ClientStateProvider: ClientStateProvider + Send + Sync;
    type ChainStateStore: ChainstateStore + Send + Sync;
    type ChainStateProvider: ChainstateProvider + Send + Sync;
    type CheckpointStore: CheckpointStore + Send + Sync;
    type CheckpointProvider: CheckpointProvider + Send + Sync;

    fn l1_store(&self) -> &Arc<Self::L1DataStore>;
    fn l1_provider(&self) -> &Arc<Self::L1Provider>;
    fn l2_store(&self) -> &Arc<Self::L2DataStore>;
    fn l2_provider(&self) -> &Arc<Self::L2DataProv>;
    fn sync_event_store(&self) -> &Arc<Self::SyncEventStore>;
    fn sync_event_provider(&self) -> &Arc<Self::SyncEventProvider>;
    fn client_state_store(&self) -> &Arc<Self::ClientStateStore>;
    fn client_state_provider(&self) -> &Arc<Self::ClientStateProvider>;
    fn chain_state_store(&self) -> &Arc<Self::ChainStateStore>;
    fn chain_state_provider(&self) -> &Arc<Self::ChainStateProvider>;
    fn checkpoint_store(&self) -> &Arc<Self::CheckpointStore>;
    fn checkpoint_provider(&self) -> &Arc<Self::CheckpointProvider>;
}

/// Storage interface to control our view of L1 data.
pub trait L1DataStore {
    /// Atomically extends the chain with a new block, providing the manifest
    /// and a list of transactions we find relevant.  Returns error if
    /// provided out-of-order.
    fn put_block_data(&self, idx: u64, mf: L1BlockManifest, txs: Vec<L1Tx>) -> DbResult<()>;

    /// Stores an MMR checkpoint so we have to query less far back.  If the
    /// provided height does not match the entries in the MMR, will return an
    /// error.
    fn put_mmr_checkpoint(&self, idx: u64, mmr: CompactMmr) -> DbResult<()>;

    /// Resets the L1 chain tip to the specified block index.  The provided
    /// index will be the new chain tip that we store.
    fn revert_to_height(&self, idx: u64) -> DbResult<()>;

    // TODO DA scraping storage
}

/// Provider interface to view L1 data.
pub trait L1DataProvider {
    /// Gets the current chain tip index.
    fn get_chain_tip(&self) -> DbResult<Option<u64>>;

    /// Gets the block manifest for a block index.
    fn get_block_manifest(&self, idx: u64) -> DbResult<Option<L1BlockManifest>>;

    /// Returns a half-open interval of block hashes, if we have all of them
    /// present.  Otherwise, returns error.
    fn get_blockid_range(&self, start_idx: u64, end_idx: u64) -> DbResult<Vec<Buf32>>;

    /// Gets the relevant txs we stored in a block.
    fn get_block_txs(&self, idx: u64) -> DbResult<Option<Vec<L1TxRef>>>;

    /// Gets the tx with proof given a tx ref, if present.
    fn get_tx(&self, tx_ref: L1TxRef) -> DbResult<Option<L1Tx>>;

    /// Gets the last MMR checkpoint we stored before the given block height.
    /// Up to the caller to advance the MMR the rest of the way to the desired
    /// state.
    fn get_last_mmr_to(&self, idx: u64) -> DbResult<Option<CompactMmr>>;

    /// Get the [`L1Tx`]'s from a certain index (including the index) in a single flattened list
    /// along with the latest index.
    ///
    /// This is an infallible RPC. If the `start_idx` is invalid, an empty `Vec` is returned along
    /// with whatever `start_idx` this method was called with.
    ///
    /// # Errors
    ///
    /// This only errors if there is an error from the underlying persistence layer.
    fn get_txs_from(&self, start_idx: u64) -> DbResult<(Vec<L1Tx>, u64)>;

    // TODO DA queries
}

/// Store to write new sync events.
pub trait SyncEventStore {
    /// Atomically writes a new sync event, returning its index.
    fn write_sync_event(&self, ev: SyncEvent) -> DbResult<u64>;

    /// Atomically clears sync events in a range, defined as a half-open
    /// interval.  This should only be used for deeply buried events where we'll
    /// never need to look at them again.
    fn clear_sync_event(&self, start_idx: u64, end_idx: u64) -> DbResult<()>;
}

/// Provider to query sync events.  This does not provide notifications, that
/// should be handled at a higher level.
pub trait SyncEventProvider {
    /// Returns the index of the most recently written sync event.
    fn get_last_idx(&self) -> DbResult<Option<u64>>;

    /// Gets the sync event with some index, if it exists.
    fn get_sync_event(&self, idx: u64) -> DbResult<Option<SyncEvent>>;

    /// Gets the unix millis timestamp that a sync event was inserted.
    fn get_event_timestamp(&self, idx: u64) -> DbResult<Option<u64>>;
}

/// Writes client state updates and checkpoints.
pub trait ClientStateStore {
    /// Writes a new consensus output for a given input index.  These input
    /// indexes correspond to indexes in [``SyncEventStore``] and
    /// [``SyncEventProvider``].  Will error if `idx - 1` does not exist (unless
    /// `idx` is 0) or if trying to overwrite a state, as this is almost
    /// certainly a bug.
    fn write_client_update_output(&self, idx: u64, output: ClientUpdateOutput) -> DbResult<()>;

    /// Writes a new consensus checkpoint that we can cheaply resume from.  Will
    /// error if trying to overwrite a state.
    fn write_client_state_checkpoint(&self, idx: u64, state: ClientState) -> DbResult<()>;
}

/// Provides client state writes and checkpoints.
pub trait ClientStateProvider {
    /// Gets the idx of the last written state.  Or returns error if a bootstrap
    /// state has not been written yet.
    fn get_last_write_idx(&self) -> DbResult<u64>;

    /// Gets the output client state writes for some input index.
    fn get_client_state_writes(&self, idx: u64) -> DbResult<Option<Vec<ClientStateWrite>>>;

    /// Gets the actions output from a client state transition.
    fn get_client_update_actions(&self, idx: u64) -> DbResult<Option<Vec<SyncAction>>>;

    /// Gets the last consensus checkpoint idx.
    fn get_last_checkpoint_idx(&self) -> DbResult<u64>;

    /// Gets the idx of the last checkpoint up to the given input idx.  This is
    /// the idx we should resume at when playing out consensus writes since the
    /// saved checkpoint, which may be the same as the given idx (if we didn't
    /// receive any sync events since the last checkpoint.
    fn get_prev_checkpoint_at(&self, idx: u64) -> DbResult<u64>;

    /// Gets a state checkpoint at a previously written index, if it exists.
    fn get_state_checkpoint(&self, idx: u64) -> DbResult<Option<ClientState>>;
}

/// L2 data store for CL blocks.  Does not store anything about what we think
/// the L2 chain tip is, that's controlled by the consensus state.
pub trait L2DataStore {
    /// Stores an L2 block, does not care about the block height of the L2
    /// block.  Also sets the block's status to "unchecked".
    fn put_block_data(&self, block: L2BlockBundle) -> DbResult<()>;

    /// Tries to delete an L2 block from the store, returning if it really
    /// existed or not.  This should only be used for blocks well before some
    /// buried L1 finalization horizon.
    fn del_block_data(&self, id: L2BlockId) -> DbResult<bool>;

    /// Sets the block's validity status.
    fn set_block_status(&self, id: L2BlockId, status: BlockStatus) -> DbResult<()>;
}

/// Data provider for L2 blocks.
pub trait L2DataProvider {
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

/// Write trait for the (consensus layer) chain state database.  For now we only
/// have a modestly sized "toplevel" chain state and no "large" state like the
/// EL does.  This trait is designed to permit a change to storing larger state
/// like that in the future without *too* much extra effort.  We decide new
/// states by providing the database with a generic "write batch" and offloading
/// the effort of deciding how to compute that write batch to the database impl.
pub trait ChainstateStore {
    /// Writes the genesis chainstate at index 0.
    fn write_genesis_state(&self, toplevel: &ChainState) -> DbResult<()>;

    /// Stores a write batch in the database, possibly computing that state
    /// under the hood from the writes.  Will not overwrite existing data,
    /// previous writes must be purged first in order to be replaced.
    fn write_state_update(&self, idx: u64, batch: &WriteBatch) -> DbResult<()>;

    /// Tells the database to purge state before a certain block index (height).
    fn purge_historical_state_before(&self, before_idx: u64) -> DbResult<()>;

    /// Rolls back any writes and state checkpoints after a specified block.
    fn rollback_writes_to(&self, new_tip_idx: u64) -> DbResult<()>;
}

/// Read trait corresponding to [``ChainstateStore``].  See that trait's doc for
/// design explanation.
pub trait ChainstateProvider {
    /// Gets the last written state.
    fn get_last_state_idx(&self) -> DbResult<u64>;

    /// Gets the earliest written state.  This corresponds to calls to
    /// `purge_historical_state_before`.
    fn get_earliest_state_idx(&self) -> DbResult<u64>;

    /// Gets the write batch stored to compute a height.
    fn get_writes_at(&self, idx: u64) -> DbResult<Option<WriteBatch>>;

    /// Gets the toplevel chain state at a particular block index (height).
    fn get_toplevel_state(&self, idx: u64) -> DbResult<Option<ChainState>>;
}

/// NOTE: We might have to merge this with the [`Database`]
/// A trait encapsulating provider and store traits to interact with the underlying database for
/// [`BlobEntry`]
pub trait SequencerDatabase {
    type BlobStore: BlobStore;
    type BlobProvider: BlobProvider;

    fn blob_store(&self) -> &Arc<Self::BlobStore>;
    fn blob_provider(&self) -> &Arc<Self::BlobProvider>;
}

/// A trait encapsulating  store traits to create/update [`BlobEntry`] in the database
pub trait BlobStore {
    /// Store the [`BlobEntry`].
    fn put_blob_entry(&self, blobid: Buf32, blobentry: BlobEntry) -> DbResult<()>;
}

/// A trait encapsulating  provider traits to fetch [`BlobEntry`] and indices from the database
pub trait BlobProvider {
    /// Get a [`BlobEntry`] by its hash
    fn get_blob_by_id(&self, id: Buf32) -> DbResult<Option<BlobEntry>>;

    /// Get the blob ID corresponding to the index
    fn get_blob_id(&self, blobidx: u64) -> DbResult<Option<Buf32>>;

    /// Get the last blob index
    fn get_last_blob_idx(&self) -> DbResult<Option<u64>>;
}

/// A trait providing access to both prover data store and prover data provider.
pub trait ProverDatabase {
    type ProverStore: ProverDataStore;
    type ProverProv: ProverDataProvider;

    fn prover_store(&self) -> &Arc<Self::ProverStore>;
    fn prover_provider(&self) -> &Arc<Self::ProverProv>;
}

pub trait ProverDataStore {
    /// Adds a new txentry to database
    fn insert_new_task_entry(&self, txid: [u8; 16], txentry: Vec<u8>) -> DbResult<u64>;

    /// Updates an existing txentry
    fn update_task_entry_by_id(&self, txid: [u8; 16], txentry: Vec<u8>) -> DbResult<()>;

    /// Updates an existing txentry
    fn update_task_entry(&self, idx: u64, txentry: Vec<u8>) -> DbResult<()>;
}

/// All methods related to fetching [`Vec<u8>`]s and indices in the database
pub trait ProverDataProvider {
    /// Fetch [`Vec<u8>`] from db
    fn get_task_entry_by_id(&self, txid: [u8; 16]) -> DbResult<Option<Vec<u8>>>;

    /// Get next index to be inserted to
    fn get_next_task_idx(&self) -> DbResult<u64>;

    /// Get transaction id for index
    fn get_taskid(&self, idx: u64) -> DbResult<Option<[u8; 16]>>;

    /// get txentry by idx
    fn get_task_entry(&self, idx: u64) -> DbResult<Option<Vec<u8>>>;
}

/// A trait encapsulating the provider and store traits for interacting with the broadcast
/// transactions([`L1TxEntry`]), their indices and ids
pub trait L1BroadcastDatabase {
    type BroadcastStore: L1BroadcastStore;
    type BroadcastProvider: L1BroadcastProvider;

    /// Return a reference to the store implementation
    fn broadcast_store(&self) -> &Arc<Self::BroadcastStore>;
    /// Return a reference to the provider implementation
    fn broadcast_provider(&self) -> &Arc<Self::BroadcastProvider>;
}

/// All methods related to storing/updating [`L1TxEntry`]s in the database
pub trait L1BroadcastStore {
    /// Updates/Inserts a txentry to database. Returns Some(idx) if newly inserted else None
    fn put_tx_entry(&self, txid: Buf32, txentry: L1TxEntry) -> DbResult<Option<u64>>;

    /// Updates an existing txentry
    fn put_tx_entry_by_idx(&self, idx: u64, txentry: L1TxEntry) -> DbResult<()>;

    // TODO: possibly add delete as well
}

/// All methods related to fetching [`L1TxEntry`]s and indices in the database
pub trait L1BroadcastProvider {
    /// Fetch [`L1TxEntry`] from db
    fn get_tx_entry_by_id(&self, txid: Buf32) -> DbResult<Option<L1TxEntry>>;

    /// Get next index to be inserted to
    fn get_next_tx_idx(&self) -> DbResult<u64>;

    /// Get transaction id for index
    fn get_txid(&self, idx: u64) -> DbResult<Option<Buf32>>;

    /// get txentry by idx
    fn get_tx_entry(&self, idx: u64) -> DbResult<Option<L1TxEntry>>;
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

/// Provides methods to manage the status of a deposit or withdrawal duty that a bridge client
/// executes.
///
/// Each such duty can be identified uniquely with a [`Txid`](bitcoin::Txid) (represented as a
/// [`Buf32`]). For a deposit duty, this `txid` refers to that of the Deposit Request and for the
/// withdrawal duty, it refers to that of the Deposit Transaction.
pub trait BridgeDutyDatabase {
    /// Get the status of a duty identified by a given `txid` if it exists.
    fn get_status(&self, txid: Buf32) -> DbResult<Option<BridgeDutyStatus>>;

    /// Remove duty from the database and return the status of the removed duty.
    fn delete_duty(&self, txid: Buf32) -> DbResult<Option<BridgeDutyStatus>>;

    /// Adds a duty status to the DB, updating the entry if one exists.
    ///
    /// # Errors
    ///
    /// If a duty for the given `txid` is not present
    fn put_duty_status(&self, txid: Buf32, status: BridgeDutyStatus) -> DbResult<()>;
}

/// Provides methods to manage the duty index for the deposit duties.
pub trait BridgeDutyIndexDatabase {
    /// Get the checkpoint upto which the duties have been fetched.
    ///
    /// This checkpoint is the same as the index in [`L1DataStore`].
    fn get_index(&self) -> DbResult<Option<u64>>;

    /// Set the checkpoint to a new value.
    ///
    /// This is done in response to the response received from the full node's RPC.
    fn set_index(&self, index: u64) -> DbResult<()>;
}

/// Provider for Checkpoint data
pub trait CheckpointProvider {
    /// Get a [`CheckpointEntry`] by it's index
    fn get_batch_checkpoint(&self, batchidx: u64) -> DbResult<Option<CheckpointEntry>>;

    /// Get last batch index
    fn get_last_batch_idx(&self) -> DbResult<Option<u64>>;
}

/// Store for Checkpoint data
pub trait CheckpointStore {
    /// Store a [`CheckpointEntry`]
    ///
    /// `batchidx` for the Checkpoint is expected to increase monotonically and
    /// correspond to the value of [`strata_state::chain_state::ChainState::epoch`].
    fn put_batch_checkpoint(&self, batchidx: u64, entry: CheckpointEntry) -> DbResult<()>;

    /// Handles checkpoint DB migration, typically for schema or encoding changes.
    ///
    /// # Returns
    /// A `DbResult` containing a tuple `(examined_count, migrated_count)` where:
    /// - `examined_count`: The number of checkpoint entries from the source/old storage that were
    ///   examined.
    /// - `migrated_count`: The number of those examined entries that were successfully migrated to
    ///   the destination/new storage.
    fn migrate_checkpoint_data(&self) -> DbResult<(u64, u64)>;
}
