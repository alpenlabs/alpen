//! OL transaction mempool.
//!
//! Stores pending OL transactions (GenericAccountMessage and SnarkAccountUpdate
//! without accumulator proofs) before they are included in blocks.

mod command;
mod error;
mod handle;
mod ordering;
mod service;
mod state;
#[cfg(test)]
mod test_utils;
mod types;
mod validation;

pub use command::MempoolCommand;
pub use error::OLMempoolError;
pub use handle::MempoolHandle;
pub use service::MempoolServiceStatus;
pub use types::{
    DEFAULT_COMMAND_BUFFER_SIZE, DEFAULT_MAX_REORG_DEPTH, DEFAULT_MAX_TX_COUNT,
    DEFAULT_MAX_TX_SIZE, MempoolOrderingKey, OLMempoolConfig, OLMempoolRejectCounts,
    OLMempoolRejectReason, OLMempoolSnarkAcctUpdateTxPayload, OLMempoolStats, OLMempoolTransaction,
    OLMempoolTxPayload,
};
pub use validation::{BasicTransactionValidator, TransactionValidator};

pub type OLMempoolResult<T> = Result<T, OLMempoolError>;
