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

    /// Notifies the chain worker that FCM accepted a block.
    BlockAccepted(OLBlockCommitment, CommandCompletionSender<WorkerResult<()>>),

    /// Notifies the chain worker that FCM rejected a block.
    BlockRejected(OLBlockCommitment, CommandCompletionSender<WorkerResult<()>>),

    /// Finalize an epoch, updating database state accordingly.
    FinalizeEpoch(EpochCommitment, CommandCompletionSender<WorkerResult<()>>),

    /// Update the safe tip.
    UpdateSafeTip(OLBlockCommitment, CommandCompletionSender<WorkerResult<()>>),

    /// Reconstruct an epoch's OL state from its checkpoint and persist it.
    ///
    /// Used by checkpoint sync. Reconstructs, persists, and records the epoch summary.
    ApplyCheckpoint {
        epoch: EpochCommitment,
        previous_epoch: EpochCommitment,
        completion: CommandCompletionSender<WorkerResult<()>>,
    },
}
