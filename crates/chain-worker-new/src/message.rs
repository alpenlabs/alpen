//! Messages from the handle to the worker.

use strata_checkpoint_types_ssz::TerminalHeaderComplement;
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

    /// Apply the Finalized DA
    ApplyFinalizedCkpt(
        FinalizedCkptPayload,
        CommandCompletionSender<WorkerResult<()>>,
    ),
}

/// Chain worker message payload for applying finalized checkpoint.
#[derive(Clone, Debug)]
pub struct FinalizedCkptPayload {
    pub(crate) da_payload: OLDaPayloadV1,
    pub(crate) manifests: OLL1ManifestContainer,
    pub(crate) epoch: EpochCommitment,
    pub(crate) terminal_header_complement: TerminalHeaderComplement,
}

impl FinalizedCkptPayload {
    pub fn new(
        da_payload: OLDaPayloadV1,
        manifests: OLL1ManifestContainer,
        epoch: EpochCommitment,
        terminal_header_complement: TerminalHeaderComplement,
    ) -> Self {
        Self {
            da_payload,
            manifests,
            epoch,
            terminal_header_complement,
        }
    }

    pub fn da_payload(&self) -> &OLDaPayloadV1 {
        &self.da_payload
    }

    pub fn manifests(&self) -> &OLL1ManifestContainer {
        &self.manifests
    }

    pub fn epoch(&self) -> EpochCommitment {
        self.epoch
    }
}
