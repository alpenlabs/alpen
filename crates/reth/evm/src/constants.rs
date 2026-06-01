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

/// The address to send transaction basefee to instead of burning.
pub const BASEFEE_ADDRESS: Address = address!("0xdECAf4F0106935C225ab91Ec33EC8F77da42659d");

/// The address to send transaction priority fees to.
pub const COINBASE_ADDRESS: Address = address!("0xdECAf4F0106935C225ab91Ec33EC8F77da42659d");
