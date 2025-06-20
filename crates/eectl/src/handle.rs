//! Exec engine controller handle types.

use std::sync::Arc;

use strata_primitives::prelude::*;
use tokio::sync::{mpsc, oneshot};

use crate::{errors::EngineResult, messages::TipState};

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
    // TODO add fns for sending messages
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
