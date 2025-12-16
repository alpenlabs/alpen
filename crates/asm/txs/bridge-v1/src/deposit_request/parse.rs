use bitcoin::Transaction;
use strata_codec::decode_buf_exact;
use strata_l1_txfmt::extract_tx_magic_and_tag;

use crate::{
    constants::BridgeTxType,
    deposit_request::{DRT_OUTPUT_INDEX, DepositRequestInfo, DrtHeaderAux},
    errors::TxStructureError,
};

/// Parses deposit request transaction to extract [`DepositRequestInfo`].
///
/// Parses a deposit request transaction following the SPS-50 specification and extracts the
/// decoded auxiliary data ([`DrtHeaderAux`]) along with the deposit amount. The
/// auxiliary data is encoded with [`strata_codec::Codec`] and includes the recovery public key
/// and destination address.
///
/// # Errors
///
/// Returns [`TxStructureError`] if the SPS-50 format cannot be parsed, the auxiliary
/// data cannot be decoded, or the expected deposit request output at index 1 is missing.
pub fn parse_drt(tx: &Transaction) -> Result<DepositRequestInfo, TxStructureError> {
    let (_magic, tag) = extract_tx_magic_and_tag(tx)
        .map_err(|e| TxStructureError::invalid_tx_format(BridgeTxType::DepositRequest, e))?;

    // Parse auxiliary data using DrtHeaderAux
    let aux_data: DrtHeaderAux = decode_buf_exact(tag.aux_data())
        .map_err(|e| TxStructureError::invalid_auxiliary_data(BridgeTxType::DepositRequest, e))?;

    // Extract the deposit request output (second output at index 1)
    let drt_output = tx
        .output
        .get(DRT_OUTPUT_INDEX)
        .ok_or_else(|| {
            TxStructureError::missing_output(
                BridgeTxType::DepositRequest,
                DRT_OUTPUT_INDEX,
                "deposit request output",
            )
        })?
        .clone()
        .into();

    // Construct the validated deposit request information
    Ok(DepositRequestInfo::new(aux_data, drt_output))
}
