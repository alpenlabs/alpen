use arbitrary::Arbitrary;
use strata_bridge_types::OperatorIdx;
use strata_codec::{Codec, encode_to_vec};
use strata_l1_txfmt::TagData;

use crate::{BRIDGE_V1_SUBPROTOCOL_ID, constants::SLASH_TX_TYPE, errors::TagDataError};

/// Auxiliary data in the SPS-50 header for bridge v1 slash transaction.
///
/// This represents the type-specific auxiliary bytes that appear after the magic, subprotocol,
/// and tx_type fields in the OP_RETURN output at position 0.
#[derive(Debug, Clone, PartialEq, Eq, Arbitrary, Codec)]
pub struct SlashTxHeaderAux {
    /// The index of the operator being slashed.
    operator_idx: OperatorIdx,
}

impl SlashTxHeaderAux {
    pub fn new(operator_idx: OperatorIdx) -> Self {
        Self { operator_idx }
    }

    pub fn operator_idx(&self) -> OperatorIdx {
        self.operator_idx
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
        let tag = TagData::new(BRIDGE_V1_SUBPROTOCOL_ID, SLASH_TX_TYPE, aux_data)?;
        Ok(tag)
    }
}
