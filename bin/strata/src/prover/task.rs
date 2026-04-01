//! Task type for checkpoint proof generation.

use serde::{Deserialize, Serialize};
use strata_identifiers::EpochCommitment;
use strata_paas::{ProgramType, ZkVmBackend};
use strata_primitives::proof::ProofZkVm;

use super::errors::ProverError;

/// Routing key required by the paas framework.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CheckpointVariant {
    Checkpoint,
}

/// Checkpoint proof generation task.
///
/// Identified by [`EpochCommitment`] (epoch index + terminal block) rather
/// than a bare epoch index, so that reorgs naturally invalidate stale tasks.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct CheckpointTask {
    pub commitment: EpochCommitment,
    pub backend: ZkVmBackend,
}

impl CheckpointTask {
    pub(crate) fn new(commitment: EpochCommitment, backend: ZkVmBackend) -> Self {
        Self {
            commitment,
            backend,
        }
    }

    /// Maps the task's [`ZkVmBackend`] to the corresponding [`ProofZkVm`].
    pub(crate) fn proof_zkvm(&self) -> Result<ProofZkVm, ProverError> {
        match &self.backend {
            ZkVmBackend::SP1 => Ok(ProofZkVm::SP1),
            ZkVmBackend::Native => Ok(ProofZkVm::Native),
            other => Err(ProverError::UnsupportedBackend(format!("{other:?}"))),
        }
    }
}

impl ProgramType for CheckpointTask {
    type RoutingKey = CheckpointVariant;

    fn routing_key(&self) -> Self::RoutingKey {
        CheckpointVariant::Checkpoint
    }
}
