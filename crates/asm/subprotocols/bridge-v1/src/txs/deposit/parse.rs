use bitcoin::{OutPoint, taproot::TAPROOT_CONTROL_NODE_SIZE};
use strata_asm_common::TxInputRef;
use strata_primitives::{
    buf::Buf32,
    l1::{BitcoinAmount, OutputRef},
};

use crate::{constants::DEPOSIT_TX_TYPE, errors::DepositError, txs::deposit::DEPOSIT_OUTPUT_INDEX};

/// Length of the deposit index field in the auxiliary data (4 bytes for u32)
pub const DEPOSIT_IDX_LEN: usize = size_of::<u32>();

/// Length of the tapscript root hash in the auxiliary data (32 bytes)
pub const TAPSCRIPT_ROOT_LEN: usize = TAPROOT_CONTROL_NODE_SIZE;

/// Minimum length of auxiliary data (fixed fields only, excluding variable destination address)
pub const MIN_AUX_DATA_LEN: usize = DEPOSIT_IDX_LEN + TAPSCRIPT_ROOT_LEN;

/// Information extracted from a Bitcoin deposit transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepositInfo {
    /// The index of the deposit in the bridge's deposit table.
    pub deposit_idx: u32,

    /// The amount of Bitcoin deposited.
    pub amt: BitcoinAmount,

    /// The destination address for the deposit.
    pub address: Vec<u8>,

    /// The outpoint of the deposit transaction.
    pub outpoint: OutputRef,

    /// The tapnode hash (merkle root) from the Deposit Request Transaction (DRT) being spent.
    ///
    /// This value is extracted from the auxiliary data and represents the merkle root of the
    /// tapscript tree from the DRT that this deposit transaction is spending. It is combined
    /// with the internal key (aggregated operator key) to reconstruct the taproot address
    /// that was used in the DRT's P2TR output.
    ///
    /// This is required to verify that the transaction was indeed signed by the claimed pubkey.
    /// Without this validation, someone could send funds to the N-of-N address without proper
    /// authorization, which would mint tokens but break the peg since there would be no presigned
    /// withdrawal transactions. This would require N-of-N trust for withdrawals instead of the
    /// intended 1-of-N trust assumption with presigned transactions.
    pub drt_tapnode_hash: Buf32,
}

