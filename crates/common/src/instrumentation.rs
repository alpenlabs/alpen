//! Instrumentation component identifiers.
//!
//! # Usage
//!
//! Instrument database trait implementations (not shims):
//!
//! ```rust,ignore
//! #[instrument(skip(self), err, fields(component = components::STORAGE_L1, block_id = %id, height))]
//! pub fn put_block_data(&self, manifest: AsmManifest) -> Result<(), DbError> { ... }
//! ```
//!
//! - Always include `err` for fallible functions
//! - Always include primary identifiers (IDs, heights) as fields
//! - Instrument at implementation layer, not shim/delegation layer

/// Service identifiers for ServiceState implementations.
pub mod services {
    pub const ASM_WORKER: &str = "asm_worker";
    pub const CHAIN_WORKER: &str = "chain_worker";
    pub const CSM_WORKER: &str = "csm_worker";
}

/// Component identifiers for tracing spans.
///
/// Use these in database trait implementations and worker state machines.
/// Always specify required fields as documented.
pub mod components {
    // Storage layer - use in crates/db/store-sled/src/*.rs trait implementations

    /// L1Database operations. Fields: block_id, height
    pub const STORAGE_L1: &str = "storage:l1";

    /// L2Database operations. Fields: block_id, height
    pub const STORAGE_L2: &str = "storage:l2";

    /// OLDatabase operations. Fields: block_id, slot
    pub const STORAGE_OL: &str = "storage:ol";

    /// OLStateDatabase operations. Fields: state_root, epoch
    pub const STORAGE_OL_STATE: &str = "storage:ol_state";

    /// AsmDatabase operations. Fields: block_id, height
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

    /// EENodeDatabase operations. Fields: account_id, block_id, finalized_height
    pub const STORAGE_EE_NODE: &str = "storage:ee_node";

    // Database layer - low-level operations only

    /// Sled transaction lifecycle. Fields: tx_id, attempt, conflict_key. DEBUG level only.
    pub const DB_SLED_TRANSACTION: &str = "db:sled:transaction";
}
