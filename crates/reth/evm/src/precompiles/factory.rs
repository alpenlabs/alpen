use reth_evm::precompiles::{DynPrecompile, PrecompilesMap};
use revm::precompile::PrecompileId;
use revm_primitives::hardfork::SpecId;

use crate::{
    constants::BRIDGEOUT_ADDRESS,
    precompiles::{bridge::bridge_context_call, AlpenEvmPrecompiles},
};

/// Creates a precompiles map with Alpen-specific precompiles, including the bridge precompile.
pub fn create_precompiles_map(spec: SpecId) -> PrecompilesMap {
    let mut precompiles = PrecompilesMap::from_static(AlpenEvmPrecompiles::new(spec).precompiles());

    // Add bridge precompile using DynPrecompile for compatibility
    precompiles.apply_precompile(&BRIDGEOUT_ADDRESS, |_| {
        Some(DynPrecompile::new(
            PrecompileId::custom("bridge-precompile"),
            bridge_context_call,
        ))
    });

    precompiles
}
