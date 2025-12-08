//! Trait definitions for low level database interfaces.  This borrows some of
//! its naming conventions from reth.

use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::Serialize;
use strata_asm_types::{L1BlockManifest, L1Tx, L1TxRef};
use strata_checkpoint_types::EpochSummary;
use strata_csm_types::{ClientState, ClientUpdateOutput};
use strata_ol_chain_types::L2BlockBundle;
use strata_primitives::{
    prelude::*,
    proof::{ProofContext, ProofKey},
};
use strata_state::asm_state::AsmState;
use zkaleido::ProofReceiptWithMetadata;

use crate::{
    chainstate::ChainstateDatabase,
    types::{BundledPayloadEntry, CheckpointEntry, IntentEntry, L1TxEntry},
    DbResult,
};

/// Common database backend interface that we can parameterize worker tasks over if
/// parameterizing them over each individual trait gets cumbersome or if we need
/// to use behavior that crosses different interfaces.
pub trait DatabaseBackend: Send + Sync {
    fn asm_db(&self) -> Arc<impl AsmDatabase>;
    fn l1_db(&self) -> Arc<impl L1Database>;
    fn l2_db(&self) -> Arc<impl L2BlockDatabase>;
    fn client_state_db(&self) -> Arc<impl ClientStateDatabase>;
    fn chain_state_db(&self) -> Arc<impl ChainstateDatabase>;
    fn checkpoint_db(&self) -> Arc<impl CheckpointDatabase>;
    fn writer_db(&self) -> Arc<impl L1WriterDatabase>;
    fn prover_db(&self) -> Arc<impl ProofDatabase>;
    fn broadcast_db(&self) -> Arc<impl L1BroadcastDatabase>;
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

    /// Stores a manifest hash at the given MMR leaf index.
    ///
    /// This is called after appending a manifest to the MMR to maintain
    /// a fast lookup index from leaf index to manifest hash.
    fn store_manifest_hash(&self, index: u64, hash: [u8; 32]) -> DbResult<()>;

    /// Gets a manifest hash by MMR leaf index.
    ///
    /// Used by aux data resolver to retrieve manifest hashes for subprotocols.
    fn get_manifest_hash(&self, index: u64) -> DbResult<Option<[u8; 32]>>;
}

/// Database interface to control our view of L1 data.
/// Operations are NOT VALIDATED at this level.
/// Ensure all operations are done through `L1BlockManager`
pub trait L1Database: Send + Sync + 'static {
    /// Atomically extends the chain with a new block, providing the manifest
    /// and a list of transactions we find relevant.  Returns error if
    /// provided out-of-order.
    fn put_block_data(&self, mf: L1BlockManifest) -> DbResult<()>;

    /// Set a specific height, blockid in canonical chain records.
    fn set_canonical_chain_entry(&self, height: u64, blockid: L1BlockId) -> DbResult<()>;

    /// remove canonical chain records in given range (inclusive)
    fn remove_canonical_chain_entries(&self, start_height: u64, end_height: u64) -> DbResult<()>;

    /// Prune earliest blocks till height
    fn prune_to_height(&self, height: u64) -> DbResult<()>;

    // TODO DA scraping storage

    // Gets current chain tip height, blockid
    fn get_canonical_chain_tip(&self) -> DbResult<Option<(u64, L1BlockId)>>;

    /// Gets the block manifest for a blockid.
    fn get_block_manifest(&self, blockid: L1BlockId) -> DbResult<Option<L1BlockManifest>>;

    /// Gets the blockid at height for the current chain.
    fn get_canonical_blockid_at_height(&self, height: u64) -> DbResult<Option<L1BlockId>>;

    // TODO: This should not exist in database level and should be handled by downstream manager.
    /// Returns a half-open interval of block hashes, if we have all of them
    /// present.  Otherwise, returns error.
    fn get_canonical_blockid_range(&self, start_idx: u64, end_idx: u64)
        -> DbResult<Vec<L1BlockId>>;

    /// Gets the relevant txs we stored in a block.
    fn get_block_txs(&self, blockid: L1BlockId) -> DbResult<Option<Vec<L1TxRef>>>;

    /// Gets the tx with proof given a tx ref, if present.
    fn get_tx(&self, tx_ref: L1TxRef) -> DbResult<Option<L1Tx>>;

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
    fn put_intent_entry(&self, payloadid: Buf32, payloadentry: IntentEntry) -> DbResult<()>;

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
