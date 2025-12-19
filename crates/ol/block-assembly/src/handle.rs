//! OL block assembly service handle for external interaction.

use std::sync::Arc;

use strata_identifiers::OLBlockId;
use strata_ol_chain_types_new::OLBlock;
use strata_service::{CommandHandle, ServiceBuilder, ServiceMonitor};
use strata_tasks::TaskExecutor;

use crate::{
    command::{BlockAssemblyCommand, create_completion},
    context::BlockAssemblyContextImpl,
    error::BlockAssemblyError,
    service::{BlockAssemblyService, BlockAssemblyServiceStatus},
    state::BlockAssemblyServiceState,
    types::{BlockCompletionData, BlockGenerationConfig, BlockTemplate},
};

/// Handle for interacting with the OL block assembly service.
#[derive(Debug)]
pub struct BlockAssemblyHandle {
    command_handle: Arc<CommandHandle<BlockAssemblyCommand>>,
    #[expect(dead_code, reason = "Kept for service lifecycle management")]
    monitor: ServiceMonitor<BlockAssemblyServiceStatus>,
}

impl BlockAssemblyHandle {
    /// Helper to map send/recv errors to service errors.
    fn service_closed_error<T>(_: T) -> BlockAssemblyError {
        BlockAssemblyError::RequestChannelClosed
    }

    /// Send command and wait for response.
    async fn send_command<R>(
        &self,
        command: BlockAssemblyCommand,
        rx: tokio::sync::oneshot::Receiver<R>,
    ) -> Result<R, BlockAssemblyError> {
        self.command_handle
            .send(command)
            .await
            .map_err(Self::service_closed_error)?;

        rx.await
            .map_err(|_| BlockAssemblyError::ResponseChannelClosed)
    }

    /// Create and launch a new OL block assembly service.
    ///
    /// # Arguments
    /// * `context` - Block assembly context (params, storage, mempool handle)
    /// * `texec` - Task executor for spawning the service task
    ///
    /// # Returns
    /// A handle to interact with the launched service
    pub async fn launch(
        context: Arc<BlockAssemblyContextImpl>,
        texec: &TaskExecutor,
    ) -> anyhow::Result<Self> {
        let state = BlockAssemblyServiceState::new(context);

        // Create service builder
        let mut service_builder =
            ServiceBuilder::<BlockAssemblyService, _>::new().with_state(state);

        // Create command handle with buffer size
        let command_handle = Arc::new(service_builder.create_command_handle(64));

        // Launch service
        let monitor = service_builder
            .launch_async("ol_block_assembly", texec)
            .await?;

        Ok(Self {
            command_handle,
            monitor,
        })
    }

    /// Generate a new block template based on provided configuration.
    ///
    /// Will return cached template for request if it exists.
    pub async fn generate_block_template(
        &self,
        config: BlockGenerationConfig,
    ) -> Result<BlockTemplate, BlockAssemblyError> {
        let (completion, rx) = create_completion();
        let command = BlockAssemblyCommand::GenerateBlockTemplate { config, completion };
        self.send_command(command, rx).await?
    }

    /// Complete specified template with completion data and return the final block.
    pub async fn complete_block_template(
        &self,
        template_id: OLBlockId,
        completion_data: BlockCompletionData,
    ) -> Result<OLBlock, BlockAssemblyError> {
        let (completion, rx) = create_completion();
        let command = BlockAssemblyCommand::CompleteBlockTemplate {
            template_id,
            completion_data,
            completion,
        };
        self.send_command(command, rx).await?
    }
}
