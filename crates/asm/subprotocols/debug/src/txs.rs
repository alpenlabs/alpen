//! Transaction parsing and handling for the debug subprotocol.
//!
//! This module implements parsing for debug transaction types that allow
//! injection of test data into the ASM for testing purposes.

use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::TxInputRef;
use strata_l1_txfmt::TxType;
use strata_primitives::{bitcoin_bosd::Descriptor, l1::BitcoinAmount};
use thiserror::Error;

use crate::constants::{FAKEWITHDRAW_TX_TYPE, OLMSG_TX_TYPE, UNLOCKDEPOSIT_TX_TYPE};

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
}

/// Information extracted from an OL message injection transaction.
///
/// This allows injection of arbitrary log messages into the ASM,
/// simulating logs that would normally come from other subprotocols.
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct OlMsgInfo {
    /// The type ID of the log message to emit.
    pub type_id: u32,

    /// The serialized payload of the log message.
    pub payload: Vec<u8>,
}

/// Information extracted from a fake withdrawal transaction.
///
/// This allows creation of withdrawal commands that are sent to
/// the bridge subprotocol, simulating withdrawals from the OL.
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct FakeWithdrawInfo {
    /// The amount to withdraw (in satoshis).
    pub amt: BitcoinAmount,

    /// The Bitcoin destination descriptor.
    pub dest: Descriptor,
}

/// Information extracted from an unlock deposit transaction.
///
/// This allows direct emission of deposit unlock authorization signals
/// for testing deposit unlock flows.
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct UnlockDepositInfo {
    /// The deposit ID to unlock.
    pub deposit_id: u64,
}

/// Parsed debug transaction types.
pub(crate) enum ParsedDebugTx {
    /// OL message injection transaction.
    OlMsg(OlMsgInfo),

    /// Fake withdrawal creation transaction.
    FakeWithdraw(FakeWithdrawInfo),

    /// Unlock deposit transaction.
    UnlockDeposit(UnlockDepositInfo),
}

/// Minimum auxiliary data length for OL message transactions.
///
/// Format: [type_id: 4 bytes][payload: variable]
const MIN_OLMSG_AUX_DATA_LEN: usize = 4;

/// Minimum auxiliary data length for fake withdrawal transactions.
///
/// Format: [amount: 8 bytes][desc_len: 4 bytes][descriptor: variable]
const MIN_FAKEWITHDRAW_AUX_DATA_LEN: usize = 12;

/// Auxiliary data length for unlock deposit transactions.
///
/// Format: [deposit_id: 8 bytes]
const UNLOCKDEPOSIT_AUX_DATA_LEN: usize = 8;

/// Parses a debug transaction from the given transaction input.
///
/// This function examines the transaction type and delegates to the appropriate
/// parsing function based on the type.
pub(crate) fn parse_debug_tx(tx: &TxInputRef<'_>) -> Result<ParsedDebugTx, DebugTxParseError> {
    match tx.tag().tx_type() {
        OLMSG_TX_TYPE => parse_olmsg_tx(tx),
        FAKEWITHDRAW_TX_TYPE => parse_fakewithdraw_tx(tx),
        UNLOCKDEPOSIT_TX_TYPE => parse_unlockdeposit_tx(tx),
        tx_type => Err(DebugTxParseError::UnsupportedTxType(tx_type)),
    }
}

/// Parses an OL message injection transaction.
///
/// Auxiliary data format:
/// - `[type_id: 4 bytes]` - The log type identifier
/// - `[payload: variable]` - The log payload
fn parse_olmsg_tx(tx: &TxInputRef<'_>) -> Result<ParsedDebugTx, DebugTxParseError> {
    let aux_data = tx.tag().aux_data();

    if aux_data.len() < MIN_OLMSG_AUX_DATA_LEN {
        return Err(DebugTxParseError::AuxDataTooShort {
            expected: MIN_OLMSG_AUX_DATA_LEN,
            actual: aux_data.len(),
        });
    }

    // Extract type_id (4 bytes)
    let type_id = u32::from_be_bytes([aux_data[0], aux_data[1], aux_data[2], aux_data[3]]);

    // Extract payload (remaining bytes)
    let payload = aux_data[4..].to_vec();

    Ok(ParsedDebugTx::OlMsg(OlMsgInfo { type_id, payload }))
}

/// Parses a fake withdrawal transaction.
///
/// Auxiliary data format:
/// - `[amount: 8 bytes]` - The withdrawal amount in satoshis
/// - `[desc_len: 4 bytes]` - The length of the descriptor string
/// - `[descriptor: desc_len bytes]` - The Bitcoin descriptor string
fn parse_fakewithdraw_tx(tx: &TxInputRef<'_>) -> Result<ParsedDebugTx, DebugTxParseError> {
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

    Ok(ParsedDebugTx::FakeWithdraw(FakeWithdrawInfo { amt, dest }))
}

/// Parses an unlock deposit transaction.
///
/// Auxiliary data format:
/// - `[deposit_id: 8 bytes]` - The deposit ID to unlock
fn parse_unlockdeposit_tx(tx: &TxInputRef<'_>) -> Result<ParsedDebugTx, DebugTxParseError> {
    let aux_data = tx.tag().aux_data();

    if aux_data.len() < UNLOCKDEPOSIT_AUX_DATA_LEN {
        return Err(DebugTxParseError::AuxDataTooShort {
            expected: UNLOCKDEPOSIT_AUX_DATA_LEN,
            actual: aux_data.len(),
        });
    }

    // Extract deposit_id (8 bytes)
    let deposit_id_bytes: [u8; 8] = aux_data[0..8].try_into().unwrap();
    let deposit_id = u64::from_be_bytes(deposit_id_bytes);

    Ok(ParsedDebugTx::UnlockDeposit(UnlockDepositInfo {
        deposit_id,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_olmsg_info_serialization() {
        let info = OlMsgInfo {
            type_id: 42,
            payload: b"test payload".to_vec(),
        };

        // Test Borsh serialization roundtrip
        let serialized = borsh::to_vec(&info).unwrap();
        let deserialized: OlMsgInfo = borsh::from_slice(&serialized).unwrap();

        assert_eq!(info.type_id, deserialized.type_id);
        assert_eq!(info.payload, deserialized.payload);
    }

    #[test]
    fn test_fakewithdraw_info_serialization() {
        let info = FakeWithdrawInfo {
            amt: BitcoinAmount::from_sat(100_000),
            dest: Descriptor::new_p2wpkh(&[0x02; 20]),
        };

        // Test Borsh serialization roundtrip
        let serialized = borsh::to_vec(&info).unwrap();
        let deserialized: FakeWithdrawInfo = borsh::from_slice(&serialized).unwrap();

        assert_eq!(info.amt, deserialized.amt);
        // Note: Descriptor equality would need proper implementation
    }

    #[test]
    fn test_unlockdeposit_info_serialization() {
        let info = UnlockDepositInfo { deposit_id: 12345 };

        // Test Borsh serialization roundtrip
        let serialized = borsh::to_vec(&info).unwrap();
        let deserialized: UnlockDepositInfo = borsh::from_slice(&serialized).unwrap();

        assert_eq!(info.deposit_id, deserialized.deposit_id);
    }
}
