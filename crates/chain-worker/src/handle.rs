use std::sync::Arc;

use strata_primitives::prelude::*;
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::WorkerError;

pub struct ChainWorkerHandle {
    shared: Arc<Mutex<WorkerShared>>,
    msg_tx: mpsc::Sender<WorkerMessage>,
}

impl ChainWorkerHandle {
    // TODO
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

/// Messages from the handle to the worker to give it work to do.
pub enum WorkerMessage {
    /// Try to execute a block.
    TryExecBlock(L2BlockCommitment, oneshot::Sender<WorkerResult<()>>),
}

/// Shared state between the worker and the handle.
pub struct WorkerShared {
    // TODO
}
