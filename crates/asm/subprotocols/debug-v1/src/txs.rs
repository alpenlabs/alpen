//! Transaction parsing and handling for the debug subprotocol.
//!
//! This module implements parsing for debug transaction types that allow
//! injection of test data into the ASM for testing purposes.

use borsh::BorshDeserialize;
use strata_asm_common::TxInputRef;
use strata_asm_logs::AsmLogType;
use strata_asm_proto_bridge_v1::WithdrawOutput;
use strata_l1_txfmt::TxType;
use strata_primitives::{bitcoin_bosd::Descriptor, l1::BitcoinAmount};
use thiserror::Error;

use crate::constants::{
    AMOUNT_OFFSET, AMOUNT_SIZE, DESCRIPTOR_OFFSET, FAKE_ASM_LOG_TX_TYPE,
    FAKE_WITHDRAW_INTENT_TX_TYPE, MIN_ASM_LOG_AUX_DATA_LEN, MIN_FAKEWITHDRAW_AUX_DATA_LEN,
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

    /// Failed to deserialize ASM log type.
    #[error("failed to deserialize ASM log type: {0}")]
    AsmLogDeserializationError(#[from] borsh::io::Error),
}

/// Type alias for fake ASM log injection info.
pub(crate) type FakeAsmLogInfo = AsmLogType;

/// Type alias for fake withdrawal info.
pub(crate) type FakeWithdrawInfo = WithdrawOutput;

/// Parsed debug transaction types.
pub(crate) enum ParsedDebugTx {
    /// ASM log injection transaction.
    FakeAsmLog(FakeAsmLogInfo),

    /// Fake withdrawal creation transaction.
    FakeWithdrawIntent(FakeWithdrawInfo),
}

/// Parses a debug transaction from the given transaction input.
///
/// This function examines the transaction type and delegates to the appropriate
/// parsing function based on the type.
pub(crate) fn parse_debug_tx(tx: &TxInputRef<'_>) -> Result<ParsedDebugTx, DebugTxParseError> {
    match tx.tag().tx_type() {
        FAKE_ASM_LOG_TX_TYPE => parse_fake_asm_log_tx(tx),
        FAKE_WITHDRAW_INTENT_TX_TYPE => parse_fake_withdraw_intent_tx(tx),
        tx_type => Err(DebugTxParseError::UnsupportedTxType(tx_type)),
    }
}

/// Parses ASM log data from auxiliary data bytes.
fn parse_asm_log_from_aux_data(aux_data: &[u8]) -> Result<AsmLogType, DebugTxParseError> {
    if aux_data.len() < MIN_ASM_LOG_AUX_DATA_LEN {
        return Err(DebugTxParseError::AuxDataTooShort {
            expected: MIN_ASM_LOG_AUX_DATA_LEN,
            actual: aux_data.len(),
        });
    }

    // Deserialize the AsmLogType from auxiliary data
    let asm_log_type = AsmLogType::try_from_slice(aux_data)?;
    Ok(asm_log_type)
}

/// Parses an ASM log injection transaction.
///
/// Auxiliary data format:
/// - `[serialized AsmLogType]` - serialized AsmLogType
fn parse_fake_asm_log_tx(tx: &TxInputRef<'_>) -> Result<ParsedDebugTx, DebugTxParseError> {
    let aux_data = tx.tag().aux_data();
    let asm_log_type = parse_asm_log_from_aux_data(aux_data)?;
    Ok(ParsedDebugTx::FakeAsmLog(asm_log_type))
}

/// Parses withdrawal data from auxiliary data bytes.
fn parse_withdrawal_from_aux_data(aux_data: &[u8]) -> Result<WithdrawOutput, DebugTxParseError> {
    if aux_data.len() < MIN_FAKEWITHDRAW_AUX_DATA_LEN {
        return Err(DebugTxParseError::AuxDataTooShort {
            expected: MIN_FAKEWITHDRAW_AUX_DATA_LEN,
            actual: aux_data.len(),
        });
    }

    // Extract amount using constants
    let amount_end = AMOUNT_OFFSET + AMOUNT_SIZE;
    let amount_bytes: [u8; AMOUNT_SIZE] = aux_data[AMOUNT_OFFSET..amount_end].try_into().unwrap();
    let amount = u64::from_be_bytes(amount_bytes);
    let amt = BitcoinAmount::from_sat(amount);

    // Extract descriptor (self-describing, no length field needed)
    let desc_bytes = &aux_data[DESCRIPTOR_OFFSET..];
    let dest = Descriptor::from_bytes(desc_bytes)
        .map_err(|e| DebugTxParseError::InvalidDescriptorFormat(e.to_string()))?;

    let withdraw_output = WithdrawOutput::new(dest, amt);
    Ok(withdraw_output)
}

/// Parses a fake withdrawal transaction.
///
/// Auxiliary data format:
/// - `[amount: 8 bytes]` - The withdrawal amount in satoshis
/// - `[descriptor: variable]` - The self-describing Bitcoin descriptor
fn parse_fake_withdraw_intent_tx(tx: &TxInputRef<'_>) -> Result<ParsedDebugTx, DebugTxParseError> {
    let aux_data = tx.tag().aux_data();
    let withdraw_output = parse_withdrawal_from_aux_data(aux_data)?;
    Ok(ParsedDebugTx::FakeWithdrawIntent(withdraw_output))
}

#[cfg(test)]
mod tests {
    use strata_asm_logs::*;
    use strata_primitives::{bitcoin_bosd::Descriptor, l1::BitcoinAmount};

    use super::*;

    #[test]
    fn test_parse_asm_log_from_aux_data() {
        // Create test AsmLogType
        let deposit_log = DepositLog::new(1, 100_000, b"test_address".to_vec());
        let log_type = AsmLogType::DepositLog(deposit_log);

        // Serialize for auxiliary data
        let aux_data = borsh::to_vec(&log_type).unwrap();

        // Test the internal parsing function directly
        let parsed_log_type = parse_asm_log_from_aux_data(&aux_data).unwrap();

        // Verify the actual content matches, not just the variant
        match (&log_type, &parsed_log_type) {
            (AsmLogType::DepositLog(original), AsmLogType::DepositLog(parsed)) => {
                assert_eq!(original.ee_id, parsed.ee_id);
                assert_eq!(original.amount, parsed.amount);
                assert_eq!(original.addr, parsed.addr);
            }
            _ => panic!("Parsed log type variant mismatch"),
        }
    }

    #[test]
    fn test_parse_withdrawal_from_aux_data_p2wpkh() {
        // P2WPKH: type tag (0x00) + 20-byte hash = 21 bytes total
        let amount = 100_000u64;
        let hash160 = [0x14; 20]; // 20-byte hash
        let p2wpkh_descriptor = Descriptor::new_p2wpkh(&hash160);

        // Create auxiliary data: [amount: 8 bytes][descriptor: 21 bytes]
        let mut aux_data = Vec::new();
        aux_data.extend_from_slice(&amount.to_be_bytes());
        aux_data.extend_from_slice(&p2wpkh_descriptor.to_bytes());

        // Test the internal parsing function directly
        let withdraw_output = parse_withdrawal_from_aux_data(&aux_data).unwrap();

        assert_eq!(withdraw_output.amt, BitcoinAmount::from_sat(100_000));
        assert_eq!(
            withdraw_output.destination.to_bytes(),
            p2wpkh_descriptor.to_bytes()
        );
    }

    #[test]
    fn test_parse_withdrawal_from_aux_data_p2wsh() {
        // P2WSH: type tag (0x00) + 32-byte hash = 33 bytes total
        let amount = 200_000u64;
        let hash256 = [0x32; 32]; // 32-byte hash
        let p2wsh_descriptor = Descriptor::new_p2wsh(&hash256);

        // Create auxiliary data: [amount: 8 bytes][descriptor: 33 bytes]
        let mut aux_data = Vec::new();
        aux_data.extend_from_slice(&amount.to_be_bytes());
        aux_data.extend_from_slice(&p2wsh_descriptor.to_bytes());

        // Test the internal parsing function directly
        let withdraw_output = parse_withdrawal_from_aux_data(&aux_data).unwrap();

        assert_eq!(withdraw_output.amt, BitcoinAmount::from_sat(200_000));
        assert_eq!(
            withdraw_output.destination.to_bytes(),
            p2wsh_descriptor.to_bytes()
        );
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

        // Create auxiliary data: [amount: 8 bytes][descriptor: 33 bytes]
        let mut aux_data = Vec::new();
        aux_data.extend_from_slice(&amount.to_be_bytes());
        aux_data.extend_from_slice(&p2tr_descriptor.to_bytes());

        // Test the internal parsing function directly
        let withdraw_output = parse_withdrawal_from_aux_data(&aux_data).unwrap();

        assert_eq!(withdraw_output.amt, BitcoinAmount::from_sat(300_000));
        assert_eq!(
            withdraw_output.destination.to_bytes(),
            p2tr_descriptor.to_bytes()
        );
    }

    #[test]
    fn test_parse_withdrawal_error_handling() {
        // Test too short auxiliary data
        let short_aux_data = vec![1, 2, 3]; // Only 3 bytes, need at least 28

        let result = parse_withdrawal_from_aux_data(&short_aux_data);
        match result {
            Err(DebugTxParseError::AuxDataTooShort { expected, actual }) => {
                assert_eq!(expected, MIN_FAKEWITHDRAW_AUX_DATA_LEN);
                assert_eq!(actual, 3);
            }
            _ => panic!("Expected AuxDataTooShort error"),
        }
    }

    #[test]
    fn test_parse_asm_log_error_handling() {
        // Test too short auxiliary data
        let empty_aux_data = vec![]; // Empty, need at least 8 bytes

        let result = parse_asm_log_from_aux_data(&empty_aux_data);
        match result {
            Err(DebugTxParseError::AuxDataTooShort { expected, actual }) => {
                assert_eq!(expected, MIN_ASM_LOG_AUX_DATA_LEN);
                assert_eq!(actual, 0);
            }
            _ => panic!("Expected AuxDataTooShort error"),
        }
    }

    #[test]
    fn test_parse_asm_log_deserialization_error() {
        // Create invalid Borsh data that's long enough but malformed
        // Use a discriminant that doesn't match any AsmLogType variant (> 4)
        let invalid_aux_data = vec![255u8; 20]; // Invalid discriminant + garbage data

        let result = parse_asm_log_from_aux_data(&invalid_aux_data);
        match result {
            Err(DebugTxParseError::AsmLogDeserializationError(_)) => {
                // Success - we got the expected deserialization error
            }
            _ => panic!("Expected AsmLogDeserializationError"),
        }
    }
}
