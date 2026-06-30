//! Common traits and types for Alpen execution environment components.

#[cfg(test)]
use mockall as _;

mod traits;
mod types;
mod utils;

pub use traits::{
    da::{BatchDaProvider, DaBlobSource, DaStatus, HeaderSummaryProvider},
    engine::{
        EnginePayload, ExecutionEngine, ExecutionEngineError, ForkchoiceState, PayloadBuilderEngine,
    },
    ol_client::{
        chain_status_checked, get_inbox_messages_checked, OLAccountStateView, OLBlockData,
        OLClient, OLClientError, SequencerOLClient,
    },
    prover::{BatchProver, ProofGenerationStatus},
    storage::{
        require_best_ee_account_state, require_best_finalized_block, require_genesis_batch,
        require_latest_batch, AccessedStateStore, BatchStorage, BlockWitnessStore, ChunkStorage,
        ExecBlockStorage, OLBlockOrEpoch, Storage, StorageError,
    },
};
#[cfg(feature = "test-utils")]
pub use traits::{
    da::{MockBatchDaProvider, MockDaBlobSource},
    ol_client::{MockOLClient, MockSequencerOLClient},
    prover::MockBatchProver,
    storage::{
        batch_storage_test_fns, chunk_storage_test_fns, exec_block_storage_test_fns,
        tests as storage_test_fns, InMemoryStorage, MockAccessedStateStore, MockBatchStorage,
        MockBlockWitnessStore, MockChunkStorage, MockExecBlockStorage, MockStorage,
    },
};
pub use types::{
    accessed_state::{AccessedAccount, AccessedStateRecord},
    batch::{Batch, BatchId, BatchStatus, L1DaBlockInfo, L1DaBlockRef},
    blocknumhash::BlockNumHash,
    chunk::{Chunk, ChunkId, ChunkStatus},
    consensus_heads::ConsensusHeads,
    ee_account_state::EeAccountStateAtEpoch,
    exec_record::{ExecBlockPayload, ExecBlockRecord},
    fees::{
        FeeBreakdown, FeeModelConfig, FeeModelError, FeeQuoteInputs, GasEquivalentQuote,
        L1FeeRateSource, DA_OVERHEAD_MULTIPLIER_SCALE_BPS,
    },
    ol_account_epoch_summary::{SnarkAccountEpochSummary, SnarkAccountUpdateInfo},
    ol_chain_status::{OLChainStatus, OLFinalizedStatus},
    payload_builder::{DepositInfo, PayloadBuildAttributes},
    prover::{Proof, ProofId},
    prover_task_key::{
        decode_batch_task_key, decode_chunk_task_key, encode_batch_task_key, encode_chunk_task_key,
        ProverTaskKeyDecodeError, ProverTaskKeyKind, BATCH_TASK_KEY_TAG, CHUNK_TASK_KEY_TAG,
        RANGE_TASK_KEY_BYTES,
    },
};
pub use utils::{
    clock::{Clock, SystemClock},
    conversions::sats_to_gwei,
    ledger_refs::build_ledger_refs_from_da,
};
