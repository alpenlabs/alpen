//! OL block assembly service handle for external interaction.

use std::sync::Arc;

use strata_identifiers::OLBlockId;
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

    /// Complete specified template with completion data and return the final block.
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
}
