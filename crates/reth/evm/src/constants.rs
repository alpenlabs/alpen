use revm::primitives::{address, Address, U256};

use crate::utils::{u256_from, WEI_PER_BTC};

/// The address for the Bridgeout precompile contract.
pub const BRIDGEOUT_ADDRESS: Address = address!("5400000000000000000000000000000000000001");

/// The fixed withdrawal amount in wei (10 BTC equivalent).
pub const FIXED_WITHDRAWAL_WEI: U256 = u256_from(10 * WEI_PER_BTC);

/// The address to send transaction basefee to instead of burning.
pub const BASEFEE_ADDRESS: Address = address!("5400000000000000000000000000000000000010");

/// The address to send transaction priority fees to.
pub const COINBASE_ADDRESS: Address = address!("5400000000000000000000000000000000000011");
