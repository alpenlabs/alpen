use alpen_reth_rpc::{eth::AlpenEthApiBuilder, SequencerClient};
use reth_chainspec::ChainSpec;
use reth_evm::{ConfigureEvm, EvmFactory, EvmFactoryFor, NextBlockEnvAttributes};
use reth_node_api::{FullNodeComponents, NodeAddOns};
use reth_node_builder::{
    components::{BasicPayloadServiceBuilder, ComponentsBuilder},
    node::{FullNodeTypes, NodeTypes},
    rpc::{
        BasicEngineApiBuilder, BasicEngineValidatorBuilder, EngineApiBuilder, EngineValidatorAddOn,
        EngineValidatorBuilder, EthApiBuilder, Identity, PayloadValidatorBuilder, RethRpcAddOns,
        RpcAddOns, RpcHandle,
    },
    Node, NodeAdapter, NodeComponentsBuilder,
};
use reth_node_ethereum::node::{EthereumConsensusBuilder, EthereumNetworkBuilder};
use reth_primitives::EthPrimitives;
use reth_provider::EthStorage;
use reth_rpc_eth_types::{error::FromEvmError, EthApiError};
use revm::context::TxEnv;

use crate::{
    args::AlpenNodeArgs, engine::AlpenEngineValidatorBuilder, evm::AlpenExecutorBuilder,
    payload_builder::AlpenPayloadBuilderBuilder, pool::AlpenEthereumPoolBuilder, AlpenEngineTypes,
};

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct AlpenEthereumNode {
    // Strata node args.
    pub args: AlpenNodeArgs,
}

impl AlpenEthereumNode {
    /// Creates a new instance of the StrataEthereum node type.
    pub fn new(args: AlpenNodeArgs) -> Self {
        Self { args }
    }
}

impl NodeTypes for AlpenEthereumNode {
    type Primitives = EthPrimitives;
    type ChainSpec = ChainSpec;
    type Storage = EthStorage;
    type Payload = AlpenEngineTypes;
}

impl<N> Node<N> for AlpenEthereumNode
where
    N: FullNodeTypes<
        Types: NodeTypes<
            Payload = AlpenEngineTypes,
            ChainSpec = ChainSpec,
            Primitives = EthPrimitives,
            Storage = EthStorage,
        >,
    >,
{
    type ComponentsBuilder = ComponentsBuilder<
        N,
        AlpenEthereumPoolBuilder,
        BasicPayloadServiceBuilder<AlpenPayloadBuilderBuilder>,
        EthereumNetworkBuilder,
        AlpenExecutorBuilder,
        EthereumConsensusBuilder,
    >;

    type AddOns = AlpenRethNodeAddOns<
        NodeAdapter<N, <Self::ComponentsBuilder as NodeComponentsBuilder<N>>::Components>,
        AlpenEthApiBuilder,
        AlpenEngineValidatorBuilder,
    >;

    fn components_builder(&self) -> Self::ComponentsBuilder {
        ComponentsBuilder::default()
            .node_types::<N>()
            .pool(AlpenEthereumPoolBuilder::default())
            .executor(AlpenExecutorBuilder::default())
            .payload(BasicPayloadServiceBuilder::default())
            .network(EthereumNetworkBuilder::default())
            .consensus(EthereumConsensusBuilder::default())
    }

    fn add_ons(&self) -> Self::AddOns {
        Self::AddOns::builder()
            .with_sequencer(self.args.strata_client_http.clone())
            .build()
    }
}

#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct AlpenRethAddOnsBuilder {
    /// Sequencer client, configured to forward submitted transactions to sequencer of given OP
    /// network.
    sequencer_client: Option<SequencerClient>,
}

impl AlpenRethAddOnsBuilder {
    /// With a [`SequencerClient`].
    pub fn with_sequencer(mut self, sequencer_client: Option<String>) -> Self {
        self.sequencer_client = sequencer_client.map(SequencerClient::new);
        self
    }
}

impl AlpenRethAddOnsBuilder {
    /// Builds an instance of [`StrataAddOns`].
    pub fn build<N>(self) -> AlpenRethNodeAddOns<N, AlpenEthApiBuilder, AlpenEngineValidatorBuilder>
    where
        N: FullNodeComponents<Types: NodeTypes<Primitives = EthPrimitives>>,
        AlpenEthApiBuilder: EthApiBuilder<N>,
    {
        let Self { sequencer_client } = self;

        let sequencer_client_clone = sequencer_client.clone();
        AlpenRethNodeAddOns {
            rpc_add_ons: RpcAddOns::new(
                AlpenEthApiBuilder::default().with_sequencer(sequencer_client_clone),
                AlpenEngineValidatorBuilder::default(),
                BasicEngineApiBuilder::default(),
                BasicEngineValidatorBuilder::default(),
                Default::default(),
            ),
        }
    }
}

