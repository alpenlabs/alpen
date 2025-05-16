use std::sync::Arc;

use strata_primitives::prelude::*;
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::{WorkerError, WorkerResult};

pub struct ChainWorkerHandle {
    shared: Arc<Mutex<WorkerShared>>,
    msg_tx: mpsc::Sender<WorkerMessage>,
}

impl ChainWorkerHandle {
    fn new(shared: Arc<Mutex<WorkerShared>>, msg_tx: mpsc::Sender<WorkerMessage>) -> Self {
        Self { shared, msg_tx }
    }

    /// Low-level caller to dispatch work to the worker thread.
    async fn send_and_wait<R>(
        &self,
        make_fn: impl FnOnce(oneshot::Sender<WorkerResult<R>>) -> WorkerMessage,
    ) -> WorkerResult<R> {
        // Construct the message with the lambda.
        let (completion_tx, completion_rx) = oneshot::channel();
        let msg = make_fn(completion_tx);

        // Then send it and wait for a response.
        if self.msg_tx.send(msg).await.is_err() {
            return Err(WorkerError::WorkerExited);
        }

        match completion_rx.await {
            Ok(r) => r,
            Err(_) => Err(WorkerError::WorkerExited),
        }
    }

    /// Low-level caller to dispatch work to the worker thread.
    fn send_and_wait_blocking<R>(
        &self,
        make_fn: impl FnOnce(oneshot::Sender<WorkerResult<R>>) -> WorkerMessage,
    ) -> WorkerResult<R> {
        // Construct the message with the lambda.
        let (completion_tx, completion_rx) = oneshot::channel();
        let msg = make_fn(completion_tx);

        if self.msg_tx.blocking_send(msg).is_err() {
            return Err(WorkerError::WorkerExited);
        }

        match completion_rx.blocking_recv() {
            Ok(r) => r,
            Err(_) => Err(WorkerError::WorkerExited),
        }
    }

    /// Tries to execute a block, returns the result.
    async fn try_exec_block(&self, block: L2BlockCommitment) -> WorkerResult<()> {
        self.send_and_wait(|tx| WorkerMessage::TryExecBlock(block, tx))
            .await
    }

    /// Tries to execute a block, returns the result.
    fn try_exec_block_blocking(&self, block: L2BlockCommitment) -> WorkerResult<()> {
        self.send_and_wait_blocking(|tx| WorkerMessage::TryExecBlock(block, tx))
    }
}

/// Messages from the handle to the worker to give it work to do.
pub enum WorkerMessage {
    /// Try to execute a block.
    TryExecBlock(L2BlockCommitment, oneshot::Sender<WorkerResult<()>>),
}

/// Input to the worker, reading inputs from the worker handle.
pub struct ChainWorkerInput {
    shared: Arc<Mutex<WorkerShared>>,
    msg_rx: mpsc::Receiver<WorkerMessage>,
}

impl ChainWorkerInput {
    pub fn recv_next(&mut self) -> Option<WorkerMessage> {
        self.msg_rx.blocking_recv()
    }
}

/// Shared state between the worker and the handle.
pub struct WorkerShared {
    // TODO
}
