//! Deposit request transaction building utilities

use arbitrary::Arbitrary;
use strata_codec::{Codec, CodecError, Decoder, Encoder, encode_to_vec};
use strata_identifiers::AccountSerial;
use strata_l1_txfmt::TagData;

use crate::{
    constants::{BRIDGE_V1_SUBPROTOCOL_ID, BridgeTxType},
    deposit_request::SubjectBytes,
};

/// Auxiliary data in the SPS-50 header for [`BridgeTxType::DepositRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrtHeaderAux {
    recovery_pk: [u8; 32],

    /// [`AccountSerial`] of the destination account.
    ///
    /// We use [`AccountSerial`] instead of [`AccountId`](strata_identifiers::AccountId) to
    /// minimize onchain cost. Since account serials are immutable and uniquely identify accounts,
    /// they provide the same identification guarantees as `AccountId` while being more
    /// space-efficient on-chain.
    dest_acct_serial: AccountSerial,

    /// [`SubjectId`](`strata_identifiers::SubjectId`) within the destination account.
    ///
    /// We use [`SubjectBytes`] instead of `SubjectId` to minimize onchain cost.
    dest_subject: SubjectBytes,
}

impl DrtHeaderAux {
    /// Creates new deposit request metadata
    ///
    /// # Errors
    ///
    /// Returns an error if the `dest_subject` length exceeds [`SUBJ_ID_LEN`] (32 bytes).
    pub fn new(
        recovery_pk: [u8; 32],
        dest_acct_serial: AccountSerial,
        dest_subject: SubjectBytes,
    ) -> Self {
        Self {
            recovery_pk,
            dest_acct_serial,
            dest_subject,
        }
    }

    /// Returns the recovery public key
    pub const fn recovery_pk(&self) -> &[u8; 32] {
        &self.recovery_pk
    }

    /// Returns the execution environment address
    pub const fn dest_acct_serial(&self) -> &AccountSerial {
        &self.dest_acct_serial
    }

    pub const fn dest_subject(&self) -> &SubjectBytes {
        &self.dest_subject
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

impl Codec for DrtHeaderAux {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        enc.write_buf(&self.recovery_pk)?;
        self.dest_acct_serial.encode(enc)?;
        enc.write_buf(self.dest_subject.as_bytes())?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let recovery_pk = <[u8; 32]>::decode(dec)?;
        let dest_acct_serial = AccountSerial::decode(dec)?;

        // Read remaining bytes as address - we need to read from a buffer
        // Since Decoder doesn't provide a way to read all remaining bytes,
        // this decode assumes the input has already been sized correctly
        let mut dest_subject_bytes = Vec::new();
        // Try to read bytes until we hit end of buffer
        while let Ok(byte) = dec.read_arr::<1>() {
            dest_subject_bytes.push(byte[0]);
        }
        let dest_subject = SubjectBytes::try_new(dest_subject_bytes)
            .map_err(|_| CodecError::MalformedField("dest subject"))?;

        Ok(Self {
            recovery_pk,
            dest_acct_serial,
            dest_subject,
        })
    }
}

impl<'a> Arbitrary<'a> for DrtHeaderAux {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let recovery_pk = u.arbitrary()?;
        let dest_acct_serial = AccountSerial::new(u.arbitrary()?);
        let dest_subject = u.arbitrary()?;
        Ok(Self::new(recovery_pk, dest_acct_serial, dest_subject))
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use strata_codec::{decode_buf_exact, encode_to_vec};
    use strata_identifiers::SUBJ_ID_LEN;

    use super::*;

    fn bytes_32() -> impl Strategy<Value = [u8; 32]> {
        prop::collection::vec(any::<u8>(), 32)
            .prop_map(|bytes| bytes.try_into().expect("length is fixed"))
    }

    fn account_serial() -> impl Strategy<Value = AccountSerial> {
        any::<u32>().prop_map(AccountSerial::new)
    }

    fn subject_bytes() -> impl Strategy<Value = SubjectBytes> {
        prop::collection::vec(any::<u8>(), 0..=SUBJ_ID_LEN)
            .prop_map(|bytes| SubjectBytes::try_new(bytes).expect("length is within bounds"))
    }

    proptest! {
        #[test]
        fn build_tag_data_is_infallible(
            recovery_pk in bytes_32(),
            dest_acct_serial in account_serial(),
            dest_subject in subject_bytes(),
        ) {
            let aux = DrtHeaderAux::new(recovery_pk, dest_acct_serial, dest_subject);
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
            let original = DrtHeaderAux::new(recovery_pk, dest_acct_serial, dest_subject);
            let encoded = encode_to_vec(&original).expect("Failed to encode");
            let decoded: DrtHeaderAux = decode_buf_exact(&encoded).expect("Failed to decode");
            prop_assert_eq!(&decoded, &original);
        }
    }
}
