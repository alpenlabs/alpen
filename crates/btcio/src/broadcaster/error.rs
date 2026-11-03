use bitcoin::consensus::encode::Error as ConsensusEncodeError;
use bitcoind_async_client::error::ClientError;
use strata_db_types::{common::L1TxId, errors::DbError};
use thiserror::Error;

use crate::rpc_error::is_retryable_client_error;

#[derive(Debug, Error)]
pub enum BroadcasterError {
    #[error("db: {0}")]
    Db(#[from] DbError),

    #[error("rpc: {0}")]
    Rpc(#[from] ClientError),

    #[error("Bitcoin RPC returned invalid blockheight {0}")]
    InvalidBlockHeight(i64),

    #[error("missing transaction entry index for txid {0}")]
    MissingEntryIndex(L1TxId),

    #[error("transaction not found in db at index {0}")]
    TxNotFound(u64),

    #[error("inconsistent next idx (expected {expected}, got {got})")]
    InconsistentNextIdx { expected: u64, got: u64 },

    #[error("invalid serialized Bitcoin transaction: {0}")]
    InvalidTransaction(#[from] ConsensusEncodeError),

    #[error("replacement chain from {txid} exceeded {max_hops} hops")]
    ReplacementChainTooLong { txid: L1TxId, max_hops: usize },
}

impl BroadcasterError {
    /// Reports whether the dependency failure may be resolved by retrying later.
    pub(crate) fn is_retryable(&self) -> bool {
        match self {
            Self::Db(error) => error.is_retryable(),
            Self::Rpc(error) => is_retryable_client_error(error),
            _ => false,
        }
    }
}

pub(crate) type BroadcasterResult<T> = Result<T, BroadcasterError>;
