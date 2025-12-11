use arbitrary::Arbitrary;
use strata_codec::{Codec, encode_to_vec};
use strata_l1_txfmt::TagData;

use crate::{
    BRIDGE_V1_SUBPROTOCOL_ID, constants::WITHDRAWAL_FULFILLMENT_TX_TYPE, errors::TagDataError,
};

/// Auxiliary data in the SPS-50 header for bridge v1 withdrawal fulfillment transactions.
///
/// This represents the type-specific auxiliary bytes that appear after the magic, subprotocol,
/// and tx_type fields in the OP_RETURN output at position 0.
#[derive(Debug, Clone, PartialEq, Eq, Arbitrary, Codec)]
pub struct WithdrawalFulfillmentTxHeaderAux {
    /// The index of the locked deposit UTXO that the operator will receive payout from.
    /// This index is used to verify that the operator correctly fulfilled their assignment
    /// (correct amount to the correct user within the assigned deadline). Upon successful
    /// verification against the state's assignments table, the operator is authorized to
    /// claim the payout from this deposit.
    deposit_idx: u32,
}

impl WithdrawalFulfillmentTxHeaderAux {
    pub fn new(deposit_idx: u32) -> Self {
        Self { deposit_idx }
    }

    pub fn deposit_idx(&self) -> u32 {
        self.deposit_idx
    }

    #[cfg(feature = "test-utils")]
    pub fn set_deposit_idx(&mut self, deposit_idx: u32) {
        self.deposit_idx = deposit_idx;
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
            WITHDRAWAL_FULFILLMENT_TX_TYPE,
            aux_data,
        )?;
        Ok(tag)
    }
}
