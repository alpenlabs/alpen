use thiserror::Error;

#[derive(Debug, Error)]
pub enum EnvError {
    #[error("decoding structure")]
    Decode,

    #[error("extra coinputs provided")]
    ExtraCoinputs,

    #[error("coinput invalid for msg")]
    MalformedCoinput,

    #[error("coinput exactly did not match msg")]
    MismatchedCoinput,

    #[error("coinput is internally inconsistent")]
    InconsistentCoinput,

    #[error("provided chain segment malformed")]
    MalformedChainSegment,
}

pub type EnvResult<T> = Result<T, EnvError>;

#[derive(Debug, Error)]
pub enum MessageDecodeError {
    /// Message not formatted like a message, so we ignore it.
    #[error("invalid message format")]
    InvalidFormat,

    /// We recognize the message type, but its body is malformed, so we should
    /// ignore it.
    #[error("failed to decode message body")]
    InvalidBody,

    /// We don't support this message type, we can ignore it.
    #[error("unknown message type {0:#x}")]
    UnsupportedType(u16),
}

pub type MessageDecodeResult<T> = Result<T, MessageDecodeError>;
