//! OL DA payload and state diff types.
//!
//! This module is organized into sub-modules for different concerns:
//! - [`encoding`]: Common encoding types (U16LenBytes, U16LenList)
//! - [`payload`]: Top-level DA payload types (OLDaPayloadV1, StateDiff, OLStateDiff)
//! - [`ledger`]: Ledger diff types (LedgerDiff, NewAccountEntry, AccountInit)
//! - [`snark`]: Snark account diff types (SnarkAccountDiff, DaProofState)
mod encoding;
mod ledger;
mod payload;
mod snark;

// Re-export all public types for API stability
pub use encoding::{U16LenBytes, U16LenList};
pub use ledger::{
    AccountDiffEntry, AccountInit, AccountTypeInit, LedgerDiff, NewAccountEntry, SnarkAccountInit,
};
pub use payload::{OLDaPayloadV1, OLStateDiff, StateDiff};
pub use snark::{DaProofState, SnarkAccountDiff, SnarkAccountTarget};

/// Maximum size for snark account update VK (64 KiB per SPS-ol-chain-structures and
/// SPS-ol-da-structure).
pub const MAX_VK_BYTES: usize = 64 * 1024;

/// Maximum size for a single message payload (4 KiB per SPS-ol-da-structure).
pub const MAX_MSG_PAYLOAD_BYTES: usize = 4 * 1024;

