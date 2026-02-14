//! Transaction parsing and handling for the debug subprotocol.
//!
//! This module implements parsing for debug transaction types that allow
//! injection of test data into the ASM for testing purposes.

use strata_asm_bridge_msgs::WithdrawOutput;
use strata_asm_common::TxInputRef;
use strata_l1_txfmt::TxType;
use strata_primitives::{bitcoin_bosd::Descriptor, l1::BitcoinAmount};
use thiserror::Error;

use crate::constants::{
    AMOUNT_OFFSET, AMOUNT_SIZE, MAX_OPERATOR_INDEX_LEN, MIN_MOCK_WITHDRAW_INTENT_AUX_DATA_LEN,
    MOCK_ASM_LOG_TX_TYPE, MOCK_WITHDRAW_INTENT_TX_TYPE, OPERATOR_LEN_OFFSET,
};

/// Errors that can occur during debug transaction parsing.
#[derive(Debug, Error)]
pub(crate) enum DebugTxParseError {
    /// The transaction type is not supported by the debug subprotocol.
    #[error("unsupported transaction type: {0}")]
    UnsupportedTxType(TxType),

    /// The auxiliary data is too short for the expected format.
    #[error("auxiliary data too short: expected at least {expected} bytes, got {actual} bytes")]
    AuxDataTooShort { expected: usize, actual: usize },

    /// Invalid descriptor format.
    #[error("invalid descriptor format: {0}")]
    InvalidDescriptorFormat(String),

    /// The operator index length byte exceeds the maximum of 4.
    #[error("invalid operator index length: {0} (max 4)")]
    InvalidOperatorIndexLen(usize),
}

/// Info for mock ASM log injection.
pub(crate) struct MockAsmLogInfo {
    pub bytes: Vec<u8>,
}

/// Type alias for mock withdrawal info.
pub(crate) type MockWithdrawInfo = WithdrawOutput;

/// Parsed debug transaction types.
pub(crate) enum ParsedDebugTx {
    /// ASM log injection transaction.
    MockAsmLog(MockAsmLogInfo),

    /// Mock withdrawal creation transaction.
    MockWithdrawIntent(MockWithdrawInfo),
}

/// Parses a debug transaction from the given transaction input.
///
/// This function examines the transaction type and delegates to the appropriate
/// parsing function based on the type.
pub(crate) fn parse_debug_tx(tx: &TxInputRef<'_>) -> Result<ParsedDebugTx, DebugTxParseError> {
    match tx.tag().tx_type() {
        MOCK_ASM_LOG_TX_TYPE => parse_mock_asm_log_tx(tx),
        MOCK_WITHDRAW_INTENT_TX_TYPE => parse_mock_withdraw_intent_tx(tx),
        tx_type => Err(DebugTxParseError::UnsupportedTxType(tx_type)),
    }
}

/// Extracts raw log bytes from auxiliary data.
/// The auxiliary data directly contains the raw log bytes - no parsing needed.
fn extract_log_bytes_from_aux_data(aux_data: &[u8]) -> Result<MockAsmLogInfo, DebugTxParseError> {
    if aux_data.is_empty() {
        return Err(DebugTxParseError::AuxDataTooShort {
            expected: 1, // At least 1 byte
            actual: 0,
        });
    }

    // The auxiliary data directly contains the raw log bytes
    Ok(MockAsmLogInfo {
        bytes: aux_data.to_vec(),
    })
}

/// Parses an ASM log injection transaction.
///
/// Auxiliary data format:
/// - `[raw log bytes]` - The raw bytes that will become the log entry
fn parse_mock_asm_log_tx(tx: &TxInputRef<'_>) -> Result<ParsedDebugTx, DebugTxParseError> {
    let aux_data = tx.tag().aux_data();
    let asm_log_info = extract_log_bytes_from_aux_data(aux_data)?;
    Ok(ParsedDebugTx::MockAsmLog(asm_log_info))
}

