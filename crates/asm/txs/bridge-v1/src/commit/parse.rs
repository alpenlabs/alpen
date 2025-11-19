use arbitrary::{Arbitrary, Unstructured};
use strata_asm_common::TxInputRef;
use strata_codec::decode_buf_exact;

use crate::{commit::aux::CommitTxHeaderAux, constants::COMMIT_TX_TYPE, errors::CommitParseError};

/// Length of auxiliary data for commit transactions.
/// - 4 bytes for deposit_idx (u32)
/// - 4 bytes for game_idx (u32)
pub const COMMIT_TX_AUX_DATA_LEN: usize = 4 + 4;

/// Information extracted from a Bitcoin commit transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    /// The index of the deposit that the operator is committing to.
    /// This must be validated against the operator's assigned deposits in the state's assignments
    /// table to ensure the operator is authorized to commit to this specific deposit.
    pub deposit_idx: u32,

    /// The index of the game being committed to.
    pub game_idx: u32,
}

impl<'a> Arbitrary<'a> for CommitInfo {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(CommitInfo {
            deposit_idx: u32::arbitrary(u)?,
            game_idx: u32::arbitrary(u)?,
        })
    }
}

/// Parses commit transaction to extract [`CommitInfo`].
///
/// Parses a commit transaction following the SPS-50 specification and extracts
/// the commit information including the deposit index that the operator is committing to.
///
/// The function validates the transaction structure and parses the auxiliary data containing:
/// - Deposit index (4 bytes, big-endian u32)
/// - Game index (4 bytes, big-endian u32)
///
/// # Parameters
///
/// - `tx` - Reference to the transaction input containing the commit transaction and its associated
///   tag data
///
/// # Returns
///
/// - `Ok(CommitInfo)` - Successfully parsed commit information
/// - `Err(CommitParseError)` - If the transaction structure is invalid, has invalid metadata size,
///   or any parsing step encounters malformed data
///
/// # Errors
///
/// This function will return an error if:
/// - The transaction type doesn't match the expected commit transaction type
/// - The auxiliary data size doesn't match the expected metadata size (8 bytes)
/// - Any of the metadata fields cannot be parsed correctly
pub fn parse_commit_tx<'t>(tx: &TxInputRef<'t>) -> Result<CommitInfo, CommitParseError> {
    if tx.tag().tx_type() != COMMIT_TX_TYPE {
        return Err(CommitParseError::InvalidTxType(tx.tag().tx_type()));
    }

    // Parse auxiliary data using CommitTxHeaderAux
    let header_aux: CommitTxHeaderAux = decode_buf_exact(tx.tag().aux_data())?;

    Ok(CommitInfo {
        deposit_idx: header_aux.deposit_idx,
        game_idx: header_aux.game_idx,
    })
}

#[cfg(test)]
mod tests {
    use strata_asm_common::TxInputRef;
    use strata_l1_txfmt::ParseConfig;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use bitcoin::secp256k1::{Keypair, Secp256k1};
    use rand::rngs::OsRng;
    use strata_crypto::even_kp;

    use crate::{
        BRIDGE_V1_SUBPROTOCOL_ID,
        test_utils::{
            TEST_MAGIC_BYTES, create_tagged_payload, create_test_commit_tx, mutate_op_return_output,
            parse_tx,
        },
    };

    /// Tests that our hardcoded size constant matches the expected format.
    /// This validates that the auxiliary data length (8 bytes) matches the sum of:
    /// - deposit_idx (4 bytes, u32)
    /// - game_idx (4 bytes, u32)
    #[test]
    fn test_valid_size() {
        let expected_len = std::mem::size_of::<u32>() * 2; // deposit_idx + game_idx
        assert_eq!(expected_len, COMMIT_TX_AUX_DATA_LEN);
    }

