//! Strata orchestration layer block execution and validation library.

use strata_acct_types as _;
use strata_ol_state_types as _;

mod account_processing;
mod assembly;
mod chain_processing;
mod constants;
mod context;
mod da_reconstruction;
mod errors;
mod manifest_processing;
mod output;
mod proof_verification;
mod sau_processing;
mod transaction_processing;
mod verification;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;
#[cfg(test)]
mod tests;

pub use assembly::*;
pub use chain_processing::{process_block_start, process_epoch_initial};
pub use constants::*;
pub use context::{
    BasicExecContext, BlockContext, BlockInfo, EpochInfo, EpochInitialContext, TxExecContext,
};
pub use da_reconstruction::apply_da_epoch;
pub use errors::{ErrorKind, ExecError, ExecResult};
pub use manifest_processing::process_block_manifests;
pub use output::*;
pub use proof_verification::TxProofVerifierImpl;
pub use sau_processing::verify_snark_acct_update_proofs;
pub use transaction_processing::{
    check_tx_constraints, process_block_tx_segment, process_single_tx, verify_effects_safe,
};
pub use verification::*;
