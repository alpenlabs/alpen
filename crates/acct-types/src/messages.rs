//! Account message types.

use ssz_types::VariableList;
use strata_codec::{Codec, CodecError, Decoder, Encoder, Varint};
use strata_identifiers::Buf32;
use tree_hash::TreeHash;

use crate::{
    AccountId, BitcoinAmount, SentTransfer,
    ssz_generated::ssz::messages::{
        MAX_MSG_PAYLOAD_DATA_BYTES, MessageEntry, MsgPayload, ReceivedMessage, SentMessage,
    },
};

/// SSZ-bounded message payload data.
pub type MsgPayloadData = VariableList<u8, { MAX_MSG_PAYLOAD_DATA_BYTES as usize }>;

/// Error constructing a [`MsgPayload`] from raw bytes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MsgPayloadError {
    /// Raw message data exceeds the SSZ maximum length.
    #[error("message payload data too large (len {len} > max {max})")]
    DataTooLarge { len: usize, max: usize },
}

impl SentMessage {
    /// Creates a new [`SentMessage`] with the given destination and payload.
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

    /// Returns the destination account ID.
    pub fn dest(&self) -> AccountId {
        self.dest
    }

    /// Returns the message payload.
    pub fn payload(&self) -> &MsgPayload {
        &self.payload
    }
}

impl SentTransfer {
    /// Creates a new [`SentTransfer`] with the given destination and value.
    pub fn new(dest: AccountId, value: BitcoinAmount) -> Self {
        Self { dest, value }
    }

    /// Creates a new instance with 0 value.
    ///
    /// This shouldn't normally happen but it's a convenience.
    pub fn zero(dest: AccountId) -> Self {
        Self::new(dest, BitcoinAmount::zero())
    }

    /// Returns the destination account ID.
    pub fn dest(&self) -> AccountId {
        self.dest
    }

    /// Returns the transfer value.
    pub fn value(&self) -> BitcoinAmount {
        self.value
    }
}

impl ReceivedMessage {
    /// Creates a new [`ReceivedMessage`] with the given source and payload.
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

    /// Returns the source account ID.
    pub fn source(&self) -> AccountId {
        self.source
    }

    /// Returns the message payload.
    pub fn payload(&self) -> &MsgPayload {
        &self.payload
    }
}

impl MsgPayload {
    /// Creates a new [`MsgPayload`] with the given value and data.
    pub fn new(value: BitcoinAmount, data: MsgPayloadData) -> Self {
        Self { value, data }
    }

    /// Creates a new instance from raw data and some value.
    ///
    /// If the data exceeds the SSZ maximum length, an error is returned.
    pub fn from_bytes(value: BitcoinAmount, data: Vec<u8>) -> Result<Self, MsgPayloadError> {
        let len = data.len();
        let data = data.try_into().map_err(|_| MsgPayloadError::DataTooLarge {
            len,
            max: MAX_MSG_PAYLOAD_DATA_BYTES as usize,
        })?;

        Ok(Self::new(value, data))
    }

    /// Creates a new instance with empty data and some value.
    pub fn new_dataless(value: BitcoinAmount) -> Self {
        Self::new(value, MsgPayloadData::default())
    }

    /// Creates a new instance with some data and 0 value.
    pub fn new_valueless(data: MsgPayloadData) -> Self {
        Self::new(BitcoinAmount::zero(), data)
    }

    /// Creates a new instance from raw data and 0 value.
    pub fn from_bytes_valueless(data: Vec<u8>) -> Result<Self, MsgPayloadError> {
        Self::from_bytes(BitcoinAmount::zero(), data)
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
                    data: data
                        .try_into()
                        .expect("message payload bytes must fit within SSZ max length"),
                }
            })
        );

        #[test]
        fn test_zero_ssz() {
            let payload = MsgPayload::from_bytes(BitcoinAmount::from_sat(0), vec![])
                .expect("message payload bytes must fit within SSZ max length");
            let encoded = payload.as_ssz_bytes();
            let decoded = MsgPayload::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(payload.value(), decoded.value());
            assert_eq!(payload.data(), decoded.data());
        }

        #[test]
        fn accepts_max_size_payload() {
            let data = vec![0u8; MAX_MSG_PAYLOAD_DATA_BYTES as usize];
            let payload = MsgPayload::from_bytes(BitcoinAmount::from_sat(0), data)
                .expect("max size message payload bytes must fit within SSZ max length");

            assert_eq!(payload.data().len(), MAX_MSG_PAYLOAD_DATA_BYTES as usize);
        }

        #[test]
        fn rejects_oversized_payload() {
            let max = MAX_MSG_PAYLOAD_DATA_BYTES as usize;
            let err = MsgPayload::from_bytes(BitcoinAmount::from_sat(0), vec![0u8; max + 1])
                .expect_err("oversized message payload bytes must fail");

            assert_eq!(err, MsgPayloadError::DataTooLarge { len: max + 1, max });
        }

        #[test]
        fn new_accepts_prebuilt_payload_data() {
            let data = MsgPayloadData::default();
            let payload = MsgPayload::new(BitcoinAmount::from_sat(42), data);

            assert_eq!(payload.value(), BitcoinAmount::from_sat(42));
            assert!(payload.data().is_empty());
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
                            data: data
                                .try_into()
                                .expect("message payload bytes must fit within SSZ max length"),
                        },
                    }
                })
        );

        #[test]
        fn test_zero_ssz() {
            let msg = SentMessage::new(
                AccountId::new([0u8; 32]),
                MsgPayload::from_bytes(BitcoinAmount::from_sat(0), vec![])
                    .expect("message payload bytes must fit within SSZ max length"),
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
                            data: data
                                .try_into()
                                .expect("message payload bytes must fit within SSZ max length"),
                        },
                    }
                })
        );

        #[test]
        fn test_zero_ssz() {
            let msg = ReceivedMessage::new(
                AccountId::new([0u8; 32]),
                MsgPayload::from_bytes(BitcoinAmount::from_sat(0), vec![])
                    .expect("message payload bytes must fit within SSZ max length"),
            );
            let encoded = msg.as_ssz_bytes();
            let decoded = ReceivedMessage::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(msg.source(), decoded.source());
        }
    }
}
