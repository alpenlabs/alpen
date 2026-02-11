//! Trait definitions for low level database interfaces.  This borrows some of
//! its naming conventions from reth.

use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::Serialize;
use strata_acct_types::CompactMmr64;
use strata_asm_common::AsmManifest;
use strata_checkpoint_types::EpochSummary;
use strata_csm_types::{ClientState, ClientUpdateOutput};
use strata_identifiers::{
    Epoch, EpochCommitment, Hash, OLBlockCommitment, OLBlockId, OLTxId, RawMmrId, Slot,
};
use strata_ol_chain_types::L2BlockBundle;
use strata_ol_chain_types_new::OLBlock;
use strata_ol_state_types::{OLAccountState, OLState, WriteBatch};
use strata_primitives::{
    prelude::*,
    proof::{ProofContext, ProofKey},
};
use strata_state::asm_state::AsmState;
use zkaleido::ProofReceiptWithMetadata;

use crate::{
    chainstate::ChainstateDatabase,
    mmr_helpers::MmrAlgorithm,
    types::{
        BundledPayloadEntry, CheckpointEntry, IntentEntry, L1TxEntry, MempoolTxData,
        OLCheckpointEntry,
    },
    DbError, DbResult,
};

/// Common database backend interface that we can parameterize worker tasks over if
/// parameterizing them over each individual trait gets cumbersome or if we need
/// to use behavior that crosses different interfaces.
pub trait DatabaseBackend: Send + Sync {
    fn asm_db(&self) -> Arc<impl AsmDatabase>;
    fn l1_db(&self) -> Arc<impl L1Database>;
    fn l2_db(&self) -> Arc<impl L2BlockDatabase>;
    fn client_state_db(&self) -> Arc<impl ClientStateDatabase>;
    fn ol_block_db(&self) -> Arc<impl OLBlockDatabase>;
    fn chain_state_db(&self) -> Arc<impl ChainstateDatabase>;
    fn ol_state_db(&self) -> Arc<impl OLStateDatabase>;
    fn checkpoint_db(&self) -> Arc<impl CheckpointDatabase>;
    fn ol_checkpoint_db(&self) -> Arc<impl OLCheckpointDatabase>;
    fn writer_db(&self) -> Arc<impl L1WriterDatabase>;
    fn prover_db(&self) -> Arc<impl ProofDatabase>;
    fn broadcast_db(&self) -> Arc<impl L1BroadcastDatabase>;
    fn mempool_db(&self) -> Arc<impl MempoolDatabase>;
}

/// Database interface to control our view of ASM state.
pub trait AsmDatabase: Send + Sync + 'static {
    /// Writes a new ASM state for a given l1 block.
    fn put_asm_state(&self, block: L1BlockCommitment, state: AsmState) -> DbResult<()>;

    /// Gets the ASM state for the given l1 block.
    fn get_asm_state(&self, block: L1BlockCommitment) -> DbResult<Option<AsmState>>;

    /// Gets latest ASM state (the entry that corresponds to the highest l1 block).
    fn get_latest_asm_state(&self) -> DbResult<Option<(L1BlockCommitment, AsmState)>>;

    /// Gets ASM states starting from a given L1BlockCommitment up to a maximum count.
    ///
    /// Returns entries in ascending order (oldest first). If `from_block` doesn't exist,
    /// starts from the next available block after it.
    fn get_asm_states_from(
        &self,
        from_block: L1BlockCommitment,
        max_count: usize,
    ) -> DbResult<Vec<(L1BlockCommitment, AsmState)>>;
}

