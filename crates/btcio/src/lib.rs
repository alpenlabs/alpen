//! Input-output with Bitcoin, implementing L1 chain trait.

pub mod broadcaster;
pub mod params;
pub mod reader;
pub(crate) mod rpc_error;
pub mod status;

#[cfg(test)]
pub mod test_utils;
pub mod writer;

pub use params::BtcioParams;
pub use rpc_error::{is_bitcoind_warmup_error, is_block_height_out_of_range_error};
