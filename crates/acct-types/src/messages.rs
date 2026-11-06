//! Account message types.

use ssz_types::VariableList;
use strata_codec::{Codec, CodecError, Decoder, Encoder, Varint};
use strata_identifiers::Buf32;
use tree_hash::{Sha256Hasher, TreeHash};

use crate::{
    AccountId, BitcoinAmount, SentTransfer,
    ssz_generated::ssz::messages::{
        MAX_MSG_PAYLOAD_DATA_BYTES, MessageEntry, MsgPayload, ReceivedMessage, SentMessage,
    },
};

/// SSZ-bounded message payload data.
pub type MsgPayloadData = VariableList<u8, { MAX_MSG_PAYLOAD_DATA_BYTES as usize }>;

/// Error constructing a [`MsgPayload`] or effect value from raw inputs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MsgPayloadError {
    /// Raw message data exceeds the SSZ maximum length.
    #[error("message payload data too large (len {len} > max {max})")]
    DataTooLarge { len: usize, max: usize },

    /// Raw satoshi value exceeds the Bitcoin money supply.
    #[error("value exceeds the Bitcoin money supply ({sats} sats)")]
    ValueTooLarge { sats: u64 },
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
        Self::new_dataless(dest, BitcoinAmount::default())
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
        Self::new(dest, BitcoinAmount::default())
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
        Self::new_dataless(dest, BitcoinAmount::default())
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
        Self::new(BitcoinAmount::default(), data)
    }

    /// Creates a new instance from raw data and 0 value.
    pub fn from_bytes_valueless(data: Vec<u8>) -> Result<Self, MsgPayloadError> {
        Self::from_bytes(BitcoinAmount::default(), data)
    }

    /// Creates a new instance with empty data and 0 value.
    pub fn new_empty() -> Self {
        Self::new_dataless(BitcoinAmount::default())
    }

    pub fn value(&self) -> BitcoinAmount {
        self.value
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Returns the SSZ-bounded message data list.
    pub fn payload_data(&self) -> &MsgPayloadData {
        &self.data
    }

    /// Decodes a Codec payload with a caller-specific data length limit.
    ///
    /// Checks the caller's limit and the SSZ maximum before allocating the data buffer.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::OverflowContainer`] if the declared data length exceeds
    /// either limit. Propagates errors decoding the value, length, or data bytes.
    pub fn decode_with_max_data_length(
        dec: &mut impl Decoder,
        max_data_len: usize,
    ) -> Result<Self, CodecError> {
        let value = BitcoinAmount::decode(dec)?;
        let len = Varint::decode(dec)?.inner() as usize;
        if len > max_data_len.min(MAX_MSG_PAYLOAD_DATA_BYTES as usize) {
            return Err(CodecError::OverflowContainer);
        }

        let mut buf = vec![0; len];
        dec.read_buf(&mut buf)?;
        let data = VariableList::new(buf).map_err(|_| CodecError::OverflowContainer)?;
        Ok(Self { data, value })
    }

    /// Wraps the payload into a [`SentMessage`] to some dest account.
    pub fn into_sent(self, dest: AccountId) -> SentMessage {
        SentMessage::new(dest, self)
    }
}

impl Codec for MsgPayload {
    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        Self::decode_with_max_data_length(dec, MAX_MSG_PAYLOAD_DATA_BYTES as usize)
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
        <Self as TreeHash>::tree_hash_root::<Sha256Hasher>(self)
            .0
            .into()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use ssz::{Decode, Encode};
    use strata_ssz_tests::ssz_proptest;

    use super::*;

    const MAX_BITCOIN_MONEY_SATS: u64 = 21_000_000 * 100_000_000;

    mod msg_payload {
        use strata_codec::{BufDecoder, decode_buf_exact, encode_to_vec};

        use super::*;

        fn codec_prefix(len: usize) -> Vec<u8> {
            let mut bytes = encode_to_vec(&BitcoinAmount::try_from(42).unwrap()).unwrap();
            Varint::new_usize(len).unwrap().encode(&mut bytes).unwrap();
            bytes
        }

        #[test]
        fn bounded_codec_round_trips_at_length_limits() {
            for len in [0, 8, MAX_MSG_PAYLOAD_DATA_BYTES as usize] {
                let payload =
                    MsgPayload::from_bytes(BitcoinAmount::try_from(42).unwrap(), vec![0x5a; len])
                        .unwrap();
                let bytes = encode_to_vec(&payload).unwrap();
                let mut decoder = BufDecoder::new(&bytes);
                assert_eq!(
                    MsgPayload::decode_with_max_data_length(&mut decoder, len).unwrap(),
                    payload
                );
                assert_eq!(decoder.remaining(), 0);
                assert_eq!(decode_buf_exact::<MsgPayload>(&bytes).unwrap(), payload);
            }
        }

        #[test]
        fn bounded_codec_rejects_excessive_length_without_body() {
            for max_data_len in [0, 8] {
                let bytes = codec_prefix(max_data_len + 1);
                let mut decoder = BufDecoder::new(&bytes);
                assert!(matches!(
                    MsgPayload::decode_with_max_data_length(&mut decoder, max_data_len),
                    Err(CodecError::OverflowContainer)
                ));
                assert_eq!(decoder.remaining(), 0);
            }
        }

