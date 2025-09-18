//! Constants for checkpointing v0 subprotocol

use strata_asm_common::SubprotocolId;

/// Subprotocol ID for checkpointing v0
///
/// This ID should be unique and not conflict with other subprotocols.
/// Following the pattern from other subprotocols:
/// - Checkpointing V0: 0x00000001 (replaces core subprotocol)
/// - Bridge V1: 0x00000002
/// - Administration: 0x00000003
pub const CHECKPOINTING_V0_SUBPROTOCOL_ID: SubprotocolId = 1;

/// Transaction type for OL STF checkpoint transactions
///
/// This follows the SPS-50 transaction tagging system for identifying
/// checkpoint transactions within envelope transactions.
pub const OL_STF_CHECKPOINT_TX_TYPE: u8 = 0x01;

/// Default timeout for checkpoint verification (in blocks)
pub const DEFAULT_CHECKPOINT_TIMEOUT: u32 = 100;

/// Maximum number of epochs to keep in state for validation
pub const MAX_CACHED_EPOCHS: usize = 10;
