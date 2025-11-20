use strata_codec::{Codec, CodecError, Decoder, Encoder};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WithdrawalFulfillmentTxTagData {
    pub(super) deposit_idx: u32,
}

impl Codec for WithdrawalFulfillmentTxTagData {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.deposit_idx.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let deposit_idx = u32::decode(dec)?;
        Ok(WithdrawalFulfillmentTxTagData { deposit_idx })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use strata_codec::BufDecoder;

    use super::*;

    proptest! {
        #[test]
        fn test_withdrawal_fulfillment_tx_tag_data_roundtrip(deposit_idx in 0u32..=u32::MAX) {
            let original = WithdrawalFulfillmentTxTagData { deposit_idx };

            let mut buf = Vec::new();
            original.encode(&mut buf).unwrap();

            let mut decoder = BufDecoder::new(buf.as_slice());
            let decoded = WithdrawalFulfillmentTxTagData::decode(&mut decoder).unwrap();

            prop_assert_eq!(original, decoded);
        }
    }
}
