//! Common traits and types for Alpen execution environment components.

#![expect(unused_crate_dependencies, reason = "wip")]
pub mod traits;
pub mod types;

pub use traits::{
    engine::{ExecutionEngine, ExecutionEngineError},
    ol_client::{OlClient, OlClientError},
    payload_builder::PayloadBuilderEngine,
    storage::{OLBlockOrSlot, Storage, StorageError},
};
pub use types::{
    consensus_heads::ConsensusHeads,
    ee_account_state::EeAccountStateAtBlock,
    ol_chain_status::OlChainStatus,
    payload_builder::{DepositInfo, PayloadBuildAttributes},
};
