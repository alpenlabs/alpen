use std::sync::OnceLock;

use revm::{
    context::{Cfg, ContextTr},
    handler::{EthPrecompiles, PrecompileProvider},
    interpreter::{Gas, InputsImpl, InstructionResult, InterpreterResult},
    precompile::{bls12_381, PrecompileError, Precompiles},
};
use revm_primitives::{hardfork::SpecId, Address, Bytes};

use crate::{constants::BRIDGEOUT_ADDRESS, precompiles::bridge::bridge_context_call};

mod bridge;
mod schnorr;

/// A custom precompile that contains static precompiles.
#[allow(missing_debug_implementations)]
#[derive(Clone, Default)]
pub struct AlpenEvmPrecompiles {
    pub inner: EthPrecompiles,
}

impl AlpenEvmPrecompiles {
    #[inline]
    pub fn new(spec: SpecId) -> Self {
        let precompiles = load_precompiles();
        Self {
            inner: EthPrecompiles { precompiles, spec },
        }
    }

    #[inline]
    pub fn precompiles(&self) -> &'static Precompiles {
        self.inner.precompiles
    }
}

impl<CTX: ContextTr> PrecompileProvider<CTX> for AlpenEvmPrecompiles {
    type Output = InterpreterResult;

    #[inline]
    fn set_spec(&mut self, spec: <CTX::Cfg as Cfg>::Spec) -> bool {
        *self = Self::new(spec.into());
        true
    }

    #[inline]
    fn run(
        &mut self,
        context: &mut CTX,
        address: &Address,
        inputs: &InputsImpl,
        _is_static: bool,
        gas_limit: u64,
    ) -> Result<Option<Self::Output>, String> {
        let Some(precompile_fn) = self.inner.precompiles.get(address) else {
            return Ok(None);
        };

        let raw_input_bytes = inputs.input.bytes(context);
        let raw_input = raw_input_bytes.as_ref();

        let mut result = InterpreterResult {
            result: InstructionResult::Return,
            gas: Gas::new(gas_limit),
            output: Bytes::new(),
        };

        let res = match *address {
            BRIDGEOUT_ADDRESS => bridge_context_call(raw_input, gas_limit, context),
            _ => (precompile_fn)(raw_input, gas_limit),
        };

        match res {
            Ok(output) => {
                let underflow = result.gas.record_cost(output.gas_used);
                assert!(underflow, "Gas underflow is not possible");
                result.output = output.bytes;
            }
            Err(PrecompileError::Fatal(e)) => return Err(e),
            Err(e) => {
                result.result = if e.is_oog() {
                    InstructionResult::PrecompileOOG
                } else {
                    InstructionResult::PrecompileError
                };
            }
        }

        Ok(Some(result))
    }

    #[inline]
    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        self.inner.warm_addresses()
    }

    #[inline]
    fn contains(&self, address: &Address) -> bool {
        self.inner.contains(address)
    }
}

/// Returns precompiles for the spec.
pub fn load_precompiles() -> &'static Precompiles {
    static INSTANCE: OnceLock<Precompiles> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        let mut precompiles = Precompiles::berlin().clone();

        // EIP-2537: Precompile for BLS12-381
        precompiles.extend(bls12_381::precompiles());

        // Custom precompile.
        precompiles.extend([
            schnorr::SCHNORR_SIGNATURE_VALIDATION,
            bridge::BRIDGEOUT_PRECOMPILE,
        ]);
        precompiles
    })
}
