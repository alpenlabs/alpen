//! Strata orchestration layer block execution and validation library.

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
mod proof_verification;
mod transaction_processing;
mod verification;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;
#[cfg(test)]
mod tests;

pub use assembly::*;
pub use chain_processing::{process_block_start, process_epoch_initial};
pub use constants::*;
pub use context::{BasicExecContext, BlockContext, BlockInfo, EpochInfo, TxExecContext};
pub use errors::{ErrorKind, ExecError, ExecResult};
pub use manifest_processing::process_block_manifests;
pub use output::*;
pub use proof_verification::TxProofVerifierImpl;
pub use transaction_processing::{
    check_tx_constraints, process_block_tx_segment, process_single_tx,
};
pub use verification::*;