/// Database interface to control our view of L1 data.
/// Operations are NOT VALIDATED at this level.
/// Ensure all operations are done through `L1BlockManager`
pub trait L1Database: Send + Sync + 'static {
    /// Stores an ASM manifest for a given L1 block.
    /// Returns error if provided out-of-order.
    fn put_block_data(&self, manifest: AsmManifest) -> DbResult<()>;

    /// Set a specific height, blockid in canonical chain records.
    fn set_canonical_chain_entry(&self, height: u64, blockid: L1BlockId) -> DbResult<()>;

    /// remove canonical chain records in given range (inclusive)
    fn remove_canonical_chain_entries(&self, start_height: u64, end_height: u64) -> DbResult<()>;

    /// Prune earliest blocks till height
    fn prune_to_height(&self, height: u64) -> DbResult<()>;

    // TODO DA scraping storage

    // Gets current chain tip height, blockid
    fn get_canonical_chain_tip(&self) -> DbResult<Option<(u64, L1BlockId)>>;

    /// Gets the ASM manifest for a blockid.
    fn get_block_manifest(&self, blockid: L1BlockId) -> DbResult<Option<AsmManifest>>;

    /// Gets the blockid at height for the current chain.
    fn get_canonical_blockid_at_height(&self, height: u64) -> DbResult<Option<L1BlockId>>;

    // TODO: This should not exist in database level and should be handled by downstream manager.
    /// Returns a half-open interval of block hashes, if we have all of them
    /// present.  Otherwise, returns error.
    fn get_canonical_blockid_range(&self, start_idx: u64, end_idx: u64)
        -> DbResult<Vec<L1BlockId>>;

    // TODO DA queries
}

/// Db for client state updates and checkpoints.
pub trait ClientStateDatabase: Send + Sync + 'static {
    /// Writes a new consensus output for a given l1 block.
    fn put_client_update(
        &self,
        block: L1BlockCommitment,
        output: ClientUpdateOutput,
    ) -> DbResult<()>;

    /// Gets the output client state writes for some input index.
    fn get_client_update(&self, block: L1BlockCommitment) -> DbResult<Option<ClientUpdateOutput>>;

    /// Gets latest client state (the entry that corresponds to the highest l1 block).
    fn get_latest_client_state(&self) -> DbResult<Option<(L1BlockCommitment, ClientState)>>;

    /// Deletes a client update for a given l1 block.
    fn del_client_update(&self, block: L1BlockCommitment) -> DbResult<()>;

    /// Gets client updates starting from a given L1BlockCommitment up to a maximum count.
    ///
    /// Returns entries in ascending order (oldest first). If `from_block` doesn't exist,
    /// starts from the next available block after it.
    fn get_client_updates_from(
        &self,
        from_block: L1BlockCommitment,
        max_count: usize,
    ) -> DbResult<Vec<(L1BlockCommitment, ClientUpdateOutput)>>;
}

/// L2 data store for CL blocks.  Does not store anything about what we think
/// the L2 chain tip is, that's controlled by the consensus state.
pub trait L2BlockDatabase: Send + Sync + 'static {
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

    /// Returns the latest valid L2 block ID, or `None` at genesis or when no valid block exists.
    // TODO do we even want to permit this as being a possible thing?
    fn get_tip_block(&self) -> DbResult<L2BlockId>;
}

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

/// Database for checkpoint data.
// TODO: Remove when we switch to using OL checkpoint database in all relevant places.
pub trait CheckpointDatabase: Send + Sync + 'static {
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

    /// Delete a specific epoch summary by epoch commitment.
    ///
    /// Returns true if the epoch summary existed and was deleted, false otherwise.
    fn del_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<bool>;

    /// Delete epoch summaries from the specified epoch onwards (inclusive).
    ///
    /// This method deletes all epoch summaries with epoch index >= start_epoch.
    /// Returns a vector of deleted epoch indices.
    fn del_epoch_summaries_from_epoch(&self, start_epoch: u64) -> DbResult<Vec<u64>>;

    /// Store a [`CheckpointEntry`]
    ///
    /// `batchidx` for the Checkpoint is expected to increase monotonically and
    /// correspond to the value of `cur_epoch` in
    /// [`strata_ol_chainstate_types::Chainstate`].
    fn put_checkpoint(&self, epoch: u64, entry: CheckpointEntry) -> DbResult<()>;

    /// Get a [`CheckpointEntry`] by its index.
    fn get_checkpoint(&self, epoch: u64) -> DbResult<Option<CheckpointEntry>>;

    /// Get last written checkpoint index.
    fn get_last_checkpoint_idx(&self) -> DbResult<Option<u64>>;

    /// Delete a specific checkpoint by epoch index.
    ///
    /// Returns true if the checkpoint existed and was deleted, false otherwise.
    fn del_checkpoint(&self, epoch: u64) -> DbResult<bool>;

    /// Delete checkpoint entries from the specified epoch onwards (inclusive).
    ///
    /// This method deletes all checkpoints with epoch index >= start_epoch.
    /// Returns a vector of deleted epoch indices.
    fn del_checkpoints_from_epoch(&self, start_epoch: u64) -> DbResult<Vec<u64>>;

    /// Get the next checkpoint index that has PendingProof status.
    /// Returns the lowest index checkpoint that still needs proof generation.
    fn get_next_unproven_checkpoint_idx(&self) -> DbResult<Option<u64>>;
}

