use std::sync::OnceLock;

use revm::{
    handler::EthPrecompiles,
    precompile::{bls12_381, Precompiles},
};
use revm_primitives::hardfork::SpecId;

mod bridge;
pub mod factory;
mod schnorr;

/// A custom precompile that contains static precompiles.
#[expect(
    missing_debug_implementations,
    reason = "Precompiles struct contains static precompiles that don't need debug implementation"
)]
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

/// Returns precompiles for the spec.
pub fn load_precompiles() -> &'static Precompiles {
    static INSTANCE: OnceLock<Precompiles> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        // Alpen EVM supports all Ethereum precompiles up to the Pectra fork.
        // However, we want to disable the point evaluation precompile introduced in the Cancun
        // fork. Therefore, we start with the Berlin precompiles and manually add the ones
        // needed for Pectra.
        let mut precompiles = Precompiles::berlin().clone();

        // EIP-2537: Precompile for BLS12-381
        precompiles.extend(bls12_381::precompiles());

        // Custom precompile.
        precompiles.extend([schnorr::SCHNORR_SIGNATURE_VALIDATION]);

        precompiles
    })
}
