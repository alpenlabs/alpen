use thiserror::Error;

#[derive(Debug, Clone, Error)]
pub enum DepositParseError {
    #[error("invalid data")]
    InvalidData,

    #[error("missing tag")]
    MissingTag,

    /// What is this used for?
    #[error("no data")]
    NoData,

    #[error("wrong subprotocol")]
    WrongSubprotocol,

    #[error("wrong tx type")]
    WrongTxType,

    /// We don't accept nonstandard deposit things.
    #[error("tag too large")]
    TagOversized,

    #[error("missing magic bytes")]
    MissingMagic,

    #[error("invalid magic bytes")]
    InvalidMagic,

    #[error("missing destination")]
    MissingDest,

    #[error("invalid destination length {0}")]
    InvalidDestLen(u8),

    #[error("expected 32 byte leaf Hash")]
    LeafHashLenMismatch,
}
