// TODO

#![allow(unused, reason = "in development")]

use strata_acct_types as _;

mod account_processing;
mod chain_processing;
mod constants;
mod context;
mod errors;
mod manifest_processing;
mod transaction_processing;

pub use chain_processing::process_epoch_initial;
pub use constants::*;
pub use errors::{ErrorKind, ExecError, ExecResult};
pub use manifest_processing::process_block_l1_update;
pub use transaction_processing::{process_block_tx_segment, process_single_tx};
