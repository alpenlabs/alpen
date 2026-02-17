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

/// Offset of the operator-length byte (`B`) in auxiliary data.
pub(crate) const OPERATOR_LEN_OFFSET: usize = AMOUNT_OFFSET + AMOUNT_SIZE;

/// Maximum number of bytes used to encode the operator index.
/// Operator index is a u32, so at most 4 bytes.
pub(crate) const MAX_OPERATOR_INDEX_LEN: usize = 4;

/// Minimum size of descriptor field in bytes.
///
/// See: <https://github.com/alpenlabs/bitcoin-bosd/blob/main/SPECIFICATION.md>
pub(crate) const MIN_DESCRIPTOR_SIZE: usize = 20;

/// Minimum auxiliary data length for mock withdrawal intent.
///
/// Format: `[amount: 8 bytes][1 byte B][B bytes: operator index][descriptor: variable]`
/// Minimum case is B=0 (no operator selection).
pub(crate) const MIN_MOCK_WITHDRAW_INTENT_AUX_DATA_LEN: usize =
    AMOUNT_SIZE + 1 + MIN_DESCRIPTOR_SIZE;
