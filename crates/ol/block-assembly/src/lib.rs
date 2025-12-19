//! Block assembly context API for OL.

mod context;
mod epoch_sealing;
mod error;
mod mempool_provider;
#[cfg(test)]
mod test_utils;

pub use context::{
    AccumulatorProofGenerator, BlockAssemblyAnchorContext, BlockAssemblyContext,
    BlockAssemblyStateAccess,
};
pub use epoch_sealing::{EpochSealingPolicy, FixedSlotSealing};
pub use error::BlockAssemblyError;
pub use mempool_provider::{MempoolProvider, MempoolProviderImpl};

/// Result type for block assembly operations.
pub type BlockAssemblyResult<T> = Result<T, BlockAssemblyError>;
