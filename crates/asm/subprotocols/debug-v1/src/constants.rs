use strata_l1_txfmt::{SubprotocolId, TxType};

/// Debug subprotocol ID (set to u8::MAX to avoid production conflicts).
pub(crate) const DEBUG_SUBPROTOCOL_ID: SubprotocolId = u8::MAX;

/// Transaction type for fake ASM log injection.
pub(crate) const FAKE_ASM_LOG_TX_TYPE: TxType = 1;

/// Transaction type for fake withdrawal intent creation.
pub(crate) const FAKE_WITHDRAW_INTENT_TX_TYPE: TxType = 2;

// Auxiliary data parsing constants

/// Minimum auxiliary data length for ASM log transactions.
///
/// Format: `[serialized AsmLogType]`
/// Based on empirical testing, the smallest serializable AsmLogType is 11 bytes.
pub(crate) const MIN_ASM_LOG_AUX_DATA_LEN: usize = 11;

/// Size of amount field in bytes.
pub(crate) const AMOUNT_SIZE: usize = 8;

/// Offset of amount field in auxiliary data.
pub(crate) const AMOUNT_OFFSET: usize = 0;

/// Offset of descriptor field in auxiliary data.
pub(crate) const DESCRIPTOR_OFFSET: usize = AMOUNT_OFFSET + AMOUNT_SIZE;

/// Minimum auxiliary data length for fake withdrawal transactions.
///
/// Format: `[amount: 8 bytes][descriptor: variable]`
pub(crate) const MIN_FAKEWITHDRAW_AUX_DATA_LEN: usize = AMOUNT_SIZE + 20; // +20 for minimum descriptor size
