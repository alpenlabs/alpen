use reth_evm::precompiles::{DynPrecompile, PrecompilesMap};
use revm::precompile::PrecompileId;
use revm_primitives::hardfork::SpecId;

use crate::{
    constants::{BRIDGEOUT_PRECOMPILE_ADDRESS, BRIDGEOUT_PRECOMPILE_ID},
    precompiles::{bridge::bridge_context_call, AlpenEvmPrecompiles},
};

/// Creates a precompiles map with Alpen-specific precompiles, including the bridge precompile.
pub fn create_precompiles_map(spec: SpecId) -> PrecompilesMap {
    let mut precompiles = PrecompilesMap::from_static(AlpenEvmPrecompiles::new(spec).precompiles());

    // Add bridge precompile using DynPrecompile for compatibility
    precompiles.apply_precompile(&BRIDGEOUT_PRECOMPILE_ADDRESS, |_| {
        Some(DynPrecompile::new_stateful(
            PrecompileId::custom(BRIDGEOUT_PRECOMPILE_ID),
            bridge_context_call,
        ))
    });

    precompiles
}