        #[test]
        fn bounded_codec_cannot_exceed_ssz_maximum() {
            let bytes = codec_prefix(MAX_MSG_PAYLOAD_DATA_BYTES as usize + 1);
            let mut decoder = BufDecoder::new(&bytes);
            assert!(matches!(
                MsgPayload::decode_with_max_data_length(&mut decoder, usize::MAX),
                Err(CodecError::OverflowContainer)
            ));
            assert!(matches!(
                decode_buf_exact::<MsgPayload>(&bytes),
                Err(CodecError::OverflowContainer)
            ));
        }

        #[test]
        fn bounded_codec_rejects_truncated_in_range_payload() {
            let mut bytes = codec_prefix(3);
            bytes.extend_from_slice(&[0x11, 0x22]);
            let mut decoder = BufDecoder::new(&bytes);
            assert!(matches!(
                MsgPayload::decode_with_max_data_length(&mut decoder, 3),
                Err(CodecError::OverrunInput)
            ));
            assert!(matches!(
                decode_buf_exact::<MsgPayload>(&bytes),
                Err(CodecError::OverrunInput)
            ));
        }

        #[test]
        fn bounded_codec_preserves_wire_vector() {
            let bytes = [0, 0, 0, 0, 0, 0, 0, 42, 3, 0x11, 0x22, 0x33];
            let mut decoder = BufDecoder::new(bytes);
            let payload = MsgPayload::decode_with_max_data_length(&mut decoder, 3).unwrap();
            assert_eq!(payload.value(), BitcoinAmount::try_from(42).unwrap());
            assert_eq!(payload.data(), &[0x11, 0x22, 0x33]);
            assert_eq!(decoder.remaining(), 0);
            assert_eq!(encode_to_vec(&payload).unwrap(), bytes);
        }

        ssz_proptest!(
            MsgPayload,
            (
                0..=MAX_BITCOIN_MONEY_SATS,
                prop::collection::vec(any::<u8>(), 0..100)
            )
                .prop_map(|(sats, data)| {
                    MsgPayload {
                        value: BitcoinAmount::try_from(sats)
                            .expect("amount must not exceed the Bitcoin money supply"),
                        data: data
                            .try_into()
                            .expect("message payload bytes must fit within SSZ max length"),
                    }
                })
        );

        #[test]
        fn test_zero_ssz() {
            let payload = MsgPayload::from_bytes(
                BitcoinAmount::try_from(0)
                    .expect("amount must not exceed the Bitcoin money supply"),
                vec![],
            )
            .expect("message payload bytes must fit within SSZ max length");
            let encoded = payload.as_ssz_bytes();
            let decoded = MsgPayload::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(payload.value(), decoded.value());
            assert_eq!(payload.data(), decoded.data());
        }

        #[test]
        fn accepts_max_size_payload() {
            let data = vec![0u8; MAX_MSG_PAYLOAD_DATA_BYTES as usize];
            let payload = MsgPayload::from_bytes(
                BitcoinAmount::try_from(0)
                    .expect("amount must not exceed the Bitcoin money supply"),
                data,
            )
            .expect("max size message payload bytes must fit within SSZ max length");

            assert_eq!(payload.data().len(), MAX_MSG_PAYLOAD_DATA_BYTES as usize);
        }

        #[test]
        fn rejects_oversized_payload() {
            let max = MAX_MSG_PAYLOAD_DATA_BYTES as usize;
            let err = MsgPayload::from_bytes(
                BitcoinAmount::try_from(0)
                    .expect("amount must not exceed the Bitcoin money supply"),
                vec![0u8; max + 1],
            )
            .expect_err("oversized message payload bytes must fail");

            assert_eq!(err, MsgPayloadError::DataTooLarge { len: max + 1, max });
        }

        #[test]
        fn new_accepts_prebuilt_payload_data() {
            let data = MsgPayloadData::default();
            let payload = MsgPayload::new(
                BitcoinAmount::try_from(42)
                    .expect("amount must not exceed the Bitcoin money supply"),
                data,
            );

            assert_eq!(
                payload.value(),
                BitcoinAmount::try_from(42)
                    .expect("amount must not exceed the Bitcoin money supply")
            );
            assert!(payload.data().is_empty());
        }
    }

    mod sent_message {
        use super::*;

        ssz_proptest!(
            SentMessage,
            (
                any::<[u8; 32]>(),
                0..=MAX_BITCOIN_MONEY_SATS,
                prop::collection::vec(any::<u8>(), 0..100)
            )
                .prop_map(|(id, sats, data)| {
                    SentMessage {
                        dest: AccountId::new(id),
                        payload: MsgPayload {
                            value: BitcoinAmount::try_from(sats)
                                .expect("amount must not exceed the Bitcoin money supply"),
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
                MsgPayload::from_bytes(
                    BitcoinAmount::try_from(0)
                        .expect("amount must not exceed the Bitcoin money supply"),
                    vec![],
                )
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
                0..=MAX_BITCOIN_MONEY_SATS,
                prop::collection::vec(any::<u8>(), 0..100)
            )
                .prop_map(|(id, sats, data)| {
                    ReceivedMessage {
                        source: AccountId::new(id),
                        payload: MsgPayload {
                            value: BitcoinAmount::try_from(sats)
                                .expect("amount must not exceed the Bitcoin money supply"),
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
                MsgPayload::from_bytes(
                    BitcoinAmount::try_from(0)
                        .expect("amount must not exceed the Bitcoin money supply"),
                    vec![],
                )
                .expect("message payload bytes must fit within SSZ max length"),
            );
            let encoded = msg.as_ssz_bytes();
            let decoded = ReceivedMessage::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(msg.source(), decoded.source());
        }
    }
}