/// Parses withdrawal data from auxiliary data bytes.
///
/// Format: `[amount: 8 bytes][1 byte B][B bytes: operator index (big-endian)][descriptor: variable]`
/// - B=0: no operator preference
/// - B=1..4: operator index encoded as B big-endian bytes
/// - B>4: invalid
fn parse_withdrawal_from_aux_data(aux_data: &[u8]) -> Result<WithdrawOutput, DebugTxParseError> {
    if aux_data.len() < MIN_MOCK_WITHDRAW_INTENT_AUX_DATA_LEN {
        return Err(DebugTxParseError::AuxDataTooShort {
            expected: MIN_MOCK_WITHDRAW_INTENT_AUX_DATA_LEN,
            actual: aux_data.len(),
        });
    }

    // Extract amount (8 bytes, big-endian)
    let amount_end = AMOUNT_OFFSET + AMOUNT_SIZE;
    let amount_bytes: [u8; AMOUNT_SIZE] = aux_data[AMOUNT_OFFSET..amount_end].try_into().unwrap();
    let amount = u64::from_be_bytes(amount_bytes);
    let amt = BitcoinAmount::from_sat(amount);

    // Extract preferred operator using variable-length encoding
    // (same format as the EE bridge precompile calldata)
    let b = aux_data[OPERATOR_LEN_OFFSET] as usize;
    let operator_data_offset = OPERATOR_LEN_OFFSET + 1;

    let (preferred_operator, descriptor_offset) = if b == 0 {
        // B=0: no operator preference
        (None, operator_data_offset)
    } else if b > MAX_OPERATOR_INDEX_LEN {
        // B>4: operator index cannot exceed u32 (4 bytes)
        return Err(DebugTxParseError::InvalidOperatorIndexLen(b));
    } else if aux_data.len() < operator_data_offset + b {
        // Not enough bytes remaining to read B operator index bytes
        return Err(DebugTxParseError::AuxDataTooShort {
            expected: operator_data_offset + b,
            actual: aux_data.len(),
        });
    } else {
        // B=1..4: decode B big-endian bytes into a u32 operator index
        let operator_idx = aux_data[operator_data_offset..operator_data_offset + b]
            .iter()
            .fold(0u32, |acc, &byte| (acc << 8) | byte as u32);
        (Some(operator_idx), operator_data_offset + b)
    };

    // Extract descriptor (self-describing, variable length, consumes rest of aux_data)
    let desc_bytes = &aux_data[descriptor_offset..];
    let dest = Descriptor::from_bytes(desc_bytes)
        .map_err(|e| DebugTxParseError::InvalidDescriptorFormat(e.to_string()))?;

    let withdraw_output = WithdrawOutput::new(dest, amt, preferred_operator);
    Ok(withdraw_output)
}

/// Parses a mock withdrawal transaction.
///
/// Auxiliary data format:
/// - `[amount: 8 bytes]` - The withdrawal amount in satoshis (big-endian)
/// - `[1 byte B]` - Operator index length (0 = no preference, 1..4 = index byte count)
/// - `[B bytes]` - Operator index (big-endian), omitted when B=0
/// - `[descriptor: variable]` - The self-describing Bitcoin descriptor
fn parse_mock_withdraw_intent_tx(tx: &TxInputRef<'_>) -> Result<ParsedDebugTx, DebugTxParseError> {
    let aux_data = tx.tag().aux_data();
    let withdraw_output = parse_withdrawal_from_aux_data(aux_data)?;
    Ok(ParsedDebugTx::MockWithdrawIntent(withdraw_output))
}

#[cfg(test)]
mod tests {
    use strata_codec::VarVec;
    use strata_primitives::{bitcoin_bosd::Descriptor, l1::BitcoinAmount};

    use super::*;

    #[test]
    fn test_extract_and_reconstruct_deposit_log() {
        use strata_asm_common::AsmLogEntry;
        use strata_asm_logs::deposit::DepositLog;

        // Step 1: Create a real DepositLog
        let original_deposit_log = DepositLog::new(VarVec::new(), 100_000);

        // Step 2: Convert it to bytes using AsmLogEntry::from_log
        let log_entry = AsmLogEntry::from_log(&original_deposit_log).unwrap();
        let log_bytes = log_entry.into_bytes();

        // Step 3: Pass the bytes through our extraction function (simulating aux data)
        let extracted_info = extract_log_bytes_from_aux_data(&log_bytes).unwrap();

        // Verify the bytes are preserved
        assert_eq!(extracted_info.bytes, log_bytes);

        // Step 4: Create a new AsmLogEntry from the extracted bytes
        let reconstructed_entry = AsmLogEntry::from_raw(extracted_info.bytes);

        // Step 5: Deserialize back to DepositLog
        let reconstructed_log: DepositLog = reconstructed_entry.try_into_log().unwrap();

        // Step 6: Verify the reconstructed log matches the original
        assert_eq!(
            reconstructed_log.destination,
            original_deposit_log.destination
        );
        assert_eq!(reconstructed_log.amount, original_deposit_log.amount);
    }

    #[test]
    fn test_parse_withdrawal_from_aux_data_p2wpkh() {
        // P2WPKH: type tag (0x00) + 20-byte hash = 21 bytes total
        let amount = 100_000u64;
        let hash160 = [0x14; 20]; // 20-byte hash
        let p2wpkh_descriptor = Descriptor::new_p2wpkh(&hash160);

        // Create auxiliary data: [amount: 8 bytes][B=0: 1 byte][descriptor: 21 bytes]
        let mut aux_data = Vec::new();
        aux_data.extend_from_slice(&amount.to_be_bytes());
        aux_data.push(0x00); // B=0, no operator preference
        aux_data.extend_from_slice(&p2wpkh_descriptor.to_bytes());

        // Test the internal parsing function directly
        let withdraw_output = parse_withdrawal_from_aux_data(&aux_data).unwrap();

        assert_eq!(withdraw_output.amt, BitcoinAmount::from_sat(100_000));
        assert_eq!(
            withdraw_output.destination.to_bytes(),
            p2wpkh_descriptor.to_bytes()
        );
        assert_eq!(withdraw_output.preferred_operator, None);
    }

