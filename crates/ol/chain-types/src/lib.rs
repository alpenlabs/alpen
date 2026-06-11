//! Orchestration layer blockchain structures.

mod block;
mod block_flags;
mod error;
mod log;
mod log_payloads;
mod proofs;
mod transaction;
mod validation;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

pub use error::ChainTypesError;
// Re-export AsmManifest from asm-common (canonical source)
pub use strata_asm_common::AsmManifest;
// Re-export the canonical OL log-payload types from the checkpoint subprotocol crate, which is
// the source of truth for the types shared with checkpoint verification.
pub use strata_asm_proto_checkpoint_types::{
    OLLogDecodeError, OLLogType, SIMPLE_WITHDRAWAL_INTENT_LOG_TYPE_ID,
    SimpleWithdrawalIntentLogData,
};
// Re-export commitment types from identifiers
pub use strata_identifiers::{
    Epoch, EpochCommitment, L1BlockCommitment, L1BlockId, OLBlockCommitment, OLBlockId, OLTxId,
    Slot,
};

/// SSZ-generated types for serialization and merkleization.
#[allow(
    clippy::all,
    unreachable_pub,
    clippy::allow_attributes,
    clippy::absolute_paths,
    reason = "generated code"
)]
mod ssz_generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

pub use block_flags::*;
pub use log_payloads::*;
// Re-export generated SSZ types with their canonical names
pub use ssz_generated::ssz::{block::*, log::*, proofs::*, transaction::*};
pub use validation::*;
