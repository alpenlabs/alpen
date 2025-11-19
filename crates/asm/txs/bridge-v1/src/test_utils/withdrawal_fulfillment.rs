//! Withdrawal Fulfillment Transaction creation utilities for testing
//!
//! Provides both simple test utilities and comprehensive transaction builders for
//! withdrawal fulfillment transactions.

use bitcoin::{
    Amount, OutPoint, ScriptBuf, Sequence, Transaction, Txid, TxIn, TxOut, Witness,
    absolute::LockTime,
    consensus::serialize,
    script::PushBytesBuf,
    transaction::Version,
};
use strata_codec::encode_to_vec;
use strata_l1_txfmt::{ParseConfig, TagData};

use crate::{
    constants::{BRIDGE_V1_SUBPROTOCOL_ID, WITHDRAWAL_FULFILLMENT_TX_TYPE},
    test_utils::TEST_MAGIC_BYTES,
    withdrawal_fulfillment::WithdrawalFulfillmentInfo,
};

/// Error type for withdrawal transaction building
#[derive(Debug, Clone, thiserror::Error)]
pub enum WithdrawalTxBuilderError {
    #[error("Transaction builder error: {0}")]
    TxBuilder(String),

    #[error("Insufficient inputs provided")]
    InsufficientInputs,

    #[error("SPS-50 format error: {0}")]
    TxFmt(String),
}

/// Withdrawal fulfillment transaction metadata
///
/// Contains all the information needed to create the OP_RETURN output for
/// a withdrawal fulfillment transaction following the SPS-50 specification.
#[derive(Debug, Clone)]
pub struct WithdrawalMetadata {
    /// The tag used to mark the withdrawal metadata transaction
    pub tag: [u8; 4],
    /// The index of the operator
    pub operator_idx: u32,
    /// The index of the deposit
    pub deposit_idx: u32,
    /// The txid of the deposit UTXO
    pub deposit_txid: Txid,
}

impl WithdrawalMetadata {
    /// Creates new withdrawal metadata
    pub fn new(tag: [u8; 4], operator_idx: u32, deposit_idx: u32, deposit_txid: Txid) -> Self {
        Self {
            tag,
            operator_idx,
            deposit_idx,
            deposit_txid,
        }
    }

    /// Generates the auxiliary data for the withdrawal metadata (for SPS-50 format)
    ///
    /// This is everything after the magic bytes, subprotocol ID, and tx type.
    /// Format: [OPERATOR_IDX (4)][DEPOSIT_IDX (4)][DEPOSIT_TXID (32)]
    pub fn aux_data(&self) -> Vec<u8> {
        let deposit_txid_data = serialize(&self.deposit_txid);
        let mut aux_data = Vec::new();
        aux_data.extend_from_slice(&self.operator_idx.to_be_bytes()); // 4 bytes
        aux_data.extend_from_slice(&self.deposit_idx.to_be_bytes());  // 4 bytes
        aux_data.extend_from_slice(&deposit_txid_data);               // 32 bytes
        aux_data
    }

    /// Generates the complete OP_RETURN data for the withdrawal metadata
    ///
    /// Returns the full SPS-50 tagged payload:
    /// [MAGIC (4)][SUBPROTOCOL_ID (1)][TX_TYPE (1)][AUX_DATA (40)]
    pub fn op_return_data(&self) -> Result<Vec<u8>, WithdrawalTxBuilderError> {
        let aux_data = self.aux_data();

        let tag_data = TagDataRef::new(BRIDGE_V1_SUBPROTOCOL_ID, WITHDRAWAL_TX_TYPE, &aux_data)
            .map_err(|e| WithdrawalTxBuilderError::TxFmt(e.to_string()))?;

        let parse_config = ParseConfig::new(self.tag);
        let data = parse_config
            .encode_tag_buf(&tag_data)
            .map_err(|e| WithdrawalTxBuilderError::TxFmt(e.to_string()))?;

        Ok(data)
    }
}

/// Input for creating a withdrawal fulfillment transaction
///
/// Represents a UTXO that will be spent in the withdrawal transaction.
#[derive(Debug, Clone)]
pub struct WithdrawalInput {
    /// Previous output being spent
    pub previous_output: OutPoint,
    /// The script_pubkey of the UTXO being spent
    pub script_pubkey: ScriptBuf,
    /// The value of the UTXO being spent
    pub value: Amount,
    /// Witness data for the input (signature, etc.)
    pub witness: Witness,
}

