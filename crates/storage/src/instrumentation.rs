//! Instrumentation component identifiers for storage operations.

/// Component identifiers for tracing spans in storage operations.
pub mod components {
    /// L1Database operations. Fields: blkid, height
    pub const STORAGE_L1: &str = "storage:l1";

    /// L2Database operations. Fields: blkid, height
    pub const STORAGE_L2: &str = "storage:l2";

    /// OLDatabase operations. Fields: blkid, slot
    pub const STORAGE_OL: &str = "storage:ol";

    /// OLStateDatabase operations. Fields: state_root, epoch
    pub const STORAGE_OL_STATE: &str = "storage:ol_state";

    /// AsmDatabase operations. Fields: blkid, height
    pub const STORAGE_ASM: &str = "storage:asm";

    /// CheckpointDatabase operations. Fields: epoch, checkpoint_id
    pub const STORAGE_CHECKPOINT: &str = "storage:checkpoint";

    /// ChainStateDatabase operations. Fields: chain_id, state_root
    pub const STORAGE_CHAINSTATE: &str = "storage:chainstate";

    /// ClientStateDatabase operations. Fields: client_id, state_version
    pub const STORAGE_CLIENT_STATE: &str = "storage:client_state";

    /// MempoolDatabase operations. Fields: tx_id, priority
    pub const STORAGE_MEMPOOL: &str = "storage:mempool";

    /// GlobalMmrDatabase operations. Fields: mmr_size, peak_count
    pub const STORAGE_GLOBAL_MMR: &str = "storage:global_mmr";

    /// L1BroadcastDatabase operations. Fields: tx_id, broadcast_index
    pub const STORAGE_L1_BROADCAST: &str = "storage:l1_broadcast";

    /// L1WriterDatabase operations. Fields: envelope_id, payload_size
    pub const STORAGE_L1_WRITER: &str = "storage:l1_writer";

    /// EENodeDatabase operations. Fields: account_id, blkid, finalized_height
    pub const STORAGE_EE_NODE: &str = "storage:ee_node";
}
