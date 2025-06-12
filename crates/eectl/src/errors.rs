use strata_state::id::L2BlockId;
use thiserror::Error;

pub type EngineResult<T> = Result<T, EngineError>;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("unknown payload ID {0}")]
    UnknownPayloadId(u64),

    #[error("amount conversion sats: {0}")]
    AmountConversion(u64),

    #[error("invalid address {0:?}")]
    InvalidAddress(Vec<u8>),

    #[error("missing block in db {0}")]
    DbMissingBlock(L2BlockId),

    #[error("not yet implemented")]
    Unimplemented,

    #[error("{0}")]
    Other(String),
}
