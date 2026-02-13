//! Instrumentation component identifiers for storage operations.

/// Component identifiers for tracing spans in storage operations.
pub(crate) mod components {
    /// L1Database operations. Fields: blkid, height
    pub(crate) const STORAGE_L1: &str = "storage:l1";

    /// L2Database operations. Fields: blkid, height
    pub(crate) const STORAGE_L2: &str = "storage:l2";

    /// OLDatabase operations. Fields: blkid, slot
    pub(crate) const STORAGE_OL: &str = "storage:ol";

    /// OLStateDatabase operations. Fields: state_root, epoch
    pub(crate) const STORAGE_OL_STATE: &str = "storage:ol_state";

    /// AsmDatabase operations. Fields: blkid, height
    pub(crate) const STORAGE_ASM: &str = "storage:asm";

    /// CheckpointDatabase operations. Fields: epoch, checkpoint_id
    pub(crate) const STORAGE_CHECKPOINT: &str = "storage:checkpoint";

    /// ChainStateDatabase operations. Fields: chain_id, state_root
    pub(crate) const STORAGE_CHAINSTATE: &str = "storage:chainstate";

    /// ClientStateDatabase operations. Fields: client_id, state_version
    pub(crate) const STORAGE_CLIENT_STATE: &str = "storage:client_state";

    /// MempoolDatabase operations. Fields: tx_id, priority
    pub(crate) const STORAGE_MEMPOOL: &str = "storage:mempool";

    /// GlobalMmrDatabase operations. Fields: mmr_size, peak_count
    pub(crate) const STORAGE_GLOBAL_MMR: &str = "storage:global_mmr";

    /// L1BroadcastDatabase operations. Fields: tx_id, broadcast_index
    pub(crate) const STORAGE_L1_BROADCAST: &str = "storage:l1_broadcast";

    /// L1WriterDatabase operations. Fields: envelope_id, payload_size
    pub(crate) const STORAGE_L1_WRITER: &str = "storage:l1_writer";

    /// OLCheckpointDatabase operations. Fields: epoch
    pub(crate) const STORAGE_OL_CHECKPOINT: &str = "storage:ol_checkpoint";

    /// AccountGenesisDatabase operations. Fields: account_id, epoch
    pub(crate) const STORAGE_ACCOUNT_GENESIS: &str = "storage:account_genesis";
}
