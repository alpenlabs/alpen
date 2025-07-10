use std::sync::Arc;

use strata_primitives::prelude::*;
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::{WorkerError, WorkerResult, message::ChainWorkerMessage};

#[derive(Debug)]
#[allow(unused)]
pub struct ChainWorkerHandle {
    shared: Arc<Mutex<WorkerShared>>,
    msg_tx: mpsc::Sender<ChainWorkerMessage>,
}

impl ChainWorkerHandle {
    pub fn new(shared: Arc<Mutex<WorkerShared>>, msg_tx: mpsc::Sender<ChainWorkerMessage>) -> Self {
        Self { shared, msg_tx }
    }

    /// Low-level caller to dispatch work to the worker thread.
    async fn send_and_wait<R>(
        &self,
        make_fn: impl FnOnce(oneshot::Sender<WorkerResult<R>>) -> ChainWorkerMessage,
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
        make_fn: impl FnOnce(oneshot::Sender<WorkerResult<R>>) -> ChainWorkerMessage,
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
    pub async fn try_exec_block(&self, block: L2BlockCommitment) -> WorkerResult<()> {
        self.send_and_wait(|tx| ChainWorkerMessage::TryExecBlock(block, tx))
            .await
    }

    /// Tries to execute a block, returns the result.
    pub fn try_exec_block_blocking(&self, block: L2BlockCommitment) -> WorkerResult<()> {
        self.send_and_wait_blocking(|tx| ChainWorkerMessage::TryExecBlock(block, tx))
    }

    /// Finalize an epoch, making whatever database changes necessary.
    pub async fn finalize_epoch(&self, epoch: EpochCommitment) -> WorkerResult<()> {
        self.send_and_wait(|tx| ChainWorkerMessage::FinalizeEpoch(epoch, tx))
            .await
    }

    /// Finalize an epoch, making whatever database changes necessary.
    pub fn finalize_epoch_blocking(&self, epoch: EpochCommitment) -> WorkerResult<()> {
        self.send_and_wait_blocking(|tx| ChainWorkerMessage::FinalizeEpoch(epoch, tx))
    }

    /// Finalize an epoch, making whatever database changes necessary.
    pub async fn update_safe_tip(&self, safe_tip: L2BlockCommitment) -> WorkerResult<()> {
        self.send_and_wait(|tx| ChainWorkerMessage::UpdateSafeTip(safe_tip, tx))
            .await
    }

    /// Finalize an epoch, making whatever database changes necessary.
    pub fn update_safe_tip_blocking(&self, safe_tip: L2BlockCommitment) -> WorkerResult<()> {
        self.send_and_wait_blocking(|tx| ChainWorkerMessage::UpdateSafeTip(safe_tip, tx))
    }
}

/// Input to the worker, reading inputs from the worker handle.
#[derive(Debug)]
pub struct ChainWorkerInput {
    shared: Arc<Mutex<WorkerShared>>,
    msg_rx: mpsc::Receiver<ChainWorkerMessage>,
}

impl ChainWorkerInput {
    pub fn new(
        shared: Arc<Mutex<WorkerShared>>,
        msg_rx: mpsc::Receiver<ChainWorkerMessage>,
    ) -> Self {
        Self { shared, msg_rx }
    }

    pub fn shared(&self) -> &Mutex<WorkerShared> {
        &self.shared
    }

    pub(crate) fn recv_next(&mut self) -> Option<ChainWorkerMessage> {
        self.msg_rx.blocking_recv()
    }
}

/// Shared state between the worker and the handle.
#[derive(Debug, Clone, Default)]
pub struct WorkerShared {
    // TODO
}
