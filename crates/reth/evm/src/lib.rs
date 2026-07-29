//! This crate holds commong evm changes shared between native and prover runtimes
//! and should not include any dependencies that cannot be run in the prover.
pub mod constants;
mod utils;

pub use alpen_reth_primitives::SubjectTransferIntent;
pub use utils::{
    accumulate_logs_bloom, address_to_subject, extract_subject_transfer_intents,
    extract_withdrawal_intents, subject_to_address, subject_to_address_unchecked,
    SubjectTransferIntentExtractionError, WithdrawalIntentExtractionError,
};

pub mod apis;
pub mod evm;
pub mod precompiles;