    #[test]
    fn test_parse_commit_tx_success() {
        let mut arb = ArbitraryGenerator::new();
        let info: CommitInfo = arb.generate();

        // Generate operator private keys for N/N multisig
        let secp = Secp256k1::new();
        let operators_privkeys: Vec<_> = (0..3)
            .map(|_| {
                let kp = Keypair::new(&secp, &mut OsRng);
                even_kp((kp.secret_key(), kp.public_key())).0
            })
            .collect();

        // Create the funding and commit transactions
        let (_funding_tx, tx) = create_test_commit_tx(&info, &operators_privkeys);

        // Parse the transaction using the SPS-50 parser
        let parser = ParseConfig::new(*TEST_MAGIC_BYTES);
        let tag_data = parser.try_parse_tx(&tx).expect("Should parse transaction");
        let tx_input_ref = TxInputRef::new(&tx, tag_data);

        // Extract commit info using the actual parser
        let extracted_info =
            parse_commit_tx(&tx_input_ref).expect("Should successfully extract commit info");

        assert_eq!(extracted_info, info);
    }

    #[test]
    fn test_parse_commit_tx_invalid_type() {
        let mut arb = ArbitraryGenerator::new();
        let info: CommitInfo = arb.generate();

        // Generate operator private keys for N/N multisig
        let secp = Secp256k1::new();
        let operators_privkeys: Vec<_> = (0..3)
            .map(|_| {
                let kp = Keypair::new(&secp, &mut OsRng);
                even_kp((kp.secret_key(), kp.public_key())).0
            })
            .collect();

        let (_funding_tx, mut tx) = create_test_commit_tx(&info, &operators_privkeys);

        // Mutate the OP_RETURN output to have wrong transaction type
        let aux_data = vec![0u8; 4]; // Some dummy aux data
        let tagged_payload = create_tagged_payload(BRIDGE_V1_SUBPROTOCOL_ID, 99, aux_data);
        mutate_op_return_output(&mut tx, tagged_payload);

        let tx_input = parse_tx(&tx);
        let err = parse_commit_tx(&tx_input).unwrap_err();
        assert!(matches!(err, CommitParseError::InvalidTxType { .. }));
        if let CommitParseError::InvalidTxType(tx_type) = err {
            assert_eq!(tx_type, tx_input.tag().tx_type());
        }
    }

    #[test]
    fn test_parse_commit_tx_invalid_aux_data() {
        let mut arb = ArbitraryGenerator::new();
        let info: CommitInfo = arb.generate();

        // Generate operator private keys for N/N multisig
        let secp = Secp256k1::new();
        let operators_privkeys: Vec<_> = (0..3)
            .map(|_| {
                let kp = Keypair::new(&secp, &mut OsRng);
                even_kp((kp.secret_key(), kp.public_key())).0
            })
            .collect();

        let (_funding_tx, mut tx) = create_test_commit_tx(&info, &operators_privkeys);

        // Mutate the OP_RETURN output to have shorter aux len
        let aux_data = vec![0u8; COMMIT_TX_AUX_DATA_LEN - 1];
        let tagged_payload =
            create_tagged_payload(BRIDGE_V1_SUBPROTOCOL_ID, COMMIT_TX_TYPE, aux_data);
        mutate_op_return_output(&mut tx, tagged_payload);

        let tx_input = parse_tx(&tx);
        let err = parse_commit_tx(&tx_input).unwrap_err();

        assert!(matches!(err, CommitParseError::InvalidAuxiliaryData(_)));

        // Mutate the OP_RETURN output to have longer aux len
        let aux_data = vec![0u8; COMMIT_TX_AUX_DATA_LEN + 1];
        let tagged_payload =
            create_tagged_payload(BRIDGE_V1_SUBPROTOCOL_ID, COMMIT_TX_TYPE, aux_data);
        mutate_op_return_output(&mut tx, tagged_payload);

        let tx_input = parse_tx(&tx);
        let err = parse_commit_tx(&tx_input).unwrap_err();
        assert!(matches!(err, CommitParseError::InvalidAuxiliaryData(_)));
    }
}
