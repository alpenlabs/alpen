//! Helpers for constructing and parsing SPS-50 checkpoint transactions.
// TODO: (@PG) Remove V0 version of parser and constants

mod constants;
mod errors;
mod parser_v0;

pub use constants::{CHECKPOINT_V0_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE};
pub use errors::{CheckpointTxError, CheckpointTxResult};
pub use parser_v0::{extract_signed_checkpoint_from_envelope, extract_withdrawal_messages};

pub mod parser;
