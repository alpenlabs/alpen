use strata_asm_common::TxInputRef;
use strata_codec::decode_buf_exact;

use crate::{
    errors::UnstakeTxParseError,
    unstake::{aux::UnstakeTxHeaderAux, info::UnstakeInfo},
};

/// Index of the stake connector input.
pub const STAKE_INPUT_INDEX: usize = 1;

/// Parse an unstake transaction to extract [`UnstakeInfo`].
///
/// Parses an unstake transaction following the SPS-50 specification and extracts the auxiliary
/// metadata along with the stake connector outpoint (input index 1).
///
/// # Parameters
/// - `tx` - Reference to the transaction input containing the unstake transaction and tag data
///
/// # Returns
/// - `Ok(UnstakeInfo)` on success
/// - `Err(SlashTxParseError)` if [`UnstakeTxHeaderAux`] data cannot be decoded, or the stake
///   connector input (at index [`STAKE_INPUT_INDEX`]) is missing.
pub fn parse_unstake_tx<'t>(tx: &TxInputRef<'t>) -> Result<UnstakeInfo, UnstakeTxParseError> {
    // Parse auxiliary data using UnstakeTxHeaderAux
    let header_aux: UnstakeTxHeaderAux = decode_buf_exact(tx.tag().aux_data())?;

    // Extract the previous outpoint from the second input
    let second_input_outpoint = tx
        .tx()
        .input
        .get(STAKE_INPUT_INDEX)
        .ok_or(UnstakeTxParseError::MissingInput(STAKE_INPUT_INDEX))?
        .previous_output
        .into();

    let info = UnstakeInfo::new(header_aux, second_input_outpoint);

    Ok(info)
}

#[cfg(test)]
mod tests {
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::test_utils::{create_test_unstake_tx, mutate_aux_data, parse_tx};

    const AUX_LEN: usize = std::mem::size_of::<UnstakeTxHeaderAux>();

    #[test]
    fn test_parse_unstake_tx_success() {
        let info: UnstakeInfo = ArbitraryGenerator::new().generate();

        let tx = create_test_unstake_tx(&info);
        let tx_input = parse_tx(&tx);

        let parsed = parse_unstake_tx(&tx_input).expect("Should parse unstake tx");

        assert_eq!(info, parsed);
    }

    #[test]
    fn test_parse_unstake_missing_stake_input() {
        let info: UnstakeInfo = ArbitraryGenerator::new().generate();
        let mut tx = create_test_unstake_tx(&info);

        // Remove the stake connector to force an input count mismatch
        tx.input.pop();

        let tx_input = parse_tx(&tx);
        let err = parse_unstake_tx(&tx_input).unwrap_err();
        assert!(matches!(
            err,
            UnstakeTxParseError::MissingInput(STAKE_INPUT_INDEX)
        ))
    }

    #[test]
    fn test_parse_invalid_aux() {
        let info: UnstakeInfo = ArbitraryGenerator::new().generate();
        let mut tx = create_test_unstake_tx(&info);

        let larger_aux = [0u8; AUX_LEN + 1].to_vec();
        mutate_aux_data(&mut tx, larger_aux);

        let tx_input = parse_tx(&tx);
        let err = parse_unstake_tx(&tx_input).unwrap_err();
        assert!(matches!(err, UnstakeTxParseError::InvalidAuxiliaryData(_)));

        let smaller_aux = [0u8; AUX_LEN - 1].to_vec();
        mutate_aux_data(&mut tx, smaller_aux);

        let tx_input = parse_tx(&tx);
        let err = parse_unstake_tx(&tx_input).unwrap_err();
        assert!(matches!(err, UnstakeTxParseError::InvalidAuxiliaryData(_)));
    }
}
