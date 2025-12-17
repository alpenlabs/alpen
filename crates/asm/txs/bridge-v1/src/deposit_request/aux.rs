//! Deposit request transaction building utilities

use arbitrary::Arbitrary;
use strata_codec::{Codec, encode_to_vec};
use strata_l1_txfmt::TagData;

use crate::{
    constants::{BRIDGE_V1_SUBPROTOCOL_ID, BridgeTxType},
    errors::TagDataError,
};

/// Auxiliary data in the SPS-50 header for [`BridgeTxType::DepositRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Codec, Arbitrary)]
pub struct DrtHeaderAux {
    recovery_pk: [u8; 32],
    // TODO:PG - Intentionally using 20 bytes for now. Will be properly handled as part of https://alpenlabs.atlassian.net/browse/STR-1950
    ee_address: [u8; 20],
}

impl DrtHeaderAux {
    /// Creates new deposit request metadata
    pub fn new(recovery_pk: [u8; 32], ee_address: [u8; 20]) -> Self {
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
    pub fn ee_address(&self) -> &[u8; 20] {
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
