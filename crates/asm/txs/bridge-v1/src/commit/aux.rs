use arbitrary::Arbitrary;
use strata_codec::{Codec, encode_to_vec};
use strata_l1_txfmt::TagData;

use crate::{BRIDGE_V1_SUBPROTOCOL_ID, constants::COMMIT_TX_TYPE, errors::TagDataError};

/// Auxiliary data in the SPS-50 header for bridge v1 commit transactions.
///
/// This represents the type-specific auxiliary bytes that appear after the magic, subprotocol,
/// and tx_type fields in the OP_RETURN output at position 0.
#[derive(Debug, Clone, PartialEq, Eq, Arbitrary, Codec)]
pub struct CommitTxHeaderAux {
    /// The index of the deposit that the operator is committing to.
    /// This must be validated against the operator's assigned deposits in the state's assignments
    /// table to ensure the operator is authorized to withdraw this specific deposit.
    deposit_idx: u32,

    /// The index of the game being played.
    /// This is needed to later constrain the bridge proof public parameters.
    game_idx: u32,
}

impl CommitTxHeaderAux {
    pub fn new(deposit_idx: u32, game_idx: u32) -> Self {
        Self {
            deposit_idx,
            game_idx,
        }
    }

    pub fn deposit_idx(&self) -> u32 {
        self.deposit_idx
    }

    pub fn game_idx(&self) -> u32 {
        self.game_idx
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
        let tag = TagData::new(BRIDGE_V1_SUBPROTOCOL_ID, COMMIT_TX_TYPE, aux_data)?;
        Ok(tag)
    }
}
