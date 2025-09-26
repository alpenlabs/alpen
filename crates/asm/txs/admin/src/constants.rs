use strata_asm_common::SubprotocolId;
use strata_l1_txfmt::TxType;

/// Unique identifier for the Administration Subprotocol.
pub const ADMINISTRATION_SUBPROTOCOL_ID: SubprotocolId = 0;

// ─── Administration Subprotocol Transaction Types ──────────────────────────────────────────────

/// Transaction type that signals the cancellation of a previously queued update.
pub const CANCEL_TX_TYPE: TxType = 0;

/// Transaction type that proposes an update to the strata admin multisignature configuration.
pub const STRATA_ADMIN_MULTISIG_UPDATE_TX_TYPE: TxType = 10;

/// Transaction type that proposes an update to the strata seq manager multisignature configuration.
pub const STRATA_SEQ_MANAGER_MULTISIG_UPDATE_TX_TYPE: TxType = 11;

/// Transaction type that proposes an update to the set of authorized operators.
pub const OPERATOR_UPDATE_TX_TYPE: TxType = 20;

/// Transaction type that proposes an update to the sequencer configuration.
pub const SEQUENCER_UPDATE_TX_TYPE: TxType = 21;

/// Transaction type that proposes an update to the verifying key for the OL STF.
pub const OL_STF_VK_UPDATE_TX_TYPE: TxType = 30;

/// Transaction type that proposes an update to the verifying key for the ASM STF.
pub const ASM_STF_VK_UPDATE_TX_TYPE: TxType = 31;
