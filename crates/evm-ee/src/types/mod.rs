//! EVM Execution Environment types.
//!
//! This module defines the types needed for EVM block execution within the
//! ExecutionEnvironment trait framework.

pub(crate) type Hash = [u8; 32];

// Module declarations
mod block;
mod block_body;
mod header;
mod partial_state;
mod witness_db;
mod write_batch;

// Re-export public types
pub use block::EvmBlock;
pub use block_body::EvmBlockBody;
pub use header::EvmHeader;
pub use partial_state::EvmPartialState;
// Internal types
pub(crate) use witness_db::WitnessDB;
pub use write_batch::EvmWriteBatch;

// Keep tests module
#[cfg(test)]
mod tests;
