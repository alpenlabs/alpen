use std::{
    ops::{Deref, DerefMut},
    sync::OnceLock,
};

use reth_evm::{
    eth::EthEvmContext, precompiles::PrecompilesMap, Database, Evm, EvmEnv, EvmFactory,
};
use revm::{
    context::{BlockEnv, Cfg, ContextTr, TxEnv},
    context_interface::result::{EVMError, HaltReason, ResultAndState},
    handler::{instructions::EthInstructions, EthPrecompiles, PrecompileProvider},
    inspector::NoOpInspector,
    interpreter::{
        interpreter::EthInterpreter, Gas, InputsImpl, InstructionResult, InterpreterResult,
    },
    precompile::{bls12_381, PrecompileError, PrecompileFn, Precompiles},
    Context, Inspector, MainBuilder, MainContext,
};
use revm_primitives::{hardfork::SpecId, Address, Bytes, TxKind, U256};

use crate::{
    api::evm::AlpenEvmInner,
    constants::{BRIDGEOUT_ADDRESS, SCHNORR_ADDRESS},
    precompiles::{
        bridge::{bridge_context_call, bridgeout_precompile},
        schnorr::verify_schnorr_precompile,
    },
};

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
    type Evm<DB: reth_evm::Database, I: revm::Inspector<Self::Context<DB>>> =
        AlpenEvm<DB, I, Self::Precompiles>;

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
        let evm_ctx = Context::mainnet()
            .with_db(db)
            .with_cfg(input.cfg_env)
            .with_block(input.block_env)
            .build_mainnet_with_inspector(NoOpInspector {})
            .with_precompiles(PrecompilesMap::from_static(
                // TODO: Revisit this
                AlpenEvmPrecompiles::new().precompiles.precompiles,
            ));

        AlpenEvm {
            inner: AlpenEvmInner::new(evm_ctx),
            inspect: false,
        }
    }

    fn create_evm_with_inspector<DB: reth_evm::Database, I: revm::Inspector<Self::Context<DB>>>(
        &self,
        db: DB,
        input: reth_evm::EvmEnv<Self::Spec>,
        inspector: I,
    ) -> Self::Evm<DB, I> {
        let evm_ctx = Context::mainnet()
            .with_db(db)
            .with_cfg(input.cfg_env)
            .with_block(input.block_env)
            .build_mainnet_with_inspector(inspector)
            .with_precompiles(PrecompilesMap::from_static(
                // TODO: Revisit this
                AlpenEvmPrecompiles::new().precompiles.precompiles,
            ));

        AlpenEvm {
            inner: AlpenEvmInner::new(evm_ctx),
            inspect: true,
        }
    }
}

/// Alpen EVM implementation.
#[allow(missing_debug_implementations)]
pub struct AlpenEvm<DB: Database, I, P = AlpenEvmPrecompiles> {
    pub inner:
        AlpenEvmInner<EthEvmContext<DB>, I, EthInstructions<EthInterpreter, EthEvmContext<DB>>, P>,
    pub inspect: bool,
}
impl<DB: Database, I, P> AlpenEvm<DB, I, P> {
    /// Provides a reference to the EVM context.
    pub const fn ctx(&self) -> &EthEvmContext<DB> {
        &self.inner.evm_ctx.ctx
    }

    /// Provides a mutable reference to the EVM context.
    pub fn ctx_mut(&mut self) -> &mut EthEvmContext<DB> {
        &mut self.inner.evm_ctx.ctx
    }
}

impl<DB: Database, I, P> Deref for AlpenEvm<DB, I, P> {
    type Target = EthEvmContext<DB>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.ctx()
    }
}

impl<DB: Database, I, P> DerefMut for AlpenEvm<DB, I, P> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.ctx_mut()
    }
}