/// Database for OL checkpoint data.
pub trait OLCheckpointDatabase: Send + Sync + 'static {
    /// Inserts an epoch summary retrievable by its epoch commitment.
    ///
    /// Fails if there's already an entry there.
    fn insert_epoch_summary(&self, epoch: EpochSummary) -> DbResult<()>;

    /// Gets an epoch summary given an epoch commitment.
    fn get_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<Option<EpochSummary>>;

    /// Gets all commitments for an epoch. This makes no guarantees about ordering.
    fn get_epoch_commitments_at(&self, epoch: u64) -> DbResult<Vec<EpochCommitment>>;

    /// Gets the index of the last epoch that we have a summary for, if any.
    fn get_last_summarized_epoch(&self) -> DbResult<Option<u64>>;

    /// Delete a specific epoch summary by epoch commitment.
    ///
    /// Returns true if the epoch summary existed and was deleted, false otherwise.
    fn del_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<bool>;

    /// Delete epoch summaries from the specified epoch onwards (inclusive).
    ///
    /// This method deletes all epoch summaries with epoch index >= start_epoch.
    /// Returns a vector of deleted epoch indices.
    fn del_epoch_summaries_from_epoch(&self, start_epoch: u64) -> DbResult<Vec<u64>>;

    /// Store an [`OLCheckpointEntry`] by epoch.
    fn put_checkpoint(&self, epoch: Epoch, entry: OLCheckpointEntry) -> DbResult<()>;

    /// Get an [`OLCheckpointEntry`] by epoch.
    fn get_checkpoint(&self, epoch: Epoch) -> DbResult<Option<OLCheckpointEntry>>;

    /// Get last written checkpoint epoch.
    fn get_last_checkpoint_epoch(&self) -> DbResult<Option<Epoch>>;

    /// Get the next checkpoint epoch that is unsigned.
    fn get_next_unsigned_checkpoint_epoch(&self) -> DbResult<Option<Epoch>>;

    /// Delete a checkpoint by epoch.
    ///
    /// Returns true if it existed and was deleted.
    fn del_checkpoint(&self, epoch: Epoch) -> DbResult<bool>;

    /// Delete checkpoints from the specified epoch onwards (inclusive).
    ///
    /// Returns a vector of deleted epochs.
    fn del_checkpoints_from_epoch(&self, start_epoch: Epoch) -> DbResult<Vec<Epoch>>;
}

