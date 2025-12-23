use arbitrary::Arbitrary;
use strata_codec::{Codec, encode_to_vec};
use strata_l1_txfmt::TagData;

use crate::{BRIDGE_V1_SUBPROTOCOL_ID, constants::BridgeTxType};

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

    fn bytes_20() -> impl Strategy<Value = [u8; 20]> {
        prop::collection::vec(any::<u8>(), 20)
            .prop_map(|bytes| bytes.try_into().expect("length is fixed"))
    }

    proptest! {
        #[test]
        fn build_tag_data_is_infallible(deposit_idx in any::<u32>(), ee_address in bytes_20()) {
            let aux = DepositTxHeaderAux::new(deposit_idx, ee_address);
            let tag = aux.build_tag_data();
            prop_assert_eq!(tag.subproto_id(), BRIDGE_V1_SUBPROTOCOL_ID);
            prop_assert_eq!(tag.tx_type(), BridgeTxType::Deposit as u8);
        }
    }
}
