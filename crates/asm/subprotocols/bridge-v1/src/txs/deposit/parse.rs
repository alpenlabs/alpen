use arbitrary::Arbitrary;
use bitcoin::{OutPoint, taproot::TAPROOT_CONTROL_NODE_SIZE};
use strata_asm_common::TxInputRef;
use strata_primitives::{
    buf::Buf32,
    l1::{BitcoinAmount, OutputRef},
};

use crate::{
    constants::DEPOSIT_TX_TYPE, errors::DepositTxParseError, txs::deposit::DEPOSIT_OUTPUT_INDEX,
};

/// Length of the deposit index field in the auxiliary data (4 bytes for u32)
const DEPOSIT_IDX_LEN: usize = 4;

/// Length of the tapscript root hash in the auxiliary data (32 bytes)
const TAPSCRIPT_ROOT_LEN: usize = TAPROOT_CONTROL_NODE_SIZE;

/// Minimum length of auxiliary data (fixed fields only, excluding variable destination address)
pub const MIN_DEPOSIT_TX_AUX_DATA_LEN: usize = DEPOSIT_IDX_LEN + TAPSCRIPT_ROOT_LEN;

/// Information extracted from a Bitcoin deposit transaction.
#[derive(Debug, Clone, PartialEq, Eq, Arbitrary)]
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
pub fn extract_deposit_info<'a>(
    tx_input: &TxInputRef<'a>,
) -> Result<DepositInfo, DepositTxParseError> {
    if tx_input.tag().tx_type() != DEPOSIT_TX_TYPE {
        return Err(DepositTxParseError::InvalidTxType(tx_input.tag().tx_type()));
    }

    let aux_data = tx_input.tag().aux_data();

    // Validate minimum auxiliary data length (must have at least the fixed fields)
    if aux_data.len() < MIN_DEPOSIT_TX_AUX_DATA_LEN {
        return Err(DepositTxParseError::InvalidAuxiliaryData(aux_data.len()));
    }

    // Parse deposit index (bytes 0-3)
    let (deposit_idx_bytes, rest) = aux_data.split_at(DEPOSIT_IDX_LEN);
    let deposit_idx = u32::from_be_bytes(
        deposit_idx_bytes
            .try_into()
            .expect("deposit index is exactly 4 bytes because we validate aux_data length early"),
    );

    // Parse tapscript root hash (bytes 4-35)
    let (tapscript_root_bytes, destination_address) = rest.split_at(TAPSCRIPT_ROOT_LEN);
    let tapscript_root =
        Buf32::new(tapscript_root_bytes.try_into().expect(
            "tapscript root is exactly 32 bytes because we validate aux_data length early",
        ));

    // Destination address is remaining bytes (bytes 36+)
    // Allow empty destination address (0 bytes is valid)

    // Extract the deposit output (second output at index 1)
    let deposit_output = tx_input
        .tx()
        .output
        .get(DEPOSIT_OUTPUT_INDEX as usize)
        .ok_or(DepositTxParseError::MissingDepositOutput)?;

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
        OutPoint, ScriptBuf, Transaction,
        secp256k1::{Secp256k1, SecretKey},
    };
    use strata_asm_common::TxInputRef;
    use strata_l1_txfmt::ParseConfig;
    use strata_primitives::{
        buf::Buf32,
        l1::{OutputRef, XOnlyPk},
    };
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::txs::deposit::create::{TEST_MAGIC_BYTES, create_test_deposit_tx};

    // Helper function to create a test operator keypair
    fn create_test_operator_keypair() -> (XOnlyPk, SecretKey) {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[1u8; 32]).unwrap();
        let keypair = bitcoin::secp256k1::Keypair::from_secret_key(&secp, &secret_key);
        let (xonly_pk, _) = keypair.x_only_public_key();
        let operators_pubkey =
            XOnlyPk::new(Buf32::new(xonly_pk.serialize())).expect("Valid public key");
        (operators_pubkey, secret_key)
    }

    // Helper function to create a test transaction (for mutation tests)
    fn create_test_tx(deposit_info: &DepositInfo) -> Transaction {
        let (_, operators_privkey) = create_test_operator_keypair();
        create_test_deposit_tx(deposit_info, &[operators_privkey])
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
        let mut arb = ArbitraryGenerator::new();
        let info: DepositInfo = arb.generate();

        let tx = create_test_tx(&info);

        let tag_data_ref = ParseConfig::new(*TEST_MAGIC_BYTES)
            .try_parse_tx(&tx)
            .expect("Should parse transaction");
        let tx_input = TxInputRef::new(&tx, tag_data_ref);
        let deposit_info =
            extract_deposit_info(&tx_input).expect("Should successfully extract deposit info");

        // The extracted info should match the original except for the outpoint,
        // which will be calculated from the created transaction
        assert_eq!(info.deposit_idx, deposit_info.deposit_idx);
        assert_eq!(info.amt, deposit_info.amt);
        assert_eq!(info.address, deposit_info.address);
        assert_eq!(info.drt_tapnode_hash, deposit_info.drt_tapnode_hash);

        // The outpoint should be from the created transaction with vout = 1 (DEPOSIT_OUTPUT_INDEX)
        let expected_outpoint = OutputRef::from(OutPoint {
            txid: tx.compute_txid(),
            vout: DEPOSIT_OUTPUT_INDEX,
        });
        assert_eq!(expected_outpoint, deposit_info.outpoint);
    }

    #[test]
    fn test_extract_deposit_info_invalid_tx_type() {
        use crate::constants::BRIDGE_V1_SUBPROTOCOL_ID;

        let mut arb = ArbitraryGenerator::new();
        let info: DepositInfo = arb.generate();

        let mut tx = create_test_tx(&info);

        // Mutate the OP_RETURN output to have wrong transaction type
        let aux_data = vec![0u8; 40]; // Some dummy aux data
        let tagged_payload = create_tagged_payload(BRIDGE_V1_SUBPROTOCOL_ID, 99, aux_data);
        mutate_op_return_output(&mut tx, tagged_payload);

        let tx_input = parse_mutated_tx(&tx);
        let result = extract_deposit_info(&tx_input);
        assert!(result.is_err(), "Should fail with invalid transaction type");

        assert!(matches!(
            result,
            Err(DepositTxParseError::InvalidTxType { .. })
        ));
        if let Err(DepositTxParseError::InvalidTxType(tx_type)) = result {
            assert_eq!(tx_type, tx_input.tag().tx_type());
        }
    }

    #[test]
    fn test_extract_deposit_info_invalid_aux_data_too_short() {
        use crate::constants::{BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_TX_TYPE};

        let mut arb = ArbitraryGenerator::new();
        let info: DepositInfo = arb.generate();

        let mut tx = create_test_tx(&info);

        // Mutate the OP_RETURN output to have short aux data
        let short_aux_data = vec![0u8; MIN_DEPOSIT_TX_AUX_DATA_LEN - 1];
        let tagged_payload =
            create_tagged_payload(BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_TX_TYPE, short_aux_data);
        mutate_op_return_output(&mut tx, tagged_payload);

        let tx_input = parse_mutated_tx(&tx);
        let result = extract_deposit_info(&tx_input);
        assert!(
            result.is_err(),
            "Should fail with insufficient auxiliary data"
        );

        assert!(matches!(
            result,
            Err(DepositTxParseError::InvalidAuxiliaryData(_))
        ));
        if let Err(DepositTxParseError::InvalidAuxiliaryData(len)) = result {
            assert_eq!(len, MIN_DEPOSIT_TX_AUX_DATA_LEN - 1);
        }
    }

    #[test]
    fn test_extract_deposit_info_empty_destination() {
        use crate::constants::{BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_TX_TYPE};

        let mut arb = ArbitraryGenerator::new();
        let info: DepositInfo = arb.generate();

        let mut tx = create_test_tx(&info);

        // Mutate the OP_RETURN output to have aux data with no destination (exactly
        // MIN_AUX_DATA_LEN)
        let aux_data = vec![0u8; MIN_DEPOSIT_TX_AUX_DATA_LEN];
        let tagged_payload =
            create_tagged_payload(BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_TX_TYPE, aux_data);
        mutate_op_return_output(&mut tx, tagged_payload);

        let tx_input = parse_mutated_tx(&tx);
        let result = extract_deposit_info(&tx_input);
        assert!(result.is_ok(), "Should succeed with empty destination");

        let deposit_info = result.unwrap();
        assert!(deposit_info.address.is_empty(), "Address should be empty");
    }

    #[test]
    fn test_extract_deposit_info_missing_output() {
        let mut arb = ArbitraryGenerator::new();
        let info: DepositInfo = arb.generate();

        let mut tx = create_test_tx(&info);

        // Remove the deposit output (keep only OP_RETURN at index 0)
        tx.output.truncate(1);

        let tx_input = parse_mutated_tx(&tx);
        let result = extract_deposit_info(&tx_input);
        assert!(result.is_err(), "Should fail with missing deposit output");

        assert!(matches!(
            result,
            Err(DepositTxParseError::MissingDepositOutput)
        ));
    }
}
