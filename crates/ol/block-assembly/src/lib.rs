//! Block assembly API for OL.

mod block_assembly;
mod builder;
mod command;
mod context;
mod epoch_sealing;
mod error;
mod handle;
mod mempool_provider;
mod service;
mod state;
#[cfg(test)]
mod test_utils;
mod types;

pub use builder::BlockasmServiceBuilder;
pub use context::{
    AccumulatorProofGenerator, BlockAssemblyContext, BlockAssemblyContextImpl,
    BlockAssemblyStateAccess,
};
pub use epoch_sealing::{EpochSealingPolicy, FixedSlotSealing};
pub use error::BlockAssemblyError;
pub use handle::BlockasmHandle;
pub use mempool_provider::{MempoolProvider, MempoolProviderImpl};
pub use types::{BlockCompletionData, BlockGenerationConfig, BlockTemplate, FullBlockTemplate};

/// Result type for block assembly operations.
pub type BlockAssemblyResult<T> = Result<T, BlockAssemblyError>;
