use strata_consensus_logic::errors::ChainTipError;
use strata_db_types::DbError;
use strata_identifiers::OLBlockId;

use crate::ClientError;

#[derive(Debug, thiserror::Error)]
pub enum OLSyncError {
    #[error("no block finalized yet")]
    NotFinalizing,

    #[error("block not found: {0}")]
    MissingBlock(OLBlockId),

    #[error("wrong fork: {0} at height {1}")]
    WrongFork(OLBlockId, u64),

    #[error("missing parent block: {0}")]
    MissingParent(OLBlockId),

    #[error("missing finalized block: {0}")]
    MissingFinalized(OLBlockId),

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
