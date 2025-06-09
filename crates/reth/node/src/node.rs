use alpen_reth_rpc::{eth::AlpenEthApiBuilder, AlpenEthApi, SequencerClient};
use reth_chainspec::ChainSpec;
use reth_evm::{ConfigureEvm, EvmFactory, EvmFactoryFor, NextBlockEnvAttributes};
use reth_node_api::{FullNodeComponents, NodeAddOns};
use reth_node_builder::{
    components::{BasicPayloadServiceBuilder, ComponentsBuilder},
    node::{FullNodeTypes, NodeTypes},
    rpc::{
        BasicEngineApiBuilder, EngineValidatorAddOn, EngineValidatorBuilder, EthApiBuilder,
        RethRpcAddOns, RpcAddOns, RpcHandle,
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
    AlpenEngineValidator,
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
    type StateCommitment = reth_trie_db::MerklePatriciaTrie;
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
    >;

    fn components_builder(&self) -> Self::ComponentsBuilder {
        ComponentsBuilder::default()
            .node_types::<N>()
            .pool(AlpenEthereumPoolBuilder::default())
            .payload(BasicPayloadServiceBuilder::default())
            .network(EthereumNetworkBuilder::default())
            .executor(AlpenExecutorBuilder::default())
            .consensus(EthereumConsensusBuilder::default())
    }

    fn add_ons(&self) -> Self::AddOns {
        Self::AddOns::builder()
            .with_sequencer(self.args.sequencer_http.clone())
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
    pub fn build<N>(self) -> AlpenRethNodeAddOns<N>
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
            ),
        }
    }
}

/// Add-ons for Strata.
#[derive(Debug)]
pub struct AlpenRethNodeAddOns<N>
where
    N: FullNodeComponents,
    AlpenEthApiBuilder: EthApiBuilder<N>,
{
    /// Rpc add-ons responsible for launching the RPC servers and instantiating the RPC handlers
    /// and eth-api.
    pub rpc_add_ons: RpcAddOns<
        N,
        AlpenEthApiBuilder,
        AlpenEngineValidatorBuilder,
        BasicEngineApiBuilder<AlpenEngineValidatorBuilder>,
    >,
}

impl<N> Default for AlpenRethNodeAddOns<N>
where
    N: FullNodeComponents<Types: NodeTypes<Primitives = EthPrimitives>>,
    AlpenEthApiBuilder: EthApiBuilder<N>,
{
    fn default() -> Self {
        Self::builder().build()
    }
}

impl<N> AlpenRethNodeAddOns<N>
where
    N: FullNodeComponents<Types: NodeTypes<Primitives = EthPrimitives>>,
    AlpenEthApiBuilder: EthApiBuilder<N>,
{
    /// Build a [`OpAddOns`] using [`OpAddOnsBuilder`].
    pub fn builder() -> AlpenRethAddOnsBuilder {
        AlpenRethAddOnsBuilder::default()
    }
}

impl<N> NodeAddOns<N> for AlpenRethNodeAddOns<N>
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
    EthApiError: FromEvmError<N::Evm>,
    EvmFactoryFor<N::Evm>: EvmFactory<Tx = TxEnv>,
{
    type Handle = RpcHandle<N, AlpenEthApi<N>>;

    async fn launch_add_ons(
        self,
        ctx: reth_node_api::AddOnsContext<'_, N>,
    ) -> eyre::Result<Self::Handle> {
        let Self { rpc_add_ons } = self;

        rpc_add_ons
            .launch_add_ons_with(ctx, move |_, _, _| Ok(()))
            .await
    }
}

impl<N> RethRpcAddOns<N> for AlpenRethNodeAddOns<N>
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
    EthApiError: FromEvmError<N::Evm>,
    EvmFactoryFor<N::Evm>: EvmFactory<Tx = TxEnv>,
{
    type EthApi = AlpenEthApi<N>;

    fn hooks_mut(&mut self) -> &mut reth_node_builder::rpc::RpcHooks<N, Self::EthApi> {
        self.rpc_add_ons.hooks_mut()
    }
}

impl<N> EngineValidatorAddOn<N> for AlpenRethNodeAddOns<N>
where
    N: FullNodeComponents<
        Types: NodeTypes<
            ChainSpec = ChainSpec,
            Primitives = EthPrimitives,
            Payload = AlpenEngineTypes,
        >,
    >,
    AlpenEthApiBuilder: EthApiBuilder<N>,
{
    type Validator = AlpenEngineValidator;
    async fn engine_validator(
        &self,
        ctx: &reth_node_api::AddOnsContext<'_, N>,
    ) -> eyre::Result<Self::Validator> {
        AlpenEngineValidatorBuilder::default().build(ctx).await
    }
}
