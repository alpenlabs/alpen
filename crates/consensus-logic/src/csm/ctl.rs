use async_trait::async_trait;
use strata_primitives::l1::L1BlockCommitment;
use strata_state::BlockSubmitter;
use tokio::sync::mpsc;
use tracing::*;

/// Controller handle for the consensus state machine.
/// Used to submit new l1 blocks for processing.
#[expect(missing_debug_implementations)]
pub struct CsmController {
    csm_tx: mpsc::Sender<L1BlockCommitment>,
}

impl CsmController {
    pub fn new(csm_tx: mpsc::Sender<L1BlockCommitment>) -> Self {
        Self { csm_tx }
    }
}

#[async_trait]
impl BlockSubmitter for CsmController {
    /// Sends a new l1 block to the csm machinery.
    fn submit_block(&self, block: L1BlockCommitment) -> anyhow::Result<()> {
        trace!(%block, "submitting l1 block");
        if self.csm_tx.blocking_send(block).is_err() {
            warn!(%block, "block consumer receiver closed when submitting");
        } else {
            trace!(%block, "sent new block");
        }

        Ok(())
    }

    /// Sends a new l1 block to the csm machinery.
    async fn submit_block_async(&self, block: L1BlockCommitment) -> anyhow::Result<()> {
        trace!(%block, "submitting l1 block");
        if self.csm_tx.send(block).await.is_err() {
            warn!(%block, "block consumer closed when submitting");
        } else {
            trace!(%block, "sent new block");
        }

        Ok(())
    }
}