    #[test]
    fn test_parse_withdrawal_from_aux_data_p2wsh() {
        // P2WSH: type tag (0x00) + 32-byte hash = 33 bytes total
        let amount = 200_000u64;
        let hash256 = [0x32; 32]; // 32-byte hash
        let p2wsh_descriptor = Descriptor::new_p2wsh(&hash256);

        // Create auxiliary data: [amount: 8][B=1: 1][operator: 1][descriptor: 33]
        let mut aux_data = Vec::new();
        aux_data.extend_from_slice(&amount.to_be_bytes());
        aux_data.push(0x01); // B=1, one byte for operator index
        aux_data.push(42); // operator index = 42
        aux_data.extend_from_slice(&p2wsh_descriptor.to_bytes());

        // Test the internal parsing function directly
        let withdraw_output = parse_withdrawal_from_aux_data(&aux_data).unwrap();

        assert_eq!(withdraw_output.amt, BitcoinAmount::from_sat(200_000));
        assert_eq!(
            withdraw_output.destination.to_bytes(),
            p2wsh_descriptor.to_bytes()
        );
        assert_eq!(withdraw_output.preferred_operator, Some(42));
    }

    #[test]
    fn test_parse_withdrawal_from_aux_data_p2tr() {
        // P2TR: type tag (0x01) + 32-byte x-only pubkey = 33 bytes total
        let amount = 300_000u64;
        // Use a known valid x-only public key (from Bitcoin Core test vectors)
        let x_only_pubkey = [
            0x79, 0xbe, 0x66, 0x7e, 0xf9, 0xdc, 0xbb, 0xac, 0x55, 0xa0, 0x62, 0x95, 0xce, 0x87,
            0x0b, 0x07, 0x02, 0x9b, 0xfc, 0xdb, 0x2d, 0xce, 0x28, 0xd9, 0x59, 0xf2, 0x81, 0x5b,
            0x16, 0xf8, 0x17, 0x98,
        ];
        let p2tr_descriptor = Descriptor::new_p2tr(&x_only_pubkey).unwrap();

        // Create auxiliary data: [amount: 8][B=0: 1][descriptor: 33]
        let mut aux_data = Vec::new();
        aux_data.extend_from_slice(&amount.to_be_bytes());
        aux_data.push(0x00); // B=0, no operator preference
        aux_data.extend_from_slice(&p2tr_descriptor.to_bytes());

        // Test the internal parsing function directly
        let withdraw_output = parse_withdrawal_from_aux_data(&aux_data).unwrap();

        assert_eq!(withdraw_output.amt, BitcoinAmount::from_sat(300_000));
        assert_eq!(
            withdraw_output.destination.to_bytes(),
            p2tr_descriptor.to_bytes()
        );
        assert_eq!(withdraw_output.preferred_operator, None);
    }

    #[test]
    fn test_parse_withdrawal_error_handling() {
        // Test too short auxiliary data
        let short_aux_data = vec![1, 2, 3]; // Only 3 bytes, need at least 29

        let result = parse_withdrawal_from_aux_data(&short_aux_data);
        match result {
            Err(DebugTxParseError::AuxDataTooShort { expected, actual }) => {
                assert_eq!(expected, MIN_MOCK_WITHDRAW_INTENT_AUX_DATA_LEN);
                assert_eq!(actual, 3);
            }
            _ => panic!("Expected AuxDataTooShort error"),
        }
    }

    #[test]
    fn test_extract_log_bytes_error_handling() {
        // Test empty auxiliary data
        let empty_aux_data = vec![];

        let result = extract_log_bytes_from_aux_data(&empty_aux_data);
        match result {
            Err(DebugTxParseError::AuxDataTooShort { expected, actual }) => {
                assert_eq!(expected, 1);
                assert_eq!(actual, 0);
            }
            _ => panic!("Expected AuxDataTooShort error"),
        }
    }

    #[test]
    fn test_parse_withdrawal_invalid_operator_len() {
        let amount = 100_000u64;
        let mut aux_data = Vec::new();
        aux_data.extend_from_slice(&amount.to_be_bytes());
        aux_data.push(0x05); // B=5, exceeds MAX_OPERATOR_INDEX_LEN
        aux_data.extend_from_slice(&[0x00; 25]); // padding to pass min length check

        let result = parse_withdrawal_from_aux_data(&aux_data);
        assert!(matches!(
            result,
            Err(DebugTxParseError::InvalidOperatorIndexLen(5))
        ));
    }
}
