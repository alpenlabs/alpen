use alpen_reth_rpc::{SequencerClient, StrataEthApi};
use reth_chainspec::{ChainSpec, EthereumHardforks};
use reth_db::transaction::{DbTx, DbTxMut};
use reth_node_api::{AddOnsContext, EngineValidator, FullNodeComponents, NodeAddOns};
use reth_node_builder::{
    components::{ComponentsBuilder, ExecutorBuilder},
    node::{FullNodeTypes, NodeTypes, NodeTypesWithEngine},
    rpc::{EngineValidatorAddOn, RethRpcAddOns, RpcAddOns, RpcHandle},
    BuilderContext, Node, NodeAdapter, NodeComponentsBuilder,
};
use reth_node_ethereum::{
    node::{EthereumConsensusBuilder, EthereumNetworkBuilder, EthereumPoolBuilder},
    BasicBlockExecutorProvider, EthExecutionStrategyFactory,
};
use reth_primitives::{BlockBody, PooledTransaction};
use reth_provider::{
    providers::{ChainStorage, NodeTypesForProvider},
    BlockBodyReader, BlockBodyWriter, ChainSpecProvider, ChainStorageReader, ChainStorageWriter,
    DBProvider, DatabaseProvider, EthStorage, ProviderResult, ReadBodyInput, StorageLocation,
};
use reth_transaction_pool::{PoolTransaction, TransactionPool};
use revm_primitives::alloy_primitives;

use crate::{
    args::StrataNodeArgs,
    engine::{StrataEngineTypes, StrataEngineValidator, StrataEngineValidatorBuilder},
    evm::StrataEvmConfig,
    payload_builder::StrataPayloadServiceBuilder,
};

/// Strata primitive types.
pub(crate) type StrataPrimitives = reth_primitives::EthPrimitives;

/// Storage implementation for Strata.
#[derive(Debug, Default, Clone)]
pub struct StrataStorage(EthStorage);

impl<Provider: DBProvider<Tx: DbTxMut>> BlockBodyWriter<Provider, BlockBody> for StrataStorage {
    fn write_block_bodies(
        &self,
        provider: &Provider,
        bodies: Vec<(u64, Option<BlockBody>)>,
        write_to: StorageLocation,
    ) -> ProviderResult<()> {
        self.0.write_block_bodies(provider, bodies, write_to)
    }

    fn remove_block_bodies_above(
        &self,
        provider: &Provider,
        block: alloy_primitives::BlockNumber,
        remove_from: StorageLocation,
    ) -> ProviderResult<()> {
        self.0
            .remove_block_bodies_above(provider, block, remove_from)
    }
}

impl<Provider: DBProvider + ChainSpecProvider<ChainSpec: EthereumHardforks>>
    BlockBodyReader<Provider> for StrataStorage
{
    type Block = reth_primitives::Block;

    fn read_block_bodies(
        &self,
        provider: &Provider,
        inputs: Vec<ReadBodyInput<'_, Self::Block>>,
    ) -> ProviderResult<Vec<BlockBody>> {
        self.0.read_block_bodies(provider, inputs)
    }
}

impl ChainStorage<StrataPrimitives> for StrataStorage {
    fn reader<TX, Types>(
        &self,
    ) -> impl ChainStorageReader<DatabaseProvider<TX, Types>, StrataPrimitives>
    where
        TX: DbTx + 'static,
        Types: NodeTypesForProvider<Primitives = StrataPrimitives>,
    {
        self
    }

    fn writer<TX, Types>(
        &self,
    ) -> impl ChainStorageWriter<DatabaseProvider<TX, Types>, StrataPrimitives>
    where
        TX: DbTxMut + DbTx + 'static,
        Types: NodeTypes<Primitives = StrataPrimitives>,
    {
        self
    }
}

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct StrataEthereumNode {
    // Strata node args.
    pub args: StrataNodeArgs,
}

impl StrataEthereumNode {
    pub fn new(args: StrataNodeArgs) -> Self {
        Self { args }
    }

    /// Returns the components for the given [`StrataNodeArgs`].
    pub fn components<N>() -> ComponentsBuilder<
        N,
        EthereumPoolBuilder,
        StrataPayloadServiceBuilder,
        EthereumNetworkBuilder,
        StrataExecutorBuilder,
        EthereumConsensusBuilder,
    >
    where
        N: FullNodeTypes<
            Types: NodeTypesWithEngine<
                Engine = StrataEngineTypes,
                ChainSpec = ChainSpec,
                Primitives = StrataPrimitives,
            >,
        >,
    {
        ComponentsBuilder::default()
            .node_types::<N>()
            .pool(EthereumPoolBuilder::default())
            .payload(StrataPayloadServiceBuilder::default())
            .network(EthereumNetworkBuilder::default())
            .executor(StrataExecutorBuilder::default())
            .consensus(EthereumConsensusBuilder::default())
    }
}

/// Implement the Node trait for the custom node
///
/// This provides a preset configuration for the node
impl<N> Node<N> for StrataEthereumNode
where
    N: FullNodeTypes<
        Types: NodeTypesWithEngine<
            Engine = StrataEngineTypes,
            ChainSpec = ChainSpec,
            Primitives = StrataPrimitives,
            Storage = StrataStorage,
        >,
    >,
{
    type ComponentsBuilder = ComponentsBuilder<
        N,
        EthereumPoolBuilder,
        StrataPayloadServiceBuilder,
        EthereumNetworkBuilder,
        StrataExecutorBuilder,
        EthereumConsensusBuilder,
    >;
    type AddOns = StrataAddOns<
        NodeAdapter<N, <Self::ComponentsBuilder as NodeComponentsBuilder<N>>::Components>,
    >;

    fn components_builder(&self) -> Self::ComponentsBuilder {
        Self::components()
    }

    fn add_ons(&self) -> Self::AddOns {
        Self::AddOns::builder()
            .with_sequencer(self.args.sequencer_http.clone())
            .build()
    }
}

