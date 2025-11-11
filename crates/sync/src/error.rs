use strata_consensus_logic::errors::ChainTipError;
use strata_db_types::DbError;
use strata_ol_chain_types::L2BlockId;

use crate::ClientError;

#[derive(Debug, thiserror::Error)]
pub enum L2SyncError {
    #[error("no block finalized yet")]
    NotFinalizing,

    #[error("block not found: {0}")]
    MissingBlock(L2BlockId),

    #[error("wrong fork: {0} at height {1}")]
    WrongFork(L2BlockId, u64),

    #[error("missing parent block: {0}")]
    MissingParent(L2BlockId),

    #[error("missing finalized block: {0}")]
    MissingFinalized(L2BlockId),

    // TODO make this not a string
    #[error("loading unfinalized blocks: {0}")]
    LoadUnfinalizedFailed(String),

    #[error("channel closed")]
    ChannelClosed,

    #[error("client: {0}")]
    Client(#[from] ClientError),

    #[error("db: {0}")]
    Db(#[from] DbError),

    #[error("chain tip: {0}")]
    ChainTip(#[from] ChainTipError),
}
