use arbitrary::Arbitrary;
use bitcoin::OutPoint;
use strata_asm_common::TxInputRef;
use strata_codec::decode_buf_exact;
use strata_primitives::l1::{BitcoinAmount, BitcoinOutPoint};

use crate::{
    deposit::{DEPOSIT_OUTPUT_INDEX, aux::DepositTxHeaderAux},
    errors::DepositTxParseError,
};

/// Minimum length of auxiliary data (fixed fields only, excluding variable destination address)
/// - 4 bytes for deposit_idx (u32)
/// - 32 bytes for drt_tapscript_merkle_root
pub const MIN_DEPOSIT_TX_AUX_DATA_LEN: usize = 4 + 32;

/// Information extracted from a Bitcoin deposit transaction.
#[derive(Debug, Clone, PartialEq, Eq, Arbitrary)]
pub struct DepositInfo {
    /// Parsed SPS-50 auxiliary data.
    pub header_aux: DepositTxHeaderAux,

    /// The amount of Bitcoin deposited.
    pub amt: BitcoinAmount,

    /// The outpoint of the deposit transaction.
    pub outpoint: BitcoinOutPoint,
}

/// Parses deposit transaction to extract [`DepositInfo`].
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
pub fn parse_deposit_tx<'a>(tx_input: &TxInputRef<'a>) -> Result<DepositInfo, DepositTxParseError> {
    // Parse auxiliary data using DepositTxHeaderAux
    let header_aux: DepositTxHeaderAux = decode_buf_exact(tx_input.tag().aux_data())?;

    // Extract the deposit output (second output at index 1)
    let deposit_output = tx_input
        .tx()
        .output
        .get(DEPOSIT_OUTPUT_INDEX)
        .ok_or(DepositTxParseError::MissingDepositOutput)?;

    // Create outpoint reference for the deposit output
    let deposit_outpoint = BitcoinOutPoint::from(OutPoint {
        txid: tx_input.tx().compute_txid(),
        vout: DEPOSIT_OUTPUT_INDEX as u32,
    });

    // Construct the validated deposit information
    Ok(DepositInfo {
        header_aux,
        amt: deposit_output.value.into(),
        outpoint: deposit_outpoint,
    })
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        OutPoint, Transaction,
        secp256k1::{Secp256k1, SecretKey},
    };
    use strata_asm_common::TxInputRef;
    use strata_crypto::EvenSecretKey;
    use strata_l1_txfmt::ParseConfig;
    use strata_primitives::{
        buf::Buf32,
        l1::{BitcoinOutPoint, BitcoinXOnlyPublicKey},
    };
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::test_utils::{
        TEST_MAGIC_BYTES, create_tagged_payload, create_test_deposit_tx, mutate_op_return_output,
        parse_tx,
    };

    // Helper function to create a test operator keypair
    fn create_test_operator_keypair() -> (BitcoinXOnlyPublicKey, EvenSecretKey) {
        let secp = Secp256k1::new();
        let secret_key = EvenSecretKey::from(SecretKey::from_slice(&[1u8; 32]).unwrap());
        let keypair = bitcoin::secp256k1::Keypair::from_secret_key(&secp, &secret_key);
        let (xonly_pk, _) = keypair.x_only_public_key();
        let operators_pubkey =
            BitcoinXOnlyPublicKey::new(Buf32::new(xonly_pk.serialize())).expect("Valid public key");
        (operators_pubkey, secret_key)
    }

    // Helper function to create a test transaction (for mutation tests)
    fn create_test_tx(deposit_info: &DepositInfo) -> Transaction {
        let (_, operators_privkey) = create_test_operator_keypair();
        create_test_deposit_tx(deposit_info, &[operators_privkey])
    }

    #[test]
    fn test_parse_deposit_tx_success() {
        let mut arb = ArbitraryGenerator::new();
        let info: DepositInfo = arb.generate();

        let tx = create_test_tx(&info);

        let tag_data_ref = ParseConfig::new(*TEST_MAGIC_BYTES)
            .try_parse_tx(&tx)
            .expect("Should parse transaction");
        let tx_input = TxInputRef::new(&tx, tag_data_ref);
        let parsed_info =
            parse_deposit_tx(&tx_input).expect("Should successfully extract deposit info");
        assert_eq!(info, parsed_info);

        // The outpoint should be from the created transaction with vout = 1 (DEPOSIT_OUTPUT_INDEX)
        let expected_outpoint = BitcoinOutPoint::from(OutPoint {
            txid: tx.compute_txid(),
            vout: DEPOSIT_OUTPUT_INDEX as u32,
        });
        assert_eq!(expected_outpoint, parsed_info.outpoint);
    }

    #[test]
    fn test_parse_deposit_aux_data_too_short() {
        use crate::constants::{BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_TX_TYPE};

        let mut arb = ArbitraryGenerator::new();
        let info: DepositInfo = arb.generate();

        let mut tx = create_test_tx(&info);

        // Mutate the OP_RETURN output to have short aux data
        let short_aux_data = vec![0u8; MIN_DEPOSIT_TX_AUX_DATA_LEN - 1];
        let tagged_payload =
            create_tagged_payload(BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_TX_TYPE, short_aux_data);
        mutate_op_return_output(&mut tx, tagged_payload);

        let tx_input = parse_tx(&tx);
        let err = parse_deposit_tx(&tx_input).unwrap_err();
        assert!(matches!(err, DepositTxParseError::InvalidAuxiliaryData(_)));
    }

    #[test]
    fn test_parse_deposit_empty_destination() {
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

        let tx_input = parse_tx(&tx);
        let result = parse_deposit_tx(&tx_input);
        assert!(result.is_ok(), "Should succeed with empty destination");

        let deposit_info = result.unwrap();
        assert!(
            deposit_info.header_aux.address.is_empty(),
            "Address should be empty"
        );
    }

    #[test]
    fn test_parse_deposit_missing_output() {
        let mut arb = ArbitraryGenerator::new();
        let info: DepositInfo = arb.generate();

        let mut tx = create_test_tx(&info);

        // Remove the deposit output (keep only OP_RETURN at index 0)
        tx.output.truncate(1);

        let tx_input = parse_tx(&tx);
        let err = parse_deposit_tx(&tx_input).unwrap_err();
        assert!(matches!(err, DepositTxParseError::MissingDepositOutput));
    }
}
