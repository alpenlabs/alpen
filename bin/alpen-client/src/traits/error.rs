use strata_identifiers::OLBlockId;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum OlClientError {
    #[error("invalid slot range: end_slot ({end_slot}) must be greater than start_slot ({start_slot})")]
    InvalidSlotRange { start_slot: u64, end_slot: u64 },

    #[error("unexpected block count: expected {expected} blocks, got {actual}")]
    UnexpectedBlockCount { expected: usize, actual: usize },

    #[error("unexpected operation count: expected {expected} operation lists, got {actual}")]
    UnexpectedOperationCount { expected: usize, actual: usize },

    #[error("network error: {0}")]
    Network(String),

    #[error("rpc error: {0}")]
    Rpc(String),

    #[error(transparent)]
    Other(#[from] eyre::Error),
}

impl OlClientError {
    pub(crate) fn network(msg: impl Into<String>) -> Self {
        Self::Network(msg.into())
    }

    pub(crate) fn rpc(msg: impl Into<String>) -> Self {
        Self::Rpc(msg.into())
    }
}
