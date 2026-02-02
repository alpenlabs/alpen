//! Common traits and types for Alpen execution environment components.

mod traits;
mod types;
mod utils;

#[cfg(feature = "test-utils")]
pub use traits::{
    da::{MockBatchDaProvider, MockDaBlobProvider},
    ol_client::{MockOLClient, MockSequencerOLClient},
    prover::MockBatchProver,
    storage::{
        batch_storage_test_fns, exec_block_storage_test_fns, tests as storage_test_fns,
        InMemoryStorage, MockBatchStorage, MockExecBlockStorage, MockStorage,
    },
};
pub use traits::{
    da::{BatchDaProvider, DaBlobProvider, DaStatus},
    engine::{EnginePayload, ExecutionEngine, ExecutionEngineError, PayloadBuilderEngine},
    ol_client::{
        chain_status_checked, get_inbox_messages_checked, OLAccountStateView, OLBlockData,
        OLClient, OLClientError, SequencerOLClient,
    },
    prover::{BatchProver, ProofGenerationStatus},
    storage::{
        require_best_ee_account_state, require_best_finalized_block, require_genesis_batch,
        require_latest_batch, BatchStorage, ExecBlockStorage, OLBlockOrEpoch, Storage,
        StorageError,
    },
};
pub use types::{
    batch::{Batch, BatchId, BatchStatus, L1DaBlockRef},
    blocknumhash::BlockNumHash,
    chunk::{Chunk, ChunkId, ChunkStatus},
    consensus_heads::ConsensusHeads,
    da::{prepare_da_chunks, reassemble_from_da_chunks},
    ee_account_state::EeAccountStateAtEpoch,
    exec_record::{ExecBlockPayload, ExecBlockRecord},
    ol_account_epoch_summary::OLEpochSummary,
    ol_chain_status::{OLChainStatus, OLFinalizedStatus},
    payload_builder::{DepositInfo, PayloadBuildAttributes},
    prover::{Proof, ProofId},
};
pub use utils::{
    clock::{Clock, SystemClock},
    conversions::sats_to_gwei,
};
