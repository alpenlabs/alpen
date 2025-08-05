//! Command worker types.

use tokio::sync::{mpsc, oneshot};

use crate::*;

/// Handle to send inputs to a command task.
#[derive(Debug)]
pub struct CommandHandle<S: Service> {
    tx: mpsc::Sender<S::Msg>,
}

impl<S: Service> CommandHandle<S> {
    /// Constructs a new instance.
    pub(crate) fn new(tx: mpsc::Sender<S::Msg>) -> Self {
        Self { tx }
    }

    /// Returns the number of pending inputs that have not been processed yet as
    /// of the moment of calling.
    pub fn pending(&self) -> usize {
        self.tx.max_capacity() - self.tx.capacity()
    }

    /// Sends a message on the channel and returns immediately.
    pub async fn send(&self, m: S::Msg) -> anyhow::Result<()> {
        if self.tx.send(m).await.is_err() {
            return Err(ServiceError::WorkerExited.into());
        }

        Ok(())
    }

    /// Sends a message on the channel and returns immediately.
    pub fn send_blocking(&self, m: S::Msg) -> anyhow::Result<()> {
        if self.tx.blocking_send(m).is_err() {
            return Err(ServiceError::WorkerExited.into());
        }

        Ok(())
    }

    /// Accepts a message constructor accepting a callback sender, sends the messagee, and then
    /// waits for a response.
    pub async fn send_and_wait<R>(
        &self,
        mfn: impl Fn(oneshot::Sender<R>) -> S::Msg,
    ) -> anyhow::Result<R> {
        let (ret_tx, ret_rx) = oneshot::channel();
        let m = mfn(ret_tx);

        self.send(m).await?;
        coerce_callback_result(ret_rx.await)
    }

    /// Accepts a message constructor accepting a callback sender, sends the messagee, and then
    /// waits for a response.
    pub fn send_and_wait_blocking<R>(
        &self,
        mfn: impl Fn(oneshot::Sender<R>) -> S::Msg,
    ) -> anyhow::Result<R> {
        let (ret_tx, ret_rx) = oneshot::channel();
        let m = mfn(ret_tx);

        self.send_blocking(m)?;
        coerce_callback_result(ret_rx.blocking_recv())
    }
}

fn coerce_callback_result<R>(v: Result<R, oneshot::error::RecvError>) -> anyhow::Result<R> {
    v.map_err(|_| ServiceError::WorkerExitedWithoutResponse.into())
}
