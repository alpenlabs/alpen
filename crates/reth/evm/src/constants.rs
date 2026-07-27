use revm::primitives::{address, Address};

/// The address for the Bridgeout precompile contract.
pub const BRIDGEOUT_PRECOMPILE_ADDRESS: Address =
    address!("5400000000000000000000000000000000000001");

/// Custom PrecompileId for the Bridgeout precompile contract.
pub const BRIDGEOUT_PRECOMPILE_ID: &str = "alpen-bridgeout-precompile";

/// The address for the Schnorr precompile contract.
pub const SCHNORR_PRECOMPILE_ADDRESS: Address =
    address!("5400000000000000000000000000000000000002");

/// Custom PrecompileId for the Schnorr precompile contract.
pub const SCHNORR_PRECOMPILE_PRECOMPILE_ID: &str = "alpen-schnorr-precompile";

/// The address of the DA fee vault predeploy.
///
/// Per-transaction Bitcoin data-availability fees are credited here as a direct balance
/// mutation. It is an ordinary genesis account, so its balance is proven EVM state.
pub const DA_FEE_VAULT_ADDRESS: Address = address!("5400000000000000000000000000000000000003");
