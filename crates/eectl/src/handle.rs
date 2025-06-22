//! Exec engine controller handle types.

use std::sync::Arc;

use strata_primitives::prelude::*;
use tokio::sync::{mpsc, oneshot};
use tracing::debug;

use crate::{
    errors::{EngineError, EngineResult},
    messages::TipState,
};

/// Commands we send from the handle to the worker, with completion channels.
#[derive(Debug)]
pub enum ExecCommand {
    /// Updates the safe and finalized tips.
    NewTipState(TipState, oneshot::Sender<EngineResult<()>>),

    /// Notifies the worker of a new block being produced.
    NewBlock(L2BlockCommitment, oneshot::Sender<EngineResult<()>>),
}

#[derive(Debug)]
pub struct ExecCtlHandle {
    shared: Arc<ExecShared>,
    msg_tx: mpsc::Sender<ExecCommand>,
}

impl ExecCtlHandle {
    async fn send_and_wait<R>(
        &self,
        make_fn: impl FnOnce(oneshot::Sender<EngineResult<R>>) -> ExecCommand,
    ) -> EngineResult<R> {
        // Construct the message with the lambda.
        let (completion_tx, completion_rx) = oneshot::channel();
        let msg = make_fn(completion_tx);

        // Then send it and wait for a response.
        if self.msg_tx.send(msg).await.is_err() {
            return Err(EngineError::WorkerExited);
        }

        match completion_rx.await {
            Ok(r) => r,
            Err(_) => Err(EngineError::WorkerExited),
        }
    }

    fn send_and_wait_blocking<R>(
        &self,
        make_fn: impl FnOnce(oneshot::Sender<EngineResult<R>>) -> ExecCommand,
    ) -> EngineResult<R> {
        // Construct the message with the lambda.
        let (completion_tx, completion_rx) = oneshot::channel();
        let msg = make_fn(completion_tx);

        // Then send it and wait for a response.
        if self.msg_tx.blocking_send(msg).is_err() {
            return Err(EngineError::WorkerExited);
        }

        match completion_rx.blocking_recv() {
            Ok(r) => r,
            Err(_) => Err(EngineError::WorkerExited),
        }
    }

    pub async fn try_exec_el_payload(&self, block: L2BlockCommitment) -> EngineResult<()> {
        self.send_and_wait(|tx| ExecCommand::NewBlock(block, tx))
            .await
    }

    pub fn try_exec_el_payload_blocking(&self, block: L2BlockCommitment) -> EngineResult<()> {
        debug!(?block, "trying to execute EL payload block");
        self.send_and_wait_blocking(|tx| ExecCommand::NewBlock(block, tx))
    }

    pub async fn update_tip_state(&self, tip_state: TipState) -> EngineResult<()> {
        self.send_and_wait(|tx| ExecCommand::NewTipState(tip_state, tx))
            .await
    }

    pub fn update_tip_state_blocking(&self, tip_state: TipState) -> EngineResult<()> {
        self.send_and_wait_blocking(|tx| ExecCommand::NewTipState(tip_state, tx))
    }
}

#[derive(Debug)]
pub struct ExecCtlInput {
    shared: Arc<ExecShared>,
    msg_rx: mpsc::Receiver<ExecCommand>,
}

impl ExecCtlInput {
    pub fn recv_msg(&mut self) -> Option<ExecCommand> {
        self.msg_rx.blocking_recv()
    }
}

/// State shared between the handle and the worker.
#[derive(Debug)]
pub struct ExecShared {
    // TODO
}

/// Make a pair of the handle and the input that can be used while constructing the worker.
pub fn make_handle_pair() -> (ExecCtlHandle, ExecCtlInput) {
    let (tx, rx) = mpsc::channel(8);
    let shared = Arc::new(ExecShared {});

    let handle = ExecCtlHandle {
        shared: shared.clone(),
        msg_tx: tx,
    };

    let input = ExecCtlInput { shared, msg_rx: rx };

    (handle, input)
}
