use strata_codec::{Codec, CodecError, Decoder, Encoder, Varint};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WithdrawalFulfillmentTxTagData {
    pub(super) deposit_idx: u32,
}

impl Codec for WithdrawalFulfillmentTxTagData {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        let deposit_idx_varint = Varint::new(self.deposit_idx).ok_or(CodecError::OobInteger)?;
        deposit_idx_varint.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let deposit_idx = Varint::decode(dec)?.inner();
        Ok(WithdrawalFulfillmentTxTagData { deposit_idx })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use strata_codec::{BufDecoder, VARINT_MAX};

    use super::*;

    proptest! {
        #[test]
        fn test_withdrawal_fulfillment_tx_tag_data_roundtrip(deposit_idx in 0u32..=VARINT_MAX) {
            let original = WithdrawalFulfillmentTxTagData { deposit_idx };

            let mut buf = Vec::new();
            original.encode(&mut buf).unwrap();

            let mut decoder = BufDecoder::new(buf.as_slice());
            let decoded = WithdrawalFulfillmentTxTagData::decode(&mut decoder).unwrap();

            prop_assert_eq!(original, decoded);
        }
    }
}
