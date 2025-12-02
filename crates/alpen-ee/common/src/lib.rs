//! Common traits and types for Alpen execution environment components.

#![expect(unused_crate_dependencies, reason = "wip")]
pub mod traits;
pub mod types;

pub use traits::{
    engine::{ExecutionEngine, ExecutionEngineError},
    ol_client::{
        block_commitments_in_range_checked, chain_status_checked,
        get_update_operations_for_blocks_checked, OlClient, OlClientError,
    },
    payload_builder::{EnginePayload, PayloadBuilderEngine},
    storage::{ExecBlockStorage, OLBlockOrSlot, Storage, StorageError},
};
#[cfg(feature = "test-utils")]
pub use traits::{
    ol_client::MockOlClient,
    storage::{MockExecBlockStorage, MockStorage},
};
pub use types::{
    consensus_heads::ConsensusHeads,
    ee_account_state::EeAccountStateAtBlock,
    exec_record::ExecBlockRecord,
    ol_chain_status::OlChainStatus,
    payload_builder::{DepositInfo, PayloadBuildAttributes},
};