/// Implements the AlloyEvm EVM trait for AlpenEvm.
///
/// This implementation closely follows the Alloy EVM implementation for Ethereum mainnet,
/// adapting it for AlpenEvm with custom precompiles and configuration.
/// <https://github.com/alloy-rs/evm/blob/65bdb46726a4cdb265f13e4ea663a57ecf0f8d6c/crates/evm/src/eth/mod.rs#L104>
impl<DB, I, P> Evm for AlpenEvm<DB, I, P>
where
    DB: Database,
    I: Inspector<EthEvmContext<DB>>,
    AlpenEvmPrecompiles: PrecompileProvider<EthEvmContext<DB>, Output = InterpreterResult>,
{
    type DB = DB;
    type Tx = TxEnv;
    type Error = EVMError<DB::Error>;
    type HaltReason = HaltReason;
    type Spec = SpecId;
    type Precompiles = PrecompilesMap;
    type Inspector = I;

    fn block(&self) -> &BlockEnv {
        &self.block
    }

    fn chain_id(&self) -> u64 {
        self.cfg.chain_id
    }

    fn transact_raw(
        &mut self,
        _tx: Self::Tx,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        // if self.inspect {
        //     self.ctx().inspect_tx(tx)
        // } else {
        //     self.inner.evm_ctx.transact(tx)
        // }
        todo!()
    }

    fn transact_system_call(
        &mut self,
        caller: Address,
        contract: Address,
        data: Bytes,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        let tx = TxEnv {
            caller,
            kind: TxKind::Call(contract),
            // Explicitly set nonce to 0 so revm does not do any nonce checks
            nonce: 0,
            gas_limit: 30_000_000,
            value: U256::ZERO,
            data,
            // Setting the gas price to zero enforces that no value is transferred as part of the
            // call, and that the call will not count against the block's gas limit
            gas_price: 0,
            // The chain ID check is not relevant here and is disabled if set to None
            chain_id: None,
            // Setting the gas priority fee to None ensures the effective gas price is derived from
            // the `gas_price` field, which we need to be zero
            gas_priority_fee: None,
            access_list: Default::default(),
            // blob fields can be None for this tx
            blob_hashes: Vec::new(),
            max_fee_per_blob_gas: 0,
            tx_type: 0,
            authorization_list: Default::default(),
        };

        let mut gas_limit = tx.gas_limit;
        let mut basefee = 0;
        let mut disable_nonce_check = true;

        // ensure the block gas limit is >= the tx
        core::mem::swap(&mut self.block.gas_limit, &mut gas_limit);
        // disable the base fee check for this call by setting the base fee to zero
        core::mem::swap(&mut self.block.basefee, &mut basefee);
        // disable the nonce check
        core::mem::swap(&mut self.cfg.disable_nonce_check, &mut disable_nonce_check);

        let mut res = self.transact(tx);

        // swap back to the previous gas limit
        core::mem::swap(&mut self.block.gas_limit, &mut gas_limit);
        // swap back to the previous base fee
        core::mem::swap(&mut self.block.basefee, &mut basefee);
        // swap back to the previous nonce check flag
        core::mem::swap(&mut self.cfg.disable_nonce_check, &mut disable_nonce_check);

        // NOTE: We assume that only the contract storage is modified. Revm currently marks the
        // caller and block beneficiary accounts as "touched" when we do the above transact calls,
        // and includes them in the result.
        //
        // We're doing this state cleanup to make sure that changeset only includes the changed
        // contract storage.
        if let Ok(res) = &mut res {
            res.state.retain(|addr, _| *addr == contract);
        }

        res
    }

    fn db_mut(&mut self) -> &mut Self::DB {
        &mut self.journaled_state.database
    }

    fn finish(self) -> (Self::DB, EvmEnv<Self::Spec>) {
        todo!()
    }

    fn set_inspector_enabled(&mut self, enabled: bool) {
        self.inspect = enabled;
    }

    fn precompiles(&self) -> &Self::Precompiles {
        todo!()
    }

    fn precompiles_mut(&mut self) -> &mut Self::Precompiles {
        todo!()
    }

    fn inspector(&self) -> &Self::Inspector {
        todo!()
    }

    fn inspector_mut(&mut self) -> &mut Self::Inspector {
        todo!()
    }
}
