//! DRT parsing using SPS-50 format.
//!
//! SPS-50 structure:
//! - OP_RETURN: [MAGIC (4)][SUBPROTOCOL_ID (1)][TX_TYPE (1)][RECOVERY_PK (32)][EE_ADDRESS
//!   (variable)]

use bitcoin::Transaction;
use strata_asm_common::TxInputRef;
use strata_l1_txfmt::ParseConfig;
use strata_primitives::l1::DepositRequestInfo;

use crate::{constants::BridgeTxType, errors::DepositRequestParseError};

const RECOVERY_PK_LEN: usize = 32;

/// Minimum length of auxiliary data for deposit request transactions
pub const MIN_DRT_AUX_DATA_LEN: usize = RECOVERY_PK_LEN;

pub fn parse_drt(
    tx_input: &TxInputRef<'_>,
) -> Result<DepositRequestInfo, DepositRequestParseError> {
    if tx_input.tag().tx_type() != BridgeTxType::DepositRequest as u8 {
        return Err(DepositRequestParseError::InvalidTxType {
            actual: tx_input.tag().tx_type(),
            expected: BridgeTxType::DepositRequest as u8,
        });
    }

    let aux_data = tx_input.tag().aux_data();

    if aux_data.len() < MIN_DRT_AUX_DATA_LEN {
        return Err(DepositRequestParseError::InvalidAuxiliaryData(
            aux_data.len(),
        ));
    }

    let (recovery_pk_bytes, ee_address) = aux_data.split_at(RECOVERY_PK_LEN);
    let recovery_pk: [u8; 32] = recovery_pk_bytes
        .try_into()
        .expect("validated aux_data length");

    // Per spec: Output 1 must be the P2TR deposit request output
    let drt_output = tx_input
        .tx()
        .output
        .get(1)
        .ok_or(DepositRequestParseError::MissingDRTOutput)?;

    let amt = drt_output.value.to_sat();

    Ok(DepositRequestInfo {
        amt,
        take_back_leaf_hash: recovery_pk,
        address: ee_address.to_vec(),
    })
}

