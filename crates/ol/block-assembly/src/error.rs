//! Error types for block assembly operations.

use strata_db_types::errors::DbError;
use strata_identifiers::OLBlockId;
use strata_ol_mempool::OLMempoolError;

/// Errors that can occur during block assembly operations.
#[derive(Debug, thiserror::Error)]
pub enum BlockAssemblyError {
    /// Database operation failed.
    #[error("db: {0}")]
    Database(#[from] DbError),

    /// Mempool operation failed.
    #[error("mempool: {0}")]
    Mempool(#[from] OLMempoolError),

    /// Invalid L1 block range where `from_block` height > `to_block` height.
    #[error("invalid L1 block height range (from {from_height} to {to_height})")]
    InvalidRange { from_height: u64, to_height: u64 },

    /// Unknown template ID (template not found in pending templates).
    #[error("unknown template id: {0}")]
    UnknownTemplateId(OLBlockId),

    /// Invalid signature for block template completion.
    #[error("invalid signature for template: {0}")]
    InvalidSignature(OLBlockId),

    /// Block timestamp is too early (violates minimum block time).
    #[error("block timestamp too early: {0}")]
    TimestampTooEarly(u64),

    /// Request channel closed (service shutdown).
    #[error("request channel closed")]
    RequestChannelClosed,

    /// Response channel closed (oneshot sender dropped).
    #[error("response channel closed")]
    ResponseChannelClosed,
}
