use alpen_reth_rpc::{eth::StrataEthApiBuilder, SequencerClient, StrataEthApi};
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
use reth_node_ethereum::node::{
    EthereumConsensusBuilder, EthereumNetworkBuilder, EthereumPoolBuilder,
};
use reth_primitives::EthPrimitives;
use reth_provider::EthStorage;
use reth_rpc_eth_types::{error::FromEvmError, EthApiError};
use revm::context::TxEnv;

use crate::{
    args::StrataNodeArgs, engine::AlpenEngineValidatorBuilder, evm::AlpenExecutorBuilder,
    payload_builder::AlpenPayloadBuilderBuilder, AlpenEngineTypes, AlpenEngineValidator,
};

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct AlpenEthereumNode {
    // Strata node args.
    pub args: StrataNodeArgs,
}

impl AlpenEthereumNode {
    /// Creates a new instance of the StrataEthereum node type.
    pub fn new(args: StrataNodeArgs) -> Self {
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
        EthereumPoolBuilder,
        BasicPayloadServiceBuilder<AlpenPayloadBuilderBuilder>,
        EthereumNetworkBuilder,
        AlpenExecutorBuilder,
        EthereumConsensusBuilder,
    >;

    type AddOns = StrataNodeAddOns<
        NodeAdapter<N, <Self::ComponentsBuilder as NodeComponentsBuilder<N>>::Components>,
    >;

    fn components_builder(&self) -> Self::ComponentsBuilder {
        ComponentsBuilder::default()
            .node_types::<N>()
            .pool(EthereumPoolBuilder::default())
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
pub struct StrataAddOnsBuilder {
    /// Sequencer client, configured to forward submitted transactions to sequencer of given OP
    /// network.
    sequencer_client: Option<SequencerClient>,
}

impl StrataAddOnsBuilder {
    /// With a [`SequencerClient`].
    pub fn with_sequencer(mut self, sequencer_client: Option<String>) -> Self {
        self.sequencer_client = sequencer_client.map(SequencerClient::new);
        self
    }
}

impl StrataAddOnsBuilder {
    /// Builds an instance of [`StrataAddOns`].
    pub fn build<N>(self) -> StrataNodeAddOns<N>
    where
        N: FullNodeComponents<Types: NodeTypes<Primitives = EthPrimitives>>,
        StrataEthApiBuilder: EthApiBuilder<N>,
    {
        let Self { sequencer_client } = self;

        let sequencer_client_clone = sequencer_client.clone();
        StrataNodeAddOns {
            rpc_add_ons: RpcAddOns::new(
                StrataEthApiBuilder::default().with_sequencer(sequencer_client_clone),
                AlpenEngineValidatorBuilder::default(),
                BasicEngineApiBuilder::default(),
            ),
        }
    }
}

/// Add-ons for Strata.
#[derive(Debug)]
pub struct StrataNodeAddOns<N>
where
    N: FullNodeComponents,
    StrataEthApiBuilder: EthApiBuilder<N>,
{
    /// Rpc add-ons responsible for launching the RPC servers and instantiating the RPC handlers
    /// and eth-api.
    pub rpc_add_ons: RpcAddOns<
        N,
        StrataEthApiBuilder,
        AlpenEngineValidatorBuilder,
        BasicEngineApiBuilder<AlpenEngineValidatorBuilder>,
    >,
}

impl<N> Default for StrataNodeAddOns<N>
where
    N: FullNodeComponents<Types: NodeTypes<Primitives = EthPrimitives>>,
    StrataEthApiBuilder: EthApiBuilder<N>,
{
    fn default() -> Self {
        Self::builder().build()
    }
}

impl<N> StrataNodeAddOns<N>
where
    N: FullNodeComponents<Types: NodeTypes<Primitives = EthPrimitives>>,
    StrataEthApiBuilder: EthApiBuilder<N>,
{
    /// Build a [`OpAddOns`] using [`OpAddOnsBuilder`].
    pub fn builder() -> StrataAddOnsBuilder {
        StrataAddOnsBuilder::default()
    }
}

impl<N> NodeAddOns<N> for StrataNodeAddOns<N>
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
    type Handle = RpcHandle<N, StrataEthApi<N>>;

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

impl<N> RethRpcAddOns<N> for StrataNodeAddOns<N>
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
    type EthApi = StrataEthApi<N>;

    fn hooks_mut(&mut self) -> &mut reth_node_builder::rpc::RpcHooks<N, Self::EthApi> {
        self.rpc_add_ons.hooks_mut()
    }
}

impl<N> EngineValidatorAddOn<N> for StrataNodeAddOns<N>
where
    N: FullNodeComponents<
        Types: NodeTypes<
            ChainSpec = ChainSpec,
            Primitives = EthPrimitives,
            Payload = AlpenEngineTypes,
        >,
    >,
    StrataEthApiBuilder: EthApiBuilder<N>,
{
    type Validator = AlpenEngineValidator;
    async fn engine_validator(
        &self,
        ctx: &reth_node_api::AddOnsContext<'_, N>,
    ) -> eyre::Result<Self::Validator> {
        AlpenEngineValidatorBuilder::default().build(ctx).await
    }
}
