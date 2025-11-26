use arbitrary::Arbitrary;
use bitcoin::taproot::TAPROOT_CONTROL_NODE_SIZE;
use strata_codec::{Codec, CodecError, Decoder, Encoder};

/// Auxiliary data in the SPS-50 header for bridge v1 deposit transactions.
///
/// This represents the type-specific auxiliary bytes that appear after the magic, subprotocol,
/// and tx_type fields in the OP_RETURN output at position 0.
#[derive(Debug, Clone, PartialEq, Eq, Arbitrary)]
pub struct DepositTxHeaderAux {
    /// idx of the deposit as given by the N/N multisig.
    pub deposit_idx: u32,

    /// The merkle root of the Script Tree from the Deposit Request Transaction (DRT) being spent.
    ///
    /// This value is extracted from the auxiliary data and represents the merkle root of the
    /// tapscript tree from the DRT that this deposit transaction is spending. It is combined
    /// with the internal key (aggregated operator key) to reconstruct the taproot address
    /// that was used in the DRT's P2TR output.
    ///
    /// This is required to verify that the transaction was indeed signed by the claimed pubkey.
    /// Without this validation, someone could send funds to the N-of-N address without proper
    /// authorization, which would mint tokens but break the peg since there would be no presigned
    /// withdrawal transactions. This would require N-of-N trust for withdrawals instead of the
    /// intended 1-of-N trust assumption with presigned transactions.
    pub drt_tapscript_merkle_root: [u8; TAPROOT_CONTROL_NODE_SIZE],

    /// The destination address for the deposit.
    pub address: Vec<u8>,
}

impl Codec for DepositTxHeaderAux {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.deposit_idx.encode(enc)?;
        self.drt_tapscript_merkle_root.encode(enc)?;
        enc.write_buf(&self.address)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let deposit_idx = u32::decode(dec)?;
        let drt_tapscript_merkle_root = <[u8; TAPROOT_CONTROL_NODE_SIZE]>::decode(dec)?;

        // Read remaining bytes as address - we need to read from a buffer
        // Since Decoder doesn't provide a way to read all remaining bytes,
        // this decode assumes the input has already been sized correctly
        let mut address = Vec::new();
        // Try to read bytes until we hit end of buffer
        while let Ok(byte) = dec.read_arr::<1>() {
            address.push(byte[0]);
        }

        Ok(DepositTxHeaderAux {
            deposit_idx,
            drt_tapscript_merkle_root,
            address,
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
        fn test_deposit_tx_header_aux_roundtrip(
            deposit_idx in 0u32..=u32::MAX,
            drt_tapscript_merkle_root in prop::array::uniform32(0u8..),
            address in prop::collection::vec(0u8.., 0..100)
        ) {
            let original = DepositTxHeaderAux {
                deposit_idx,
                drt_tapscript_merkle_root,
                address,
            };

            let mut buf = Vec::new();
            original.encode(&mut buf).unwrap();

            let mut decoder = BufDecoder::new(buf.as_slice());
            let decoded = DepositTxHeaderAux::decode(&mut decoder).unwrap();

            prop_assert_eq!(original, decoded);
        }
    }
}
