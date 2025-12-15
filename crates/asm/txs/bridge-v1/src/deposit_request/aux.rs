//! Deposit request transaction building utilities

use arbitrary::{Arbitrary, Unstructured};
use strata_codec::{Codec, CodecError, Decoder, Encoder, encode_to_vec};
use strata_l1_txfmt::TagData;

use crate::{
    constants::{BRIDGE_V1_SUBPROTOCOL_ID, BridgeTxType},
    errors::TagDataError,
};

/// Auxiliary data in the SPS-50 header for [`BridgeTxType::DepositRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrtHeaderAux {
    recovery_pk: [u8; 32],
    ee_address: Vec<u8>,
}

impl DrtHeaderAux {
    /// Creates new deposit request metadata
    pub fn new(recovery_pk: [u8; 32], ee_address: Vec<u8>) -> Self {
        Self {
            recovery_pk,
            ee_address,
        }
    }

    /// Returns the recovery public key
    pub fn recovery_pk(&self) -> &[u8; 32] {
        &self.recovery_pk
    }

    /// Returns the execution environment address
    pub fn ee_address(&self) -> &[u8] {
        &self.ee_address
    }

    /// Builds a `TagData` instance from this auxiliary data.
    ///
    /// This method encodes the auxiliary data and constructs the tag data for inclusion
    /// in the SPS-50 OP_RETURN output.
    ///
    /// # Errors
    ///
    /// Returns [`TagDataError`] if:
    /// - Encoding the auxiliary data fails
    /// - The encoded auxiliary data exceeds the maximum allowed size (74 bytes)
    pub fn build_tag_data(&self) -> Result<TagData, TagDataError> {
        let aux_data = encode_to_vec(self)?;
        let tag = TagData::new(
            BRIDGE_V1_SUBPROTOCOL_ID,
            BridgeTxType::DepositRequest as u8,
            aux_data,
        )?;
        Ok(tag)
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

        Ok(DrtHeaderAux::new(recovery_pk, ee_address))
    }
}

impl<'a> Arbitrary<'a> for DrtHeaderAux {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let recovery_pk = <[u8; 32]>::arbitrary(u)?;
        // Generate address between 20 and 64 bytes (reasonable range for EE addresses)
        let addr_len = u.int_in_range(20..=64)?;
        let ee_address = (0..addr_len)
            .map(|_| u8::arbitrary(u))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(DrtHeaderAux {
            recovery_pk,
            ee_address,
        })
    }
}
