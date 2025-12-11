use strata_asm_common::TxInputRef;
use strata_codec::decode_buf_exact;

use crate::{
    commit::{CommitInfo, aux::CommitTxHeaderAux},
    errors::{CommitParseError, Mismatch},
};

/// Expected number of inputs in a commit transaction.
const EXPECTED_COMMIT_TX_INPUT_COUNT: usize = 1;

/// Parses a commit transaction into [`CommitInfo`], decoding the SPS-50 auxiliary data as
/// [`CommitTxHeaderAux`] (via [`strata_codec::Codec`]) and validating basic structure.
///
/// # Errors
///
/// Returns [`CommitParseError`] if the transaction does not have exactly one input, the auxiliary
/// data fails to decode into [`CommitTxHeaderAux`], or the required N/N output at index 1 is
/// missing.
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

    Ok(CommitInfo::new(
        header_aux,
        first_input_outpoint,
        second_output_script,
    ))
}

#[cfg(test)]
mod tests {
    use strata_asm_common::TxInputRef;
    use strata_l1_txfmt::ParseConfig;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::test_utils::{
        TEST_MAGIC_BYTES, create_test_commit_tx, mutate_aux_data, parse_sps50_tx,
    };

    const COMMIT_TX_AUX_DATA_LEN: usize = std::mem::size_of::<CommitTxHeaderAux>();

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
        let shorter_aux = vec![0u8; COMMIT_TX_AUX_DATA_LEN - 1];
        mutate_aux_data(&mut tx, shorter_aux);

        let tx_input = parse_sps50_tx(&tx);
        let err = parse_commit_tx(&tx_input).unwrap_err();

        assert!(matches!(err, CommitParseError::InvalidAuxiliaryData(_)));

        // Mutate the OP_RETURN output to have longer aux len
        let longer_aux = vec![0u8; COMMIT_TX_AUX_DATA_LEN + 1];
        mutate_aux_data(&mut tx, longer_aux);

        let tx_input = parse_sps50_tx(&tx);
        let err = parse_commit_tx(&tx_input).unwrap_err();
        assert!(matches!(err, CommitParseError::InvalidAuxiliaryData(_)));
    }
}
