//! Error types for OL MMR index reconciliation.

use std::io;

use strata_db_types::{DbError, MmrId};
use strata_ol_mmr_index::OLMmrIndexError;
use thiserror::Error;

/// Result type for OL MMR index reconciliation.
pub type OLMmrReconcileResult<T> = Result<T, OLMmrReconcileError>;

/// Errors returned while reconciling OL-owned MMR index namespaces.
#[derive(Debug, Error)]
pub enum OLMmrReconcileError {
    /// A persisted raw MMR id is not a valid known [`MmrId`].
    #[error("invalid raw MMR id: {raw_mmr_id}")]
    InvalidRawMmrId {
        /// Raw namespace key formatted as lowercase hex.
        raw_mmr_id: String,

        /// Decode error.
        source: io::Error,
    },

    /// The shared OL MMR index classifier rejected the plan.
    #[error(transparent)]
    InvalidIndex(#[from] OLMmrIndexError),

    /// A persisted ahead index does not contain the target as a prefix.
    #[error(
        "MMR {mmr_id} target OL state prefix not found in index at leaf count {target_leaf_count}"
    )]
    TargetPrefixNotInIndex {
        /// MMR namespace that failed the prefix check.
        mmr_id: MmrId,

        /// Target leaf count.
        target_leaf_count: u64,
    },

    /// The MMR leaf count does not match the target after truncation.
    #[error("MMR {mmr_id} post-truncate leaf count mismatch (target leaf count {target_leaf_count}, final leaf count {final_leaf_count})")]
    PostTruncateLeafCountMismatch {
        /// MMR namespace whose final leaf count mismatched.
        mmr_id: MmrId,

        /// Target leaf count.
        target_leaf_count: u64,

        /// Leaf count read after truncation.
        final_leaf_count: u64,
    },

    /// The MMR state does not match the target after truncation.
    #[error("MMR {mmr_id} post-truncate state mismatch at leaf count {leaf_count}")]
    PostTruncateStateMismatch {
        /// MMR namespace whose final state mismatched.
        mmr_id: MmrId,

        /// Leaf count where the state differs.
        leaf_count: u64,
    },

    /// A database operation failed.
    #[error("db: {0}")]
    Db(#[from] DbError),
}
