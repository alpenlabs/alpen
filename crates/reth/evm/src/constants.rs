use revm::primitives::{address, Address, U256};

use crate::utils::{u256_from, WEI_PER_BTC};

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

/// The fixed withdrawal amount in wei (10 BTC equivalent).
pub const FIXED_WITHDRAWAL_WEI: U256 = u256_from(10 * WEI_PER_BTC);

/// The address to send transaction basefee to instead of burning.
pub const BASEFEE_ADDRESS: Address = address!("5400000000000000000000000000000000000010");

/// The address to send transaction priority fees to.
pub const COINBASE_ADDRESS: Address = address!("5400000000000000000000000000000000000011");
