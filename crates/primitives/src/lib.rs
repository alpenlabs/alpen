//! Collection of generic internal data types that are used widely.

// TODO import address types
// TODO import generic account types

// Suppress unused crate dependencies warnings
#[cfg(not(test))]
use bincode as _;
#[cfg(not(test))]
use num_enum as _;
#[cfg(not(test))]
use strata_crypto as _;
#[cfg(not(test))]
use strata_l1_txfmt as _;

#[macro_use]
mod macros;

pub mod block_credential;
pub mod bridge;
pub mod buf;
pub mod constants;
pub mod epoch;
pub mod errors;
pub mod evm_exec;
pub mod hash;
pub mod indexed;
pub mod keys;
pub mod l1;
pub mod l2;
pub mod operator;
pub mod prelude;
pub mod proof;
pub mod relay;
pub mod roles;
pub mod serde_helpers;
pub mod sorted_vec;
pub mod utils;

pub use bitcoin_bosd;
