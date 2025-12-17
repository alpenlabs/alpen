use arbitrary::Arbitrary;
use strata_codec::{Codec, encode_to_vec};
use strata_l1_txfmt::TagData;

use crate::{BRIDGE_V1_SUBPROTOCOL_ID, constants::BridgeTxType, errors::TagDataError};

/// Auxiliary data in the SPS-50 header for [`BridgeTxType::Deposit`].
#[derive(Debug, Clone, PartialEq, Eq, Arbitrary, Codec)]
pub struct DepositTxHeaderAux {
    /// idx of the deposit as given by the N/N multisig.
    deposit_idx: u32,
    // TODO:PG- This is not really required, we are adding it here just to make sure that the
    // existing functional tests pass.
    ee_address: [u8; 20],
}

impl DepositTxHeaderAux {
    pub fn new(deposit_idx: u32, ee_address: [u8; 20]) -> Self {
        Self {
            deposit_idx,
            ee_address,
        }
    }

    pub fn deposit_idx(&self) -> u32 {
        self.deposit_idx
    }

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
            BridgeTxType::Deposit as u8,
            aux_data,
        )?;
        Ok(tag)
    }
}
