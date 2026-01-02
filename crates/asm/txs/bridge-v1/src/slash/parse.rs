use strata_asm_common::TxInputRef;
use strata_codec::decode_buf_exact;

use crate::{
    constants::BridgeTxType,
    errors::TxStructureError,
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
/// - `Err(TxStructureError)` if [`SlashTxHeaderAux`] data cannot be decoded, or the stake connector
///   input (at index [`STAKE_INPUT_INDEX`]) is missing.
pub fn parse_slash_tx<'t>(tx: &TxInputRef<'t>) -> Result<SlashInfo, TxStructureError> {
    // Parse auxiliary data using CommitTxHeaderAux
    let header_aux: SlashTxHeaderAux = decode_buf_exact(tx.tag().aux_data())
        .map_err(|e| TxStructureError::invalid_auxiliary_data(BridgeTxType::Slash, e))?;

    // Extract the stake inpoint (previous outpoint from the second input)
    let stake_inpoint = tx
        .tx()
        .input
        .get(STAKE_INPUT_INDEX)
        .ok_or_else(|| {
            TxStructureError::missing_input(
                BridgeTxType::Slash,
                STAKE_INPUT_INDEX,
                "stake connector input",
            )
        })?
        .previous_output
        .into();

    let info = SlashInfo::new(header_aux, stake_inpoint);

    Ok(info)
}

#[cfg(test)]
mod tests {
    use std::mem;

    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::{
        errors::TxStructureErrorKind,
        test_utils::{create_test_slash_tx, mutate_aux_data, parse_sps50_tx},
    };

    const AUX_LEN: usize = mem::size_of::<SlashTxHeaderAux>();

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
        assert_eq!(err.tx_type(), BridgeTxType::Slash);
        assert!(matches!(
            err.kind(),
            TxStructureErrorKind::MissingInput {
                index: STAKE_INPUT_INDEX
            }
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
        assert_eq!(err.tx_type(), BridgeTxType::Slash);
        assert!(matches!(
            err.kind(),
            TxStructureErrorKind::InvalidAuxiliaryData(_)
        ));

        let smaller_aux = [0u8; AUX_LEN - 1].to_vec();
        mutate_aux_data(&mut tx, smaller_aux);

        let tx_input = parse_sps50_tx(&tx);
        let err = parse_slash_tx(&tx_input).unwrap_err();
        assert_eq!(err.tx_type(), BridgeTxType::Slash);
        assert!(matches!(
            err.kind(),
            TxStructureErrorKind::InvalidAuxiliaryData(_)
        ));
    }
}
