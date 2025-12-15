use bitcoin::Transaction;
use strata_codec::decode_buf_exact;
use strata_l1_txfmt::extract_tx_magic_and_tag;

use crate::{
    deposit_request::{DRT_OUTPUT_INDEX, DepositRequestInfo, DrtHeaderAux},
    errors::DepositRequestParseError,
};

/// Parses deposit request transaction to extract [`DepositRequestInfo`].
///
/// Parses a deposit request transaction following the SPS-50 specification and extracts the
/// decoded auxiliary data ([`DepositRequestAuxData`]) along with the deposit amount. The
/// auxiliary data is encoded with [`strata_codec::Codec`] and includes the recovery public key
/// and destination address.
///
/// # Errors
///
/// Returns [`DepositRequestParseError`] if the auxiliary data cannot be decoded or if the expected
/// deposit request output at index 1 is missing.
pub fn parse_drt(tx: &Transaction) -> Result<DepositRequestInfo, DepositRequestParseError> {
    let (_magic, tag) = extract_tx_magic_and_tag(tx)
        .map_err(|e| DepositRequestParseError::Sps50ParseError(e.to_string()))?;

    // Parse auxiliary data using DepositRequestAuxData
    let aux_data: DrtHeaderAux = decode_buf_exact(tag.aux_data())?;

    // Extract the deposit request output (second output at index 1)
    let drt_output = tx
        .output
        .get(DRT_OUTPUT_INDEX)
        .ok_or(DepositRequestParseError::MissingDRTOutput)?
        .clone()
        .into();

    // Construct the validated deposit request information
    Ok(DepositRequestInfo::new(aux_data, drt_output))
}
