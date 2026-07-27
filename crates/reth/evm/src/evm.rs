use core::error;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

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
use strata_bridge_params::BridgeParams;

use crate::{apis::AlpenAlloyEvm, precompiles::factory, utils::wei_to_sats};

/// Custom EVM configuration.
///
/// Carries bridge withdrawal policy for precompile validation and the current per-block
/// DA rate (wei per byte) used by the in-EVM DA fee charge.
///
/// The DA rate is interior-mutable and shared across clones: reth's EVM-env plumbing
/// cannot thread a per-block value into [`EvmFactory::create_evm`], so the caller (which
/// holds the block header) sets it via [`AlpenEvmFactory::set_da_rate`] just before
/// executing a block, and `create_evm` reads it into the EVM instance. Block execution is
/// sequential (the proof processes blocks in order), so this is deterministic.
#[derive(Debug, Clone)]
pub struct AlpenEvmFactory {
    bridge_params: BridgeParams,
    da_rate: Arc<AtomicU64>,
}

impl AlpenEvmFactory {
    pub fn new_with_descriptor_limit(
        denomination_wei: U256,
        max_withdrawal_wei: Option<U256>,
        max_withdrawal_descriptor_len: u32,
    ) -> Self {
        let denomination = wei_to_sats_exact(denomination_wei, "denomination_wei");
        let max_withdrawal_amount =
            max_withdrawal_wei.map(|max| wei_to_sats_exact(max, "max_withdrawal_wei"));

        Self {
            bridge_params: BridgeParams::new_with_descriptor_limit(
                denomination,
                max_withdrawal_amount,
                max_withdrawal_descriptor_len,
            )
            .expect("withdrawal policy constructed from wei must be valid"),
            da_rate: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn max_withdrawal_descriptor_len(&self) -> u32 {
        self.bridge_params.max_withdrawal_descriptor_len()
    }

    pub fn bridge_params(&self) -> &BridgeParams {
        &self.bridge_params
    }

    /// Creates an [`AlpenEvmFactory`] from [`BridgeParams`].
    pub fn from_bridge_params(bp: &BridgeParams) -> Self {
        Self {
            bridge_params: *bp,
            da_rate: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Sets the per-block DA rate (wei per byte) used by the DA fee charge.
    ///
    /// The caller must set this from the block's committed DA rate before executing the
    /// block's transactions.
    pub fn set_da_rate(&self, da_rate: u64) {
        self.da_rate.store(da_rate, Ordering::Relaxed);
    }

    /// Returns the currently configured per-block DA rate (wei per byte).
    pub fn da_rate(&self) -> u64 {
        self.da_rate.load(Ordering::Relaxed)
    }
}

#[cfg(any(test, feature = "test-defaults"))]
impl Default for AlpenEvmFactory {
    fn default() -> Self {
        Self::from_bridge_params(&BridgeParams::default())
    }
}

fn wei_to_sats_exact(wei: U256, field: &str) -> u64 {
    let (sats, remainder) = wei_to_sats(wei);
    assert!(
        remainder.is_zero(),
        "{field} must be an exact number of satoshis"
    );
    sats.try_into()
        .expect("withdrawal policy amount must fit in u64 satoshis")
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
        let precompiles = factory::create_precompiles_map(input.cfg_env.spec, self.bridge_params);

        let evm = Context::mainnet()
            .with_db(db)
            .with_cfg(input.cfg_env)
            .with_block(input.block_env)
            .build_mainnet_with_inspector(NoOpInspector {})
            .with_precompiles(precompiles);

        AlpenAlloyEvm::new(evm, false, U256::from(self.da_rate()))
    }

    fn create_evm_with_inspector<DB: Database, I: Inspector<Self::Context<DB>, EthInterpreter>>(
        &self,
        db: DB,
        input: EvmEnv,
        inspector: I,
    ) -> Self::Evm<DB, I> {
        let da_rate = U256::from(self.da_rate());
        AlpenAlloyEvm::new(
            self.create_evm(db, input)
                .into_inner()
                .with_inspector(inspector),
            true,
            da_rate,
        )
    }
}
