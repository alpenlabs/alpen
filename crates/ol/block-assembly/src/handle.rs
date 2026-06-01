//! OL block assembly service handle for external interaction.

use std::sync::Arc;

use strata_identifiers::{OLBlockCommitment, OLBlockId};
use strata_ol_chain_types_new::OLBlock;
use strata_service::{CommandHandle, ServiceMonitor};
use tokio::sync::oneshot;

use crate::{
    BlockAssemblyResult,
    command::{BlockasmCommand, create_completion},
    error::BlockAssemblyError,
    service::BlockasmServiceStatus,
    types::{BlockCompletionData, BlockGenerationConfig, FullBlockTemplate},
};

/// Handle for interacting with the OL block assembly service.
#[derive(Debug)]
pub struct BlockasmHandle {
    command_handle: Arc<CommandHandle<BlockasmCommand>>,
    #[expect(dead_code, reason = "Kept for service lifecycle management")]
    monitor: ServiceMonitor<BlockasmServiceStatus>,
}

impl BlockasmHandle {
    pub(crate) fn new(
        command_handle: Arc<CommandHandle<BlockasmCommand>>,
        monitor: ServiceMonitor<BlockasmServiceStatus>,
    ) -> Self {
        Self {
            command_handle,
            monitor,
        }
    }

    fn service_closed_error<T>(_: T) -> BlockAssemblyError {
        BlockAssemblyError::RequestChannelClosed
    }

    async fn send_command<R>(
        &self,
        command: BlockasmCommand,
        rx: oneshot::Receiver<R>,
    ) -> BlockAssemblyResult<R> {
        self.command_handle
            .send(command)
            .await
            .map_err(Self::service_closed_error)?;

        rx.await
            .map_err(|_| BlockAssemblyError::ResponseChannelClosed)
    }

    /// Generate a new block template based on provided configuration.
    pub async fn generate_block_template(
        &self,
        config: BlockGenerationConfig,
    ) -> BlockAssemblyResult<FullBlockTemplate> {
        let (completion, rx) = create_completion();
        let command = BlockasmCommand::GenerateBlockTemplate { config, completion };
        self.send_command(command, rx).await?
    }

    /// Look up a pending block template by parent block ID.
    pub async fn get_block_template(
        &self,
        parent_block_id: OLBlockId,
    ) -> BlockAssemblyResult<FullBlockTemplate> {
        let (completion, rx) = create_completion();
        let command = BlockasmCommand::GetBlockTemplate {
            parent_block_id,
            completion,
        };
        self.send_command(command, rx).await?
    }

    /// Completes a cached template with completion data and returns the block.
    ///
    /// This validates the completion data and does not remove the template from the cache. Call
    /// [`Self::record_persisted_block`] after the block is durably persisted.
    pub async fn complete_block_template(
        &self,
        template_id: OLBlockId,
        data: BlockCompletionData,
    ) -> BlockAssemblyResult<OLBlock> {
        let (completion, rx) = create_completion();
        let command = BlockasmCommand::CompleteBlockTemplate {
            template_id,
            data,
            completion,
        };
        self.send_command(command, rx).await?
    }

    /// Release a completed-template status if it references `block`.
    pub async fn release_completed_template_status(
        &self,
        parent_block_id: OLBlockId,
        block: OLBlockCommitment,
    ) -> BlockAssemblyResult<bool> {
        let (completion, rx) = create_completion();
        let command = BlockasmCommand::ReleaseCompletedTemplateStatus {
            parent_block_id,
            block,
            completion,
        };
        self.send_command(command, rx).await
    }

    /// Records that the block produced from this template has been persisted.
    pub async fn record_persisted_block(&self, template_id: OLBlockId) -> BlockAssemblyResult<()> {
        let (completion, rx) = create_completion();
        let command = BlockasmCommand::RecordPersistedBlock {
            template_id,
            completion,
        };
        self.send_command(command, rx).await?
    }
}
