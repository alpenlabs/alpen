use async_trait::async_trait;
use strata_primitives::prelude::*;
use strata_service::CommandHandle;
use strata_state::BlockSubmitter;
use tracing::warn;

/// Handle for interacting with the ASM worker service.
#[derive(Debug)]
pub struct AsmWorkerHandle {
    command_handle: CommandHandle<L1BlockCommitment>,
}

impl AsmWorkerHandle {
    /// Create a new ASM worker handle from a service command handle.
    pub fn new(command_handle: CommandHandle<L1BlockCommitment>) -> Self {
        Self { command_handle }
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
