//! Reth node for the Alpen codebase.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod engine;
mod evm;
mod node;
mod payload;
mod payload_builder;

pub mod args;
pub use alpen_reth_primitives::WithdrawalIntent;
pub use engine::{StrataEngineTypes, StrataEngineValidator, StrataPayloadTypes};
pub use node::StrataEthereumNode;
pub use payload::{
    ExecutionPayloadEnvelopeV2, ExecutionPayloadFieldV2, StrataExecutionPayloadEnvelopeV2,
    StrataPayloadAttributes,
};