/// Add-ons for Strata.
#[derive(Debug)]
pub struct AlpenRethNodeAddOns<
    N: FullNodeComponents,
    EthB: EthApiBuilder<N>,
    PVB,
    EB = BasicEngineApiBuilder<PVB>,
    EVB = BasicEngineValidatorBuilder<PVB>,
    RpcMiddleware = Identity,
> {
    /// Rpc add-ons responsible for launching the RPC servers and instantiating the RPC handlers
    /// and eth-api.
    pub rpc_add_ons: RpcAddOns<N, EthB, PVB, EB, EVB, RpcMiddleware>,
}

impl<N> Default for AlpenRethNodeAddOns<N, AlpenEthApiBuilder, AlpenEngineValidatorBuilder>
where
    N: FullNodeComponents<Types: NodeTypes<Primitives = EthPrimitives>>,
    AlpenEthApiBuilder: EthApiBuilder<N>,
{
    fn default() -> Self {
        Self::builder().build()
    }
}

impl<N> AlpenRethNodeAddOns<N, AlpenEthApiBuilder, AlpenEngineValidatorBuilder>
where
    N: FullNodeComponents<Types: NodeTypes<Primitives = EthPrimitives>>,
    AlpenEthApiBuilder: EthApiBuilder<N>,
{
    /// Build a [`OpAddOns`] using [`OpAddOnsBuilder`].
    pub fn builder() -> AlpenRethAddOnsBuilder {
        AlpenRethAddOnsBuilder::default()
    }
}

impl<N, EthB, PVB, EB, EVB> NodeAddOns<N> for AlpenRethNodeAddOns<N, EthB, PVB, EB, EVB>
where
    N: FullNodeComponents<
        Types: NodeTypes<
            ChainSpec = ChainSpec,
            Primitives = EthPrimitives,
            Storage = EthStorage,
            Payload = AlpenEngineTypes,
        >,
        Evm: ConfigureEvm<NextBlockEnvCtx = NextBlockEnvAttributes>,
    >,
    EthB: EthApiBuilder<N>,
    PVB: PayloadValidatorBuilder<N>,
    EB: EngineApiBuilder<N>,
    EVB: EngineValidatorBuilder<N>,
    EthApiError: FromEvmError<N::Evm>,
    EvmFactoryFor<N::Evm>: EvmFactory<Tx = TxEnv>,
{
    type Handle = RpcHandle<N, EthB::EthApi>;

    async fn launch_add_ons(
        self,
        ctx: reth_node_api::AddOnsContext<'_, N>,
    ) -> eyre::Result<Self::Handle> {
        self.rpc_add_ons.launch_add_ons(ctx).await
    }
}

impl<N, EthB, PVB, EB, EVB> RethRpcAddOns<N> for AlpenRethNodeAddOns<N, EthB, PVB, EB, EVB>
where
    N: FullNodeComponents<
        Types: NodeTypes<
            ChainSpec = ChainSpec,
            Primitives = EthPrimitives,
            Storage = EthStorage,
            Payload = AlpenEngineTypes,
        >,
        Evm: ConfigureEvm<NextBlockEnvCtx = NextBlockEnvAttributes>,
    >,
    EthB: EthApiBuilder<N>,
    PVB: PayloadValidatorBuilder<N>,
    EB: EngineApiBuilder<N>,
    EVB: EngineValidatorBuilder<N>,
    EthApiError: FromEvmError<N::Evm>,
    EvmFactoryFor<N::Evm>: EvmFactory<Tx = TxEnv>,
{
    type EthApi = EthB::EthApi;

    fn hooks_mut(&mut self) -> &mut reth_node_builder::rpc::RpcHooks<N, Self::EthApi> {
        self.rpc_add_ons.hooks_mut()
    }
}

impl<N, EthB, PVB, EB, EVB> EngineValidatorAddOn<N> for AlpenRethNodeAddOns<N, EthB, PVB, EB, EVB>
where
    N: FullNodeComponents<
        Types: NodeTypes<
            ChainSpec = ChainSpec,
            Primitives = EthPrimitives,
            Payload = AlpenEngineTypes,
        >,
    >,
    EthB: EthApiBuilder<N>,
    PVB: Send,
    EB: EngineApiBuilder<N>,
    EVB: EngineValidatorBuilder<N>,
    EthApiError: FromEvmError<N::Evm>,
    EvmFactoryFor<N::Evm>: EvmFactory<Tx = TxEnv>,
{
    type ValidatorBuilder = EVB;

    fn engine_validator_builder(&self) -> Self::ValidatorBuilder {
        self.rpc_add_ons.engine_validator_builder()
    }
}
