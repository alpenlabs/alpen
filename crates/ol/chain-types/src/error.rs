//! Error types for OL chain types.

use thiserror::Error;

/// Errors that can occur when constructing or validating OL chain types.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ChainTypesError {
    /// Too many transactions in a transaction segment.
    #[error("too many transactions in segment: provided {provided}, max {max}")]
    TooManyTransactions { provided: usize, max: usize },

    /// Too many manifests in an L1 manifest container.
    #[error("too many manifests in L1 manifest container: provided {provided}, max {max}")]
    TooManyManifests { provided: usize, max: usize },
}
