//! Account message types.

// Include SSZ type definitions from acct-ssz-types
// This brings in: MsgPayload, MsgPayloadData, SentMessage, ReceivedMessage
include!("../../acct-ssz-types/src/messages.rs");

// Business logic for SentMessage
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

// Business logic for ReceivedMessage
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

// Business logic for MsgPayload
impl MsgPayload {
    pub fn new(value: BitcoinAmount, data: impl Into<MsgPayloadData>) -> Self {
        Self {
            value,
            data: data.into(),
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

    use super::*;
    use crate::BitcoinAmount;

    #[test]
    fn test_msg_payload_ssz_roundtrip() {
        let payload = MsgPayload::new(BitcoinAmount::from_sat(1000), vec![1, 2, 3]);
        let encoded = payload.as_ssz_bytes();
        let decoded = MsgPayload::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(payload, decoded);
    }

    #[test]
    fn test_sent_message_ssz_roundtrip() {
        let msg = SentMessage {
            dest: AccountId([42u8; 32]),
            payload: MsgPayload::new(BitcoinAmount::from_sat(500), vec![4, 5, 6]),
        };
        let encoded = msg.as_ssz_bytes();
        let decoded = SentMessage::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_received_message_ssz_roundtrip() {
        let msg = ReceivedMessage {
            source: AccountId([99u8; 32]),
            payload: MsgPayload::new(BitcoinAmount::from_sat(750), vec![7, 8, 9]),
        };
        let encoded = msg.as_ssz_bytes();
        let decoded = ReceivedMessage::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(msg, decoded);
    }
}
