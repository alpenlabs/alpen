//! This crate holds commong evm changes shared between native and prover runtimes
//! and should not include any dependencies that cannot be run in the prover.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
mod config;
pub mod constants;
mod precompiles;
mod utils;

pub use config::set_evm_handles;
pub use utils::collect_withdrawal_intents;
