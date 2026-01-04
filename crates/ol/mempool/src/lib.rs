//! OL transaction mempool.
//!
//! Stores pending OL transactions (GenericAccountMessage and SnarkAccountUpdate
//! without accumulator proofs) before they are included in blocks.

mod builder;
mod command;
mod error;
mod handle;
mod service;
mod state;
#[cfg(test)]
mod test_utils;
mod types;
mod validation;

pub use builder::MempoolBuilder;
pub use command::MempoolCommand;
pub use error::OLMempoolError;
pub use handle::MempoolHandle;
pub use service::MempoolServiceStatus;
pub use types::{
    DEFAULT_COMMAND_BUFFER_SIZE, DEFAULT_MAX_MEMPOOL_BYTES, DEFAULT_MAX_REORG_DEPTH,
    DEFAULT_MAX_TX_COUNT, DEFAULT_MAX_TX_SIZE, MempoolOrderingKey, MempoolTxRemovalReason,
    OLMempoolConfig, OLMempoolRejectCounts, OLMempoolRejectReason,
    OLMempoolSnarkAcctUpdateTxPayload, OLMempoolStats, OLMempoolTransaction, OLMempoolTxPayload,
};

pub type OLMempoolResult<T> = Result<T, OLMempoolError>;
