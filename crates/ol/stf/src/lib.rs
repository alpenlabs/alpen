// TODO

#![allow(unused, reason = "in development")]

use strata_acct_types as _;
use strata_ol_state_types as _;

mod account_processing;
mod assembly;
mod chain_processing;
mod constants;
mod context;
mod errors;
mod manifest_processing;
mod output;
mod transaction_processing;
mod verification;

#[cfg(test)]
mod test_utils;
#[cfg(test)]
mod tests;

pub use assembly::*;
pub use chain_processing::{process_block_start, process_epoch_initial};
pub use constants::*;
pub use errors::{ErrorKind, ExecError, ExecResult};
pub use manifest_processing::process_block_manifests;
pub use output::*;
pub use transaction_processing::{process_block_tx_segment, process_single_tx};
pub use verification::*;
