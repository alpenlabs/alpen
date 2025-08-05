use std::any::Any;

use thiserror::Error;

/// Errors originating in the service framework.
#[derive(Debug, Error)]
pub enum ServiceError {
    /// We cancelled the wait for input.
    #[error("wait for input cancelled")]
    WaitCancelled,

    #[error("panic in blocking thread (info: {0})")]
    BlockingThreadPanic(Option<String>),

    #[error("unknown error waiting for input")]
    UnknownInputErr,

    #[error("command worker exited")]
    WorkerExited,

    #[error("command worker exited without us reciving response")]
    WorkerExitedWithoutResponse,
}
