use arbitrary::Arbitrary;
use strata_bridge_types::OperatorIdx;
use strata_codec::{Codec, encode_to_vec};
use strata_l1_txfmt::TagData;

use crate::{BRIDGE_V1_SUBPROTOCOL_ID, constants::BridgeTxType};

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
    /// # Panics
    ///
    /// Panics if encoding fails or if the encoded auxiliary data violates SPS-50 size
    /// limits.
    pub fn build_tag_data(&self) -> TagData {
        let aux_data = encode_to_vec(self).expect("auxiliary data encoding should be infallible");
        TagData::new(
            BRIDGE_V1_SUBPROTOCOL_ID,
            BridgeTxType::Unstake as u8,
            aux_data,
        )
        .expect("unstake tag data should always fit within SPS-50 limits")
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn build_tag_data_is_infallible(operator_idx in any::<OperatorIdx>()) {
            let aux = UnstakeTxHeaderAux::new(operator_idx);
            let tag = aux.build_tag_data();
            prop_assert_eq!(tag.subproto_id(), BRIDGE_V1_SUBPROTOCOL_ID);
            prop_assert_eq!(tag.tx_type(), BridgeTxType::Unstake as u8);
        }
    }
}
