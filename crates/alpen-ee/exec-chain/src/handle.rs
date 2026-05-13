use std::sync::Arc;

use alpen_ee_common::{ConsensusHeads, ExecBlockRecord};
use strata_acct_types::Hash;
use strata_service::CommandHandle;

use crate::service::ExecChainMsg;

/// Handle for interacting with the execution chain tracker service.
///
/// Provides methods to query chain state and submit new blocks or consensus updates.
#[derive(Debug, Clone)]
pub struct ExecChainHandle {
    command_handle: Arc<CommandHandle<ExecChainMsg>>,
}

impl ExecChainHandle {
    /// Creates a new handle from a command handle.
    pub fn new(command_handle: CommandHandle<ExecChainMsg>) -> Self {
        Self {
            command_handle: Arc::new(command_handle),
        }
    }

    /// Fetch the best canonical exec block.
    pub async fn get_best_block(&self) -> eyre::Result<ExecBlockRecord> {
        Ok(self
            .command_handle
            .send_and_wait(ExecChainMsg::GetBestBlock)
            .await?)
    }

    /// Check if a block is on the canonical chain.
    ///
    /// Returns `true` if the block with the given hash lies on the path from
    /// the finalized block to the current best tip.
    pub async fn is_canonical(&self, hash: Hash) -> eyre::Result<bool> {
        Ok(self
            .command_handle
            .send_and_wait(|completion| ExecChainMsg::IsCanonical(hash, completion))
            .await?)
    }

    /// Get the block number of the current finalized block.
    pub async fn get_finalized_blocknum(&self) -> eyre::Result<u64> {
        Ok(self
            .command_handle
            .send_and_wait(ExecChainMsg::GetFinalizedBlocknum)
            .await?)
    }

    /// Submit new exec block to be tracked.
    pub async fn new_block(&self, hash: Hash) -> eyre::Result<()> {
        Ok(self
            .command_handle
            .send(ExecChainMsg::NewBlock(hash))
            .await?)
    }

    /// Submit new OL consensus state.
    pub async fn new_consensus_state(&self, consensus: ConsensusHeads) -> eyre::Result<()> {
        Ok(self
            .command_handle
            .send(ExecChainMsg::OLUpdate(consensus))
            .await?)
    }
}
