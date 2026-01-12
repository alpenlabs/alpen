//! Block assembly context API for OL.

mod context;
mod epoch_sealing;
mod error;
mod mempool_provider;
#[cfg(test)]
mod test_utils;
mod types;

pub use context::{
    AccumulatorProofGenerator, BlockAssemblyAnchorContext, BlockAssemblyContext,
    BlockAssemblyStateAccess,
};
pub use epoch_sealing::{EpochSealingPolicy, FixedSlotSealing};
pub use error::BlockAssemblyError;
pub use mempool_provider::{MempoolProvider, MempoolProviderImpl};
pub use types::{BlockCompletionData, BlockGenerationConfig, BlockTemplate, FullBlockTemplate};

/// Result type for block assembly operations.
pub type BlockAssemblyResult<T> = Result<T, BlockAssemblyError>;
