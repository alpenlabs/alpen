//! Block assembly API for OL.

mod block_assembly;
mod builder;
mod checkpoint_size;
mod command;
mod context;
mod da_tracker;
mod epoch_sealing;
mod error;
mod handle;
mod mempool_provider;
mod service;
mod state;
#[cfg(test)]
mod test_utils;
#[cfg(test)]
mod tests;
mod types;

pub use builder::BlockasmBuilder;
pub use context::{
    AccumulatorProofGenerator, BlockAssemblyAnchorContext, BlockAssemblyContext,
    BlockAssemblyStateAccess,
};
pub use epoch_sealing::{EpochSealingPolicy, FixedSlotSealing};
pub use error::BlockAssemblyError;
pub use handle::BlockasmHandle;
pub use mempool_provider::{MempoolProvider, MempoolProviderImpl};
#[cfg(test)]
use strata_codec as _;
pub use types::{BlockCompletionData, BlockGenerationConfig, BlockTemplate, FullBlockTemplate};

/// Result type for block assembly operations.
pub type BlockAssemblyResult<T> = Result<T, BlockAssemblyError>;
