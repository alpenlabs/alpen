//! Trait definitions for low level database interfaces.  This borrows some of
//! its naming conventions from reth.

use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::Serialize;
use strata_asm_common::{AsmManifest, AuxData};
use strata_asm_proto_checkpoint_types::CheckpointPayload;
use strata_checkpoint_types::EpochSummary;
use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput};
use strata_identifiers::{
    AccountId, Epoch, EpochCommitment, Hash, L1Height, OLBlockCommitment, OLBlockId, OLTxId, Slot,
};
use strata_ol_chain_types_new::OLBlock;
use strata_ol_state_types::{OLAccountState, OLState, WriteBatch};
use strata_paas::TaskRecordData;
use strata_primitives::prelude::*;
use strata_state::asm_state::AsmState;
use zkaleido::ProofReceiptWithMetadata;

use crate::{
    mmr_index::{LeafPos, MmrBatchWrite, MmrNodePos, MmrNodeTable, NodePos},
    ol_state_index::{AccountUpdateRecord, EpochIndexingData, InboxMessageRecord, IndexingWrites},
    types::{
        BundledPayloadEntry, ChunkedEnvelopeEntry, IntentEntry, L1PayloadIntentIndex, L1TxEntry,
        MempoolTxData,
    },
    DbResult, RawMmrId,
};

/// Common database backend interface that we can parameterize worker tasks over if
/// parameterizing them over each individual trait gets cumbersome or if we need
/// to use behavior that crosses different interfaces.
pub trait DatabaseBackend: Send + Sync {
    fn asm_db(&self) -> Arc<impl AsmDatabase>;
    fn l1_db(&self) -> Arc<impl L1Database>;
    fn client_state_db(&self) -> Arc<impl ClientStateDatabase>;
    fn ol_block_db(&self) -> Arc<impl OLBlockDatabase>;
    fn ol_state_db(&self) -> Arc<impl OLStateDatabase>;
    fn ol_checkpoint_db(&self) -> Arc<impl OLCheckpointDatabase>;
    fn writer_db(&self) -> Arc<impl L1WriterDatabase>;
    fn checkpoint_proof_db(&self) -> Arc<impl CheckpointProofDatabase>;
    fn prover_task_db(&self) -> Arc<impl ProverTaskDatabase>;
    fn broadcast_db(&self) -> Arc<impl L1BroadcastDatabase>;
    fn chunked_envelope_db(&self) -> Arc<impl L1ChunkedEnvelopeDatabase>;
    fn mempool_db(&self) -> Arc<impl MempoolDatabase>;
    fn ol_state_indexing_db(&self) -> Arc<impl OLStateIndexingDatabase>;
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

    /// Writes auxiliary data for a given L1 block.
    fn put_aux_data(&self, block: L1BlockCommitment, data: AuxData) -> DbResult<()>;

    /// Gets auxiliary data for the given L1 block.
    fn get_aux_data(&self, block: L1BlockCommitment) -> DbResult<Option<AuxData>>;
}

