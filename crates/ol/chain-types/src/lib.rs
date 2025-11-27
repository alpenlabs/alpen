//! Orchestration layer blockchain structures.

mod block;
mod block_flags;
mod log;
mod log_payloads;
mod transaction;

// Re-export commitment types from identifiers
// Re-export AsmManifest from asm-common (canonical source)
pub use strata_asm_common::AsmManifest;
pub use strata_identifiers::{
    EpochCommitment, L1BlockCommitment, L1BlockId, OLBlockCommitment, OLBlockId, OLTxId,
};

/// SSZ-generated types for serialization and merkleization.
#[allow(
    clippy::all,
    unreachable_pub,
    clippy::allow_attributes,
    reason = "generated code"
)]
mod ssz_generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

pub use block_flags::*;
pub use log_payloads::*;
// Re-export generated SSZ types with their canonical names
pub use ssz_generated::ssz::{
    block::{
        Epoch, OLBlock, OLBlockBody, OLBlockHeader, OLBlockHeaderRef, OLBlockRef,
        OLL1ManifestContainer, OLL1Update, OLTxSegment, SignedOLBlockHeader,
        SignedOLBlockHeaderRef, Slot,
    },
    log::{OLLog, OLLogRef},
    transaction::{
        GamTxPayload, GamTxPayloadRef, GenericAccountMessage, OLTransaction, OLTransactionRef,
        SnarkAccountUpdate, SnarkAccountUpdateTxPayload, SnarkAccountUpdateTxPayloadRef,
        TransactionAttachment, TransactionAttachmentRef, TransactionPayload, TransactionPayloadRef,
    },
};