/// Creates a withdrawal fulfillment transaction from explicit inputs (CLI/Production use)
///
/// This is the primary API for building withdrawal fulfillment transactions.
/// The caller is responsible for providing properly funded inputs with valid witnesses.
///
/// # Arguments
/// * `metadata` - Withdrawal metadata containing operator/deposit information
/// * `recipient_script` - Script pubkey for the withdrawal recipient
/// * `withdrawal_amount` - Amount to send to recipient
/// * `inputs` - Vector of inputs to fund the transaction (must be non-empty)
/// * `change_script` - Optional change output (script and amount)
///
/// # Returns
/// A `Transaction` ready for broadcast
///
/// # Errors
/// Returns `WithdrawalTxBuilderError::InsufficientInputs` if inputs vector is empty
pub fn create_withdrawal_fulfillment_tx(
    metadata: WithdrawalMetadata,
    recipient_script: ScriptBuf,
    withdrawal_amount: Amount,
    inputs: Vec<WithdrawalInput>,
    change_script: Option<(ScriptBuf, Amount)>,
) -> Result<Transaction, WithdrawalTxBuilderError> {
    if inputs.is_empty() {
        return Err(WithdrawalTxBuilderError::InsufficientInputs);
    }

    // Create transaction inputs
    let tx_ins: Vec<TxIn> = inputs
        .iter()
        .map(|input| TxIn {
            previous_output: input.previous_output,
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: input.witness.clone(),
        })
        .collect();

    // Create OP_RETURN output with metadata
    let op_return_script = create_withdrawal_op_return(&metadata)?;

    let mut tx_outs = vec![
        // OP_RETURN output
        TxOut {
            value: Amount::ZERO,
            script_pubkey: op_return_script,
        },
        // Withdrawal recipient output
        TxOut {
            value: withdrawal_amount,
            script_pubkey: recipient_script,
        },
    ];

    // Add change output if provided
    if let Some((change_script, change_amount)) = change_script {
        tx_outs.push(TxOut {
            value: change_amount,
            script_pubkey: change_script,
        });
    }

    Ok(Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: tx_ins,
        output: tx_outs,
    })
}

/// Creates a simple withdrawal fulfillment transaction with a single input
///
/// Simplified version for the common case of single UTXO spending.
/// This is a convenience wrapper around `create_withdrawal_fulfillment_tx`.
///
/// # Arguments
/// * `metadata` - Withdrawal metadata
/// * `recipient_script` - Script pubkey for recipient
/// * `withdrawal_amount` - Amount to send
/// * `input` - Single input to spend
///
/// # Returns
/// A `Transaction` with no change output
pub fn create_simple_withdrawal_fulfillment_tx(
    metadata: WithdrawalMetadata,
    recipient_script: ScriptBuf,
    withdrawal_amount: Amount,
    input: WithdrawalInput,
) -> Result<Transaction, WithdrawalTxBuilderError> {
    create_withdrawal_fulfillment_tx(metadata, recipient_script, withdrawal_amount, vec![input], None)
}

/// Creates a withdrawal fulfillment transaction for testing purposes
///
/// This function constructs a Bitcoin transaction that follows the full SPS-50 specification
/// for withdrawal fulfillment transactions. The transaction contains:
/// - Input: A dummy input spending from a previous output
/// - Output 0: OP_RETURN with full SPS-50 format: MAGIC + SUBPROTOCOL_ID + TX_TYPE + AUX_DATA
/// - Output 1: The actual withdrawal payment to the recipient address
///
/// The transaction is fully compatible with the SPS-50 parser and can be parsed by `ParseConfig`.
///
/// # Arguments
/// * `withdrawal_info` - The withdrawal information specifying operator, deposit details,
///   recipient address, and withdrawal amount
///
/// # Returns
/// A `Transaction` that follows the SPS-50 specification, ready for use in tests
pub fn create_test_withdrawal_fulfillment_tx(
    withdrawal_info: &WithdrawalFulfillmentInfo,
) -> Transaction {
    let aux_data = encode_to_vec(withdrawal_info.header_aux()).unwrap();
    let td = TagData::new(
        BRIDGE_V1_SUBPROTOCOL_ID,
        WITHDRAWAL_FULFILLMENT_TX_TYPE,
        aux_data,
    )
    .unwrap();
    let sps_50_script = ParseConfig::new(*TEST_MAGIC_BYTES)
        .encode_script_buf(&td.as_ref())
        .unwrap();

    Transaction {
        version: Version(2),
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(), // Dummy input
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::from_slice(&[vec![0u8; 64]]), // Dummy witness
        }],
        output: vec![
            // OP_RETURN output with SPS-50 tagged payload
            TxOut {
                value: Amount::from_sat(0),
                script_pubkey: sps_50_script,
            },
            // Withdrawal fulfillment output
            TxOut {
                value: Amount::from_sat(withdrawal_info.withdrawal_amount().to_sat()),
                script_pubkey: withdrawal_info.withdrawal_destination().clone(),
            },
        ],
    }
}

