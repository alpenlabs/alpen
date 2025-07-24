use bitcoin::{ScriptBuf, Txid, consensus::encode};
use strata_asm_common::TxInputRef;
use strata_primitives::{bridge::OperatorIdx, l1::BitcoinAmount};

use crate::errors::WithdrawalParseError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WithdrawalInfo {
    pub(crate) operator_idx: OperatorIdx,
    pub(crate) deposit_idx: u32,
    pub(crate) deposit_txid: Txid,
    pub(crate) withdrawal_address: ScriptBuf,
    pub(crate) withdrawal_amount: BitcoinAmount,
}

// TODO: make this standard
pub(crate) fn extract_withdrawal_info<'t>(
    tx: &TxInputRef<'t>,
) -> Result<WithdrawalInfo, WithdrawalParseError> {
    if tx.tx().output.len() < 2 {
        return Err(WithdrawalParseError::InsufficientOutputs(
            tx.tx().output.len(),
        ));
    }

    let withdrawal_fulfillment_output = &tx.tx().output[0];
    let withdrawal_metadata = tx.tag().aux_data();

    const OPERATOR_IDX_SIZE: usize = std::mem::size_of::<OperatorIdx>();
    const DEPOSIT_IDX_SIZE: usize = std::mem::size_of::<u32>();
    const DEPOSIT_TXID_SIZE: usize = std::mem::size_of::<Txid>();

    let expected_metadata_size: usize = OPERATOR_IDX_SIZE + DEPOSIT_IDX_SIZE + DEPOSIT_TXID_SIZE;

    if withdrawal_metadata.len() != expected_metadata_size {
        return Err(WithdrawalParseError::InvalidMetadataSize {
            expected: expected_metadata_size,
            actual: withdrawal_metadata.len(),
        });
    }

    let mut offset = 0;
    let operator_idx_bytes = &withdrawal_metadata[offset..offset + OPERATOR_IDX_SIZE];

    offset += OPERATOR_IDX_SIZE;
    let deposit_idx_bytes = &withdrawal_metadata[offset..offset + DEPOSIT_IDX_SIZE];

    offset += DEPOSIT_IDX_SIZE;
    let deposit_txid_bytes = &withdrawal_metadata[offset..offset + DEPOSIT_TXID_SIZE];

    let operator_idx =
        u32::from_be_bytes(operator_idx_bytes.try_into().map_err(|_| {
            WithdrawalParseError::InvalidOperatorIdxBytes(operator_idx_bytes.len())
        })?);

    let deposit_idx = u32::from_be_bytes(
        deposit_idx_bytes
            .try_into()
            .map_err(|_| WithdrawalParseError::InvalidDepositIdxBytes(deposit_idx_bytes.len()))?,
    );

    let deposit_txid: Txid = encode::deserialize(deposit_txid_bytes)
        .map_err(|_| WithdrawalParseError::InvalidDepositTxidBytes(deposit_txid_bytes.len()))?;

    let withdrawal_amount = BitcoinAmount::from_sat(withdrawal_fulfillment_output.value.to_sat());
    let withdrawal_address = withdrawal_fulfillment_output.script_pubkey.clone();

    Ok(WithdrawalInfo {
        operator_idx,
        deposit_idx,
        deposit_txid,
        withdrawal_address,
        withdrawal_amount,
    })
}
