use strata_asm_common::TxInputRef;
use strata_codec::decode_buf_exact;

use crate::{
    errors::SlashTxParseError,
    slash::{aux::SlashTxHeaderAux, info::SlashInfo},
};

/// Index of the stake connector input.
pub const STAKE_INPUT_INDEX: usize = 1;

/// Parse a slash transaction to extract [`SlashInfo`].
///
/// Parses a slash transaction following the SPS-50 specification and extracts the auxiliary
/// metadata along with the stake connector outpoint (input index 1).
///
/// # Parameters
/// - `tx` - Reference to the transaction input containing the slash transaction and tag data
///
/// # Returns
/// - `Ok(SlashInfo)` on success
/// - `Err(SlashTxParseError)` if [`SlashTxHeaderAux`] data cannot be decoded, or the stake
///   connector input (at index [`STAKE_INPUT_INDEX`]) is missing.
pub fn parse_slash_tx<'t>(tx: &TxInputRef<'t>) -> Result<SlashInfo, SlashTxParseError> {
    // Parse auxiliary data using CommitTxHeaderAux
    let header_aux: SlashTxHeaderAux = decode_buf_exact(tx.tag().aux_data())?;

    // Extract the previous outpoint from the second input
    let second_input_outpoint = tx
        .tx()
        .input
        .get(STAKE_INPUT_INDEX)
        .ok_or(SlashTxParseError::MissingInput(STAKE_INPUT_INDEX))?
        .previous_output
        .into();

    let info = SlashInfo::new(header_aux, second_input_outpoint);

    Ok(info)
}

#[cfg(test)]
mod tests {
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::test_utils::{create_test_slash_tx, mutate_aux_data, parse_sps50_tx};

    const AUX_LEN: usize = std::mem::size_of::<SlashTxHeaderAux>();

    #[test]
    fn test_parse_slash_tx_success() {
        let info: SlashInfo = ArbitraryGenerator::new().generate();

        let tx = create_test_slash_tx(&info);
        let tx_input = parse_sps50_tx(&tx);

        let parsed = parse_slash_tx(&tx_input).expect("Should parse slash tx");

        assert_eq!(info, parsed);
    }

    #[test]
    fn test_parse_slash_missing_stake_input() {
        let info: SlashInfo = ArbitraryGenerator::new().generate();
        let mut tx = create_test_slash_tx(&info);

        // Remove the stake connector to force an input count mismatch
        tx.input.pop();

        let tx_input = parse_sps50_tx(&tx);
        let err = parse_slash_tx(&tx_input).unwrap_err();
        assert!(matches!(
            err,
            SlashTxParseError::MissingInput(STAKE_INPUT_INDEX)
        ))
    }

    #[test]
    fn test_parse_invalid_aux() {
        let info: SlashInfo = ArbitraryGenerator::new().generate();
        let mut tx = create_test_slash_tx(&info);

        let larger_aux = [0u8; AUX_LEN + 1].to_vec();
        mutate_aux_data(&mut tx, larger_aux);

        let tx_input = parse_sps50_tx(&tx);
        let err = parse_slash_tx(&tx_input).unwrap_err();
        assert!(matches!(err, SlashTxParseError::InvalidAuxiliaryData(_)));

        let smaller_aux = [0u8; AUX_LEN - 1].to_vec();
        mutate_aux_data(&mut tx, smaller_aux);

        let tx_input = parse_sps50_tx(&tx);
        let err = parse_slash_tx(&tx_input).unwrap_err();
        assert!(matches!(err, SlashTxParseError::InvalidAuxiliaryData(_)));
    }
}
