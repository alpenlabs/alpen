//! Deposit request transaction building utilities

use arbitrary::{Arbitrary, Unstructured};
use strata_codec::{Codec, CodecError, Decoder, Encoder, encode_to_vec};
use strata_l1_txfmt::TagData;

use crate::{
    BRIDGE_V1_SUBPROTOCOL_ID, constants::DEPOSIT_REQUEST_TX_TYPE, errors::DepositTxParseError,
};

/// Auxiliary data in the SPS-50 header for bridge v1 deposit request transactions.
///
/// This represents the type-specific auxiliary bytes that appear after the magic, subprotocol,
/// and tx_type fields in the OP_RETURN output at position 0.
#[derive(Debug, Clone)]
pub struct DrtHeaderAux {
    recovery_pk: [u8; 32],
    ee_address: Vec<u8>,
}

impl DrtHeaderAux {
    pub fn new(recovery_pk: [u8; 32], ee_address: Vec<u8>) -> Self {
        Self {
            recovery_pk,
            ee_address,
        }
    }

    pub fn recovery_pk(&self) -> &[u8; 32] {
        &self.recovery_pk
    }

    pub fn ee_address(&self) -> &[u8] {
        &self.ee_address
    }

    pub fn encode_tag(&self) -> Result<TagData, DepositTxParseError> {
        let aux_data = encode_to_vec(self).map_err(DepositTxParseError::InvalidAuxiliaryData)?;
        Ok(TagData::new(BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_REQUEST_TX_TYPE, aux_data).unwrap())
    }
}

impl<'a> Arbitrary<'a> for DrtHeaderAux {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let recovery_pk = <[u8; 32]>::arbitrary(u)?;

        const MAX_AUX_LEN: usize = 74;
        const MAX_ALLOWED_EE_ADDR_LEN: usize = MAX_AUX_LEN - 32;

        let addr_len = u.int_in_range(20..=MAX_ALLOWED_EE_ADDR_LEN)?;
        let ee_address = (0..addr_len)
            .map(|_| u8::arbitrary(u))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(DrtHeaderAux {
            recovery_pk,
            ee_address,
        })
    }
}

impl Codec for DrtHeaderAux {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.recovery_pk.encode(enc)?;
        enc.write_buf(&self.ee_address)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let recovery_pk = <[u8; 32]>::decode(dec)?;

        // Read remaining bytes as address - we need to read from a buffer
        // Since Decoder doesn't provide a way to read all remaining bytes,
        // this decode assumes the input has already been sized correctly
        let mut ee_address = Vec::new();
        // Try to read bytes until we hit end of buffer
        while let Ok(byte) = dec.read_arr::<1>() {
            ee_address.push(byte[0]);
        }

        Ok(DrtHeaderAux {
            recovery_pk,
            ee_address,
        })
    }
}