/// Parses a DRT from a raw transaction with magic bytes
///
/// Validates that the transaction follows the SPS-50 DRT specification:
/// - Output 0 must be OP_RETURN with tagged data
/// - Output 1 must be the P2TR deposit output
///
/// # Arguments
/// * `tx` - The DRT transaction to parse
/// * `magic_bytes` - The SPS-50 magic bytes for this network
///
/// # Returns
/// The parsed deposit request information
pub fn parse_drt_from_tx(
    tx: &Transaction,
    magic_bytes: &[u8; 4],
) -> Result<DepositRequestInfo, DepositRequestParseError> {
    // Validate OP_RETURN is at index 0 per spec
    if tx.output.is_empty() || !tx.output[0].script_pubkey.is_op_return() {
        return Err(DepositRequestParseError::NoOpReturnOutput);
    }

    let parse_config = ParseConfig::new(*magic_bytes);
    let tag_data = parse_config
        .try_parse_tx(tx)
        .map_err(|e| DepositRequestParseError::Sps50ParseError(e.to_string()))?;

    let tx_input = TxInputRef::new(tx, tag_data);
    parse_drt(&tx_input)
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
        absolute::LockTime, transaction::Version,
    };
    use strata_l1_txfmt::{ParseConfig, TagData};

    use super::*;
    use crate::{
        constants::{BRIDGE_V1_SUBPROTOCOL_ID, BridgeTxType},
        test_utils::{TEST_MAGIC_BYTES, parse_tx},
    };

    fn create_test_drt_tx(
        recovery_pk: [u8; 32],
        ee_address: &[u8],
        amount_sats: u64,
    ) -> Transaction {
        // Build aux_data
        let mut aux_data = Vec::new();
        aux_data.extend_from_slice(&recovery_pk);
        aux_data.extend_from_slice(ee_address);

        // Create OP_RETURN script following SPS-50 format
        let td = TagData::new(
            BRIDGE_V1_SUBPROTOCOL_ID,
            BridgeTxType::DepositRequest as u8,
            aux_data,
        )
        .expect("valid tag data");
        let sps_50_script = ParseConfig::new(*TEST_MAGIC_BYTES)
            .encode_script_buf(&td.as_ref())
            .expect("encode OP_RETURN script");

        // Create base transaction
        let mut tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::new(),
            }],
            output: vec![
                // Output 0: Placeholder for OP_RETURN (will be mutated)
                TxOut {
                    value: Amount::ZERO,
                    script_pubkey: ScriptBuf::new(),
                },
                // Output 1: P2TR with the deposited amount
                TxOut {
                    value: Amount::from_sat(amount_sats),
                    script_pubkey: ScriptBuf::new(), // Simplified for test
                },
            ],
        };

        // Set OP_RETURN output script
        tx.output[0].script_pubkey = sps_50_script;
        tx
    }

    #[test]
    fn test_parse_drt_valid() {
        let recovery_pk = [0x05; 32];
        let ee_address = [0x06; 20];
        let amount = 1_000_000_000;

        let tx = create_test_drt_tx(recovery_pk, &ee_address, amount);
        let tx_input = parse_tx(&tx);

        let result = parse_drt(&tx_input);
        assert!(result.is_ok());

        let info = result.unwrap();
        assert_eq!(info.take_back_leaf_hash, recovery_pk);
        assert_eq!(info.address, ee_address.to_vec());
        assert_eq!(info.amt, amount);
    }

    #[test]
    fn test_parse_drt_invalid_tx_type() {
        let recovery_pk = [0x05; 32];
        let ee_address = [0x06; 20];

        // Build aux_data
        let mut aux_data = Vec::new();
        aux_data.extend_from_slice(&recovery_pk);
        aux_data.extend_from_slice(&ee_address);

        // Create OP_RETURN script with wrong tx_type (99 instead of DEPOSIT_REQUEST_TX_TYPE)
        let td = TagData::new(BRIDGE_V1_SUBPROTOCOL_ID, 99, aux_data).unwrap();
        let sps_50_script = ParseConfig::new(*TEST_MAGIC_BYTES)
            .encode_script_buf(&td.as_ref())
            .unwrap();

        let mut tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![],
            output: vec![
                TxOut {
                    value: Amount::ZERO,
                    script_pubkey: ScriptBuf::new(),
                },
                TxOut {
                    value: Amount::from_sat(1000),
                    script_pubkey: ScriptBuf::new(),
                },
            ],
        };

        tx.output[0].script_pubkey = sps_50_script;
        let tx_input = parse_tx(&tx);

        let result = parse_drt(&tx_input);
        assert!(matches!(
            result,
            Err(DepositRequestParseError::InvalidTxType {
                actual: 99,
                expected: 0
            })
        ));
    }

    #[test]
    fn test_parse_drt_insufficient_aux_data() {
        // Create aux_data with only 10 bytes (need at least 32)
        let aux_data = vec![0x05; 10];

        // Create OP_RETURN script with insufficient aux_data
        let td = TagData::new(
            BRIDGE_V1_SUBPROTOCOL_ID,
            BridgeTxType::DepositRequest as u8,
            aux_data,
        )
        .unwrap();
        let sps_50_script = ParseConfig::new(*TEST_MAGIC_BYTES)
            .encode_script_buf(&td.as_ref())
            .unwrap();

        let mut tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![],
            output: vec![
                TxOut {
                    value: Amount::ZERO,
                    script_pubkey: ScriptBuf::new(),
                },
                TxOut {
                    value: Amount::from_sat(1000),
                    script_pubkey: ScriptBuf::new(),
                },
            ],
        };

        tx.output[0].script_pubkey = sps_50_script;
        let tx_input = parse_tx(&tx);

        let result = parse_drt(&tx_input);
        assert!(matches!(
            result,
            Err(DepositRequestParseError::InvalidAuxiliaryData(10))
        ));
    }

    #[test]
    fn test_parse_drt_variable_ee_address_lengths() {
        let recovery_pk = [0x05; 32];

        // Test with 20-byte EVM address
        let ee_address_20 = [0x06; 20];
        let tx_20 = create_test_drt_tx(recovery_pk, &ee_address_20, 1_000_000);
        let tx_input_20 = parse_tx(&tx_20);
        let result_20 = parse_drt(&tx_input_20).unwrap();
        assert_eq!(result_20.address.len(), 20);

        // Test with 32-byte address
        let ee_address_32 = [0x07; 32];
        let tx_32 = create_test_drt_tx(recovery_pk, &ee_address_32, 1_000_000);
        let tx_input_32 = parse_tx(&tx_32);
        let result_32 = parse_drt(&tx_input_32).unwrap();
        assert_eq!(result_32.address.len(), 32);
    }
}
