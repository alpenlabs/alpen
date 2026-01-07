use arbitrary::Arbitrary;
use strata_codec::{Codec, encode_to_vec};
use strata_l1_txfmt::TagData;

use crate::{BRIDGE_V1_SUBPROTOCOL_ID, constants::BridgeTxType};

/// Auxiliary data in the SPS-50 header for [`BridgeTxType::Deposit`].
#[derive(Debug, Clone, PartialEq, Eq, Arbitrary, Codec)]
pub struct DepositTxHeaderAux {
    /// idx of the deposit as given by the N/N multisig.
    deposit_idx: u32,
}

impl DepositTxHeaderAux {
    pub fn new(deposit_idx: u32) -> Self {
        Self { deposit_idx }
    }

    pub fn deposit_idx(&self) -> u32 {
        self.deposit_idx
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
            BridgeTxType::Deposit as u8,
            aux_data,
        )
        .expect("deposit tag data should always fit within SPS-50 limits")
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn build_tag_data_is_infallible(deposit_idx in any::<u32>()) {
            let aux = DepositTxHeaderAux::new(deposit_idx);
            let tag = aux.build_tag_data();
            prop_assert_eq!(tag.subproto_id(), BRIDGE_V1_SUBPROTOCOL_ID);
            prop_assert_eq!(tag.tx_type(), BridgeTxType::Deposit as u8);
        }
    }
}
