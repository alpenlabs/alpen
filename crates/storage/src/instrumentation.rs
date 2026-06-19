//! Instrumentation component identifiers for storage operations.

/// Component identifiers for tracing spans in storage operations.
///
/// These are the canonical component strings for the storage layer. The
/// per-database-op spans are emitted by the `gen_proxy`-generated proxies in
/// `strata-db-types`, whose `tracing_component` attributes mirror these exact
/// values; the manager layer also tags its own spans with them. Some entries
/// are therefore only referenced from the (string-literal) proxy attributes,
/// so the module allows dead code to keep the full registry documented here.
#[expect(
    dead_code,
    reason = "mirrored into gen_proxy `tracing_component` attributes"
)]
pub(crate) mod components {
    /// L1Database operations. Fields: blkid, height
    pub(crate) const STORAGE_L1: &str = "storage:l1";

    /// OLDatabase operations. Fields: blkid, slot
    pub(crate) const STORAGE_OL: &str = "storage:ol";

    /// OLStateDatabase operations. Fields: state_root, epoch
    pub(crate) const STORAGE_OL_STATE: &str = "storage:ol_state";

    /// AsmDatabase operations. Fields: blkid, height
    pub(crate) const STORAGE_ASM: &str = "storage:asm";

    /// ClientStateDatabase operations. Fields: client_id, state_version
    pub(crate) const STORAGE_CLIENT_STATE: &str = "storage:client_state";

    /// MempoolDatabase operations. Fields: tx_id, priority
    pub(crate) const STORAGE_MEMPOOL: &str = "storage:mempool";

    /// MmrIndexDatabase operations. Fields: mmr_id, node_pos
    pub(crate) const STORAGE_MMR_INDEX: &str = "storage:mmr_index";

    /// L1BroadcastDatabase operations. Fields: tx_id, broadcast_index
    pub(crate) const STORAGE_L1_BROADCAST: &str = "storage:l1_broadcast";

    /// L1WriterDatabase operations. Fields: envelope_id, payload_size
    pub(crate) const STORAGE_L1_WRITER: &str = "storage:l1_writer";

    /// OLCheckpointDatabase operations. Fields: epoch
    pub(crate) const STORAGE_OL_CHECKPOINT: &str = "storage:ol_checkpoint";

    /// L1ChunkedEnvelopeDatabase operations. Fields: idx
    pub(crate) const STORAGE_CHUNKED_ENVELOPE: &str = "storage:chunked_envelope";

    /// CheckpointProofDatabase operations. Fields: epoch
    pub(crate) const STORAGE_CHECKPOINT_PROOF: &str = "storage:checkpoint_proof";

    /// ProverTaskDatabase operations. Fields: key
    pub(crate) const STORAGE_PROVER_TASK: &str = "storage:prover_task";

    /// OLStateIndexingDatabase operations. Fields: epoch, account_id
    pub(crate) const STORAGE_OL_STATE_INDEXING: &str = "storage:ol_state_indexing";
}
