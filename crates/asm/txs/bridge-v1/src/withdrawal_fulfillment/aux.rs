use strata_codec::{Codec, CodecError, Decoder, Encoder};

/// Auxiliary data in the SPS-50 header for bridge v1 withdrawal fulfillment transactions.
///
/// This represents the type-specific auxiliary bytes that appear after the magic, subprotocol,
/// and tx_type fields in the OP_RETURN output at position 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WithdrawalFulfillmentTxHeaderAux {
    pub(super) deposit_idx: u32,
}

impl Codec for WithdrawalFulfillmentTxHeaderAux {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.deposit_idx.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let deposit_idx = u32::decode(dec)?;
        Ok(WithdrawalFulfillmentTxHeaderAux { deposit_idx })
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
            let original = WithdrawalFulfillmentTxHeaderAux { deposit_idx };

            let mut buf = Vec::new();
            original.encode(&mut buf).unwrap();

            let mut decoder = BufDecoder::new(buf.as_slice());
            let decoded = WithdrawalFulfillmentTxHeaderAux::decode(&mut decoder).unwrap();

            prop_assert_eq!(original, decoded);
        }
    }
}
