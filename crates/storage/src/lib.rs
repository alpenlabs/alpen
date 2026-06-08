//! Storage for the Alpen codebase.

mod cache;
mod exec;
mod instrumentation;
mod managers;
mod node_storage;
pub mod ops;

pub use managers::{
    asm::AsmStateManager,
    checkpoint_proof::CheckpointProofDbManager,
    client_state::ClientStateManager,
    l1::L1BlockManager,
    mempool::MempoolDbManager,
    mmr_index::{MmrAppendRequest, MmrIndexHandle, MmrIndexManager, MmrStateView},
    ol::OLBlockManager,
    ol_checkpoint::OLCheckpointManager,
    ol_state::OLStateManager,
    ol_state_indexing::OLStateIndexingManager,
    prover_task::ProverTaskDbManager,
    writer::L1WriterManager,
};
pub use node_storage::{create_node_storage, NodeStorage};
pub use ops::l1tx_broadcast::BroadcastDbOps;
pub use strata_db_types::MmrId;