/// Creates an OP_RETURN script with withdrawal metadata using SPS-50 format
fn create_withdrawal_op_return(
    metadata: &WithdrawalMetadata,
) -> Result<ScriptBuf, WithdrawalTxBuilderError> {
    // Get auxiliary data (operator_idx + deposit_idx + deposit_txid)
    let aux_data = metadata.aux_data();

    // Create SPS-50 tagged data
    let tag_data = TagDataRef::new(BRIDGE_V1_SUBPROTOCOL_ID, WITHDRAWAL_TX_TYPE, &aux_data)
        .map_err(|e| WithdrawalTxBuilderError::TxFmt(e.to_string()))?;

    // Encode to OP_RETURN script using ParseConfig
    let op_return_script = ParseConfig::new(metadata.tag)
        .encode_script_buf(&tag_data)
        .map_err(|e| WithdrawalTxBuilderError::TxFmt(e.to_string()))?;

    Ok(op_return_script)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn test_withdrawal_metadata_creation() {
        let txid = Txid::from_str(
            "ae86b8c8912594427bf148eb7660a86378f2fb4ac9c8d2ea7d3cb7f3fcfd7c1c",
        )
        .unwrap();

        let metadata = WithdrawalMetadata::new(*b"TEST", 1, 2, txid);

        assert_eq!(metadata.operator_idx, 1);
        assert_eq!(metadata.deposit_idx, 2);
        assert_eq!(metadata.deposit_txid, txid);

        // Test auxiliary data (operator_idx + deposit_idx + deposit_txid)
        let aux_data = metadata.aux_data();
        assert_eq!(aux_data.len(), 40); // 4 + 4 + 32 bytes

        // Check operator index
        assert_eq!(&aux_data[0..4], &1u32.to_be_bytes());

        // Check deposit index
        assert_eq!(&aux_data[4..8], &2u32.to_be_bytes());

        // Check deposit txid
        let deposit_txid_data = serialize(&txid);
        assert_eq!(&aux_data[8..40], deposit_txid_data.as_slice());
    }

    #[test]
    fn test_create_simple_withdrawal_tx() {
        let txid = Txid::from_str(
            "ae86b8c8912594427bf148eb7660a86378f2fb4ac9c8d2ea7d3cb7f3fcfd7c1c",
        )
        .unwrap();

        let metadata = WithdrawalMetadata::new(*b"TEST", 1, 2, txid);

        let input = WithdrawalInput {
            previous_output: OutPoint::new(txid, 0),
            script_pubkey: ScriptBuf::new(),
            value: Amount::from_sat(1_000_000),
            witness: Witness::from_slice(&[vec![0u8; 64]]),
        };

        let recipient_script = ScriptBuf::new();
        let withdrawal_amount = Amount::from_sat(500_000);

        let tx = create_simple_withdrawal_fulfillment_tx(
            metadata,
            recipient_script.clone(),
            withdrawal_amount,
            input,
        )
        .unwrap();

        // Verify structure
        assert_eq!(tx.input.len(), 1);
        assert_eq!(tx.output.len(), 2);

        // Verify OP_RETURN output
        assert!(tx.output[0].script_pubkey.is_op_return());
        assert_eq!(tx.output[0].value, Amount::ZERO);

        // Verify withdrawal output
        assert_eq!(tx.output[1].script_pubkey, recipient_script);
        assert_eq!(tx.output[1].value, withdrawal_amount);
    }

    #[test]
    fn test_create_withdrawal_tx_with_change() {
        let txid = Txid::from_str(
            "ae86b8c8912594427bf148eb7660a86378f2fb4ac9c8d2ea7d3cb7f3fcfd7c1c",
        )
        .unwrap();

        let metadata = WithdrawalMetadata::new(*b"TEST", 1, 2, txid);

        let input = WithdrawalInput {
            previous_output: OutPoint::new(txid, 0),
            script_pubkey: ScriptBuf::new(),
            value: Amount::from_sat(1_000_000),
            witness: Witness::from_slice(&[vec![0u8; 64]]),
        };

        let recipient_script = ScriptBuf::new();
        let withdrawal_amount = Amount::from_sat(500_000);
        let change_script = ScriptBuf::new();
        let change_amount = Amount::from_sat(450_000);

        let tx = create_withdrawal_fulfillment_tx(
            metadata,
            recipient_script.clone(),
            withdrawal_amount,
            vec![input],
            Some((change_script.clone(), change_amount)),
        )
        .unwrap();

        // Verify structure
        assert_eq!(tx.input.len(), 1);
        assert_eq!(tx.output.len(), 3);

        // Verify OP_RETURN output
        assert!(tx.output[0].script_pubkey.is_op_return());

        // Verify withdrawal output
        assert_eq!(tx.output[1].value, withdrawal_amount);

        // Verify change output
        assert_eq!(tx.output[2].script_pubkey, change_script);
        assert_eq!(tx.output[2].value, change_amount);
    }

    #[test]
    fn test_empty_inputs_error() {
        let txid = Txid::from_str(
            "ae86b8c8912594427bf148eb7660a86378f2fb4ac9c8d2ea7d3cb7f3fcfd7c1c",
        )
        .unwrap();

        let metadata = WithdrawalMetadata::new(*b"TEST", 1, 2, txid);
        let result = create_withdrawal_fulfillment_tx(
            metadata,
            ScriptBuf::new(),
            Amount::from_sat(500_000),
            vec![],
            None,
        );

        assert!(matches!(
            result,
            Err(WithdrawalTxBuilderError::InsufficientInputs)
        ));
    }
}
