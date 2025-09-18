//! Constants for checkpointing v0 subprotocol

use strata_asm_common::SubprotocolId;

/// Subprotocol ID for checkpointing v0
///
/// This ID should be unique and not conflict with other subprotocols.
pub const CHECKPOINTING_V0_SUBPROTOCOL_ID: SubprotocolId = 1;

/// Transaction type for OL STF checkpoint transactions
///
/// This follows the SPS-50 transaction tagging system for identifying
/// checkpoint transactions within envelope transactions.
pub const OL_STF_CHECKPOINT_TX_TYPE: u8 = 1;
