use arbitrary::{Arbitrary, Unstructured};
use bitcoin::ScriptBuf;
use strata_asm_common::TxInputRef;
use strata_codec::decode_buf_exact;
use strata_primitives::l1::BitcoinOutPoint;

use crate::{
    commit::aux::CommitTxHeaderAux,
    errors::{CommitParseError, Mismatch},
};

/// Length of auxiliary data for commit transactions.
/// - 4 bytes for deposit_idx (u32)
/// - 4 bytes for game_idx (u32)
pub const COMMIT_TX_AUX_DATA_LEN: usize = 4 + 4;

/// Expected number of inputs in a commit transaction.
const EXPECTED_COMMIT_TX_INPUT_COUNT: usize = 1;

/// Information extracted from a Bitcoin commit transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    /// Parsed SPS-50 auxiliary data.
    pub header_aux: CommitTxHeaderAux,

    /// The outpoint spent by the first input.
    /// Must be validated that it spends from an N/N-locked output during transaction validation.
    pub first_input_outpoint: BitcoinOutPoint,

    /// The script from the second output (index 1).
    /// Must be validated as N/N-locked during transaction validation.
    pub second_output_script: ScriptBuf,
}

impl<'a> Arbitrary<'a> for CommitInfo {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let header_aux = CommitTxHeaderAux::arbitrary(u)?;
        let first_input_outpoint = BitcoinOutPoint::arbitrary(u)?;
        let second_output_script = ScriptBuf::new();

        Ok(CommitInfo {
            header_aux,
            first_input_outpoint,
            second_output_script,
        })
    }
}

/// Parses commit transaction to extract [`CommitInfo`].
///
/// The function validates the transaction structure and parses the auxiliary data (encoded using
/// [`strata_codec::Codec`] with big-endian for integers) containing:
/// - Deposit index (4 bytes, u32)
/// - Game index (4 bytes, u32)
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
/// - The transaction doesn't have exactly one input
/// - The auxiliary data size doesn't match the expected metadata size (8 bytes)
/// - Any of the metadata fields cannot be parsed correctly
/// - The second output (N/N output at index 1) is missing
pub fn parse_commit_tx<'t>(tx: &TxInputRef<'t>) -> Result<CommitInfo, CommitParseError> {
    // Parse auxiliary data using CommitTxHeaderAux
    let header_aux: CommitTxHeaderAux = decode_buf_exact(tx.tag().aux_data())?;

    // Validate that the transaction has exactly one input
    if tx.tx().input.len() != EXPECTED_COMMIT_TX_INPUT_COUNT {
        return Err(CommitParseError::InvalidInputCount(Mismatch {
            expected: EXPECTED_COMMIT_TX_INPUT_COUNT,
            got: tx.tx().input.len(),
        }));
    }

    // Extract the N/N output script from the second output (index 1)
    let second_output_script = tx
        .tx()
        .output
        .get(1)
        .ok_or(CommitParseError::MissingNnOutput)?
        .script_pubkey
        .clone();

    // Extract the previous outpoint from the first (and only) input
    let first_input_outpoint = tx.tx().input[0].previous_output.into();

    Ok(CommitInfo {
        header_aux,
        first_input_outpoint,
        second_output_script,
    })
}

#[cfg(test)]
mod tests {
    use strata_asm_common::TxInputRef;
    use strata_l1_txfmt::ParseConfig;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::{
        BRIDGE_V1_SUBPROTOCOL_ID,
        constants::COMMIT_TX_TYPE,
        test_utils::{
            TEST_MAGIC_BYTES, create_tagged_payload, create_test_commit_tx,
            mutate_op_return_output, parse_tx,
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

        // Create the commit transaction with proper SPS-50 format
        let tx = create_test_commit_tx(&info);

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
    fn test_parse_commit_tx_invalid_aux_data() {
        let mut arb = ArbitraryGenerator::new();
        let info: CommitInfo = arb.generate();

        let mut tx = create_test_commit_tx(&info);

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
