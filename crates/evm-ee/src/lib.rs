//! EVM Execution Environment implementation for Alpen.
//!
//! This crate provides an implementation of the `ExecutionEnvironment` trait
//! for Ethereum Virtual Machine (EVM) block execution, enabling EVM blocks
//! to be executed and proven within the Alpen rollup system.

mod execution;
mod types;
mod utils;

pub use execution::EvmExecutionEnvironment;
pub use types::{EvmBlock, EvmBlockBody, EvmHeader, EvmPartialState, EvmWriteBatch};
