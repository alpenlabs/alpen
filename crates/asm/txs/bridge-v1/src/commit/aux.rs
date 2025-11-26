use strata_codec::{Codec, CodecError, Decoder, Encoder};

/// Auxiliary data in the SPS-50 header for bridge v1 commit transactions.
///
/// This represents the type-specific auxiliary bytes that appear after the magic, subprotocol,
/// and tx_type fields in the OP_RETURN output at position 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CommitTxHeaderAux {
    /// The index of the deposit that the operator is committing to.
    /// This must be validated against the operator's assigned deposits in the state's assignments
    /// table to ensure the operator is authorized to withdraw this specific deposit.
    pub deposit_idx: u32,

    /// The index of the game being played.
    /// This is needed to later constrain the bridge proof public parameters.
    pub game_idx: u32,
}

impl Codec for CommitTxHeaderAux {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.deposit_idx.encode(enc)?;
        self.game_idx.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let deposit_idx = u32::decode(dec)?;
        let game_idx = u32::decode(dec)?;
        Ok(CommitTxHeaderAux {
            deposit_idx,
            game_idx,
        })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use strata_codec::BufDecoder;

    use super::*;

    proptest! {
        #[test]
        fn test_commit_tx_tag_data_roundtrip(deposit_idx in 0u32..=u32::MAX, game_idx in 0u32..=u32::MAX) {
            let original = CommitTxHeaderAux { deposit_idx, game_idx };

            let mut buf = Vec::new();
            original.encode(&mut buf).unwrap();

            let mut decoder = BufDecoder::new(buf.as_slice());
            let decoded = CommitTxHeaderAux::decode(&mut decoder).unwrap();

            prop_assert_eq!(original, decoded);
        }
    }
}
