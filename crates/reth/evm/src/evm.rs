use std::sync::OnceLock;

use revm::{
    context::{Cfg, ContextTr},
    handler::{EthPrecompiles, PrecompileProvider},
    interpreter::{Gas, InputsImpl, InstructionResult, InterpreterResult},
    precompile::{PrecompileError, PrecompileFn, Precompiles},
};
use revm_primitives::{Address, Bytes};

use crate::{
    constants::{BRIDGEOUT_ADDRESS, SCHNORR_ADDRESS},
    precompiles::{
        bridge::{bridge_context_call, bridgeout_precompile},
        schnorr::verify_schnorr_precompile,
    },
};

/// A custom precompile that contains static precompiles.
#[derive(Clone, Default)]
pub struct StrataEvmPrecompiles {
    pub precompiles: EthPrecompiles,
}

impl StrataEvmPrecompiles {
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
        // Custom precompile.
        precompiles.extend([
            (SCHNORR_ADDRESS, verify_schnorr_precompile as PrecompileFn).into(),
            (BRIDGEOUT_ADDRESS, bridgeout_precompile as PrecompileFn).into(),
        ]);
        precompiles
    })
}

impl<CTX: ContextTr> PrecompileProvider<CTX> for StrataEvmPrecompiles {
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
        _context: &mut CTX,
        address: &Address,
        inputs: &InputsImpl,
        _is_static: bool,
        gas_limit: u64,
    ) -> Result<Option<Self::Output>, String> {
        let Some(precompile) = self.precompiles.precompiles.get(address) else {
            return Ok(None);
        };

        let mut result = InterpreterResult {
            result: InstructionResult::Return,
            gas: Gas::new(gas_limit),
            output: Bytes::new(),
        };

        if *address == BRIDGEOUT_ADDRESS {
            let res = bridge_context_call(&inputs.input, gas_limit, _context);
            match res {
                Ok(output) => {
                    let underflow = result.gas.record_cost(output.gas_used);
                    assert!(underflow, "Gas underflow is not possible");
                    result.result = InstructionResult::Return;
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
            return Ok(Some(result));
        }

        match (*precompile)(&inputs.input, gas_limit) {
            Ok(output) => {
                let underflow = result.gas.record_cost(output.gas_used);
                assert!(underflow, "Gas underflow is not possible");
                result.result = InstructionResult::Return;
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

    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        self.precompiles.warm_addresses()
    }

    fn contains(&self, address: &Address) -> bool {
        self.precompiles.contains(address)
    }
}
