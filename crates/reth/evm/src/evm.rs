use core::error;

use reth_evm::{eth::EthEvmContext, precompiles::PrecompilesMap, Database, EvmEnv, EvmFactory};
use revm::{
    context::{
        result::{EVMError, HaltReason},
        BlockEnv, TxEnv,
    },
    inspector::NoOpInspector,
    interpreter::interpreter::EthInterpreter,
    Context, Inspector, MainBuilder, MainContext,
};
use revm_primitives::hardfork::SpecId;

use crate::{apis::AlpenAlloyEvm, precompiles::factory};

/// Custom EVM configuration.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct AlpenEvmFactory;

impl EvmFactory for AlpenEvmFactory {
    type Evm<DB: Database, I: Inspector<EthEvmContext<DB>, EthInterpreter>> = AlpenAlloyEvm<DB, I>;
    type Tx = TxEnv;
    type Error<DBError: error::Error + Send + Sync + 'static> = EVMError<DBError>;
    type HaltReason = HaltReason;
    type Context<DB: Database> = EthEvmContext<DB>;
    type Spec = SpecId;
    type BlockEnv = BlockEnv;
    type Precompiles = PrecompilesMap;

    fn create_evm<DB: Database>(&self, db: DB, input: EvmEnv) -> Self::Evm<DB, NoOpInspector> {
        let precompiles = factory::create_precompiles_map(input.cfg_env.spec);

        let evm = Context::mainnet()
            .with_db(db)
            .with_cfg(input.cfg_env)
            .with_block(input.block_env)
            .build_mainnet_with_inspector(NoOpInspector {})
            .with_precompiles(precompiles);

        AlpenAlloyEvm::new(evm, false)
    }

    fn create_evm_with_inspector<DB: Database, I: Inspector<Self::Context<DB>, EthInterpreter>>(
        &self,
        db: DB,
        input: EvmEnv,
        inspector: I,
    ) -> Self::Evm<DB, I> {
        AlpenAlloyEvm::new(
            self.create_evm(db, input)
                .into_inner()
                .with_inspector(inspector),
            true,
        )
    }
}
