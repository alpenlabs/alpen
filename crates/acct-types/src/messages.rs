//! Account message types.

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
        encoder.write_buf(&self.value.to_be_bytes())?;

        // encode data length
        let len = self.data.len() as u64;
        encoder.write_buf(&len.to_be_bytes())?;

        // encode data
        encoder.write_buf(&self.data)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let amt_raw: [u8; 8] = dec.read_arr()?;
        let amt = BitcoinAmount::from_sat(u64::from_be_bytes(amt_raw));

        // decode data length
        let len = u64::from_be_bytes(dec.read_arr::<8>()?);

        // decode data
        let mut data = Vec::with_capacity(len as usize);
        dec.read_buf(&mut data)?;

        Ok(Self { value: amt, data })
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
