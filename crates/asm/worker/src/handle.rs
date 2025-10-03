use async_trait::async_trait;
use strata_primitives::prelude::*;
use strata_service::{CommandHandle, ServiceMonitor};
use strata_state::BlockSubmitter;
use tracing::warn;

use crate::AsmWorkerStatus;

/// Handle for interacting with the ASM worker service.
#[derive(Debug)]
pub struct AsmWorkerHandle {
    command_handle: CommandHandle<L1BlockCommitment>,
    service_monitor: ServiceMonitor<AsmWorkerStatus>,
}

impl AsmWorkerHandle {
    /// Create a new ASM worker handle from a service command handle.
    pub fn new(
        command_handle: CommandHandle<L1BlockCommitment>,
        monitor: ServiceMonitor<AsmWorkerStatus>,
    ) -> Self {
        Self {
            command_handle,
            service_monitor: monitor,
        }
    }

    /// Allows other services to listen to status updates.
    ///
    /// Can be useful for logic that want to listen to logs/updates of ASM state.
    pub fn get_monitor(&self) -> &ServiceMonitor<AsmWorkerStatus> {
        &self.service_monitor
    }
}

#[async_trait]
impl BlockSubmitter for AsmWorkerHandle {
    /// Sends a new l1 block to the ASM service.
    fn submit_block(&self, block: L1BlockCommitment) -> anyhow::Result<()> {
        if self.command_handle.send_blocking(block).is_err() {
            warn!(%block, "ASM handle closed when submitting");
        }

        Ok(())
    }

    /// Sends a new l1 block to the ASM service.
    async fn submit_block_async(&self, block: L1BlockCommitment) -> anyhow::Result<()> {
        if self.command_handle.send(block).await.is_err() {
            warn!(%block, "ASM handle closed when submitting");
        }

        Ok(())
    }
}