/// Encapsulates provider and store traits to create/update [`BundledPayloadEntry`] in the
/// database and to fetch [`BundledPayloadEntry`] and indices from the database
pub trait L1WriterDatabase: Send + Sync + 'static {
    /// Store the [`BundledPayloadEntry`].
    fn put_payload_entry(&self, idx: u64, payloadentry: BundledPayloadEntry) -> DbResult<()>;

    /// Get a [`BundledPayloadEntry`] by its index.
    fn get_payload_entry_by_idx(&self, idx: u64) -> DbResult<Option<BundledPayloadEntry>>;

    /// Get the next payload index
    fn get_next_payload_idx(&self) -> DbResult<u64>;

    /// Delete a specific payload entry by its index.
    ///
    /// Returns true if the payload existed and was deleted, false otherwise.
    fn del_payload_entry(&self, idx: u64) -> DbResult<bool>;

    /// Delete payload entries from the specified index onwards (inclusive).
    ///
    /// This method deletes all payload entries with index >= start_idx.
    /// Returns a vector of deleted payload indices.
    fn del_payload_entries_from_idx(&self, start_idx: u64) -> DbResult<Vec<u64>>;

    /// Store the [`IntentEntry`].
    fn put_intent_entry(&self, payloadid: Buf32, payloadentry: IntentEntry) -> DbResult<u64>;

    /// Get a [`IntentEntry`] by its hash
    fn get_intent_by_id(&self, id: Buf32) -> DbResult<Option<IntentEntry>>;

    /// Get a [`IntentEntry`] by its idx
    fn get_intent_by_idx(&self, idx: u64) -> DbResult<Option<IntentEntry>>;

    /// Get  the next intent index
    fn get_next_intent_idx(&self) -> DbResult<u64>;

    /// Delete a specific intent entry by its ID.
    ///
    /// Returns true if the intent existed and was deleted, false otherwise.
    fn del_intent_entry(&self, id: Buf32) -> DbResult<bool>;

    /// Delete intent entries from the specified index onwards (inclusive).
    ///
    /// This method deletes all intent entries with index >= start_idx.
    /// Returns a vector of deleted intent indices.
    fn del_intent_entries_from_idx(&self, start_idx: u64) -> DbResult<Vec<u64>>;
}

pub trait ProofDatabase: Send + Sync + 'static {
    /// Inserts a proof into the database.
    ///
    /// Returns `Ok(())` on success, or an error on failure.
    fn put_proof(&self, proof_key: ProofKey, proof: ProofReceiptWithMetadata) -> DbResult<()>;

    /// Retrieves a proof by its key.
    ///
    /// Returns `Some(proof)` if found, or `None` if not.
    fn get_proof(&self, proof_key: &ProofKey) -> DbResult<Option<ProofReceiptWithMetadata>>;

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

/// A trait encapsulating the provider and store traits for interacting with the broadcast
/// transactions([`L1TxEntry`]), their indices and ids
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
}

// =============================================================================
// MMR Database Traits
// =============================================================================

/// Global MMR database trait using MmrId for identification
///
/// This trait provides a simpler, more direct API where the MMR instance
/// is identified by an `MmrId` enum rather than a generic scope parameter.
/// This is the preferred interface for new code.
pub trait GlobalMmrDatabase: Send + Sync + 'static {
    type MmrAlgorithm: MmrAlgorithm;

    /// Append a new leaf to the specified MMR
    fn append_leaf(&self, mmr_id: RawMmrId, hash: Hash) -> DbResult<u64>;

    /// Get a node hash by MMR position
    fn get_node(&self, mmr_id: RawMmrId, pos: u64) -> DbResult<Option<Hash>>;

    /// Get the total MMR size
    fn get_mmr_size(&self, mmr_id: RawMmrId) -> DbResult<u64>;

    /// Get the total number of leaves
    fn get_num_leaves(&self, mmr_id: RawMmrId) -> DbResult<u64>;

    /// Get the peaks
    fn get_peaks(&self, mmr_id: RawMmrId) -> DbResult<Vec<Hash>>;

    /// Get a compact representation of the MMR
    fn get_compact(&self, mmr_id: RawMmrId) -> DbResult<CompactMmr64>;

    /// Remove the last leaf from the MMR
    fn pop_leaf(&self, mmr_id: RawMmrId) -> DbResult<Option<Hash>>;

    /// Append a leaf with its pre-image data (optional operation)
    ///
    /// Atomically stores both the MMR hash and the original data.
    /// Default implementation returns Unimplemented error.
    fn append_leaf_with_preimage(
        &self,
        _mmr_id: RawMmrId,
        _hash: Hash,
        _preimage: Vec<u8>,
    ) -> DbResult<u64> {
        Err(DbError::Unimplemented)
    }

    /// Get pre-image data by leaf index (optional operation)
    ///
    /// Returns None if no pre-image exists at the given index.
    /// Default implementation returns Unimplemented error.
    fn get_preimage(&self, _mmr_id: RawMmrId, _index: u64) -> DbResult<Option<Vec<u8>>> {
        Err(DbError::Unimplemented)
    }
}

