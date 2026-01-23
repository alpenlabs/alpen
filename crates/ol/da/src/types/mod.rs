//! OL DA payload and state diff types.
//!
//! This module is organized into sub-modules for different concerns:
//! - [`encoding`]: Common encoding types (U16LenBytes, U16LenList)
//! - [`payload`]: Top-level DA payload types (OLDaPayloadV1, StateDiff, OLStateDiff)
//! - [`global`]: Global state diff types (GlobalStateDiff)
//! - [`ledger`]: Ledger diff types (LedgerDiff, NewAccountEntry, AccountInit)
//! - [`account`]: Account diff types (AccountDiff)
//! - [`snark`]: Snark account diff types (SnarkAccountDiff, DaProofState)
//! - [`inbox`]: Inbox message buffer types (DaMessageEntry, InboxBuffer)

mod account;
mod encoding;
mod global;
mod inbox;
mod ledger;
mod payload;
mod snark;

// Re-export all public types for API stability
pub use account::{AccountDiff, AccountDiffTarget};
pub use encoding::{U16LenBytes, U16LenList};
pub use global::{GlobalStateDiff, GlobalStateTarget};
pub use inbox::{DaMessageEntry, InboxBuffer};
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

#[cfg(test)]
mod tests {
    use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload};
    use strata_codec::{decode_buf_exact, encode_to_vec};

    use super::*;

    #[test]
    fn test_da_message_entry_decode_rejects_oversize_payload() {
        let payload = MsgPayload::new(
            BitcoinAmount::from_sat(0),
            vec![0u8; MAX_MSG_PAYLOAD_BYTES + 1],
        );
        let entry = DaMessageEntry {
            source: AccountId::zero(),
            incl_epoch: 0,
            payload,
        };

        let encoded = encode_to_vec(&entry).expect("encode da message entry");
        let decoded: Result<DaMessageEntry, _> = decode_buf_exact(&encoded);
        assert!(decoded.is_err());
    }
}
