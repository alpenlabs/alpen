//! Registry-based Prover implementation

use std::sync::Arc;

use crate::error::ProverServiceResult;
use crate::task_id::TaskId;
use crate::ZkVmBackend;
use crate::{Prover, ProgramType};

use super::core::ProgramRegistry;

/// Prover that uses the program registry for dynamic dispatch
pub struct RegistryProver<P: ProgramType> {
    registry: Arc<ProgramRegistry<P>>,
}

impl<P: ProgramType> RegistryProver<P> {
    /// Create a new registry prover
    pub fn new(registry: Arc<ProgramRegistry<P>>) -> Self {
        Self { registry }
    }

    /// Get a reference to the registry
    pub fn registry(&self) -> &Arc<ProgramRegistry<P>> {
        &self.registry
    }
}

impl<P: ProgramType> Prover for RegistryProver<P> {
    type TaskId = TaskId<P>;
    type Backend = ZkVmBackend;

    fn backend(&self, task_id: &Self::TaskId) -> Self::Backend {
        task_id.backend.clone()
    }

    async fn prove(&self, task_id: Self::TaskId) -> ProverServiceResult<()> {
        // Fetch input using registry
        let input = self.registry.fetch_input(&task_id.program).await?;

        // Prove using registry
        let proof = self
            .registry
            .prove(&task_id.program, input, &task_id.backend)
            .await?;

        // Store proof using registry
        self.registry.store_proof(&task_id.program, proof).await?;

        Ok(())
    }
}
