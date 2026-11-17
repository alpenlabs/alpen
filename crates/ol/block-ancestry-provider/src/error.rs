//! Errors returned while resolving parent-linked OL block ancestry.

use strata_db_types::DbError;
use strata_identifiers::{Epoch, EpochCommitment, OLBlockCommitment, OLBlockId, Slot};
use thiserror::Error;

/// Failures while following a block's parent chain.
#[derive(Debug, Error)]
pub enum BlockAncestryError {
    /// Reading ancestry data failed, or the blocking task performing the read failed.
    #[error("block ancestry read failed: {0}")]
    Db(#[from] DbError),

    /// The terminal header for a requested epoch commitment is not stored.
    #[error("missing epoch terminal header for {0}")]
    MissingEpochTerminal(EpochCommitment),

    /// A loaded epoch terminal header does not compute to the requested epoch commitment.
    #[error("epoch terminal mismatch (expected {expected}, got {actual})")]
    EpochTerminalMismatch {
        expected: EpochCommitment,
        actual: EpochCommitment,
    },

    /// A requested epoch commitment resolves to a header without the terminal flag.
    #[error("epoch terminal header is not terminal for {0}")]
    MissingTerminalFlag(EpochCommitment),

    /// Collected ancestry ends at a different preceding terminal than the caller requires.
    #[error("previous terminal mismatch (expected {expected}, got {actual})")]
    PreviousTerminalMismatch {
        expected: OLBlockCommitment,
        actual: OLBlockCommitment,
    },

    /// A parent link names a header that is not stored.
    #[error("missing ancestor header for block {0}")]
    MissingHeader(OLBlockId),

    /// A header stored under a parent link does not hash to that link's block ID.
    #[error("ancestor block ID mismatch (expected {expected}, got {actual})")]
    BlockIdMismatch {
        expected: OLBlockId,
        actual: OLBlockId,
    },

    /// The walk reaches the history-base slot through a block other than the history base.
    #[error("unexpected ancestor block at history base (expected {expected}, got {actual})")]
    UnexpectedAncestor {
        expected: OLBlockId,
        actual: OLBlockId,
    },

    /// A previous-epoch lookup starts from epoch zero, which has no predecessor.
    #[error("cannot resolve a previous epoch from epoch-zero block {0}")]
    EpochZeroHasNoPreviousEpoch(OLBlockCommitment),

    /// A parent link does not point to the slot immediately before its child.
    #[error("parent slot {parent_slot} is not immediately before child block {child}")]
    ParentSlotMismatch {
        child: OLBlockCommitment,
        parent_slot: Slot,
    },

    /// A nonterminal ancestor is outside the start's epoch, or the first terminal ancestor is
    /// outside the preceding epoch.
    #[error("unexpected ancestor epoch at block {block} (expected {expected}, got {actual})")]
    UnexpectedEpoch {
        block: OLBlockCommitment,
        expected: Epoch,
        actual: Epoch,
    },
}
