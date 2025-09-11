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

use crate::constants::{FAKE_ASM_LOG_TX_TYPE, FAKE_WITHDRAW_INTENT_TX_TYPE};

/// Errors that can occur during debug transaction parsing.
#[derive(Debug, Error)]
pub(crate) enum DebugTxParseError {
    /// The transaction type is not supported by the debug subprotocol.
    #[error("unsupported transaction type: {0}")]
    UnsupportedTxType(TxType),

    /// The auxiliary data is too short for the expected format.
    #[error("auxiliary data too short: expected at least {expected} bytes, got {actual} bytes")]
    AuxDataTooShort { expected: usize, actual: usize },

    /// Invalid UTF-8 in descriptor string.
    #[error("invalid descriptor string: {0}")]
    InvalidDescriptor(#[from] std::str::Utf8Error),

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

/// Minimum auxiliary data length for ASM log transactions.
///
/// Format: [serialized AsmLogType]
const MIN_ASM_LOG_AUX_DATA_LEN: usize = 1;

/// Minimum auxiliary data length for fake withdrawal transactions.
///
/// Format: [amount: 8 bytes][desc_len: 4 bytes][descriptor: variable]
const MIN_FAKEWITHDRAW_AUX_DATA_LEN: usize = 12;

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

/// Parses an ASM log injection transaction.
///
/// Auxiliary data format:
/// - `[serialized AsmLogType]` - Borsh-serialized AsmLogType
fn parse_fake_asm_log_tx(tx: &TxInputRef<'_>) -> Result<ParsedDebugTx, DebugTxParseError> {
    let aux_data = tx.tag().aux_data();

    if aux_data.len() < MIN_ASM_LOG_AUX_DATA_LEN {
        return Err(DebugTxParseError::AuxDataTooShort {
            expected: MIN_ASM_LOG_AUX_DATA_LEN,
            actual: aux_data.len(),
        });
    }

    // Deserialize the AsmLogType from auxiliary data
    let asm_log_type = AsmLogType::try_from_slice(aux_data)?;

    Ok(ParsedDebugTx::FakeAsmLog(asm_log_type))
}

/// Parses a fake withdrawal transaction.
///
/// Auxiliary data format:
/// - `[amount: 8 bytes]` - The withdrawal amount in satoshis
/// - `[desc_len: 4 bytes]` - The length of the descriptor string
/// - `[descriptor: desc_len bytes]` - The Bitcoin descriptor string
fn parse_fake_withdraw_intent_tx(tx: &TxInputRef<'_>) -> Result<ParsedDebugTx, DebugTxParseError> {
    let aux_data = tx.tag().aux_data();

    if aux_data.len() < MIN_FAKEWITHDRAW_AUX_DATA_LEN {
        return Err(DebugTxParseError::AuxDataTooShort {
            expected: MIN_FAKEWITHDRAW_AUX_DATA_LEN,
            actual: aux_data.len(),
        });
    }

    // Extract amount (8 bytes)
    let amount_bytes: [u8; 8] = aux_data[0..8].try_into().unwrap();
    let amount = u64::from_be_bytes(amount_bytes);
    let amt = BitcoinAmount::from_sat(amount);

    // Extract descriptor length (4 bytes)
    let desc_len_bytes: [u8; 4] = aux_data[8..12].try_into().unwrap();
    let desc_len = u32::from_be_bytes(desc_len_bytes) as usize;

    // Check if we have enough data for the descriptor
    let expected_total_len = MIN_FAKEWITHDRAW_AUX_DATA_LEN + desc_len;
    if aux_data.len() < expected_total_len {
        return Err(DebugTxParseError::AuxDataTooShort {
            expected: expected_total_len,
            actual: aux_data.len(),
        });
    }

    // Extract and parse descriptor
    let desc_bytes = &aux_data[12..12 + desc_len];
    let dest = Descriptor::from_bytes(desc_bytes)
        .map_err(|e| DebugTxParseError::InvalidDescriptorFormat(e.to_string()))?;

    let withdraw_output = WithdrawOutput::new(dest, amt);
    Ok(ParsedDebugTx::FakeWithdrawIntent(withdraw_output))
}

#[cfg(test)]
mod tests {
    use strata_asm_logs::*;
    use strata_primitives::{bitcoin_bosd::Descriptor, l1::BitcoinAmount};

    use super::*;

    #[test]
    fn test_asm_log_type_serialization() {
        // Test with DepositLog
        let deposit_log = DepositLog::new(1, 100_000, b"test_address".to_vec());
        let log_type = AsmLogType::DepositLog(deposit_log);

        // Test Borsh serialization roundtrip
        let serialized = borsh::to_vec(&log_type).unwrap();
        let deserialized: AsmLogType = borsh::from_slice(&serialized).unwrap();

        // Verify the variant matches
        match (&log_type, &deserialized) {
            (AsmLogType::DepositLog(_), AsmLogType::DepositLog(_)) => {}
            _ => panic!("Deserialized log type variant mismatch"),
        }
    }

    #[test]
    fn test_withdraw_output_creation() {
        let withdraw_output = WithdrawOutput::new(
            Descriptor::new_p2wpkh(&[0x02; 20]),
            BitcoinAmount::from_sat(100_000),
        );

        assert_eq!(withdraw_output.amt, BitcoinAmount::from_sat(100_000));
    }
}
