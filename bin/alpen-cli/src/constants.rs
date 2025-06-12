use std::time::Duration;

use alloy::consensus::constants::ETH_TO_WEI;
use bdk_wallet::bitcoin::{bip32::ChildNumber, Amount, Network};
use strata_primitives::constants::RECOVER_DELAY;

/// Number of blocks that the wallet considers a transaction "buried" or final taking into account
/// reorgs that might happen.
pub const FINALITY_DEPTH: u32 = 6;

/// Number of blocks after which the wallet actually enables recovery. This is mostly to account
/// for any reorgs that may happen at the recovery height.
pub const RECOVER_AT_DELAY: u32 = RECOVER_DELAY + FINALITY_DEPTH;

pub const RECOVERY_DESC_CLEANUP_DELAY: u32 = 100;

/// 10 BTC + 1,000 satoshi to cover fees in the following transaction where the bridge spends it
/// into the federation.
pub const BRIDGE_IN_AMOUNT: Amount = Amount::from_sat(1_000_001_000);

/// Bridge outs are enforced to be exactly 10 BTC
pub const BRIDGE_OUT_AMOUNT: Amount = Amount::from_int_btc(10);

pub const BTC_TO_WEI: u128 = ETH_TO_WEI;
pub const SATS_TO_WEI: u128 = BTC_TO_WEI / 100_000_000;

/// Length of salt used for password hashing
pub const PW_SALT_LEN: usize = 16;
/// Length of nonce in bytes
pub const AES_NONCE_LEN: usize = 12;
/// Length of seed in bytes
pub const SEED_LEN: usize = 16;
/// AES-256-GCM-SIV tag len
pub const AES_TAG_LEN: usize = 16;

pub const DEFAULT_NETWORK: Network = Network::Signet;
pub const BRIDGE_ALPEN_ADDRESS: &str = "0x5400000000000000000000000000000000000001";
pub const SIGNET_BLOCK_TIME: Duration = Duration::from_secs(10 * 60); // 10 minutes

/// Alpen CLI [`DerivationPath`](bdk_wallet::bitcoin::bip32::DerivationPath) for Alpen EVM wallet
///
/// This corresponds to the path: `m/44'/60'/0'/0/0`.
pub const BIP44_ALPEN_EVM_WALLET_PATH: &[ChildNumber] = &[
    // Purpose index for HD wallets.
    ChildNumber::Hardened { index: 44 },
    // Coin type index for Ethereum mainnet
    ChildNumber::Hardened { index: 60 },
    // Account index for user wallets.
    ChildNumber::Hardened { index: 0 },
    // Change index for receiving (external) addresses.
    ChildNumber::Normal { index: 0 },
    // Address index.
    ChildNumber::Normal { index: 0 },
];
