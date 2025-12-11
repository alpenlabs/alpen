use arbitrary::Arbitrary;
use strata_bridge_types::OperatorIdx;
use strata_codec::{Codec, encode_to_vec};
use strata_l1_txfmt::TagData;

use crate::{BRIDGE_V1_SUBPROTOCOL_ID, constants::BridgeTxType, errors::TagDataError};

/// Auxiliary data in the SPS-50 header for [`BridgeTxType::Unstake`].
#[derive(Debug, Clone, PartialEq, Eq, Arbitrary, Codec)]
pub struct UnstakeTxHeaderAux {
    /// The index of the operator whose stake is being unlocked.
    operator_idx: OperatorIdx,
}

impl UnstakeTxHeaderAux {
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
        let tag = TagData::new(
            BRIDGE_V1_SUBPROTOCOL_ID,
            BridgeTxType::Unstake as u8,
            aux_data,
        )?;
        Ok(tag)
    }
}