/// Configure the node types
impl NodeTypes for StrataEthereumNode {
    type Primitives = StrataPrimitives;
    type ChainSpec = ChainSpec;
    type StateCommitment = reth_trie_db::MerklePatriciaTrie;
    type Storage = StrataStorage;
}

/// Configure the node types with the custom engine types
impl NodeTypesWithEngine for StrataEthereumNode {
    // use the custom engine types
    type Engine = StrataEngineTypes;
}

/// Add-ons for Strata.
#[derive(Debug)]
pub struct StrataAddOns<N: FullNodeComponents> {
    pub rpc_add_ons: RpcAddOns<N, StrataEthApi<N>, StrataEngineValidatorBuilder>,
}

impl<N: FullNodeComponents<Types: NodeTypes<Primitives = StrataPrimitives>>> Default
    for StrataAddOns<N>
{
    fn default() -> Self {
        Self::builder().build()
    }
}

impl<N: FullNodeComponents<Types: NodeTypes<Primitives = StrataPrimitives>>> StrataAddOns<N> {
    /// Build a [`StrataAddOns`] using [`StrataAddOnsBuilder`].
    pub fn builder() -> StrataAddOnsBuilder {
        StrataAddOnsBuilder::default()
    }
}

impl<N> NodeAddOns<N> for StrataAddOns<N>
where
    N: FullNodeComponents<
        Types: NodeTypesWithEngine<
            ChainSpec = ChainSpec,
            Primitives = StrataPrimitives,
            Storage = StrataStorage,
            Engine = StrataEngineTypes,
        >,
        Pool: TransactionPool<Transaction: PoolTransaction<Pooled = PooledTransaction>>,
    >,
    StrataEngineValidator: EngineValidator<<N::Types as NodeTypesWithEngine>::Engine>,
{
    type Handle = RpcHandle<N, StrataEthApi<N>>;

    async fn launch_add_ons(
        self,
        ctx: reth_node_api::AddOnsContext<'_, N>,
    ) -> eyre::Result<Self::Handle> {
        let Self { rpc_add_ons } = self;

        rpc_add_ons
            .launch_add_ons_with(ctx, move |_, _| Ok(()))
            .await
    }
}

impl<N> RethRpcAddOns<N> for StrataAddOns<N>
where
    N: FullNodeComponents<
        Types: NodeTypesWithEngine<
            ChainSpec = ChainSpec,
            Primitives = StrataPrimitives,
            Storage = StrataStorage,
            Engine = StrataEngineTypes,
        >,
        Pool: TransactionPool<Transaction: PoolTransaction<Pooled = PooledTransaction>>,
    >,
    StrataEngineValidator: EngineValidator<<N::Types as NodeTypesWithEngine>::Engine>,
{
    type EthApi = StrataEthApi<N>;

    fn hooks_mut(&mut self) -> &mut reth_node_builder::rpc::RpcHooks<N, Self::EthApi> {
        self.rpc_add_ons.hooks_mut()
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
    pub fn build<N>(self) -> StrataAddOns<N>
    where
        N: FullNodeComponents<Types: NodeTypes<Primitives = StrataPrimitives>>,
    {
        let Self { sequencer_client } = self;

        StrataAddOns {
            rpc_add_ons: RpcAddOns::new(
                move |ctx| {
                    StrataEthApi::<N>::builder()
                        .with_sequencer(sequencer_client)
                        .build(ctx)
                },
                Default::default(),
            ),
        }
    }
}

/// Engine validator add-on for Strata.
impl<N> EngineValidatorAddOn<N> for StrataAddOns<N>
where
    N: FullNodeComponents<
        Types: NodeTypesWithEngine<
            ChainSpec = ChainSpec,
            Primitives = StrataPrimitives,
            Engine = StrataEngineTypes,
        >,
    >,
{
    type Validator = StrataEngineValidator;

    async fn engine_validator(&self, ctx: &AddOnsContext<'_, N>) -> eyre::Result<Self::Validator> {
        Ok(StrataEngineValidator::new(ctx.config.chain.clone()))
    }
}

/// Builds a regular ethereum block executor that uses the custom EVM.
#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct StrataExecutorBuilder;

impl<Node> ExecutorBuilder<Node> for StrataExecutorBuilder
where
    Node: FullNodeTypes<Types: NodeTypes<ChainSpec = ChainSpec, Primitives = StrataPrimitives>>,
{
    type EVM = StrataEvmConfig;
    type Executor = BasicBlockExecutorProvider<EthExecutionStrategyFactory<Self::EVM>>;

    async fn build_evm(
        self,
        ctx: &BuilderContext<Node>,
    ) -> eyre::Result<(Self::EVM, Self::Executor)> {
        let evm_config = StrataEvmConfig::new(ctx.chain_spec());

        Ok((
            evm_config.clone(),
            BasicBlockExecutorProvider::new(EthExecutionStrategyFactory::new(
                ctx.chain_spec(),
                evm_config,
            )),
        ))
    }
}
