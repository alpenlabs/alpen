use strata_db_types::DbError;
use strata_identifiers::{Epoch, OLBlockCommitment, OLBlockId, Slot};
use thiserror::Error;

/// Failures while following a block's parent chain.
#[derive(Debug, Error)]
pub enum BlockAncestryError {
    #[error("block ancestry read failed: {0}")]
    Db(#[from] DbError),

    #[error("missing ancestor header for block {0}")]
    MissingHeader(OLBlockId),

    #[error("ancestor block ID mismatch (expected {expected}, got {actual})")]
    BlockIdMismatch {
        expected: OLBlockId,
        actual: OLBlockId,
    },

    #[error("unexpected ancestor block at history base (expected {expected}, got {actual})")]
    UnexpectedAncestor {
        expected: OLBlockId,
        actual: OLBlockId,
    },

    #[error("epoch-zero block {0} has no previous epoch")]
    NoPreviousEpoch(OLBlockCommitment),

    #[error("parent slot {parent_slot} is not below child slot {child_slot} at block {block}")]
    ParentSlotNotDescending {
        block: OLBlockCommitment,
        parent_slot: Slot,
        child_slot: Slot,
    },

    #[error("unexpected ancestor epoch at block {block} (expected {expected}, got {actual})")]
    UnexpectedEpoch {
        block: OLBlockCommitment,
        expected: Epoch,
        actual: Epoch,
    },
}