/// Extracts deposit information from a Bitcoin bridge deposit transaction.
///
/// Parses a deposit transaction following the SPS-50 specification and extracts
/// the deposit information including amount, destination address, and validation data.
/// See the module-level documentation for the complete transaction structure.
///
/// # Parameters
///
/// - `tx_input` - Reference to the transaction input containing the deposit transaction and its
///   associated tag data
///
/// # Returns
///
/// - `Ok(DepositInfo)` - Successfully parsed deposit information
/// - `Err(DepositError)` - If the transaction structure is invalid, signature verification fails,
///   or any parsing step encounters malformed data
pub fn extract_deposit_info<'a>(tx_input: &TxInputRef<'a>) -> Result<DepositInfo, DepositError> {
    if tx_input.tag().tx_type() != DEPOSIT_TX_TYPE {
        return Err(DepositError::InvalidTxType {
            expected: DEPOSIT_TX_TYPE,
            actual: tx_input.tag().tx_type(),
        });
    }

    let aux_data = tx_input.tag().aux_data();

    // Validate minimum auxiliary data length (must have at least the fixed fields)
    if aux_data.len() < MIN_AUX_DATA_LEN {
        return Err(DepositError::InvalidAuxiliaryData(aux_data.len()));
    }

    // Parse deposit index (bytes 0-3)
    let (deposit_idx_bytes, rest) = aux_data.split_at(DEPOSIT_IDX_LEN);
    let deposit_idx = u32::from_be_bytes(
        deposit_idx_bytes
            .try_into()
            .expect("Expected deposit index to be 4 bytes"),
    );

    // Parse tapscript root hash (bytes 4-35)
    let (tapscript_root_bytes, destination_address) = rest.split_at(TAPSCRIPT_ROOT_LEN);
    let tapscript_root = Buf32::new(
        tapscript_root_bytes
            .try_into()
            .expect("Expected tapscript root to be 32 bytes"),
    );

    // Destination address is remaining bytes (bytes 36+)
    // Must have at least 1 byte for destination address
    if destination_address.is_empty() {
        return Err(DepositError::InvalidAuxiliaryData(aux_data.len()));
    }

    // Extract the deposit output (second output at index 1)
    let deposit_output = tx_input
        .tx()
        .output
        .get(DEPOSIT_OUTPUT_INDEX as usize)
        .ok_or(DepositError::MissingOutput(1))?;

    // Create outpoint reference for the deposit output
    let deposit_outpoint = OutputRef::from(OutPoint {
        txid: tx_input.tx().compute_txid(),
        vout: DEPOSIT_OUTPUT_INDEX,
    });

    // Construct the validated deposit information
    Ok(DepositInfo {
        deposit_idx,
        amt: deposit_output.value.into(),
        address: destination_address.to_vec(),
        outpoint: deposit_outpoint,
        drt_tapnode_hash: tapscript_root,
    })
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        Amount, ScriptBuf, Transaction,
        secp256k1::{Secp256k1, SecretKey},
    };
    use strata_asm_common::TxInputRef;
    use strata_l1_txfmt::ParseConfig;
    use strata_primitives::{buf::Buf32, l1::XOnlyPk};

    use super::*;
    use crate::txs::deposit::test::{TEST_MAGIC_BYTES, create_test_deposit_tx};

    // Test data constants
    const TEST_DEPOSIT_IDX: u32 = 123;
    const TEST_TAPSCRIPT_ROOT: [u8; 32] = [0xAB; 32];
    const TEST_DESTINATION: &[u8] = b"test_destination";
    const TEST_DEPOSIT_AMOUNT: u64 = 1000000;

    // Helper function to create a test operator keypair
    fn create_test_operator_keypair() -> (XOnlyPk, SecretKey) {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[1u8; 32]).unwrap();
        let keypair = bitcoin::secp256k1::Keypair::from_secret_key(&secp, &secret_key);
        let (xonly_pk, _) = keypair.x_only_public_key();
        let operators_pubkey = XOnlyPk::new(Buf32::new(xonly_pk.serialize())).expect("Valid public key");
        (operators_pubkey, secret_key)
    }

    // Helper function to create a test transaction and parse it (for success test)
    fn create_and_parse_test_tx() -> TxInputRef<'static> {
        let (operators_pubkey, operators_privkey) = create_test_operator_keypair();
        let tx = create_test_deposit_tx(
            TEST_DEPOSIT_IDX,
            TEST_TAPSCRIPT_ROOT,
            TEST_DESTINATION,
            Amount::from_sat(TEST_DEPOSIT_AMOUNT),
            &operators_pubkey,
            &operators_privkey,
        );

        // Leak the transaction to get 'static lifetime for TxInputRef
        let tx_static: &'static Transaction = Box::leak(Box::new(tx));
        let parser = ParseConfig::new(*TEST_MAGIC_BYTES);
        let tag_data = parser
            .try_parse_tx(tx_static)
            .expect("Should parse transaction");
        TxInputRef::new(tx_static, tag_data)
    }

    // Helper function to create a test transaction (for mutation tests)
    fn create_test_tx() -> Transaction {
        let (operators_pubkey, operators_privkey) = create_test_operator_keypair();
        create_test_deposit_tx(
            TEST_DEPOSIT_IDX,
            TEST_TAPSCRIPT_ROOT,
            TEST_DESTINATION,
            Amount::from_sat(TEST_DEPOSIT_AMOUNT),
            &operators_pubkey,
            &operators_privkey,
        )
    }

    // Helper function to create tagged payload with custom parameters
    fn create_tagged_payload(subprotocol_id: u8, tx_type: u8, aux_data: Vec<u8>) -> Vec<u8> {
        let mut tagged_payload = Vec::new();
        tagged_payload.extend_from_slice(TEST_MAGIC_BYTES);
        tagged_payload.push(subprotocol_id); // 1 byte subprotocol ID
        tagged_payload.push(tx_type); // 1 byte transaction type
        tagged_payload.extend_from_slice(&aux_data);
        tagged_payload
    }

    // Helper function to mutate transaction OP_RETURN output
    fn mutate_op_return_output(tx: &mut Transaction, tagged_payload: Vec<u8>) {
        use bitcoin::script::PushBytesBuf;
        tx.output[0].script_pubkey =
            ScriptBuf::new_op_return(PushBytesBuf::try_from(tagged_payload).unwrap());
    }

    // Helper function to parse mutated transaction
    fn parse_mutated_tx(tx: &Transaction) -> TxInputRef<'_> {
        let parser = ParseConfig::new(*TEST_MAGIC_BYTES);
        let tag_data = parser.try_parse_tx(tx).expect("Should parse transaction");
        TxInputRef::new(tx, tag_data)
    }

    #[test]
    fn test_extract_deposit_info_success() {
        let tx_input = create_and_parse_test_tx();

        // Test the actual parsing logic by calling extract_deposit_info
        let result = extract_deposit_info(&tx_input);
        assert!(result.is_ok(), "Should successfully parse valid deposit info");

        let deposit_info = result.unwrap();
        assert_eq!(deposit_info.deposit_idx, TEST_DEPOSIT_IDX);
        assert_eq!(deposit_info.amt, Amount::from_sat(TEST_DEPOSIT_AMOUNT).into());
        assert_eq!(deposit_info.address, TEST_DESTINATION);
        assert_eq!(deposit_info.drt_tapnode_hash, Buf32::new(TEST_TAPSCRIPT_ROOT));
    }

    #[test]
    fn test_extract_deposit_info_invalid_tx_type() {
        use crate::constants::BRIDGE_V1_SUBPROTOCOL_ID;

        let mut tx = create_test_tx();

        // Mutate the OP_RETURN output to have wrong transaction type
        let aux_data = vec![0u8; 40]; // Some dummy aux data
        let tagged_payload = create_tagged_payload(BRIDGE_V1_SUBPROTOCOL_ID, 99, aux_data);
        mutate_op_return_output(&mut tx, tagged_payload);

        let tx_input = parse_mutated_tx(&tx);
        let result = extract_deposit_info(&tx_input);
        assert!(result.is_err(), "Should fail with invalid transaction type");

        assert!(matches!(result, Err(DepositError::InvalidTxType { .. })));
        if let Err(DepositError::InvalidTxType { expected, actual }) = result {
            assert_eq!(expected, DEPOSIT_TX_TYPE);
            assert_eq!(actual, tx_input.tag().tx_type());
        }
    }

    #[test]
    fn test_extract_deposit_info_invalid_aux_data_too_short() {
        use crate::constants::{BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_TX_TYPE};

        let mut tx = create_test_tx();

        // Mutate the OP_RETURN output to have short aux data
        let short_aux_data = vec![0u8; MIN_AUX_DATA_LEN - 1];
        let tagged_payload = create_tagged_payload(BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_TX_TYPE, short_aux_data);
        mutate_op_return_output(&mut tx, tagged_payload);

        let tx_input = parse_mutated_tx(&tx);
        let result = extract_deposit_info(&tx_input);
        assert!(result.is_err(), "Should fail with insufficient auxiliary data");

        assert!(matches!(result, Err(DepositError::InvalidAuxiliaryData(_))));
        if let Err(DepositError::InvalidAuxiliaryData(len)) = result {
            assert_eq!(len, MIN_AUX_DATA_LEN - 1);
        }
    }

    #[test]
    fn test_extract_deposit_info_no_destination() {
        use crate::constants::{BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_TX_TYPE};

        let mut tx = create_test_tx();

        // Mutate the OP_RETURN output to have aux data with no destination (exactly MIN_AUX_DATA_LEN)
        let aux_data = vec![0u8; MIN_AUX_DATA_LEN];
        let tagged_payload = create_tagged_payload(BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_TX_TYPE, aux_data);
        mutate_op_return_output(&mut tx, tagged_payload);

        let tx_input = parse_mutated_tx(&tx);
        let result = extract_deposit_info(&tx_input);
        assert!(result.is_err(), "Should fail with empty destination");

        assert!(matches!(result, Err(DepositError::InvalidAuxiliaryData(_))));
        if let Err(DepositError::InvalidAuxiliaryData(len)) = result {
            assert_eq!(len, MIN_AUX_DATA_LEN);
        }
    }

    #[test]
    fn test_extract_deposit_info_missing_output() {
        let mut tx = create_test_tx();

        // Remove the deposit output (keep only OP_RETURN at index 0)
        tx.output.truncate(1);

        let tx_input = parse_mutated_tx(&tx);
        let result = extract_deposit_info(&tx_input);
        assert!(result.is_err(), "Should fail with missing deposit output");

        assert!(matches!(result, Err(DepositError::MissingOutput(_))));
        if let Err(DepositError::MissingOutput(index)) = result {
            assert_eq!(index, 1);
        }
    }
}
