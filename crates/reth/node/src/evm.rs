use alpen_reth_evm::evm::StrataEvmPrecompiles;
use reth_chainspec::ChainSpec;
use reth_evm::{eth::EthEvmContext, EthEvm, EvmEnv, EvmFactory};
use reth_evm_ethereum::EthEvmConfig;
use reth_node_api::{FullNodeTypes, NodeTypes};
use reth_node_builder::{components::ExecutorBuilder, BuilderContext};
use reth_node_ethereum::BasicBlockExecutorProvider;
use reth_primitives::EthPrimitives;
use revm::{
    context::{
        result::{EVMError, HaltReason},
        TxEnv,
    },
    inspector::NoOpInspector,
    Context, MainBuilder, MainContext,
};
use revm_primitives::hardfork::SpecId;

/// Custom EVM configuration.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct StrataEvmFactory;

impl EvmFactory for StrataEvmFactory {
    type Evm<DB: reth_evm::Database, I: revm::Inspector<Self::Context<DB>>> =
        EthEvm<DB, I, StrataEvmPrecompiles>;

    type Context<DB: reth_evm::Database> = EthEvmContext<DB>;

    type Tx = TxEnv;
    type Error<DBError: std::error::Error + Send + Sync + 'static> = EVMError<DBError>;

    type HaltReason = HaltReason;

    type Spec = SpecId;

    fn create_evm<DB: reth_evm::Database>(
        &self,
        db: DB,
        input: EvmEnv,
    ) -> Self::Evm<DB, revm::inspector::NoOpInspector> {
        let evm = Context::mainnet()
            .with_db(db)
            .with_cfg(input.cfg_env)
            .with_block(input.block_env)
            .build_mainnet_with_inspector(NoOpInspector {})
            .with_precompiles(StrataEvmPrecompiles::new());

        EthEvm::new(evm, false)
    }

    fn create_evm_with_inspector<DB: reth_evm::Database, I: revm::Inspector<Self::Context<DB>>>(
        &self,
        db: DB,
        input: reth_evm::EvmEnv<Self::Spec>,
        inspector: I,
    ) -> Self::Evm<DB, I> {
        EthEvm::new(
            self.create_evm(db, input)
                .into_inner()
                .with_inspector(inspector),
            true,
        )
    }
}

/// Builds a regular ethereum block executor that uses the custom EVM.
#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct StrataExecutorBuilder;

impl<Node> ExecutorBuilder<Node> for StrataExecutorBuilder
where
    Node: FullNodeTypes<Types: NodeTypes<ChainSpec = ChainSpec, Primitives = EthPrimitives>>,
{
    type EVM = EthEvmConfig<StrataEvmFactory>;

    type Executor = BasicBlockExecutorProvider<Self::EVM>;

    async fn build_evm(
        self,
        ctx: &BuilderContext<Node>,
    ) -> eyre::Result<(Self::EVM, Self::Executor)> {
        let evm_config =
            EthEvmConfig::new_with_evm_factory(ctx.chain_spec(), StrataEvmFactory::default());
        Ok((
            evm_config.clone(),
            BasicBlockExecutorProvider::new(evm_config),
        ))
    }
}
