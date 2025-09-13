use strata_asm_common::SubprotocolId;
use strata_l1_txfmt::TxType;

/// Unique identifier for the Administration Subprotocol.
pub const ADMINISTRATION_SUBPROTOCOL_ID: SubprotocolId = 0;

// ─── Administration Subprotocol Transaction Types ──────────────────────────────────────────────

/// Transaction type that signals the cancellation of a previously queued update.
pub const CANCEL_TX_TYPE: TxType = 0;

/// Transaction type that proposes an update to the on-chain multisignature configuration.
pub const MULTISIG_CONFIG_UPDATE_TX_TYPE: TxType = 10;

/// Transaction type that proposes an update to the set of authorized operators.
pub const OPERATOR_UPDATE_TX_TYPE: TxType = 11;

/// Transaction type that proposes an update to the sequencer configuration.
pub const SEQUENCER_UPDATE_TX_TYPE: TxType = 12;

/// Transaction type that proposes an update to the verifying key for the OL STF.
pub const OL_STF_VK_UPDATE_TX_TYPE: TxType = 13;

/// Transaction type that proposes an update to the verifying key for the ASM STF.
pub const ASM_STF_VK_UPDATE_TX_TYPE: TxType = 14;
