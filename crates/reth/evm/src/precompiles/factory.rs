use reth_evm::precompiles::{DynPrecompile, PrecompilesMap};
use revm::precompile::PrecompileId;
use revm_primitives::hardfork::SpecId;
use strata_bridge_params::BridgeParams;

use crate::{
    constants::{BRIDGEOUT_PRECOMPILE_ADDRESS, BRIDGEOUT_PRECOMPILE_ID},
    precompiles::{bridge::bridge_context_call, AlpenEvmPrecompiles},
};

/// Creates a precompiles map with Alpen-specific precompiles, including the bridge precompile.
pub fn create_precompiles_map(spec: SpecId, bridge_params: BridgeParams) -> PrecompilesMap {
    let mut precompiles = PrecompilesMap::from_static(AlpenEvmPrecompiles::new(spec).precompiles());

    // Add bridge precompile using DynPrecompile for compatibility.
    // The closure captures withdrawal params so the precompile can validate amounts.
    precompiles.apply_precompile(&BRIDGEOUT_PRECOMPILE_ADDRESS, |_| {
        Some(DynPrecompile::new_stateful(
            PrecompileId::custom(BRIDGEOUT_PRECOMPILE_ID),
            move |input| bridge_context_call(input, bridge_params),
        ))
    });

    precompiles
}
