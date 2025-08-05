//! Command worker types.

use tokio::sync::{mpsc, oneshot};

use crate::*;

/// Stupid macro to improve readability because I don't want to wait on the
/// nightly feature to stabilize.
///
/// Another case where having intra-file C-style textual macros would be really
/// nice even in Rust.
macro_rules! msg {
    ($s:ty) => {
        <<$s>::Input as ServiceInput>::Msg
    };
}

/// Handle to send inputs to a command task.
#[derive(Debug)]
pub struct CommandHandle<S: Service> {
    tx: mpsc::Sender<<S::Input as ServiceInput>::Msg>,
}

impl<S: Service> CommandHandle<S> {
    /// Constructs a new instance.
    pub(crate) fn new(tx: mpsc::Sender<msg!(S)>) -> Self {
        Self { tx }
    }

    /// Returns the number of pending inputs that have not been processed yet as
    /// of the moment of calling.
    pub fn pending(&self) -> usize {
        self.tx.max_capacity() - self.tx.capacity()
    }

    /// Sends a message on the channel and returns immediately.
    pub async fn send(&self, m: msg!(S)) -> anyhow::Result<()> {
        if self.tx.send(m).await.is_err() {
            // FIXME typed errors
            anyhow::bail!("service/cmd: worker exited")
        }

        Ok(())
    }

    /// Sends a message on the channel and returns immediately.
    pub fn send_blocking(&self, m: msg!(S)) -> anyhow::Result<()> {
        if self.tx.blocking_send(m).is_err() {
            // FIXME typed errors
            anyhow::bail!("service/cmd: worker exited");
        }

        Ok(())
    }

    /// Accepts a message constructor accepting a callback sender, sends the messagee, and then
    /// waits for a response.
    pub async fn send_and_wait<R>(
        &self,
        mfn: impl Fn(oneshot::Sender<R>) -> msg!(S),
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
        mfn: impl Fn(oneshot::Sender<R>) -> msg!(S),
    ) -> anyhow::Result<R> {
        let (ret_tx, ret_rx) = oneshot::channel();
        let m = mfn(ret_tx);

        self.send_blocking(m)?;
        coerce_callback_result(ret_rx.blocking_recv())
    }
}

fn coerce_callback_result<R>(v: Result<R, oneshot::error::RecvError>) -> anyhow::Result<R> {
    // FIXME typed errors
    v.map_err(|_| anyhow::format_err!("service/cmd: worked exited before receiving response"))
}
