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
use revm_primitives::{hardfork::SpecId, U256};
use strata_bridge_params::{BridgeParams, DEFAULT_MAX_WITHDRAWAL_DESCRIPTOR_LEN};

use crate::{
    apis::AlpenAlloyEvm,
    precompiles::factory,
    utils::{u256_from, WEI_PER_BTC, WEI_PER_SAT},
};

/// Custom EVM configuration.
///
/// Carries withdrawal denomination and optional cap (in wei) for bridge
/// precompile validation. Use [`AlpenEvmFactory::new`] to construct, or
/// [`Default`] for the standard 1 BTC denomination / 10 BTC cap.
#[derive(Debug, Clone)]
pub struct AlpenEvmFactory {
    denomination_wei: U256,
    max_withdrawal_wei: Option<U256>,
    max_withdrawal_descriptor_len: u32,
    bridge_params: BridgeParams,
}

impl Default for AlpenEvmFactory {
    fn default() -> Self {
        Self {
            denomination_wei: u256_from(WEI_PER_BTC),
            max_withdrawal_wei: Some(u256_from(WEI_PER_BTC * 10)),
            max_withdrawal_descriptor_len: DEFAULT_MAX_WITHDRAWAL_DESCRIPTOR_LEN,
            bridge_params: BridgeParams::default(),
        }
    }
}

impl AlpenEvmFactory {
    pub fn new(denomination_wei: U256, max_withdrawal_wei: Option<U256>) -> Self {
        Self {
            denomination_wei,
            max_withdrawal_wei,
            max_withdrawal_descriptor_len: DEFAULT_MAX_WITHDRAWAL_DESCRIPTOR_LEN,
            bridge_params: BridgeParams::default(),
        }
    }

    pub fn max_withdrawal_descriptor_len(&self) -> u32 {
        self.max_withdrawal_descriptor_len
    }

    pub fn bridge_params(&self) -> &BridgeParams {
        &self.bridge_params
    }

    /// Creates an [`AlpenEvmFactory`] from [`BridgeParams`] (sats-denominated),
    /// converting to wei.
    pub fn from_bridge_params(bp: &BridgeParams) -> Self {
        let denom_wei = U256::from(bp.denomination()) * WEI_PER_SAT;
        let max_wei = bp
            .max_withdrawal_amount()
            .map(|m| U256::from(m) * WEI_PER_SAT);
        Self {
            denomination_wei: denom_wei,
            max_withdrawal_wei: max_wei,
            max_withdrawal_descriptor_len: bp.max_withdrawal_descriptor_len(),
            bridge_params: *bp,
        }
    }
}

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
        let precompiles = factory::create_precompiles_map(
            input.cfg_env.spec,
            self.denomination_wei,
            self.max_withdrawal_wei,
            self.max_withdrawal_descriptor_len,
        );

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
