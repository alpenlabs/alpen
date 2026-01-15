//! Deposit request transaction building utilities

use arbitrary::Arbitrary;
use strata_codec::{Codec, encode_to_vec};
use strata_identifiers::{AccountSerial, SubjectIdBytes};
use strata_l1_txfmt::TagData;

use crate::{
    constants::{BRIDGE_V1_SUBPROTOCOL_ID, BridgeTxType},
    deposit_request::DepositDescriptor,
};

/// Auxiliary data in the SPS-50 header for [`BridgeTxType::DepositRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Codec, Arbitrary)]
pub struct DrtHeaderAux {
    recovery_pk: [u8; 32],
    destination: DepositDescriptor,
}

impl DrtHeaderAux {
    /// Creates new deposit request metadata
    pub fn new(recovery_pk: [u8; 32], destination: DepositDescriptor) -> Self {
        Self {
            recovery_pk,
            destination,
        }
    }

    /// Returns the recovery public key
    pub const fn recovery_pk(&self) -> &[u8; 32] {
        &self.recovery_pk
    }

    /// Returns the destination descriptor.
    pub const fn destination(&self) -> &DepositDescriptor {
        &self.destination
    }

    /// Returns the destination account serial.
    pub const fn dest_acct_serial(&self) -> &AccountSerial {
        self.destination.dest_acct_serial()
    }

    /// Returns the destination subject bytes.
    pub const fn dest_subject(&self) -> &SubjectIdBytes {
        self.destination.dest_subject()
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
    use strata_codec::{decode_buf_exact, encode_to_vec};
    use strata_identifiers::SUBJ_ID_LEN;

    use super::*;
    use crate::deposit_request::MAX_SERIAL_VALUE;

    fn bytes_32() -> impl Strategy<Value = [u8; 32]> {
        prop::collection::vec(any::<u8>(), 32)
            .prop_map(|bytes| bytes.try_into().expect("length is fixed"))
    }

    fn account_serial() -> impl Strategy<Value = AccountSerial> {
        (0..=MAX_SERIAL_VALUE).prop_map(AccountSerial::new)
    }

    fn subject_bytes() -> impl Strategy<Value = SubjectIdBytes> {
        prop::collection::vec(any::<u8>(), 0..=SUBJ_ID_LEN)
            .prop_map(|bytes| SubjectIdBytes::try_new(bytes).expect("length is within bounds"))
    }

    proptest! {
        #[test]
        fn build_tag_data_is_infallible(
            recovery_pk in bytes_32(),
            dest_acct_serial in account_serial(),
            dest_subject in subject_bytes(),
        ) {
            let destination = DepositDescriptor::new(dest_acct_serial, dest_subject);
            let aux = DrtHeaderAux::new(recovery_pk, destination);
            let tag = aux.build_tag_data();
            prop_assert_eq!(tag.subproto_id(), BRIDGE_V1_SUBPROTOCOL_ID);
            prop_assert_eq!(tag.tx_type(), BridgeTxType::DepositRequest as u8);
        }

        #[test]
        fn codec_roundtrip(
            recovery_pk in bytes_32(),
            dest_acct_serial in account_serial(),
            dest_subject in subject_bytes(),
        ) {
            let destination = DepositDescriptor::new(dest_acct_serial, dest_subject);
            let original = DrtHeaderAux::new(recovery_pk, destination);
            let encoded = encode_to_vec(&original).expect("Failed to encode");
            let decoded: DrtHeaderAux = decode_buf_exact(&encoded).expect("Failed to decode");
            prop_assert_eq!(&decoded, &original);
        }
    }
}
