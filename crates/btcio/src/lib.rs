//! Input-output with Bitcoin, implementing L1 chain trait.

pub mod broadcaster;
pub mod params;
pub mod reader;
pub(crate) mod rpc_error;
mod serialized_client;
pub mod status;

#[cfg(test)]
pub mod test_utils;
pub mod writer;

pub use params::BtcioParams;
pub use serialized_client::SerializedBitcoinClient;
