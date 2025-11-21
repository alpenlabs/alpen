//! Deposit request transaction building utilities

use arbitrary::{Arbitrary, Unstructured};
use bitcoin::XOnlyPublicKey;
use strata_l1_txfmt::{ParseConfig, TagDataRef};

use crate::{
    constants::{BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_REQUEST_TX_TYPE},
    errors::DepositRequestBuildError,
};

/// Metadata for creating deposit request transactions
///
/// Contains the information needed to create the OP_RETURN output for
/// a deposit request transaction following the SPS-50 specification.
///
/// SPS-50 format: \[MAGIC\]\[SUBPROTOCOL_ID\]\[TX_TYPE\]\[RECOVERY_PK (32)\]\[EE_ADDRESS\]
#[derive(Debug, Clone)]
pub struct DepositRequestMetadata {
    recovery_pk: [u8; 32],
    ee_address: Vec<u8>,
}

impl<'a> Arbitrary<'a> for DepositRequestMetadata {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let recovery_pk = <[u8; 32]>::arbitrary(u)?;
        // Generate address between 20 and 64 bytes (reasonable range for EE addresses)
        let addr_len = u.int_in_range(20..=64)?;
        let ee_address = (0..addr_len)
            .map(|_| u8::arbitrary(u))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(DepositRequestMetadata {
            recovery_pk,
            ee_address,
        })
    }
}

impl DepositRequestMetadata {
    /// Creates new deposit request metadata
    pub fn new(recovery_pk: XOnlyPublicKey, ee_address: Vec<u8>) -> Self {
        Self {
            recovery_pk: recovery_pk.serialize(),
            ee_address,
        }
    }

    /// Generates the OP_RETURN data for the deposit request
    ///
    /// # Arguments
    /// * `magic_bytes` - Network-specific magic bytes for SPS-50 tagging
    ///
    /// # Returns
    /// Complete OP_RETURN payload including magic bytes, protocol markers, and auxiliary data
    pub fn op_return_data(
        &self,
        magic_bytes: [u8; 4],
    ) -> Result<Vec<u8>, DepositRequestBuildError> {
        let mut aux_data = Vec::new();
        aux_data.extend_from_slice(&self.recovery_pk);
        aux_data.extend_from_slice(&self.ee_address);

        let tag_data =
            TagDataRef::new(BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_REQUEST_TX_TYPE, &aux_data)
                .map_err(|e| DepositRequestBuildError::TxFmt(e.to_string()))?;

        let parse_config = ParseConfig::new(magic_bytes);
        let data = parse_config
            .encode_tag_buf(&tag_data)
            .map_err(|e| DepositRequestBuildError::TxFmt(e.to_string()))?;

        Ok(data)
    }

    /// Returns the recovery public key
    pub fn recovery_pk(&self) -> &[u8; 32] {
        &self.recovery_pk
    }

    /// Returns the execution environment address
    pub fn ee_address(&self) -> &[u8] {
        &self.ee_address
    }
}

#[cfg(test)]
mod tests {
    use strata_test_utils::ArbitraryGenerator;

    use super::*;

    #[test]
    fn test_deposit_request_metadata_creation() {
        let mut arb = ArbitraryGenerator::new();
        let metadata: DepositRequestMetadata = arb.generate();

        // Verify accessors work
        assert_eq!(metadata.recovery_pk().len(), 32);
        assert!(!metadata.ee_address().is_empty());
    }

    #[test]
    fn test_op_return_data_format() {
        let mut arb = ArbitraryGenerator::new();
        let metadata: DepositRequestMetadata = arb.generate();
        let magic: [u8; 4] = arb.generate();

        let op_return_data = metadata.op_return_data(magic).unwrap();

        // Verify total length: magic(4) + subprotocol_id(1) + tx_type(1) + recovery_pk(32) +
        // address
        let expected_len = 4 + 1 + 1 + 32 + metadata.ee_address().len();
        assert_eq!(op_return_data.len(), expected_len);

        // Verify magic bytes at start
        assert_eq!(&op_return_data[0..4], &magic);

        // Verify subprotocol_id
        assert_eq!(op_return_data[4], BRIDGE_V1_SUBPROTOCOL_ID);

        // Verify tx_type
        assert_eq!(op_return_data[5], DEPOSIT_REQUEST_TX_TYPE);

        // Verify recovery pk follows (starts at offset 6)
        assert_eq!(&op_return_data[6..38], metadata.recovery_pk());

        // Verify ee address at end (starts at offset 38)
        assert_eq!(&op_return_data[38..], metadata.ee_address());
    }

    #[test]
    fn test_op_return_data_different_address_lengths() {
        let mut arb = ArbitraryGenerator::new();
        let magic: [u8; 4] = arb.generate();

        // Test with 20-byte address (EVM standard)
        let metadata_20 = DepositRequestMetadata {
            recovery_pk: arb.generate(),
            ee_address: vec![0x06; 20],
        };
        assert_eq!(
            metadata_20.op_return_data(magic).unwrap().len(),
            4 + 1 + 1 + 32 + 20
        );

        // Test with 32-byte address (different EE)
        let metadata_32 = DepositRequestMetadata {
            recovery_pk: arb.generate(),
            ee_address: vec![0x07; 32],
        };
        assert_eq!(
            metadata_32.op_return_data(magic).unwrap().len(),
            4 + 1 + 1 + 32 + 32
        );
    }
}
