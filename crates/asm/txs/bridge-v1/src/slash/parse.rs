use arbitrary::Arbitrary;
use strata_asm_common::TxInputRef;
use strata_codec::decode_buf_exact;
use strata_primitives::l1::BitcoinOutPoint;

use crate::{errors::SlashTxParseError, slash::aux::SlashTxHeaderAux};

/// Index of the stake connector input.
const STAKE_INPUT_INDEX: usize = 1;

/// Information extracted from a Bitcoin slash transaction.
#[derive(Debug, Clone, PartialEq, Eq, Arbitrary)]
pub struct SlashInfo {
    /// SPS-50 auxiliary data from the transaction tag.
    header_aux: SlashTxHeaderAux,
    /// Previous outpoint referenced second input (stake connector).
    second_input_outpoint: BitcoinOutPoint,
}

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
/// - `Err(SlashTxParseError)` if auxiliary data cannot be decoded, the input count is wrong, or the
///   stake connector input is missing
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

    let info = SlashInfo {
        header_aux,
        second_input_outpoint,
    };

    Ok(info)
}
