//! OL transaction mempool.
//!
//! Stores pending OL transactions (GenericAccountMessage and SnarkAccountUpdate
//! without accumulator proofs) before they are included in blocks.

mod command;
mod error;
#[cfg(test)]
mod test_utils;
mod types;

pub use command::MempoolCommand;
pub use error::OLMempoolError;
pub use types::{
    DEFAULT_MAX_MEMPOOL_BYTES, DEFAULT_MAX_TX_COUNT, DEFAULT_MAX_TX_SIZE, MempoolOrderingKey,
    MempoolTxRemovalReason, OLMempoolConfig, OLMempoolRejectCounts, OLMempoolRejectReason,
    OLMempoolSnarkAcctUpdateTxPayload, OLMempoolStats, OLMempoolTransaction, OLMempoolTxPayload,
};

pub type OLMempoolResult<T> = Result<T, OLMempoolError>;
