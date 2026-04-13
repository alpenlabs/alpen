use std::sync::Arc;

use async_trait::async_trait;
use strata_asm_worker::AsmWorkerHandle;
use strata_primitives::L1BlockCommitment;
use strata_state::BlockSubmitter;

/// Adapter for using [`AsmWorkerHandle`] as a [`BlockSubmitter`].
#[derive(Clone, Debug)]
pub struct AsmBlockSubmitter {
    handle: Arc<AsmWorkerHandle>,
}

impl AsmBlockSubmitter {
    pub fn new(handle: Arc<AsmWorkerHandle>) -> Self {
        Self { handle }
    }
}

#[async_trait]
impl BlockSubmitter for AsmBlockSubmitter {
    fn submit_block(&self, block: L1BlockCommitment) -> anyhow::Result<()> {
        self.handle.submit_block(block)
    }

    async fn submit_block_async(&self, block: L1BlockCommitment) -> anyhow::Result<()> {
        self.handle.submit_block_async(block).await
    }
}
