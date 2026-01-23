//! Deposit request transaction building utilities

use arbitrary::{Arbitrary, Unstructured};
use strata_codec::VarVec;
use strata_l1_txfmt::TagData;
use thiserror::Error;

use crate::constants::{BRIDGE_V1_SUBPROTOCOL_ID, BridgeTxType};

/// Maximum destination size in bytes.
///
/// SPS-50 defines a maximum OP_RETURN size of 80 bytes, which is split as:
/// - 4 bytes: magic
/// - 1 byte: subprotocol ID
/// - 1 byte: tx type
/// - 74 bytes: aux data (maximum)
///
/// Since aux data contains 32 bytes for recovery_pk, the maximum destination size is:
/// 74 - 32 = 42 bytes
///
/// Reference: <https://github.com/alpenlabs/strata-common/blob/93511df/crates/l1proto/txfmt/src/tag.rs>
const MAX_DESTINATION_LEN: usize = 42;

/// Error type for parsing deposit request auxiliary data.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DrtHeaderAuxError {
    /// Auxiliary data is too short to contain recovery public key.
    #[error("auxiliary data too short: expected at least 32 bytes, got {0}")]
    TooShort(usize),

    /// Destination bytes exceed the maximum allowed length.
    #[error("destination too long: expected at most {expected} bytes, got {actual}")]
    DestinationTooLong { expected: usize, actual: usize },
}

/// Auxiliary data in the SPS-50 header for [`BridgeTxType::DepositRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrtHeaderAux {
    /// The depositor's public key used in the takeback script. If operators fail to process the
    /// deposit within the timeout period, the depositor can use the corresponding private key to
    /// reclaim their Bitcoin via the takeback tapscript.
    recovery_pk: [u8; 32],
    /// Destination specifying where BTC should be minted.
    destination: VarVec<u8>,
}

impl DrtHeaderAux {
    /// Creates new deposit request metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if the destination exceeds the maximum allowed length
    pub fn new(recovery_pk: [u8; 32], destination: VarVec<u8>) -> Result<Self, DrtHeaderAuxError> {
        if destination.len() > MAX_DESTINATION_LEN {
            return Err(DrtHeaderAuxError::DestinationTooLong {
                expected: MAX_DESTINATION_LEN,
                actual: destination.len(),
            });
        }

        Ok(Self {
            recovery_pk,
            destination,
        })
    }

    /// Returns the recovery public key
    pub const fn recovery_pk(&self) -> &[u8; 32] {
        &self.recovery_pk
    }

    /// Returns the destination descriptor.
    pub fn destination(&self) -> &VarVec<u8> {
        &self.destination
    }

    /// Parses auxiliary data from a byte slice.
    pub fn from_aux_data(aux_data: &[u8]) -> Result<Self, DrtHeaderAuxError> {
        if aux_data.len() < 32 {
            return Err(DrtHeaderAuxError::TooShort(aux_data.len()));
        }

        let recovery_pk: [u8; 32] = aux_data[..32]
            .try_into()
            .expect("slice is exactly 32 bytes");
        let destination = aux_data[32..].to_vec();

        if destination.len() > MAX_DESTINATION_LEN {
            return Err(DrtHeaderAuxError::DestinationTooLong {
                expected: MAX_DESTINATION_LEN,
                actual: destination.len(),
            });
        }

        let destination = VarVec::from_vec(destination).expect("valid destination");

        Ok(Self {
            recovery_pk,
            destination,
        })
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
        // Create aux data: first 32 bytes from recovery_pk, remaining bytes from destination
        let mut aux_data = Vec::with_capacity(32 + self.destination.len());
        aux_data.extend_from_slice(&self.recovery_pk);
        aux_data.extend_from_slice(&self.destination);

        TagData::new(
            BRIDGE_V1_SUBPROTOCOL_ID,
            BridgeTxType::DepositRequest as u8,
            aux_data,
        )
        .expect("deposit request tag data should always fit within SPS-50 limits")
    }
}

impl<'a> Arbitrary<'a> for DrtHeaderAux {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let mut recovery_pk = [0u8; 32];
        u.fill_buffer(&mut recovery_pk)?;

        let destination_len = u.int_in_range(0..=MAX_DESTINATION_LEN)?;
        let mut destination_bytes = vec![0u8; destination_len];
        u.fill_buffer(&mut destination_bytes)?;

        let destination =
            VarVec::from_vec(destination_bytes).expect("destination is within bounds");

        Ok(Self {
            recovery_pk,
            destination,
        })
    }
}
