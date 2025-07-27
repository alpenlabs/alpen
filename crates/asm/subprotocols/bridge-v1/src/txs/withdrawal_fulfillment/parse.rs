use bitcoin::{ScriptBuf, Txid, consensus::encode};
use strata_asm_common::TxInputRef;
use strata_primitives::{bridge::OperatorIdx, l1::BitcoinAmount};

use crate::errors::WithdrawalParseError;

/// Information extracted from a Bitcoin withdrawal transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithdrawalInfo {
    /// The index of the operator who processed this withdrawal.
    pub(crate) operator_idx: OperatorIdx,

    /// The index of the deposit that the operator wishes to receive payout from later.
    /// This must be validated against the operator's assigned deposits in the state's assignments
    /// table to ensure the operator is authorized to claim this specific deposit.
    pub(crate) deposit_idx: u32,

    /// The transaction ID of the deposit that the operator wishes to claim for payout.
    /// This must match the deposit referenced by `deposit_idx` in the assignments table.
    pub(crate) deposit_txid: Txid,

    /// The Bitcoin script address where the withdrawn funds are being sent.
    pub(crate) withdrawal_address: ScriptBuf,

    /// The amount of Bitcoin being withdrawn (may be less than the original deposit due to fees).
    pub(crate) withdrawal_amount: BitcoinAmount,
}

/// Extracts withdrawal information from a Bitcoin bridge withdrawal transaction.
///
/// Parses a withdrawal transaction following the SPS-50 specification and extracts
/// the withdrawal information including operator index, deposit references, recipient address,
/// and withdrawal amount. See the module-level documentation for the complete transaction
/// structure.
///
/// The function validates the transaction structure and parses the auxiliary data containing:
/// - Operator index (4 bytes, big-endian u32)
/// - Deposit index (4 bytes, big-endian u32)
/// - Deposit transaction ID (32 bytes)
///
/// # Parameters
///
/// - `tx` - Reference to the transaction input containing the withdrawal transaction and its
///   associated tag data
///
/// # Returns
///
/// - `Ok(WithdrawalInfo)` - Successfully parsed withdrawal information
/// - `Err(WithdrawalParseError)` - If the transaction structure is invalid, has insufficient
///   outputs, invalid metadata size, or any parsing step encounters malformed data
///
/// # Errors
///
/// This function will return an error if:
/// - The transaction has fewer than 2 outputs (missing withdrawal fulfillment or OP_RETURN)
/// - The auxiliary data size doesn't match the expected metadata size
/// - Any of the metadata fields cannot be parsed correctly
pub fn extract_withdrawal_info<'t>(
    tx: &TxInputRef<'t>,
) -> Result<WithdrawalInfo, WithdrawalParseError> {
    if tx.tx().output.len() < 2 {
        return Err(WithdrawalParseError::InsufficientOutputs(
            tx.tx().output.len(),
        ));
    }

    let withdrawal_fulfillment_output = &tx.tx().output[1];
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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bitcoin::{Address, hashes::Hash};
    use strata_asm_common::TxInputRef;
    use strata_l1_txfmt::ParseConfig;

    use super::*;
    use crate::txs::{deposit::test::TEST_MAGIC_BYTES, withdrawal_fulfillment::extract_withdrawal_info};
    
    use crate::txs::withdrawal_fulfillment::create_withdrawal_fulfillment_tx;

    fn generate_withdrawal_info() -> WithdrawalInfo {
        WithdrawalInfo {
            operator_idx: 42,
            deposit_idx: 1337,
            deposit_txid: Txid::from_byte_array([0xab; 32]),
            withdrawal_address: Address::from_str("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4")
                .unwrap()
                .assume_checked()
                .script_pubkey(),
            withdrawal_amount: BitcoinAmount::from_sat(95_000),
        }
    }

    #[test]
    fn test_create_withdrawal_fulfillment_tx_and_extract_info() {
        let original_withdrawal_info = generate_withdrawal_info();
        // Create the withdrawal fulfillment transaction with proper SPS-50 format
        let tx = create_withdrawal_fulfillment_tx(&original_withdrawal_info);

        // Parse the transaction using the SPS-50 parser
        let parser = ParseConfig::new(*TEST_MAGIC_BYTES);
        let tag_data = parser.try_parse_tx(&tx).expect("Should parse transaction");
        let tx_input_ref = TxInputRef::new(&tx, tag_data);

        // Extract withdrawal info using the actual parser
        let extracted_info = extract_withdrawal_info(&tx_input_ref)
            .expect("Should successfully extract withdrawal info");

        assert_eq!(extracted_info, original_withdrawal_info);
    }
}
