//! Account message types.

use strata_codec::{Codec, CodecError, Decoder, Encoder, Varint};

use crate::{
    AccountId, BitcoinAmount,
    ssz_generated::ssz::messages::{MsgPayload, ReceivedMessage, SentMessage},
};

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

impl MsgPayload {
    pub fn new(value: BitcoinAmount, data: Vec<u8>) -> Self {
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

impl Codec for MsgPayload {
    fn encode(&self, encoder: &mut impl Encoder) -> Result<(), CodecError> {
        self.value.encode(encoder)?;

        // encode data length
        let len = Varint::new_usize(self.data.len()).ok_or(CodecError::OobInteger)?;
        len.encode(encoder)?;

        // encode data
        encoder.write_buf(&self.data)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let amt_raw = u64::decode(dec)?;
        let amt = BitcoinAmount::from_sat(amt_raw);

        // decode data length
        let len = Varint::decode(dec)?.inner();

        // decode data
        let mut data = vec![0u8; len as usize];
        dec.read_buf(&mut data)?;

        Ok(Self {
            value: amt,
            data: data.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use ssz::{Decode, Encode};
    use strata_codec::{BufDecoder, Codec};
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;
    use crate::MsgPayload;

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

    proptest! {
        #[test]
        fn test_msg_payload_codec_roundtrip(
            value in 0u64..u64::MAX,
            data in prop::collection::vec(any::<u8>(), 0..1000)
        ) {
            let msg = MsgPayload::new(value.into(), data);
            let mut buf = vec![];

            msg.encode(&mut buf).unwrap();

            let mut dec = BufDecoder::new(buf);
            let decoded = MsgPayload::decode(&mut dec).unwrap();
            assert_eq!(msg, decoded);
        }
    }
}
