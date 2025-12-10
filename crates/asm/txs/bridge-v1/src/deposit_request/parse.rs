//! DRT parsing using SPS-50 format.
//!
//! SPS-50 structure:
//! - OP_RETURN: [MAGIC (4)][SUBPROTOCOL_ID (1)][TX_TYPE (1)][RECOVERY_PK (32)][EE_ADDRESS
//!   (variable)]

use bitcoin::Transaction;
use strata_asm_common::TxInputRef;
use strata_codec::decode_buf_exact;
use strata_l1_txfmt::{ParseConfig, extract_tx_magic_and_tag};
use strata_primitives::l1::DepositRequestInfo;

use crate::{
    constants::DEPOSIT_REQUEST_TX_TYPE,
    deposit_request::{aux::DrtHeaderAux, info::DrtInfo},
    errors::DepositRequestParseError,
};

const RECOVERY_PK_LEN: usize = 32;

/// Minimum length of auxiliary data for deposit request transactions
pub const MIN_DRT_AUX_DATA_LEN: usize = RECOVERY_PK_LEN;

pub fn parse_drt(
    tx_input: &TxInputRef<'_>,
) -> Result<DepositRequestInfo, DepositRequestParseError> {
    if tx_input.tag().tx_type() != DEPOSIT_REQUEST_TX_TYPE {
        return Err(DepositRequestParseError::InvalidTxType {
            actual: tx_input.tag().tx_type(),
            expected: DEPOSIT_REQUEST_TX_TYPE,
        });
    }

    let aux_data = tx_input.tag().aux_data();

    if aux_data.len() < MIN_DRT_AUX_DATA_LEN {
        return Err(DepositRequestParseError::InvalidAuxiliaryData(
            aux_data.len(),
        ));
    }

    let (recovery_pk_bytes, ee_address) = aux_data.split_at(RECOVERY_PK_LEN);
    let recovery_pk: [u8; 32] = recovery_pk_bytes
        .try_into()
        .expect("validated aux_data length");

    // Per spec: Output 1 must be the P2TR deposit request output
    let drt_output = tx_input
        .tx()
        .output
        .get(1)
        .ok_or(DepositRequestParseError::MissingDRTOutput)?;

    let amt = drt_output.value.to_sat();

    Ok(DepositRequestInfo {
        amt,
        take_back_leaf_hash: recovery_pk,
        address: ee_address.to_vec(),
    })
}

/// Parses a DRT from a raw transaction with magic bytes
///
/// Validates that the transaction follows the SPS-50 DRT specification:
/// - Output 0 must be OP_RETURN with tagged data
/// - Output 1 must be the P2TR deposit output
///
/// # Arguments
/// * `tx` - The DRT transaction to parse
/// * `magic_bytes` - The SPS-50 magic bytes for this network
///
/// # Returns
/// The parsed deposit request information
pub fn parse_drt_from_tx(
    tx: &Transaction,
    magic_bytes: &[u8; 4],
) -> Result<DepositRequestInfo, DepositRequestParseError> {
    // Validate OP_RETURN is at index 0 per spec
    if tx.output.is_empty() || !tx.output[0].script_pubkey.is_op_return() {
        return Err(DepositRequestParseError::NoOpReturnOutput);
    }

    let parse_config = ParseConfig::new(*magic_bytes);
    let tag_data = parse_config
        .try_parse_tx(tx)
        .map_err(|e| DepositRequestParseError::Sps50ParseError(e.to_string()))?;

    let tx_input = TxInputRef::new(tx, tag_data);
    parse_drt(&tx_input)
}

pub fn parse_drt_new(tx: &Transaction) -> Result<DrtInfo, DepositRequestParseError> {
    let (_, tag) = extract_tx_magic_and_tag(tx).unwrap();
    let header_aux: DrtHeaderAux = decode_buf_exact(tag.aux_data()).unwrap();

    let drt_output = tx
        .output
        .get(1)
        .ok_or(DepositRequestParseError::MissingDRTOutput)?
        .clone();
    Ok(DrtInfo::new(header_aux, drt_output.into()))
}
