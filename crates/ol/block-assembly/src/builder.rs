//! Block assembly service builder for initialization and launch.

use std::{
    fmt::{Debug, Display, Formatter},
    sync::Arc,
};

use strata_config::{BlockAssemblyConfig, SequencerConfig};
use strata_ledger_types::{IAccountStateMut, IStateAccessor, IStateAccessorMut};
use strata_ol_params::OLParams;
use strata_ol_state_provider::StateProvider;
use strata_predicate::PredicateKey;
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
pub struct BlockasmBuilder<M, E, P>
where
    M: MempoolProvider,
    E: EpochSealingPolicy,
    P: StateProvider,
{
    ol_params: Arc<OLParams>,
    blockasm_config: Arc<BlockAssemblyConfig>,
    storage: Arc<NodeStorage>,
    mempool_provider: M,
    epoch_sealing_policy: E,
    state_provider: P,
    sequencer_config: SequencerConfig,
    sequencer_predicate: PredicateKey,
    l1_reorg_safe_depth: u32,
    command_buffer_size: usize,
}

impl<M, E, P> BlockasmBuilder<M, E, P>
where
    M: MempoolProvider,
    E: EpochSealingPolicy,
    P: StateProvider,
{
    #[expect(
        clippy::too_many_arguments,
        reason = "builder collects unrelated service inputs"
    )]
    pub fn new(
        ol_params: Arc<OLParams>,
        blockasm_config: Arc<BlockAssemblyConfig>,
        storage: Arc<NodeStorage>,
        mempool_provider: M,
        epoch_sealing_policy: E,
        state_provider: P,
        sequencer_config: SequencerConfig,
        sequencer_predicate: PredicateKey,
        l1_reorg_safe_depth: u32,
    ) -> Self {
        Self {
            ol_params,
            blockasm_config,
            storage,
            mempool_provider,
            epoch_sealing_policy,
            state_provider,
            sequencer_config,
            sequencer_predicate,
            l1_reorg_safe_depth,
            command_buffer_size: 64,
        }
    }

    pub fn with_command_buffer_size(mut self, size: usize) -> Self {
        self.command_buffer_size = size;
        self
    }

    pub async fn launch(self, texec: &TaskExecutor) -> anyhow::Result<BlockasmHandle>
    where
        M: Send + Sync + 'static,
        E: Send + Sync + 'static,
        P: Send + Sync + 'static,
        P::Error: Display,
        P::State: BlockAssemblyStateAccess,
    <<P::State as IStateAccessor>::AccountState as IAccountStateMut>::SnarkAccountStateMut:
            Clone,
    <P::State as IStateAccessorMut>::AccountStateMut: Clone,
    <<P::State as IStateAccessorMut>::AccountStateMut as IAccountStateMut>::SnarkAccountStateMut:
            Clone,
    {
        let context = Arc::new(BlockAssemblyContext::new(
            self.storage,
            self.mempool_provider,
            self.state_provider,
            self.l1_reorg_safe_depth,
        ));

        let state = BlockasmServiceState::new(
            self.ol_params,
            self.blockasm_config,
            self.sequencer_config,
            self.sequencer_predicate,
            context,
            self.epoch_sealing_policy,
        );

        let mut service_builder =
            ServiceBuilder::<BlockasmService<M, E, P>, _>::new().with_state(state);

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
            .field("blockasm_config", &self.blockasm_config)
            .field("storage", &"<NodeStorage>")
            .field("sequencer_config", &self.sequencer_config)
            .field("sequencer_predicate", &self.sequencer_predicate)
            .field("command_buffer_size", &self.command_buffer_size)
            .finish()
    }
}