/// Database interface to control our view of L1 data.
/// Operations are NOT VALIDATED at this level.
/// Ensure all operations are done through `L1BlockManager`
pub trait L1Database: Send + Sync + 'static {
    /// Stores an ASM manifest for a given L1 block.
    /// Returns error if provided out-of-order.
    fn put_block_data(&self, manifest: AsmManifest) -> DbResult<()>;

    /// Set a specific height, blockid in canonical chain records.
    fn set_canonical_chain_entry(&self, height: L1Height, blockid: L1BlockId) -> DbResult<()>;

    /// remove canonical chain records in given range (inclusive)
    fn remove_canonical_chain_entries(
        &self,
        start_height: L1Height,
        end_height: L1Height,
    ) -> DbResult<()>;

    /// Prune earliest blocks till height
    fn prune_to_height(&self, height: L1Height) -> DbResult<()>;

    // TODO(STR-2653): DA scraping storage

    // Gets current chain tip height, blockid
    fn get_canonical_chain_tip(&self) -> DbResult<Option<(L1Height, L1BlockId)>>;

    /// Gets the ASM manifest for a blockid.
    fn get_block_manifest(&self, blockid: L1BlockId) -> DbResult<Option<AsmManifest>>;

    /// Gets the blockid at height for the current chain.
    fn get_canonical_blockid_at_height(&self, height: L1Height) -> DbResult<Option<L1BlockId>>;

    // TODO(STR-2653): This should not exist in database level and should be handled by downstream
    // manager.
    /// Returns a half-open interval of block hashes, if we have all of them
    /// present.  Otherwise, returns error.
    fn get_canonical_blockid_range(
        &self,
        start_idx: L1Height,
        end_idx: L1Height,
    ) -> DbResult<Vec<L1BlockId>>;

    // TODO(STR-2653): DA queries
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

