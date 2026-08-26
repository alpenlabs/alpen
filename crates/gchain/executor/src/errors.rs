use strata_gchain_types::ProcId;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GExecError {
    // TODO add link ref somehow
    #[error("missing link")]
    MissingLink,

    /// A stage was handed an artifact that isn't of the type it produces.  This
    /// means artifacts got crossed between stages somewhere in the executor.
    #[error("artifact for proc {0} was not of that proc's artifact type")]
    ArtifactTypeMismatch(ProcId),

    /// The executor expected to have an artifact on hand for a stage but didn't.
    #[error("missing artifact for proc {0}")]
    MissingArtifact(ProcId),
}
