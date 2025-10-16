//! Account message types.

use ssz_derive::{Decode, Encode};
use ssz_types::VariableList;
use tree_hash_derive::TreeHash as TreeHashDerive;

use crate::{amount::BitcoinAmount, id::AccountId};

/// Variable-length byte list for message payload data (max 1 MiB)
type MsgPayloadData = VariableList<u8, 1048576>;

/// Describes a message we're getting ready to send.
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TreeHashDerive)]
pub struct SentMessage {
    /// Destination orchestration layer account ID.
    dest: AccountId,

    /// Message payload.
    payload: MsgPayload,
}

impl SentMessage {
    pub fn new(dest: AccountId, payload: MsgPayload) -> Self {
        Self { dest, payload }
    }

    pub fn dest(&self) -> AccountId {
        self.dest
    }

    pub fn payload(&self) -> &MsgPayload {
        &self.payload
    }
}

/// Describes a message being received by an account.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceivedMessage {
    source: AccountId,
    payload: MsgPayload,
}

impl ReceivedMessage {
    pub fn new(source: AccountId, payload: MsgPayload) -> Self {
        Self { source, payload }
    }

    pub fn source(&self) -> AccountId {
        self.source
    }

    pub fn payload(&self) -> &MsgPayload {
        &self.payload
    }
}

/// Contents of a message, ie the data and sent value payload components.
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TreeHashDerive)]
pub struct MsgPayload {
    value: BitcoinAmount,
    data: MsgPayloadData,
}

impl MsgPayload {
    pub fn new(value: BitcoinAmount, data: Vec<u8>) -> Self {
        Self {
            value,
            data: MsgPayloadData::from(data),
        }
    }

    pub fn value(&self) -> BitcoinAmount {
        self.value
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

#[cfg(test)]
mod tests {
    use ssz::{Decode, Encode};
    use tree_hash::TreeHash;

    use super::*;

    #[test]
    fn test_msg_payload_ssz_roundtrip() {
        let payload = MsgPayload::new(BitcoinAmount::from(1000), vec![1, 2, 3, 4]);
        let encoded = payload.as_ssz_bytes();
        let decoded = MsgPayload::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(payload, decoded);
    }

    #[test]
    fn test_msg_payload_tree_hash() {
        let payload1 = MsgPayload::new(BitcoinAmount::from(1000), vec![1, 2, 3, 4]);
        let payload2 = MsgPayload::new(BitcoinAmount::from(1000), vec![1, 2, 3, 4]);
        assert_eq!(payload1.tree_hash_root(), payload2.tree_hash_root());
    }

    #[test]
    fn test_msg_payload_empty_data() {
        let payload = MsgPayload::new(BitcoinAmount::zero(), vec![]);
        let encoded = payload.as_ssz_bytes();
        let decoded = MsgPayload::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(payload, decoded);
        assert_eq!(decoded.data(), &[]);
    }

    #[test]
    fn test_sent_message_ssz_roundtrip() {
        let account_id = AccountId::from([1u8; 32]);
        let payload = MsgPayload::new(BitcoinAmount::from(5000), vec![10, 20, 30]);
        let msg = SentMessage::new(account_id, payload);

        let encoded = msg.as_ssz_bytes();
        let decoded = SentMessage::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_sent_message_tree_hash() {
        let account_id = AccountId::from([1u8; 32]);
        let payload = MsgPayload::new(BitcoinAmount::from(5000), vec![10, 20, 30]);
        let msg1 = SentMessage::new(account_id, payload.clone());
        let msg2 = SentMessage::new(account_id, payload);
        assert_eq!(msg1.tree_hash_root(), msg2.tree_hash_root());
    }

    #[test]
    fn test_msg_payload_large_data() {
        let large_data = vec![42u8; 10000];
        let payload = MsgPayload::new(BitcoinAmount::from(9999), large_data.clone());
        let encoded = payload.as_ssz_bytes();
        let decoded = MsgPayload::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(payload, decoded);
        assert_eq!(decoded.data(), &large_data[..]);
    }
}
