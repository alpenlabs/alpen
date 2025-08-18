//! Messages from the handle to the worker.

use strata_primitives::prelude::*;
use tokio::sync::{oneshot, Mutex};

use crate::WorkerResult;

/// Messages from the handle to the worker to give it work to do, with a
/// completion to return a result.
#[derive(Debug)]
pub enum ChainWorkerMessage {
    TryExecBlock(L2BlockCommitment, Mutex<Option<oneshot::Sender<WorkerResult<()>>>>),
    FinalizeEpoch(EpochCommitment, Mutex<Option<oneshot::Sender<WorkerResult<()>>>>),
    UpdateSafeTip(L2BlockCommitment, Mutex<Option<oneshot::Sender<WorkerResult<()>>>>),
}
