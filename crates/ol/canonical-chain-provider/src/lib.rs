//! Provides read-only access to the canonical OL chain selected by its fork-choice owner.

mod provider;
mod traits;

pub use provider::OLCanonicalChainProviderImpl;
pub use traits::OLCanonicalChainProvider;
