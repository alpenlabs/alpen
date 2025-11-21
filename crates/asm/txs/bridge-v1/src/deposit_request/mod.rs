//! DRT building and parsing. This is present because we are treating asm/txs as a canonical for
//! building and parsing transactions

use bitcoin::XOnlyPublicKey;
use strata_l1_txfmt::{ParseConfig, TagDataRef};

use crate::constants::{BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_REQUEST_TX_TYPE};

pub mod parse;

pub use parse::{parse_drt, parse_drt_from_tx, DepositRequestParseError};

/// SPS-50 format: [MAGIC][SUBPROTOCOL_ID][TX_TYPE][RECOVERY_PK (32)][EE_ADDRESS]
#[derive(Debug, Clone)]
pub struct DepositRequestMetadata {
    recovery_pk: [u8; 32],
    ee_address: Vec<u8>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum DepositRequestBuildError {
    #[error("SPS-50 format error: {0}")]
    TxFmt(String),
}

impl DepositRequestMetadata {
    pub fn new(recovery_pk: XOnlyPublicKey, ee_address: Vec<u8>) -> Self {
        Self {
            recovery_pk: recovery_pk.serialize(),
            ee_address,
        }
    }

    pub fn op_return_data(&self, magic_bytes: [u8; 4]) -> Result<Vec<u8>, DepositRequestBuildError> {
        let mut aux_data = Vec::new();
        aux_data.extend_from_slice(&self.recovery_pk);
        aux_data.extend_from_slice(&self.ee_address);

        let tag_data = TagDataRef::new(BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_REQUEST_TX_TYPE, &aux_data)
            .map_err(|e| DepositRequestBuildError::TxFmt(e.to_string()))?;

        let parse_config = ParseConfig::new(magic_bytes);
        let data = parse_config
            .encode_tag_buf(&tag_data)
            .map_err(|e| DepositRequestBuildError::TxFmt(e.to_string()))?;

        Ok(data)
    }

    pub fn recovery_pk(&self) -> &[u8; 32] {
        &self.recovery_pk
    }

    pub fn ee_address(&self) -> &[u8] {
        &self.ee_address
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};

    use super::*;

    fn generate_test_xonly_pk() -> XOnlyPublicKey {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[
            0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
            0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
            0x01, 0x01, 0x01, 0x01,
        ])
        .unwrap();
        let keypair = Keypair::from_secret_key(&secp, &secret_key);
        XOnlyPublicKey::from(keypair.public_key())
    }

    #[test]
    fn test_deposit_request_metadata_creation() {
        let recovery_pk = generate_test_xonly_pk();
        let recovery_pk_bytes = recovery_pk.serialize();
        let ee_address = vec![0x06; 20];

        let metadata = DepositRequestMetadata::new(recovery_pk, ee_address.clone());

        assert_eq!(metadata.recovery_pk(), &recovery_pk_bytes);
        assert_eq!(metadata.ee_address(), &ee_address);
    }

    #[test]
    fn test_op_return_data_format() {
        let magic = [0xAA, 0xBB, 0xCC, 0xDD];
        let recovery_pk = generate_test_xonly_pk();
        let recovery_pk_bytes = recovery_pk.serialize();
        let ee_address = vec![0x22; 20];

        let metadata = DepositRequestMetadata::new(recovery_pk, ee_address);
        let op_return_data = metadata.op_return_data(magic).unwrap();

        // Verify total length: magic(4) + subprotocol_id(1) + tx_type(1) + recovery_pk(32) + address(20)
        assert_eq!(op_return_data.len(), 4 + 1 + 1 + 32 + 20);

        // Verify magic bytes at start
        assert_eq!(&op_return_data[0..4], &[0xAA, 0xBB, 0xCC, 0xDD]);

        // Verify subprotocol_id
        assert_eq!(op_return_data[4], BRIDGE_V1_SUBPROTOCOL_ID);

        // Verify tx_type
        assert_eq!(op_return_data[5], DEPOSIT_REQUEST_TX_TYPE);

        // Verify recovery pk follows (starts at offset 6)
        assert_eq!(&op_return_data[6..38], &recovery_pk_bytes);

        // Verify ee address at end (starts at offset 38)
        assert_eq!(&op_return_data[38..58], &[0x22; 20]);
    }

    #[test]
    fn test_op_return_data_different_address_lengths() {
        let magic = [0x01, 0x02, 0x03, 0x04];
        let recovery_pk = generate_test_xonly_pk();

        // Test with 20-byte address (EVM standard)
        // SPS-50: magic(4) + subprotocol_id(1) + tx_type(1) + recovery_pk(32) + address(20)
        let ee_address_20 = vec![0x06; 20];
        let metadata = DepositRequestMetadata::new(recovery_pk, ee_address_20);
        assert_eq!(metadata.op_return_data(magic).unwrap().len(), 4 + 1 + 1 + 32 + 20);

        // Test with 32-byte address (different EE)
        // SPS-50: magic(4) + subprotocol_id(1) + tx_type(1) + recovery_pk(32) + address(32)
        let ee_address_32 = vec![0x07; 32];
        let metadata = DepositRequestMetadata::new(recovery_pk, ee_address_32);
        assert_eq!(metadata.op_return_data(magic).unwrap().len(), 4 + 1 + 1 + 32 + 32);
    }
}