// =============================================================================
// Database traits for OL state and other components
// =============================================================================

/// Database trait for toplevel OL state storage.
///
/// Stores OLState snapshots keyed by OLBlockCommitment (block ID + slot).
/// This allows retrieving state for any block in the chain.
pub trait OLStateDatabase: Send + Sync + 'static {
    /// Stores a toplevel OLState snapshot for a given block commitment.
    fn put_toplevel_ol_state(&self, commitment: OLBlockCommitment, state: OLState) -> DbResult<()>;

    /// Retrieves a toplevel OLState snapshot for a given block commitment.
    fn get_toplevel_ol_state(&self, commitment: OLBlockCommitment) -> DbResult<Option<OLState>>;

    /// Gets the latest toplevel OLState (highest slot).
    fn get_latest_toplevel_ol_state(&self) -> DbResult<Option<(OLBlockCommitment, OLState)>>;

    /// Deletes a toplevel OLState snapshot for a given block commitment.
    fn del_toplevel_ol_state(&self, commitment: OLBlockCommitment) -> DbResult<()>;

    /// Stores an OL write batch for a given block commitment.
    ///
    /// Write batches represent state changes that can be applied to a state.
    fn put_ol_write_batch(
        &self,
        commitment: OLBlockCommitment,
        wb: WriteBatch<OLAccountState>,
    ) -> DbResult<()>;

    /// Retrieves an OL write batch for a given block commitment.
    fn get_ol_write_batch(
        &self,
        commitment: OLBlockCommitment,
    ) -> DbResult<Option<WriteBatch<OLAccountState>>>;

    /// Deletes an OL write batch for a given block commitment.
    fn del_ol_write_batch(&self, commitment: OLBlockCommitment) -> DbResult<()>;
}

/// OL data store for OL blocks. Does not store anything about what we think
/// the OL chain tip is, that's controlled by the consensus state.
///
/// This stores OL blocks (header + body) keyed by block commitment (slot + block ID).
pub trait OLBlockDatabase: Send + Sync + 'static {
    /// Stores an OL block. The slot is extracted from the block header. Also sets the block's
    /// status to "unchecked" if this is a new block.
    fn put_block_data(&self, block: OLBlock) -> DbResult<()>;

    /// Retrieves an OL block for a given block ID.
    fn get_block_data(&self, id: OLBlockId) -> DbResult<Option<OLBlock>>;

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

    /// Returns the highest slot that has a valid OL block, or an error at genesis or when no valid
    /// block exists.
    fn get_tip_slot(&self) -> DbResult<Slot>;
}

/// Database interface for OL mempool transactions.
///
/// Stores transactions as opaque bytes with ordering metadata.
pub trait MempoolDatabase: Send + Sync + 'static {
    /// Store a transaction in the mempool.
    ///
    /// Does not validate that txid matches the transaction bytes.
    fn put_tx(&self, data: MempoolTxData) -> DbResult<()>;

    /// Get a transaction by its ID.
    ///
    /// Returns transaction data if found.
    fn get_tx(&self, txid: OLTxId) -> DbResult<Option<MempoolTxData>>;

    /// Get all transactions in the mempool
    ///
    /// Does not validate or parse transaction format.
    fn get_all_txs(&self) -> DbResult<Vec<MempoolTxData>>;

    /// Delete a transaction from the mempool.
    ///
    /// Returns true if the transaction existed and was deleted, false otherwise.
    fn del_tx(&self, txid: OLTxId) -> DbResult<bool>;
}
