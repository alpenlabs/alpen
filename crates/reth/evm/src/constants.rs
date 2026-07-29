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

/// The address for the inter-EE subject-transfer precompile contract.
pub const SUBJECT_TRANSFER_PRECOMPILE_ADDRESS: Address =
    address!("5400000000000000000000000000000000000003");

/// Custom PrecompileId for the inter-EE subject-transfer precompile contract.
pub const SUBJECT_TRANSFER_PRECOMPILE_ID: &str = "alpen-subject-transfer-precompile";
