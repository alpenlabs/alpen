//! Utilities relating to "command worker" services.

use std::fmt;

use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::warn;

use crate::ServiceError;

/// Handle to send inputs to a command worker service.
///
/// This is essentially just a wrapper over a MPSC sender, but with some
/// convenience functions for common patterns.  It's expected that an instance
/// of this type will be used inside of a handle type for the particular
/// service.
#[derive(Debug)]
pub struct CommandHandle<M> {
    tx: mpsc::Sender<M>,
}

impl<M> CommandHandle<M> {
    /// Constructs a new instance.
    pub(crate) fn new(tx: mpsc::Sender<M>) -> Self {
        Self { tx }
    }

    /// Returns the number of pending inputs that have not been processed yet as
    /// of the moment of calling.
    pub fn pending(&self) -> usize {
        self.tx.max_capacity() - self.tx.capacity()
    }

    /// Sends a message on the channel and returns immediately.
    pub async fn send(&self, m: M) -> Result<(), ServiceError> {
        if self.tx.send(m).await.is_err() {
            return Err(ServiceError::WorkerExited);
        }

        Ok(())
    }

    /// Sends a message on the channel and returns immediately.
    pub fn send_blocking(&self, m: M) -> Result<(), ServiceError> {
        if self.tx.blocking_send(m).is_err() {
            return Err(ServiceError::WorkerExited);
        }

        Ok(())
    }

    /// Accepts a message constructor accepting a callback sender, sends the messagee, and then
    /// waits for a response.
    pub async fn send_and_wait<R>(
        &self,
        mfn: impl Fn(CommandCompletionSender<R>) -> M,
    ) -> Result<R, ServiceError> {
        let (ret_tx, ret_rx) = oneshot::channel();
        let completion = CommandCompletionSender::new(ret_tx);
        let m = mfn(completion);

        self.send(m).await?;
        coerce_callback_result(ret_rx.await)
    }

    /// Accepts a message constructor accepting a callback sender, sends the messagee, and then
    /// waits for a response.
    pub fn send_and_wait_blocking<R>(
        &self,
        mfn: impl Fn(CommandCompletionSender<R>) -> M,
    ) -> Result<R, ServiceError> {
        let (ret_tx, ret_rx) = oneshot::channel();
        let completion = CommandCompletionSender::new(ret_tx);
        let m = mfn(completion);

        self.send_blocking(m)?;
        coerce_callback_result(ret_rx.blocking_recv())
    }
}

fn coerce_callback_result<R>(v: Result<R, oneshot::error::RecvError>) -> Result<R, ServiceError> {
    v.map_err(|_| ServiceError::WorkerExitedWithoutResponse)
}

/// A wrapper around a [`oneshot::Sender`] to allow it to be shared but only
/// completed once.
pub struct CommandCompletionSender<T> {
    sender: Mutex<Option<oneshot::Sender<T>>>,
}

impl<T> CommandCompletionSender<T> {
    /// Creates a new instance.
    pub fn new(sender: oneshot::Sender<T>) -> Self {
        Self {
            sender: Mutex::new(Some(sender)),
        }
    }

    /// Send the response.
    ///
    /// Logs a warning if the sender has already been consumed.
    pub async fn send(&self, value: T) {
        match self.sender.lock().await.take() {
            Some(sender) => {
                let _ = sender.send(value);
            }
            None => {
                warn!("attempted to send response for already completed command");
            }
        }
    }

    /// Send the response.
    ///
    /// Logs a warning if the sender has already been consumed.
    pub fn send_blocking(&self, value: T) {
        match self.sender.blocking_lock().take() {
            Some(sender) => {
                let _ = sender.send(value);
            }
            None => {
                warn!("attempted to send response for already completed command");
            }
        }
    }
}

impl<T> fmt::Debug for CommandCompletionSender<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<completion>")
    }
}
