//! Messages from the handle to the worker.

use strata_identifiers::OLBlockCommitment;
use strata_primitives::epoch::EpochCommitment;
use strata_service::CommandCompletionSender;

use crate::WorkerResult;

/// Messages from the handle to the worker to give it work to do, with a
/// completion sender to return a result.
#[derive(Debug)]
pub enum ChainWorkerMessage {
    /// Try to execute a block at the given commitment.
    TryExecBlock(OLBlockCommitment, CommandCompletionSender<WorkerResult<()>>),

    /// Finalize an epoch, updating database state accordingly.
    FinalizeEpoch(EpochCommitment, CommandCompletionSender<WorkerResult<()>>),

    /// Update the safe tip.
    UpdateSafeTip(OLBlockCommitment, CommandCompletionSender<WorkerResult<()>>),
}
