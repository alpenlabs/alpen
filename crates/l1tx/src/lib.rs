//! L1 transaction processing.

pub mod deposit;
pub mod filter;
pub mod messages;
pub mod utils;

pub const BRIDGE_V1_SUBPROTOCOL_ID_LEN: usize = 1;
pub const TX_TYPE_LEN: usize = 1;

pub use filter::types::TxFilterConfig;
