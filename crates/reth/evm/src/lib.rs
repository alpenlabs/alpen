//! This crate holds commong evm changes shared between native and prover runtimes
//! and should not include any dependencies that cannot be run in the prover.
pub mod constants;
mod utils;

pub use evm::StrataEvmPrecompiles;
pub use utils::collect_withdrawal_intents;

pub mod evm;
mod precompiles;
