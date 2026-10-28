//! Provides read-only access to canonical OL views from fork choice or finalized checkpoints.

mod finalized;
mod observation;
mod provider;
mod traits;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

pub use finalized::FinalizedOLChainProvider;
pub use observation::resolve_observed_epoch_blocking;
pub use provider::OLCanonicalChainProviderImpl;
pub use traits::OLCanonicalChainProvider;
