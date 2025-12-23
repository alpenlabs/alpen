use arbitrary::Arbitrary;
use strata_codec::{Codec, encode_to_vec};
use strata_l1_txfmt::TagData;

use crate::{BRIDGE_V1_SUBPROTOCOL_ID, constants::BridgeTxType};

/// Auxiliary data in the SPS-50 header for [`BridgeTxType::WithdrawalFulfillment`].
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
    /// # Panics
    ///
    /// Panics if encoding fails or if the encoded auxiliary data violates SPS-50 size
    /// limits.
    pub fn build_tag_data(&self) -> TagData {
        let aux_data = encode_to_vec(self).expect("auxiliary data encoding should be infallible");
        TagData::new(
            BRIDGE_V1_SUBPROTOCOL_ID,
            BridgeTxType::WithdrawalFulfillment as u8,
            aux_data,
        )
        .expect("withdrawal fulfillment tag data should always fit within SPS-50 limits")
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn build_tag_data_is_infallible(deposit_idx in any::<u32>()) {
            let aux = WithdrawalFulfillmentTxHeaderAux::new(deposit_idx);
            let tag = aux.build_tag_data();
            prop_assert_eq!(tag.subproto_id(), BRIDGE_V1_SUBPROTOCOL_ID);
            prop_assert_eq!(tag.tx_type(), BridgeTxType::WithdrawalFulfillment as u8);
        }
    }
}
