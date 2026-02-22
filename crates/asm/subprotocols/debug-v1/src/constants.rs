use std::mem::size_of;

use strata_l1_txfmt::{SubprotocolId, TxType};

/// Debug subprotocol ID (set to u8::MAX to avoid production conflicts).
pub(crate) const DEBUG_SUBPROTOCOL_ID: SubprotocolId = u8::MAX;

/// Transaction type for mock ASM log injection.
pub(crate) const MOCK_ASM_LOG_TX_TYPE: TxType = 1;

/// Transaction type for mock withdrawal intent creation.
pub(crate) const MOCK_WITHDRAW_INTENT_TX_TYPE: TxType = 2;

// Auxiliary data parsing constants

/// Size of amount field in bytes.
pub(crate) const AMOUNT_SIZE: usize = 8;

/// Offset of amount field in auxiliary data.
pub(crate) const AMOUNT_OFFSET: usize = 0;

/// Offset of the operator index field (4-byte big-endian u32) in auxiliary data.
pub(crate) const OPERATOR_INDEX_OFFSET: usize = AMOUNT_OFFSET + AMOUNT_SIZE;

/// Size of the operator index field in bytes.
pub(crate) const OPERATOR_INDEX_SIZE: usize = size_of::<u32>();

/// Offset of the descriptor field in auxiliary data.
pub(crate) const DESCRIPTOR_OFFSET: usize = OPERATOR_INDEX_OFFSET + OPERATOR_INDEX_SIZE;

/// Minimum size of descriptor field in bytes.
///
/// See: <https://github.com/alpenlabs/bitcoin-bosd/blob/main/SPECIFICATION.md>
pub(crate) const MIN_DESCRIPTOR_SIZE: usize = 20;

/// Minimum auxiliary data length for mock withdrawal intent.
///
/// Format: `[amount: 8 bytes][selected_operator: 4 bytes][descriptor: variable]`
pub(crate) const MIN_MOCK_WITHDRAW_INTENT_AUX_DATA_LEN: usize =
    AMOUNT_SIZE + OPERATOR_INDEX_SIZE + MIN_DESCRIPTOR_SIZE;
