use std::time::Duration;

use alloy::consensus::constants::ETH_TO_WEI;
use bdk_wallet::bitcoin::{bip32::ChildNumber, Amount, Network};
use strata_identifiers::{AccountSerial, SYSTEM_RESERVED_ACCTS};

/// Number of blocks that the wallet considers a transaction "buried" or final taking into account
/// reorgs that might happen.
pub const DEFAULT_FINALITY_DEPTH: u32 = 6;

pub const RECOVERY_DESC_CLEANUP_DELAY: u32 = 100;

/// Number of consecutive unused reclaim-key counters `recover --from-seed` tries before giving
/// up. There's no persisted "last used counter" to resume from when reconstructing purely from the
/// seed, so this is the only signal for when to stop scanning.
pub const SEED_RECOVERY_GAP_LIMIT: u32 = 1_000;

/// Fee to cover the mining fees for creating the deposit transaction from the deposit request
/// transaction. This includes the cost for the bridge to spend the deposit request output into the
/// federation.
pub const DEFAULT_BRIDGE_FEE: Amount = Amount::from_sat(1_000);

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
/// OP_RETURN magic bytes len
pub const MAGIC_BYTES_LEN: usize = 4;

pub const DEFAULT_NETWORK: Network = Network::Signet;
pub const DEFAULT_BRIDGE_ALPEN_ADDRESS: &str = "0x5400000000000000000000000000000000000001";
pub const SIGNET_BLOCK_TIME: Duration = Duration::from_secs(10 * 60); // 10 minutes

/// Serial of the Alpen EE account used in deposit descriptors.
///
/// System serials occupy `0..SYSTEM_RESERVED_ACCTS`, so the Alpen EE account
/// currently lands at `SYSTEM_RESERVED_ACCTS` by genesis registration order.
pub const ALPEN_EE_ACCT_SERIAL: AccountSerial = AccountSerial::new(SYSTEM_RESERVED_ACCTS);

/// Hardened branch reserved for Alpen CLI deposit-request reclaim keys.
///
/// Unregistered — picked above the 10001-19999 range BIP43 reserves for SLIPs, so it can't
/// collide with anything registered.
///
/// Separates this key material from the wallet's BIP-86 path (`m/86'/0'/0'`). Each deposit derives
/// `m/<DRT_RECLAIM_PURPOSE>'/<counter>'`, where `counter` is durable local state (see
/// [`DescriptorRecovery::next_reclaim_counter`](crate::recovery::DescriptorRecovery::next_reclaim_counter)),
/// making the reclaim key recoverable from the seed alone rather than only from the descriptor DB.
///
/// Don't change this. A deposit's reclaim key can only be reconstructed from the seed if this
/// value is still the same as when that deposit was made.
pub const DRT_RECLAIM_PURPOSE: ChildNumber = ChildNumber::Hardened { index: 43_000 };

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
