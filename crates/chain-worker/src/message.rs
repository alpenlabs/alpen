//! Messages from the handle to the worker.

use strata_primitives::prelude::*;
use strata_service::CommandCompletionSender;

use crate::WorkerResult;

/// Messages from the handle to the worker to give it work to do, with a
/// completion to return a result.
#[derive(Debug)]
pub enum ChainWorkerMessage {
    TryExecBlock(L2BlockCommitment, CommandCompletionSender<WorkerResult<()>>),
    FinalizeEpoch(EpochCommitment, CommandCompletionSender<WorkerResult<()>>),
    UpdateSafeTip(L2BlockCommitment, CommandCompletionSender<WorkerResult<()>>),
}
