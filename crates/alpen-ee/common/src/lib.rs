//! Common traits and types for Alpen execution environment components.

mod traits;
mod types;
mod utils;

pub use traits::{
    engine::{ExecutionEngine, ExecutionEngineError},
    ol_client::{
        chain_status_checked, get_inbox_messages_checked, OLBlockData, OLClient, OLClientError,
        SequencerOLClient,
    },
    payload_builder::{EnginePayload, PayloadBuilderEngine},
    storage::{ExecBlockStorage, OLBlockOrEpoch, Storage, StorageError},
};
#[cfg(feature = "test-utils")]
pub use traits::{
    ol_client::{MockOLClient, MockSequencerOLClient},
    storage::{
        exec_block_storage_test_fns, tests as storage_test_fns, MockExecBlockStorage, MockStorage,
    },
};
pub use types::{
    consensus_heads::ConsensusHeads,
    ee_account_state::EeAccountStateAtEpoch,
    exec_record::ExecBlockRecord,
    ol_account_epoch_summary::OLEpochSummary,
    ol_chain_status::{OLFinalizedStatus, OLChainStatus},
    payload_builder::{DepositInfo, PayloadBuildAttributes},
};
pub use utils::conversions::sats_to_gwei;
