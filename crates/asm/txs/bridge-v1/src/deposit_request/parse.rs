use strata_asm_common::TxInputRef;
use strata_codec::decode_buf_exact;
use strata_primitives::l1::DepositRequestInfo;

use crate::{
    deposit_request::{DRT_OUTPUT_INDEX, DrtHeaderAux},
    errors::DepositRequestParseError,
};

/// Parses deposit request transaction to extract [`DepositRequestInfo`].
///
/// Parses a deposit request transaction following the SPS-50 specification and extracts the
/// decoded auxiliary data ([`DepositRequestAuxData`]) along with the deposit amount. The
/// auxiliary data is encoded with [`strata_codec::Codec`] and includes the recovery public key
/// and destination address.
///
/// # Errors
///
/// Returns [`DepositRequestParseError`] if the auxiliary data cannot be decoded or if the expected
/// deposit request output at index 1 is missing.
pub fn parse_drt(
    tx_input: &TxInputRef<'_>,
) -> Result<DepositRequestInfo, DepositRequestParseError> {
    // Parse auxiliary data using DepositRequestAuxData
    let aux_data: DrtHeaderAux = decode_buf_exact(tx_input.tag().aux_data())?;

    // Extract the deposit request output (second output at index 1)
    let drt_output = tx_input
        .tx()
        .output
        .get(DRT_OUTPUT_INDEX)
        .ok_or(DepositRequestParseError::MissingDRTOutput)?;

    let amt = drt_output.value.to_sat();

    // Construct the validated deposit request information
    Ok(DepositRequestInfo {
        amt,
        take_back_leaf_hash: *aux_data.recovery_pk(),
        address: aux_data.ee_address().to_vec(),
    })
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
        test_utils::{TEST_MAGIC_BYTES, parse_sps50_tx},
    };

    // Helper function to create a test DRT transaction
    fn create_test_drt_tx(
        recovery_pk: [u8; 32],
        ee_address: &[u8],
        amount_sats: u64,
    ) -> Transaction {
        // Build aux_data: recovery_pk + ee_address
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
                // Output 0: OP_RETURN with tagged data
                TxOut {
                    value: Amount::ZERO,
                    script_pubkey: ScriptBuf::new(),
                },
                // Output 1: P2TR deposit request output
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
    fn test_parse_drt_success() {
        let recovery_pk = [0x05; 32];
        let ee_address = [0x06; 20];
        let amount = 1_000_000_000;

        let tx = create_test_drt_tx(recovery_pk, &ee_address, amount);
        let tx_input = parse_sps50_tx(&tx);

        let result = parse_drt(&tx_input).expect("Should successfully parse DRT");

        assert_eq!(result.take_back_leaf_hash, recovery_pk);
        assert_eq!(result.address, ee_address.to_vec());
        assert_eq!(result.amt, amount);
    }

    #[test]
    fn test_parse_drt_aux_data_too_short() {
        // Create aux_data with only 10 bytes (need at least 32 for recovery_pk)
        let aux_data = vec![0x05; 10];

        // Create OP_RETURN script with insufficient aux_data
        let td = TagData::new(
            BRIDGE_V1_SUBPROTOCOL_ID,
            BridgeTxType::DepositRequest as u8,
            aux_data,
        )
        .expect("valid tag data");
        let sps_50_script = ParseConfig::new(*TEST_MAGIC_BYTES)
            .encode_script_buf(&td.as_ref())
            .expect("encode OP_RETURN script");

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
        let tx_input = parse_sps50_tx(&tx);

        let err = parse_drt(&tx_input).unwrap_err();
        assert!(matches!(
            err,
            DepositRequestParseError::InvalidAuxiliaryData(_)
        ));
    }

    #[test]
    fn test_parse_drt_empty_ee_address() {
        let recovery_pk = [0x05; 32];
        let amount = 1_000_000;

        let tx = create_test_drt_tx(recovery_pk, &[], amount);
        let tx_input = parse_sps50_tx(&tx);

        let result = parse_drt(&tx_input);
        assert!(result.is_ok(), "Should succeed with empty EE address");

        let info = result.unwrap();
        assert!(info.address.is_empty(), "Address should be empty");
        assert_eq!(info.take_back_leaf_hash, recovery_pk);
    }

    #[test]
    fn test_parse_drt_missing_output() {
        let recovery_pk = [0x05; 32];
        let ee_address = [0x06; 20];

        // Build aux_data
        let mut aux_data = Vec::new();
        aux_data.extend_from_slice(&recovery_pk);
        aux_data.extend_from_slice(&ee_address);

        // Create OP_RETURN script
        let td = TagData::new(
            BRIDGE_V1_SUBPROTOCOL_ID,
            BridgeTxType::DepositRequest as u8,
            aux_data,
        )
        .expect("valid tag data");
        let sps_50_script = ParseConfig::new(*TEST_MAGIC_BYTES)
            .encode_script_buf(&td.as_ref())
            .expect("encode OP_RETURN script");

        let mut tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![],
            output: vec![TxOut {
                value: Amount::ZERO,
                script_pubkey: ScriptBuf::new(),
            }],
        };

        // Remove the DRT output (keep only OP_RETURN at index 0)
        tx.output[0].script_pubkey = sps_50_script;
        let tx_input = parse_sps50_tx(&tx);

        let err = parse_drt(&tx_input).unwrap_err();
        assert!(matches!(err, DepositRequestParseError::MissingDRTOutput));
    }

    #[test]
    fn test_parse_drt_variable_ee_address_lengths() {
        let recovery_pk = [0x05; 32];

        // Test with 20-byte EVM address
        let ee_address_20 = [0x06; 20];
        let tx_20 = create_test_drt_tx(recovery_pk, &ee_address_20, 1_000_000);
        let tx_input_20 = parse_sps50_tx(&tx_20);
        let result_20 = parse_drt(&tx_input_20).expect("Should parse 20-byte address");
        assert_eq!(result_20.address.len(), 20);

        // Test with 32-byte address
        let ee_address_32 = [0x07; 32];
        let tx_32 = create_test_drt_tx(recovery_pk, &ee_address_32, 1_000_000);
        let tx_input_32 = parse_sps50_tx(&tx_32);
        let result_32 = parse_drt(&tx_input_32).expect("Should parse 32-byte address");
        assert_eq!(result_32.address.len(), 32);
    }
}
