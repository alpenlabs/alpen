//! OL block ancestry queries along an explicitly selected branch.

mod error;
mod provider;

pub use error::BlockAncestryError;
pub use provider::{BlockAncestryProvider, EpochBlockAncestry};
