//! Errors reported across the gchain trait surface.
//!
//! Chain providers and processor stages report their failures through these
//! shared enums instead of an associated error type, so the executor can act on
//! the variants it understands without being generic over every implementer.
//! Anything an implementer needs to report that isn't covered goes in `Custom`,
//! where it stays available as an error source.

use std::error::Error;

use thiserror::Error as ThisError;

use crate::processor::ProcId;

/// An implementer-specific error carried inside a `Custom` variant.
pub type BoxedError = Box<dyn Error + Send + Sync>;

/// Failures a [`ChainProvider`](crate::ChainProvider) reports.
///
/// A query that simply has no answer is reported as `Ok(None)`, never as an
/// error, so every variant here means the provider actually failed.
#[derive(Debug, ThisError)]
pub enum ProviderError {
    /// The provider couldn't be reached or read right now, but the same query
    /// may succeed later.  The executor may retry rather than abandoning the
    /// path it's working on.
    #[error("transient chain provider failure")]
    Transient(#[source] BoxedError),

    /// Data was present but couldn't be decoded or didn't hold together.  This
    /// won't fix itself on a retry.
    #[error("malformed stored data for {0}")]
    Malformed(String),

    /// A link's body doesn't match the commitment in its header.
    #[error("link {0} is not structurally consistent")]
    InconsistentLink(String),

    /// A failure specific to this provider implementation.
    #[error(transparent)]
    Custom(BoxedError),
}

impl ProviderError {
    /// Wraps an implementer-specific error.
    pub fn custom(err: impl Into<BoxedError>) -> Self {
        Self::Custom(err.into())
    }

    /// Wraps an implementer-specific error that may resolve on a retry.
    pub fn transient(err: impl Into<BoxedError>) -> Self {
        Self::Transient(err.into())
    }
}

/// Failures a [`GChainProc`](crate::GChainProc) stage reports.
///
/// These only ever mean the stage *failed to do its work*.  A stage deciding
/// that a link is invalid reports that through
/// [`ProcArtifact::is_link_valid`](crate::ProcArtifact::is_link_valid) instead,
/// so that the executor can go look for a different path through the graph
/// rather than giving up.
#[derive(Debug, ThisError)]
pub enum ProcError {
    /// An artifact the stage declared a dependency on wasn't available.
    #[error("missing dep artifact from proc {0}")]
    MissingDep(ProcId),

    /// An artifact couldn't be encoded for persistence.
    #[error("failed encoding artifact")]
    Encode(#[source] BoxedError),

    /// A persisted artifact couldn't be decoded.
    #[error("failed decoding artifact")]
    Decode(#[source] BoxedError),

    /// The stage's aggregated state couldn't be read or written.
    #[error("proc state storage failure")]
    Storage(#[source] BoxedError),

    /// A failure specific to this processor implementation.
    #[error(transparent)]
    Custom(BoxedError),
}

impl ProcError {
    /// Wraps an implementer-specific error.
    pub fn custom(err: impl Into<BoxedError>) -> Self {
        Self::Custom(err.into())
    }

    /// Wraps an artifact encoding failure.
    pub fn encode(err: impl Into<BoxedError>) -> Self {
        Self::Encode(err.into())
    }

    /// Wraps an artifact decoding failure.
    pub fn decode(err: impl Into<BoxedError>) -> Self {
        Self::Decode(err.into())
    }
}
