//! Error types.
//!
//! Variants are typed (no erased `anyhow::Error` wrapped inside the enum).
//! Wrapping `anyhow` inside a `thiserror` enum muddies downcast behavior
//! for callers — keep the library boundary typed and let applications
//! decide whether to erase upstream.

use zkaleido::ZkVmError;

use crate::classify::classify_zkvm_error;

pub type ProverResult<T> = Result<T, ProverError>;

/// What the task layer should do with a failed proving attempt.
///
/// The decision is a property of the *error*, classified once in the crate's
/// `classify` module, not of the call-site that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureAction {
    /// Retry, resuming any saved remote state (poll the same `ProofId`).
    RetryResume,
    /// Retry, but discard saved remote state first so the next attempt submits
    /// a fresh request (e.g. the prior request expired or hit no capacity).
    RetryFresh,
    /// Terminal — do not retry.
    Permanent,
}

#[derive(Debug, thiserror::Error)]
pub enum ProverError {
    #[error("task not found: {0}")]
    TaskNotFound(String),

    #[error("task already exists: {0}")]
    TaskAlreadyExists(String),

    #[error("no receipt store configured")]
    NoReceiptStore,

    /// A proving-pipeline failure carrying its classified retry decision.
    ///
    /// Constructed via [`ProverError::transient`] / [`ProverError::resubmit`] /
    /// [`ProverError::permanent`] (for consumer-side input errors) or
    /// `ProverError::from_zkvm` (for upstream zkaleido errors).
    #[error("{msg}")]
    Failed { action: FailureAction, msg: String },

    /// Backend IO failure (sled, filesystem, tokio runtime, ...).
    #[error("storage: {0}")]
    Storage(String),

    /// Encode or decode of a stored record failed.
    #[error("codec: {0}")]
    Codec(String),

    /// Command channel failure (send/recv/cancelled).
    #[error("command channel: {0}")]
    Command(String),
}

impl ProverError {
    /// A transient failure: retry, resuming any saved remote state.
    pub fn transient(msg: impl Into<String>) -> Self {
        Self::Failed {
            action: FailureAction::RetryResume,
            msg: msg.into(),
        }
    }

    /// A resubmit failure: retry after discarding saved remote state.
    pub fn resubmit(msg: impl Into<String>) -> Self {
        Self::Failed {
            action: FailureAction::RetryFresh,
            msg: msg.into(),
        }
    }

    /// A permanent failure: terminal, never retried.
    pub fn permanent(msg: impl Into<String>) -> Self {
        Self::Failed {
            action: FailureAction::Permanent,
            msg: msg.into(),
        }
    }

    /// Build a failure from an upstream [`ZkVmError`], classifying the retry
    /// decision from the error's typed variant.
    pub(crate) fn from_zkvm(err: ZkVmError) -> Self {
        Self::Failed {
            action: classify_zkvm_error(&err),
            msg: err.to_string(),
        }
    }

    /// The retry decision for this error.
    ///
    /// `Failed` carries its own classified action. Infra errors
    /// (storage/codec/command) are transient — a backend hiccup, not a fatal
    /// proving fault. API errors are terminal (they should not reach the retry
    /// path, but default conservatively).
    pub fn action(&self) -> FailureAction {
        match self {
            Self::Failed { action, .. } => *action,
            Self::Storage(_) | Self::Codec(_) | Self::Command(_) => FailureAction::RetryResume,
            Self::TaskNotFound(_) | Self::TaskAlreadyExists(_) | Self::NoReceiptStore => {
                FailureAction::Permanent
            }
        }
    }
}
