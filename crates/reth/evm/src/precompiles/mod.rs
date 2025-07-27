use revm::{
    context::{Cfg, ContextTr},
    handler::{EthPrecompiles, PrecompileProvider},
    interpreter::{InputsImpl, InterpreterResult},
    precompile::Precompiles,
};
use revm_primitives::{hardfork::SpecId, Address};

/// A custom precompile that contains static precompiles.
#[allow(missing_debug_implementations)]
#[derive(Clone, Default)]
pub struct AlpenEvmPrecompiles {
    pub inner: EthPrecompiles,
}

impl AlpenEvmPrecompiles {
    /// Given a [`PrecompileProvider`] and cache for a specific precompiles, create a
    /// wrapper that can be used inside Evm.
    #[inline]
    pub fn new(_spec: SpecId) -> Self {
        Self::default()
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
        is_static: bool,
        gas_limit: u64,
    ) -> Result<Option<Self::Output>, String> {
        self.inner
            .run(context, address, inputs, is_static, gas_limit)
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