/// Database for OL checkpoint data.
pub trait OLCheckpointDatabase: Send + Sync + 'static {
    /// Inserts an epoch summary retrievable by its epoch commitment.
    ///
    /// Fails if there's already an entry there.
    fn insert_epoch_summary(&self, epoch: EpochSummary) -> DbResult<()>;

    /// Gets an epoch summary given an epoch commitment.
    fn get_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<Option<EpochSummary>>;

    /// Gets all commitments for an epoch. This makes no guarantees about ordering.
    fn get_epoch_commitments_at(&self, epoch: Epoch) -> DbResult<Vec<EpochCommitment>>;

    /// Gets the index of the last epoch that we have a summary for, if any.
    fn get_last_summarized_epoch(&self) -> DbResult<Option<Epoch>>;

    /// Delete a specific epoch summary by epoch commitment.
    ///
    /// Returns true if the epoch summary existed and was deleted, false otherwise.
    fn del_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<bool>;

    /// Delete epoch summaries from the specified epoch onwards (inclusive).
    ///
    /// This method deletes all epoch summaries with epoch index >= start_epoch.
    /// Returns a vector of deleted epoch commitments.
    fn del_epoch_summaries_from_epoch(&self, start_epoch: Epoch) -> DbResult<Vec<EpochCommitment>>;

    /// Store an OL checkpoint payload entry by epoch commitment.
    fn put_checkpoint_payload_entry(
        &self,
        epoch: EpochCommitment,
        payload: CheckpointPayload,
    ) -> DbResult<()>;

    /// Get an OL checkpoint payload entry by epoch commitment.
    fn get_checkpoint_payload_entry(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<Option<CheckpointPayload>>;

    /// Get last written checkpoint payload commitment.
    fn get_last_checkpoint_payload_epoch(&self) -> DbResult<Option<EpochCommitment>>;

    /// Delete a checkpoint payload entry by epoch commitment.
    ///
    /// Returns true if it existed and was deleted.
    /// If present, the signing entry for the same commitment is also deleted.
    fn del_checkpoint_payload_entry(&self, epoch: EpochCommitment) -> DbResult<bool>;

    /// Delete checkpoint payload entries from the specified epoch onwards (inclusive).
    ///
    /// Returns a vector of deleted epoch commitments.
    /// Signing entries for deleted payload commitments are also deleted.
    fn del_checkpoint_payload_entries_from_epoch(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>>;

    /// Delete locally-built checkpoint payload entries from the specified epoch onwards.
    ///
    /// Returns a vector of deleted epoch commitments. Signing entries for deleted
    /// payload commitments are also deleted. L1-observed checkpoint payloads and
    /// L1 refs are preserved.
    fn del_local_checkpoint_payload_entries_from_epoch(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>>;

    /// Store an OL checkpoint signing entry by epoch.
    fn put_checkpoint_signing_entry(
        &self,
        epoch: EpochCommitment,
        payload_intent_idx: L1PayloadIntentIndex,
    ) -> DbResult<()>;

    /// Get an OL checkpoint signing entry by epoch.
    fn get_checkpoint_signing_entry(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<Option<L1PayloadIntentIndex>>;

    /// Delete an OL checkpoint signing entry by epoch.
    ///
    /// Returns true if it existed and was deleted.
    fn del_checkpoint_signing_entry(&self, epoch: EpochCommitment) -> DbResult<bool>;

    /// Delete checkpoint signing entries from the specified epoch onwards (inclusive).
    ///
    /// Returns a vector of deleted epoch commitments.
    fn del_checkpoint_signing_entries_from_epoch(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>>;

    /// Get the next checkpoint epoch that is unsigned.
    fn get_next_unsigned_checkpoint_epoch(&self) -> DbResult<Option<Epoch>>;

    /// Store an OL checkpoint L1 ref by epoch commitment.
    fn put_checkpoint_l1_ref(
        &self,
        epoch: EpochCommitment,
        l1_ref: CheckpointL1Ref,
    ) -> DbResult<()>;

    /// Get an OL checkpoint L1 ref by epoch commitment.
    fn get_checkpoint_l1_ref(&self, epoch: EpochCommitment) -> DbResult<Option<CheckpointL1Ref>>;

    /// Get the highest epoch commitment that has an L1 ref.
    fn get_last_checkpoint_l1_ref_epoch(&self) -> DbResult<Option<EpochCommitment>>;

    /// Get all observed `(epoch commitment, L1 ref)` pairs at or above
    /// `start_epoch`, ordered by ascending epoch.
    fn get_checkpoint_l1_refs_from(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<(EpochCommitment, CheckpointL1Ref)>>;

    /// Delete an OL checkpoint L1 ref by epoch commitment.
    ///
    /// Returns true if it existed and was deleted.
    fn del_checkpoint_l1_ref(&self, epoch: EpochCommitment) -> DbResult<bool>;

    /// Delete checkpoint L1 refs from the specified epoch onwards (inclusive).
    ///
    /// Returns a vector of deleted epoch commitments.
    fn del_checkpoint_l1_refs_from_epoch(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>>;

    /// Atomically inserts the L1-observed checkpoint payload and the L1 ref
    /// for `commitment`.
    ///
    /// The payload is stored in a separate table from the sequencer's
    /// locally-built payloads so the two sources of truth stay distinct.
    /// Overwrites any existing entries.
    fn put_checkpoint_l1_observation(
        &self,
        commitment: EpochCommitment,
        payload: CheckpointPayload,
        l1_ref: CheckpointL1Ref,
    ) -> DbResult<()>;

    /// Get the L1-observed checkpoint payload by epoch commitment.
    fn get_checkpoint_l1_observed_payload(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<Option<CheckpointPayload>>;

    /// Delete the L1-observed checkpoint payload by epoch commitment.
    ///
    /// Returns true if it existed and was deleted.
    fn del_checkpoint_l1_observed_payload(&self, epoch: EpochCommitment) -> DbResult<bool>;

    /// Delete L1-observed checkpoint payloads from the specified epoch onwards
    /// (inclusive). Returns a vector of deleted epoch commitments.
    fn del_checkpoint_l1_observed_payloads_from_epoch(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>>;
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

/// Database interface backing [`strata_paas::TaskStore`] for the integrated
/// prover service.
///
/// Keyed by the serialized `ProofSpec::Task` bytes — same contract as the
/// in-memory `TaskStore`. All methods are synchronous and expected to be
/// called through a blocking threadpool by the `strata_storage` manager.
pub trait ProverTaskDatabase: Send + Sync + 'static {
    /// Fetch a record by key. `None` if the key is absent.
    fn get_task(&self, key: Vec<u8>) -> DbResult<Option<TaskRecordData>>;

    /// Insert a new record. Fails with `DbError::EntryAlreadyExists` if
    /// the key is already present — implementations must do this atomically
    /// (e.g. `compare_and_swap(None, Some)`).
    fn insert_task(&self, key: Vec<u8>, record: TaskRecordData) -> DbResult<()>;

    /// Upsert a record — overwrites any existing entry under the key.
    fn put_task(&self, key: Vec<u8>, record: TaskRecordData) -> DbResult<()>;

    /// Removes a task record. Returns `true` if the key existed prior to the
    /// call, `false` otherwise.
    ///
    /// Intended for offline admin tooling (e.g. `strata-dbtool`) — the
    /// runtime task lifecycle is driven by status transitions, not deletion.
    fn delete_task(&self, key: Vec<u8>) -> DbResult<bool>;

    /// All records where `status` is retriable and `retry_after_secs <= now_secs`.
    fn list_retriable(&self, now_secs: u64) -> DbResult<Vec<(Vec<u8>, TaskRecordData)>>;

    /// All records whose status is not yet terminal (Pending / Proving).
    fn list_unfinished(&self) -> DbResult<Vec<(Vec<u8>, TaskRecordData)>>;

    /// Every record in the store, in implementation-defined order.
    ///
    /// Intended for offline admin tooling — the runtime path uses the
    /// filtered iterators above to avoid scanning terminal entries.
    fn list_all_tasks(&self) -> DbResult<Vec<(Vec<u8>, TaskRecordData)>>;

    /// Number of records in the store.
    fn count_tasks(&self) -> DbResult<usize>;
}

/// Checkpoint-proof storage.
///
/// Keyed by [`EpochCommitment`] — the commitment whose checkpoint this
/// proof attests to. Each proof kind has its own peer trait + manager
/// (no shared enum, no opaque-byte scheme). Future EE chunk / EE acct
/// proofs will be `EeChunkProofDatabase`, `EeAcctProofDatabase`, etc.
pub trait CheckpointProofDatabase: Send + Sync + 'static {
    /// Upserts a checkpoint proof for the given epoch.
    ///
    /// Overwrites any existing proof for the same epoch. Re-proves attest to
    /// the same statement, so overwriting is safe and keeps the receipt hook
    /// idempotent — refusing the write would surface as a spurious storage
    /// error on the prover task.
    fn put_proof(&self, epoch: EpochCommitment, proof: ProofReceiptWithMetadata) -> DbResult<()>;

    /// Retrieves the checkpoint proof for the given epoch.
    ///
    /// Returns `Some(proof)` if found, or `None` if not.
    fn get_proof(&self, epoch: EpochCommitment) -> DbResult<Option<ProofReceiptWithMetadata>>;

    /// Deletes the checkpoint proof for the given epoch.
    ///
    /// Tries to delete the proof, returning whether it really existed.
    fn del_proof(&self, epoch: EpochCommitment) -> DbResult<bool>;
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

/// Storage for chunked envelope entries.
///
/// Each entry represents one commit tx funding N reveal txs, tracked through
/// signing, broadcasting, and L1 confirmation.
pub trait L1ChunkedEnvelopeDatabase: Send + Sync + 'static {
    /// Stores a [`ChunkedEnvelopeEntry`] at the given index.
    fn put_chunked_envelope_entry(&self, idx: u64, entry: ChunkedEnvelopeEntry) -> DbResult<()>;

    /// Gets a [`ChunkedEnvelopeEntry`] by its index.
    fn get_chunked_envelope_entry(&self, idx: u64) -> DbResult<Option<ChunkedEnvelopeEntry>>;

    /// Gets chunked envelope entries starting from a given index up to a maximum count.
    ///
    /// Returns entries in ascending index order. If `start_idx` doesn't exist,
    /// starts from the next available entry after it.
    fn get_chunked_envelope_entries_from(
        &self,
        start_idx: u64,
        max_count: usize,
    ) -> DbResult<Vec<(u64, ChunkedEnvelopeEntry)>>;

    /// Gets the next available index.
    fn get_next_chunked_envelope_idx(&self) -> DbResult<u64>;

    /// Deletes a single entry by index.
    ///
    /// Returns true if the entry existed and was deleted.
    fn del_chunked_envelope_entry(&self, idx: u64) -> DbResult<bool>;

    /// Deletes all entries from the given index onwards (inclusive).
    ///
    /// Returns indices of deleted entries.
    fn del_chunked_envelope_entries_from_idx(&self, start_idx: u64) -> DbResult<Vec<u64>>;
}

/// Storage-only MMR indexing database interface.
///
/// This interface intentionally contains only primitive reads and one
/// backend-agnostic atomic batch write entry point.
pub trait MmrIndexDatabase: Send + Sync + 'static {
    /// Returns the node hash for a namespace and node position.
    fn get_node(&self, mmr_id: RawMmrId, pos: NodePos) -> DbResult<Option<Hash>>;

    /// Returns optional preimage bytes for a namespace and leaf position.
    fn get_preimage(&self, mmr_id: RawMmrId, pos: LeafPos) -> DbResult<Option<Vec<u8>>>;

    /// Returns optional preimage bytes for a namespace and leaf range.
    ///
    /// The returned vector has one slot per leaf in `[start, end_exclusive)`.
    /// Missing preimages are returned as `None`.
    ///
    /// Empty ranges return an empty vector. Backends must reject reversed
    /// ranges with [`crate::DbError::MmrInvalidRange`].
    fn get_preimage_range(
        &self,
        mmr_id: RawMmrId,
        start: LeafPos,
        end_exclusive: LeafPos,
    ) -> DbResult<Vec<Option<Vec<u8>>>>;

    /// Returns the current leaf count for a namespace.
    ///
    /// Implementations should return `0` when the namespace has no leaves.
    fn get_leaf_count(&self, mmr_id: RawMmrId) -> DbResult<u64>;

    /// Fetches requested nodes and available parent path nodes in one read.
    ///
    /// If `preimages` is true, implementations should also include available
    /// preimages for requested leaf positions.
    // NOTE: Takes an owned Vec so generated async/chan wrappers can move the
    // argument into 'static worker closures without borrowing/lifetime issues.
    fn fetch_node_paths(&self, nodes: Vec<MmrNodePos>, preimages: bool) -> DbResult<MmrNodeTable>;

    /// Applies an atomic batch write with compare-and-set preconditions.
    ///
    /// If any precondition fails, no writes are applied.
    fn apply_update(&self, batch: MmrBatchWrite) -> DbResult<()>;
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

    /// Returns the latest OL block committed through the high-watermark path, if any.
    ///
    /// This is not the highest block in the OL block database. Plain
    /// [`Self::put_block_data`] does not read or update it.
    fn get_block_high_watermark(&self) -> DbResult<Option<OLBlockCommitment>>;

    /// Stores an OL block and advances the block high-watermark atomically.
    ///
    /// Block persistence semantics match [`Self::put_block_data`]. If the block's slot is not
    /// strictly greater than the current high-watermark slot, this writes nothing and returns
    /// [`DbError::BlockHighWatermarkConflict`](crate::DbError::BlockHighWatermarkConflict).
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

    /// Gets the canonical OL block id at a slot, as recorded by fork choice.
    ///
    /// Returns `None` for slots above the current canonical tip or never written.
    fn get_canonical_block(&self, slot: Slot) -> DbResult<Option<OLBlockId>>;

    /// Replaces canonical blocks from `start_slot`.
    ///
    /// Atomically removes every canonical entry for slots greater than or equal to `start_slot`,
    /// then writes each `(slot, id)` in `blocks`.
    fn replace_canonical_blocks_from(
        &self,
        start_slot: Slot,
        blocks: Vec<(Slot, OLBlockId)>,
    ) -> DbResult<()>;
}

/// Database for OL state indexing data.
///
/// Two write paths reflect the two producer modes:
/// - [`apply_epoch_indexing`](Self::apply_epoch_indexing): single atomic write for an entire epoch.
///   Used by checkpoint-sync producers.
/// - [`apply_block_indexing`](Self::apply_block_indexing): incremental per-block write. Used by
///   block-sync producers.
///
/// Block-sync also calls [`set_epoch_commitment`](Self::set_epoch_commitment)
/// once at epoch finalization to stamp the commitment onto the existing common
/// row; checkpoint-sync includes the commitment in its single write.
///
/// Both paths target the same tables; atomicity granularity differs.
pub trait OLStateIndexingDatabase: Send + Sync + 'static {
    /// Atomically persists an epoch's indexing data in a single call.
    ///
    /// Writes the common record, per-account update entries, per-account
    /// inbox entries, and creation-epoch index entries for newly created
    /// accounts. The common record's `epoch_commitment` is set from
    /// `commitment`. All in one transaction.
    fn apply_epoch_indexing(
        &self,
        commitment: EpochCommitment,
        writes: IndexingWrites,
    ) -> DbResult<()>;

    /// Atomically applies a single block's incremental indexing writes.
    ///
    /// Appends to existing per-(account, epoch) entries, updates the common
    /// row's `created_accounts`, and inserts creation-epoch index entries
    /// for any newly created accounts. Errors with
    /// [`DbError::BlockIndexingConflict`](crate::DbError::BlockIndexingConflict)
    /// when `block.slot()` does not strictly advance past the last applied
    /// block for `epoch`.
    fn apply_block_indexing(
        &self,
        epoch: Epoch,
        block: OLBlockCommitment,
        writes: IndexingWrites,
    ) -> DbResult<()>;

    /// Atomically rolls back all block-attributed writes in `epoch` whose
    /// block slot is strictly greater than `block.slot()`. Records and
    /// creators tagged with `block.slot()` itself are kept. Entries with
    /// `None` attribution (checkpoint-sync) are preserved; they only drop
    /// when the entire epoch is dropped via [`Self::rollback_to_epoch`].
    ///
    /// Idempotent. Does not clear `EpochIndexingData.epoch_commitment`.
    fn rollback_to_block(&self, epoch: Epoch, block: OLBlockCommitment) -> DbResult<()>;

    /// Atomically drops all indexing data for epochs strictly greater than
    /// `epoch`. The given `epoch` is preserved. Idempotent.
    fn rollback_to_epoch(&self, epoch: Epoch) -> DbResult<()>;

    /// Sets the epoch commitment on the existing common row.
    ///
    /// Called once by block-sync producers at epoch finalization. Errors if
    /// no common row exists for the epoch.
    fn set_epoch_commitment(&self, epoch: Epoch, commitment: EpochCommitment) -> DbResult<()>;

    /// Returns the common indexing data for the given epoch.
    fn get_epoch_indexing_data(&self, epoch: Epoch) -> DbResult<Option<EpochIndexingData>>;

    /// Returns the per-(account, epoch) update records.
    ///
    /// Returns `None` when the account had no indexed activity in the epoch.
    fn get_account_update_records(
        &self,
        epoch: Epoch,
        account: AccountId,
    ) -> DbResult<Option<Vec<AccountUpdateRecord>>>;

    /// Returns the per-(account, epoch) inbox records.
    ///
    /// Returns `None` when no inbox writes were recorded for the account in the epoch.
    fn get_account_inbox_records(
        &self,
        epoch: Epoch,
        account: AccountId,
    ) -> DbResult<Option<Vec<InboxMessageRecord>>>;

    /// Returns the epoch in which an account was created.
    fn get_account_creation_epoch(&self, acct: AccountId) -> DbResult<Option<Epoch>>;
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
