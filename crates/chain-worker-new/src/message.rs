//! Messages from the handle to the worker.

use strata_identifiers::OLBlockCommitment;
use strata_ol_chain_types_new::OLL1ManifestContainer;
use strata_ol_da::OLDaPayloadV1;
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

    /// Apply the DA
    ApplyDA(ApplyDAPayload, CommandCompletionSender<WorkerResult<()>>),
}

/// CSM message payload for applying DA.
#[derive(Clone, Debug)]
pub struct ApplyDAPayload {
    da: OLDaPayloadV1,
    manifests: OLL1ManifestContainer,
    epoch: EpochCommitment,
}

impl ApplyDAPayload {
    pub fn new(
        da: OLDaPayloadV1,
        manifests: OLL1ManifestContainer,
        epoch: EpochCommitment,
    ) -> Self {
        Self {
            da,
            manifests,
            epoch,
        }
    }
}
