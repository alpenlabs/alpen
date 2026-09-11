use strata_gchain_types::{BoxedError, ProcError, ProcId, ProviderError};
use thiserror::Error;

/// Failures the executor reports while driving a processor pipeline.
///
/// The provider and processor failures keep their own error types rather than
/// being flattened into this one, so a caller can still tell a transient
/// storage failure apart from a stage failing to do its work.
#[derive(Debug, Error)]
pub enum GExecError {
    /// A link the executor was told to process isn't in the provider.
    #[error("missing link {0}")]
    MissingLink(String),

    /// The provider doesn't know where a link sits in the graph, so the
    /// executor can't tell what it connects.
    #[error("missing endpoints for link {0}")]
    MissingLinkEndpoints(String),

    /// The chain provider failed to answer a query.
    #[error("chain provider failed")]
    Provider(#[from] ProviderError),

    /// A processor stage failed while doing its work.  This never means the
    /// link was invalid, only that the stage couldn't process it.
    #[error("proc {0} failed")]
    Proc(ProcId, #[source] ProcError),

    /// A stage was handed an artifact that isn't of the type it produces.  This
    /// means artifacts got crossed between stages somewhere in the executor.
    #[error("artifact for proc {0} was not of that proc's artifact type")]
    ArtifactTypeMismatch(ProcId),

    /// The executor expected to have an artifact on hand for a stage but didn't.
    #[error("missing artifact for proc {0}")]
    MissingArtifact(ProcId),

    /// Two stages were registered under the same ID, which would make a dep
    /// naming that ID ambiguous.
    #[error("duplicate proc {0} in pipeline")]
    DuplicateProc(ProcId),

    /// The executor's own tracking or artifact storage failed.
    #[error("executor storage failure")]
    Storage(#[source] BoxedError),
}
