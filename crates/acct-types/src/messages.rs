//! Account message types.

use ssz_types::VariableList;
use strata_codec::{Codec, CodecError, Decoder, Encoder, Varint};
use strata_identifiers::Buf32;
use tree_hash::TreeHash;

use crate::{
    AccountId, BitcoinAmount, SentTransfer,
    ssz_generated::ssz::messages::{MessageEntry, MsgPayload, ReceivedMessage, SentMessage},
};

impl SentMessage {
    pub fn new(dest: AccountId, payload: MsgPayload) -> Self {
        Self { dest, payload }
    }

    /// Creates a new instance with empty data and some value.
    pub fn new_dataless(dest: AccountId, value: BitcoinAmount) -> Self {
        Self::new(dest, MsgPayload::new_dataless(value))
    }

    /// Creates a new instance with empty data and 0 value.
    pub fn new_empty(dest: AccountId) -> Self {
        Self::new_dataless(dest, BitcoinAmount::zero())
    }

    pub fn dest(&self) -> AccountId {
        self.dest
    }

    pub fn payload(&self) -> &MsgPayload {
        &self.payload
    }
}

impl SentTransfer {
    pub fn new(dest: AccountId, value: BitcoinAmount) -> Self {
        Self { dest, value }
    }

    /// Creates a new instance with 0 value.
    ///
    /// This shouldn't normally happen but it's a convenience.
    pub fn zero(dest: AccountId) -> Self {
        Self::new(dest, BitcoinAmount::zero())
    }

    pub fn dest(&self) -> AccountId {
        self.dest
    }

    pub fn value(&self) -> BitcoinAmount {
        self.value
    }
}

impl ReceivedMessage {
    pub fn new(source: AccountId, payload: MsgPayload) -> Self {
        Self { source, payload }
    }

    /// Creates a new instance with empty data and some value.
    pub fn new_dataless(dest: AccountId, value: BitcoinAmount) -> Self {
        Self::new(dest, MsgPayload::new_dataless(value))
    }

    /// Creates a new instance with empty data and 0 value.
    pub fn new_empty(dest: AccountId) -> Self {
        Self::new_dataless(dest, BitcoinAmount::zero())
    }

    pub fn source(&self) -> AccountId {
        self.source
    }

    pub fn payload(&self) -> &MsgPayload {
        &self.payload
    }
}

impl MsgPayload {
    pub fn new(value: BitcoinAmount, data: Vec<u8>) -> Self {
        Self {
            value,
            // FIXME size limits
            data: data.into(),
        }
    }

    /// Creates a new instance with empty data and some value.
    pub fn new_dataless(value: BitcoinAmount) -> Self {
        Self::new(value, Vec::new())
    }

    /// Creates a new instance with some data and 0 value.
    pub fn new_valueless(data: Vec<u8>) -> Self {
        Self::new(BitcoinAmount::zero(), data)
    }

    /// Creates a new instance with empty data and 0 value.
    pub fn new_empty() -> Self {
        Self::new_dataless(BitcoinAmount::zero())
    }

    pub fn value(&self) -> BitcoinAmount {
        self.value
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Wraps the payload into a [`SentMessage`] to some dest account.
    pub fn into_sent(self, dest: AccountId) -> SentMessage {
        SentMessage::new(dest, self)
    }
}

impl Codec for MsgPayload {
    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let value = BitcoinAmount::decode(dec)?;

        let len_vi = Varint::decode(dec)?;
        let mut buf = vec![0; len_vi.inner() as usize];
        dec.read_buf(&mut buf)?;
        let data = VariableList::new(buf).map_err(|_| CodecError::OverflowContainer)?;

        Ok(Self { data, value })
    }

    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.value.encode(enc)?;

        let len_vi = Varint::new_usize(self.data.len()).ok_or(CodecError::OverflowContainer)?;
        len_vi.encode(enc)?;
        enc.write_buf(&self.data)?;

        Ok(())
    }
}

impl MessageEntry {
    /// Creates a new message entry.
    pub fn new(source: AccountId, incl_epoch: u32, payload: MsgPayload) -> Self {
        Self {
            source,
            incl_epoch,
            payload,
        }
    }

    /// Gets the source account ID.
    pub fn source(&self) -> AccountId {
        self.source
    }

    /// Gets the inclusion epoch.
    pub fn incl_epoch(&self) -> u32 {
        self.incl_epoch
    }

    /// Gets the message payload.
    pub fn payload(&self) -> &MsgPayload {
        &self.payload
    }

    /// Gets the data payload buf.
    pub fn payload_buf(&self) -> &[u8] {
        self.payload().data()
    }

    /// Gets the payload value.
    pub fn payload_value(&self) -> BitcoinAmount {
        self.payload().value()
    }

    /// Computes the commitment that we store in the MMR accumulator.
    pub fn compute_msg_commitment(&self) -> Buf32 {
        <Self as TreeHash>::tree_hash_root(self).0.into()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use ssz::{Decode, Encode};
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;

    mod msg_payload {
        use super::*;

        ssz_proptest!(
            MsgPayload,
            (any::<u64>(), prop::collection::vec(any::<u8>(), 0..100)).prop_map(|(sats, data)| {
                MsgPayload {
                    value: BitcoinAmount::from_sat(sats),
                    data: data.into(),
                }
            })
        );

        #[test]
        fn test_zero_ssz() {
            let payload = MsgPayload::new(BitcoinAmount::from_sat(0), vec![]);
            let encoded = payload.as_ssz_bytes();
            let decoded = MsgPayload::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(payload.value(), decoded.value());
            assert_eq!(payload.data(), decoded.data());
        }
    }

    mod sent_message {
        use super::*;

        ssz_proptest!(
            SentMessage,
            (
                any::<[u8; 32]>(),
                any::<u64>(),
                prop::collection::vec(any::<u8>(), 0..100)
            )
                .prop_map(|(id, sats, data)| {
                    SentMessage {
                        dest: AccountId::new(id),
                        payload: MsgPayload {
                            value: BitcoinAmount::from_sat(sats),
                            data: data.into(),
                        },
                    }
                })
        );

        #[test]
        fn test_zero_ssz() {
            let msg = SentMessage::new(
                AccountId::new([0u8; 32]),
                MsgPayload::new(BitcoinAmount::from_sat(0), vec![]),
            );
            let encoded = msg.as_ssz_bytes();
            let decoded = SentMessage::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(msg.dest(), decoded.dest());
        }
    }

    mod received_message {
        use super::*;

        ssz_proptest!(
            ReceivedMessage,
            (
                any::<[u8; 32]>(),
                any::<u64>(),
                prop::collection::vec(any::<u8>(), 0..100)
            )
                .prop_map(|(id, sats, data)| {
                    ReceivedMessage {
                        source: AccountId::new(id),
                        payload: MsgPayload {
                            value: BitcoinAmount::from_sat(sats),
                            data: data.into(),
                        },
                    }
                })
        );

        #[test]
        fn test_zero_ssz() {
            let msg = ReceivedMessage::new(
                AccountId::new([0u8; 32]),
                MsgPayload::new(BitcoinAmount::from_sat(0), vec![]),
            );
            let encoded = msg.as_ssz_bytes();
            let decoded = ReceivedMessage::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(msg.source(), decoded.source());
        }
    }
}
