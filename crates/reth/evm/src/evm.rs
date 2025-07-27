use std::sync::OnceLock;

use reth_evm::{eth::EthEvmContext, precompiles::PrecompilesMap, EvmEnv, EvmFactory};
use revm::{
    context::{result::HaltReason, Cfg, ContextTr, TxEnv},
    context_interface::result::EVMError,
    handler::{EthPrecompiles, PrecompileProvider},
    inspector::NoOpInspector,
    interpreter::{Gas, InputsImpl, InstructionResult, InterpreterResult},
    precompile::{bls12_381, Precompiles},
    Context, MainBuilder, MainContext,
};
use revm_primitives::{hardfork::SpecId, Address, Bytes};

use crate::api::AlpenEvm;

/// A custom precompile that contains static precompiles.
#[allow(missing_debug_implementations)]
#[derive(Clone, Default)]
pub struct AlpenEvmPrecompiles {
    pub precompiles: EthPrecompiles,
}

impl AlpenEvmPrecompiles {
    /// Given a [`PrecompileProvider`] and cache for a specific precompiles, create a
    /// wrapper that can be used inside Evm.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Returns precompiles for Fjor spec.
pub fn load_precompiles() -> &'static Precompiles {
    static INSTANCE: OnceLock<Precompiles> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        let mut precompiles = Precompiles::berlin().clone();

        // EIP-2537: Precompile for BLS12-381
        precompiles.extend(bls12_381::precompiles());

        // Custom precompile.
        // precompiles.extend([
        //     (SCHNORR_ADDRESS, verify_schnorr_precompile as PrecompileFn).into(),
        //     (BRIDGEOUT_ADDRESS, bridgeout_precompile as PrecompileFn).into(),
        // ]);
        precompiles
    })
}

impl<CTX: ContextTr> PrecompileProvider<CTX> for AlpenEvmPrecompiles {
    type Output = InterpreterResult;

    fn set_spec(&mut self, spec: <CTX::Cfg as Cfg>::Spec) -> bool {
        self.precompiles = EthPrecompiles {
            precompiles: load_precompiles(),
            spec: spec.into(),
        };
        true
    }

    fn run(
        &mut self,
        context: &mut CTX,
        address: &Address,
        inputs: &InputsImpl,
        _is_static: bool,
        gas_limit: u64,
    ) -> Result<Option<Self::Output>, String> {
        let Some(precompile_fn) = self.precompiles.precompiles.get(address) else {
            return Ok(None);
        };

        let mut result = InterpreterResult {
            result: InstructionResult::Return,
            gas: Gas::new(gas_limit),
            output: Bytes::new(),
        };

        // let res = match *address {
        //     BRIDGEOUT_ADDRESS => bridge_context_call(&inputs.input, gas_limit, context),
        //     _ => (precompile_fn)(&inputs.input, gas_limit),
        // };

        // match res {
        //     Ok(output) => {
        //         let underflow = result.gas.record_cost(output.gas_used);
        //         assert!(underflow, "Gas underflow is not possible");
        //         result.output = output.bytes;
        //     }
        //     Err(PrecompileError::Fatal(e)) => return Err(e),
        //     Err(e) => {
        //         result.result = if e.is_oog() {
        //             InstructionResult::PrecompileOOG
        //         } else {
        //             InstructionResult::PrecompileError
        //         };
        //     }
        // }
        Ok(Some(result))
    }

    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        self.precompiles.warm_addresses()
    }

    fn contains(&self, address: &Address) -> bool {
        self.precompiles.contains(address)
    }
}

/// Custom EVM configuration.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct AlpenEvmFactory;

impl EvmFactory for AlpenEvmFactory {
    type Evm<DB: reth_evm::Database, I: revm::Inspector<Self::Context<DB>>> = AlpenEvm<DB, I>;

    type Context<DB: reth_evm::Database> = EthEvmContext<DB>;

    type Tx = TxEnv;
    type Error<DBError: std::error::Error + Send + Sync + 'static> = EVMError<DBError>;

    type HaltReason = HaltReason;

    type Spec = SpecId;

    type Precompiles = PrecompilesMap;

    fn create_evm<DB: reth_evm::Database>(
        &self,
        db: DB,
        input: EvmEnv,
    ) -> Self::Evm<DB, revm::inspector::NoOpInspector> {
        AlpenEvm::new(input, db, NoOpInspector {}, false)
    }

    fn create_evm_with_inspector<DB: reth_evm::Database, I: revm::Inspector<Self::Context<DB>>>(
        &self,
        db: DB,
        input: reth_evm::EvmEnv<Self::Spec>,
        inspector: I,
    ) -> Self::Evm<DB, I> {
        AlpenEvm::new(input, db, inspector, true)
    }
}
