//! OL DA payload and state diff types.
//!
//! This module is organized into sub-modules for different concerns:
//! - [`payload`]: Top-level DA payload types (OLDaPayloadV1, OLStateDiffV1, OLStateDiffWriterV1)
//! - [`global`]: Global state diff types (GlobalStateDiffV1)
//! - [`ledger`]: Ledger diff types (LedgerDiffV1, NewAccountEntryV1, AccountInitV1)
//! - [`account`]: Account diff types (AccountDiffV1)
//! - [`snark`]: Snark account diff types (SnarkAccountDiffV1)
//! - [`inbox`]: Inbox message buffer types (DaMessageEntryV1, InboxBufferV1)
//! - [`v0`]: Decoding of V0 payloads, by the V0 rules

mod account;
mod global;
mod inbox;
mod ledger;
mod payload;
mod snark;
mod v0;

// Re-export all public types for API stability
pub use account::{AccountDiffTargetV1, AccountDiffV1};
pub use global::{GlobalStateDiffV1, GlobalStateTargetV1};
pub use inbox::{DaMessageEntryV1, InboxBufferV1};
pub use ledger::{
    AccountDiffEntryV1, AccountInitV1, AccountTypeInitV1, LedgerDiffV1, NewAccountEntryV1,
    SnarkAccountInitV1,
};
pub use payload::{OLDaPayloadV1, OLStateDiffV1, OLStateDiffWriterV1, decode_ol_da_payload_bytes};
pub use snark::{DaProofStateDiffV1, DaProofStateV1, SnarkAccountDiffV1, SnarkAccountTargetV1};
pub use v0::decode_v0_payload_bytes;

/// Maximum size for snark account update VK in DA payloads.
///
/// The DA encoding uses a big-endian u16 length prefix, so the maximum
/// representable length is `u16::MAX` (65_535 bytes).
pub const MAX_VK_BYTES: usize = (1 << 16) - 1;

/// Maximum size for a single message payload (4 KiB per SPS-ol-da-structure).
pub const MAX_MSG_PAYLOAD_BYTES: usize = 4 * 1024;

#[cfg(test)]
mod tests {
    use strata_acct_types::{AccountId, BitcoinAmount, MAX_MSG_PAYLOAD_DATA_BYTES, MsgPayload};
    use strata_codec::{Codec, CodecError, decode_buf_exact, encode_to_vec};

    use super::*;

    fn message_entry_with_payload_len(len: usize) -> DaMessageEntryV1 {
        let payload = MsgPayload::from_bytes(
            BitcoinAmount::try_from(42).expect("amount must not exceed the Bitcoin money supply"),
            vec![0x5a; len],
        )
        .expect("message payload bytes must fit within SSZ max length");
        DaMessageEntryV1::new(AccountId::from([0x11; 32]), 7, payload)
    }

    #[test]
    fn test_da_message_entry_encode_rejects_oversize_payload_without_writing() {
        for len in [
            MAX_MSG_PAYLOAD_BYTES + 1,
            MAX_MSG_PAYLOAD_DATA_BYTES as usize,
        ] {
            let entry = message_entry_with_payload_len(len);
            let mut encoded = vec![0xa5];
            assert!(matches!(
                entry.encode(&mut encoded),
                Err(CodecError::OverflowContainer)
            ));
            assert_eq!(encoded, [0xa5]);
        }
    }

    #[test]
    fn test_da_message_entry_decode_rejects_oversize_payload() {
        let entry = message_entry_with_payload_len(MAX_MSG_PAYLOAD_BYTES + 1);
        // Build the invalid wire input without using the entry's checked encoder.
        let mut encoded = encode_to_vec(&entry.source).unwrap();
        entry.incl_epoch.encode(&mut encoded).unwrap();
        entry.payload.encode(&mut encoded).unwrap();
        assert!(matches!(
            decode_buf_exact::<DaMessageEntryV1>(&encoded),
            Err(CodecError::OverflowContainer)
        ));
    }

    #[test]
    fn test_da_message_entry_round_trips_payload_boundaries() {
        for len in [0, MAX_MSG_PAYLOAD_BYTES] {
            let entry = message_entry_with_payload_len(len);
            let encoded = encode_to_vec(&entry).unwrap();
            assert_eq!(
                decode_buf_exact::<DaMessageEntryV1>(&encoded).unwrap(),
                entry
            );
        }
    }

    #[test]
    fn test_da_message_entry_empty_payload_wire_vector() {
        let entry = message_entry_with_payload_len(0);
        let mut expected = vec![0x11; 32];
        expected.extend_from_slice(&[0, 0, 0, 7]);
        expected.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 42]);
        expected.push(0);
        assert_eq!(encode_to_vec(&entry).unwrap(), expected);
        assert_eq!(
            decode_buf_exact::<DaMessageEntryV1>(&expected).unwrap(),
            entry
        );
    }
}
