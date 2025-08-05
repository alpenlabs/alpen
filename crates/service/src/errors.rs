use thiserror::Error;

/// Errors originating in the service framework.
#[derive(Debug, Error)]
pub enum ServiceError {
    /// We cancelled the wait for input, somehow.
    #[error("wait for input cancelled")]
    WaitCancelled,

    /// Blocking thread panic.
    #[error("panic in blocking thread (info: {0})")]
    BlockingThreadPanic(String),

    /// Some other unknown error while accepting input.
    #[error("unknown error waiting for input")]
    UnknownInputErr,

    /// For when the worker task has exited when we try to send a message.
    #[error("command worker exited")]
    WorkerExited,

    /// For when we send a message but then the worker task exits before it
    /// handles it.
    #[error("command worker exited without us receiving response")]
    WorkerExitedWithoutResponse,
}
