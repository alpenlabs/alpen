//! Block assembly service builder for initialization and launch.

use std::{
    fmt::{Debug, Display, Formatter},
    sync::Arc,
};

use strata_config::SequencerConfig;
use strata_ol_state_types::StateProvider;
use strata_params::Params;
use strata_service::ServiceBuilder;
use strata_storage::NodeStorage;
use strata_tasks::TaskExecutor;

use crate::{
    BlockAssemblyStateAccess, BlockasmHandle, EpochSealingPolicy, MempoolProvider,
    context::BlockAssemblyContext, service::BlockasmService, state::BlockasmServiceState,
};

/// Builder for creating and launching block assembly service.
///
/// Separates service initialization logic from the handle interface.
pub struct BlockasmBuilder<M, E, S>
where
    M: MempoolProvider,
    E: EpochSealingPolicy,
    S: StateProvider,
{
    params: Arc<Params>,
    storage: Arc<NodeStorage>,
    mempool_provider: M,
    epoch_sealing_policy: E,
    state_provider: S,
    sequencer_config: SequencerConfig,
    command_buffer_size: usize,
}

impl<M, E, S> BlockasmBuilder<M, E, S>
where
    M: MempoolProvider,
    E: EpochSealingPolicy,
    S: StateProvider,
{
    pub fn new(
        params: Arc<Params>,
        storage: Arc<NodeStorage>,
        mempool_provider: M,
        epoch_sealing_policy: E,
        state_provider: S,
        sequencer_config: SequencerConfig,
    ) -> Self {
        Self {
            params,
            storage,
            mempool_provider,
            epoch_sealing_policy,
            state_provider,
            sequencer_config,
            command_buffer_size: 64,
        }
    }

    pub fn with_command_buffer_size(mut self, size: usize) -> Self {
        self.command_buffer_size = size;
        self
    }

    pub async fn launch(self, texec: &TaskExecutor) -> anyhow::Result<BlockasmHandle>
    where
        // tighten bounds here to match spawned-service reality + your context impl bounds
        M: Send + Sync + 'static,
        E: Send + Sync + 'static,
        S: Send + Sync + 'static,
        S::Error: Display,
        S::State: BlockAssemblyStateAccess,
    {
        let context = Arc::new(BlockAssemblyContext::new(
            self.storage,
            self.mempool_provider,
            self.state_provider,
        ));

        let state = BlockasmServiceState::new(
            self.params,
            self.sequencer_config,
            context,
            self.epoch_sealing_policy,
        );

        let mut service_builder =
            ServiceBuilder::<BlockasmService<M, E, S>, _>::new().with_state(state);

        let command_handle =
            Arc::new(service_builder.create_command_handle(self.command_buffer_size));

        let monitor = service_builder
            .launch_async("ol_block_assembly", texec)
            .await?;

        Ok(BlockasmHandle::new(command_handle, monitor))
    }
}

impl<M, E, S> Debug for BlockasmBuilder<M, E, S>
where
    M: MempoolProvider,
    E: EpochSealingPolicy,
    S: StateProvider,
{
    #[expect(
        clippy::absolute_paths,
        reason = "Need to distinguish std::fmt::Result"
    )]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockasmBuilder")
            .field("params", &"<Params>")
            .field("storage", &"<NodeStorage>")
            .field("sequencer_config", &self.sequencer_config)
            .field("command_buffer_size", &self.command_buffer_size)
            .finish()
    }
}
