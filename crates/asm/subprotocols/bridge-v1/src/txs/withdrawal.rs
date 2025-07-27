//! Withdrawal Transaction Parser and Validation
//!
//! This module provides functionality for parsing and validating Bitcoin withdrawal transactions
//! that follow the SPS-50 specification for the Strata bridge protocol.
//!
//! ## Withdrawal Transaction Structure
//!
//! A withdrawal transaction is a **frontpayment transaction** where an operator pays out
//! the withdrawal request before being able to withdraw the corresponding locked deposit.
//! This transaction has the following structure:
//!
//! ### Inputs
//! - **Operator Inputs** (flexible): Any inputs controlled by the operator making the frontpayment
//!   - The operator is responsible for funding this transaction from their own UTXOs
//!   - No specific input structure is enforced - it's up to the operator to handle funding
//!   - The operator will later be able to withdraw the corresponding N/N locked deposit
//!
//! ### Outputs
//! 1. **OP_RETURN Output (Index 0)** (required): Contains SPS-50 tagged data with:
//!    - Magic number (4 bytes): Protocol instance identifier
//!    - Subprotocol ID (1 byte): Bridge v1 subprotocol identifier
//!    - Transaction type (1 byte): Withdrawal transaction type
//!    - Auxiliary data (≤74 bytes):
//!      - Operator index (4 bytes, big-endian u32): Index of the operator processing the withdrawal
//!      - Deposit index (4 bytes, big-endian u32): Index of the original deposit being withdrawn
//!      - Deposit transaction ID (32 bytes): TXID of the deposit transaction being spent
//!
//! 2. **Withdrawal Fulfillment Output (Index 1)** (required): The actual withdrawal containing:
//!    - The recipient's Bitcoin address (script_pubkey)
//!    - The withdrawal amount (may be less than deposit due to fees)
//!
//! Additional outputs may be present (e.g., change outputs) but are ignored during validation.

use bitcoin::{OutPoint, ScriptBuf, Transaction, Txid, consensus::encode, hashes::Hash};
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

/// Creates a withdrawal fulfillment transaction for testing purposes.
///
/// This function constructs a Bitcoin transaction that follows the full SPS-50 specification
/// for withdrawal fulfillment transactions. The transaction contains:
/// - Input: A dummy input spending from a previous output
/// - Output 0: OP_RETURN with full SPS-50 format: [MAGIC][SUBPROTOCOL_ID][TX_TYPE][AUX_DATA]
/// - Output 1: The actual withdrawal payment to the recipient address
///
/// The transaction is fully compatible with the SPS-50 parser and can be parsed by `ParseConfig`.
///
/// # Parameters
///
/// - `withdrawal_info` - The withdrawal information specifying operator, deposit details, recipient
///   address, and withdrawal amount
///
/// # Returns
///
/// A [`Transaction`] that follows the SPS-50 specification and can be parsed for testing.
#[cfg(test)]
pub fn create_withdrawal_fulfillment_tx(withdrawal_info: &WithdrawalInfo) -> Transaction {
    use bitcoin::{Sequence, TxIn, Witness, script::PushBytesBuf};

    use crate::{
        constants::{BRIDGE_V1_SUBPROTOCOL_ID, WITHDRAWAL_TX_TYPE},
        txs::deposit::test::TEST_MAGIC_BYTES,
    };

    // Create SPS-50 tagged payload: [MAGIC][SUBPROTOCOL_ID][TX_TYPE][AUX_DATA]
    let mut tagged_payload = Vec::new();
    tagged_payload.extend_from_slice(TEST_MAGIC_BYTES); // 4 bytes magic
    tagged_payload.push(BRIDGE_V1_SUBPROTOCOL_ID); // 1 byte subprotocol ID
    tagged_payload.push(WITHDRAWAL_TX_TYPE); // 1 byte transaction type

    // Auxiliary data: [OPERATOR_IDX][DEPOSIT_IDX][DEPOSIT_TXID]
    tagged_payload.extend_from_slice(&withdrawal_info.operator_idx.to_be_bytes()); // 4 bytes
    tagged_payload.extend_from_slice(&withdrawal_info.deposit_idx.to_be_bytes()); // 4 bytes
    tagged_payload.extend_from_slice(withdrawal_info.deposit_txid.as_byte_array()); // 32 bytes

    // Create OP_RETURN script with the tagged payload
    let op_return_script = ScriptBuf::new_op_return(
        PushBytesBuf::try_from(tagged_payload).expect("Tagged payload should fit in push bytes"),
    );

    Transaction {
        version: bitcoin::transaction::Version(2),
        lock_time: bitcoin::absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(), // Dummy input
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::from_slice(&[vec![0u8; 64]]), // Dummy witness
        }],
        output: vec![
            // OP_RETURN output with SPS-50 tagged payload
            bitcoin::TxOut {
                value: bitcoin::Amount::from_sat(0),
                script_pubkey: op_return_script,
            },
            // Withdrawal fulfillment output
            bitcoin::TxOut {
                value: bitcoin::Amount::from_sat(withdrawal_info.withdrawal_amount.to_sat()),
                script_pubkey: withdrawal_info.withdrawal_address.clone(),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bitcoin::Address;
    use strata_asm_common::TxInputRef;
    use strata_l1_txfmt::ParseConfig;

    use super::*;
    use crate::txs::{deposit::test::TEST_MAGIC_BYTES, withdrawal::extract_withdrawal_info};

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
