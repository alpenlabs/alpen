use strata_gchain_types::{BoxedError, ProcError, ProcId, ProviderError};
use thiserror::Error;

/// Failures the executor reports while driving a processor pipeline.
///
/// The provider and processor failures keep their own error types rather than
/// being flattened into this one, so a caller can still tell a transient
/// storage failure apart from a stage failing to do its work.
///
/// Refs are rendered with `Debug` since this type isn't generic over the chain
/// spec.
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
    #[error("chain provider failed: {0}")]
    Provider(#[from] ProviderError),

    /// A processor stage failed while doing its work.  This never means the
    /// link was invalid, only that the stage couldn't process it.
    #[error("proc {0} failed: {1}")]
    Proc(ProcId, #[source] ProcError),

    /// A stage was handed an artifact that isn't of the type it produces.  This
    /// means artifacts got crossed between stages somewhere in the executor.
    #[error("artifact for proc {0} was not of that proc's artifact type")]
    ArtifactTypeMismatch(ProcId),

    /// The executor expected to have an artifact on hand for a stage but didn't.
    #[error("missing artifact from proc {proc_id} for link {link}")]
    MissingArtifact { link: String, proc_id: ProcId },

    /// A stage has no usable artifact for a committed link, because the
    /// stored one was produced by a different version of the stage or the
    /// stage was added after the link was committed, so the link can't be
    /// rolled back or committed again.
    #[error("no usable artifact from proc {proc_id} for committed link {link}")]
    StaleArtifact { link: String, proc_id: ProcId },

    /// Two stages were registered under the same ID, which would make a dep
    /// naming that ID ambiguous.
    #[error("duplicate proc {0} in pipeline")]
    DuplicateProc(ProcId),

    /// A stage declared a dep on an ID that no stage before it in the pipeline
    /// is registered under, so either the dep is unknown or it would run too
    /// late to be satisfied.
    #[error("proc {stage} depends on proc {dep}, which is not registered before it")]
    DepNotRegistered { stage: ProcId, dep: ProcId },

    /// A stage was about to run without a dep's artifacts in place.  The
    /// executor is what orders stages, so this is a bug in it.
    #[error("proc {stage} not ready: dep {dep} unmet")]
    UnmetDep { stage: ProcId, dep: ProcId },

    /// A link's origin node isn't reachable from the committed node through
    /// processed links, so there's no path to build its pre-state from.
    #[error("origin of link {0} is not reachable from the committed node")]
    OriginUnreachable(String),

    /// An operation named a link the executor hasn't processed.
    #[error("link {0} has not been processed")]
    LinkNotProcessed(String),

    /// An operation that only applies to uncommitted links named a committed
    /// one.
    #[error("link {0} is on the committed path")]
    LinkOnCommittedPath(String),

    /// An operation that walks the committed path named a node that isn't on
    /// it.
    #[error("node {0} is not on the committed path")]
    NodeNotOnCommittedPath(String),

    /// A stage's committed node isn't on the committed path, so the executor
    /// can't bring it level with the rest of the pipeline.
    #[error("proc {proc_id} committed at {node}, which is not on the committed path")]
    StageDiverged { proc_id: ProcId, node: String },

    /// The executor's own tracking or artifact storage failed.
    #[error("executor storage failure: {0}")]
    Storage(#[source] BoxedError),
}
