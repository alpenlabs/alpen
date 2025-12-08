//! OL transaction mempool.
//!
//! Stores pending OL transactions (GenericAccountMessage and SnarkAccountUpdate
//! without accumulator proofs) before they are included in blocks.

mod command;
mod error;
mod ordering;
#[cfg(test)]
mod test_utils;
mod types;
mod validation;

pub use command::MempoolCommand;
pub use error::OLMempoolError;
pub use types::{
    DEFAULT_MAX_TX_COUNT, DEFAULT_MAX_TX_SIZE, MempoolOrderingKey, OLMempoolConfig,
    OLMempoolRejectCounts, OLMempoolRejectReason, OLMempoolSnarkAcctUpdateTxPayload,
    OLMempoolStats, OLMempoolTransaction, OLMempoolTxPayload,
};
pub use validation::{BasicTransactionValidator, TransactionValidator};

pub type OLMempoolResult<T> = Result<T, OLMempoolError>;
