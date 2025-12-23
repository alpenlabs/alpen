//! Deposit request transaction building utilities

use arbitrary::Arbitrary;
use strata_codec::{Codec, encode_to_vec};
use strata_l1_txfmt::TagData;

use crate::constants::{BRIDGE_V1_SUBPROTOCOL_ID, BridgeTxType};

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
    /// # Panics
    ///
    /// Panics if encoding fails or if the encoded auxiliary data violates SPS-50 size
    /// limits.
    pub fn build_tag_data(&self) -> TagData {
        let aux_data = encode_to_vec(self).expect("auxiliary data encoding should be infallible");
        TagData::new(
            BRIDGE_V1_SUBPROTOCOL_ID,
            BridgeTxType::DepositRequest as u8,
            aux_data,
        )
        .expect("deposit request tag data should always fit within SPS-50 limits")
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

    fn bytes_32() -> impl Strategy<Value = [u8; 32]> {
        prop::collection::vec(any::<u8>(), 32)
            .prop_map(|bytes| bytes.try_into().expect("length is fixed"))
    }

    proptest! {
        #[test]
        fn build_tag_data_is_infallible(recovery_pk in bytes_32(), ee_address in bytes_20()) {
            let aux = DrtHeaderAux::new(recovery_pk, ee_address);
            let tag = aux.build_tag_data();
            prop_assert_eq!(tag.subproto_id(), BRIDGE_V1_SUBPROTOCOL_ID);
            prop_assert_eq!(tag.tx_type(), BridgeTxType::DepositRequest as u8);
        }
    }
}
