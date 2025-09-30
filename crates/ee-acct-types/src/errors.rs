use thiserror::Error;

#[derive(Debug, Error)]
pub enum EnvError {
    /// An issue decoding a structure.
    #[error("decoding structure")]
    Decode,

    /// Malformed extra data.
    #[error("malformed extra data")]
    MalformedExtraData,

    /// Extra coinputs were provided than needed.
    #[error("mismatched coinput count")]
    MismatchedCoinputCnt,

    /// Coinput could not be parsed.
    #[error("coinput invalid for msg")]
    MalformedCoinput,

    /// Coinput could not be parsed.
    #[error("coinput invalid for message (idx {0})")]
    MalformedCoinputIdx(usize),

    /// Coinput was parsed but did not match msg.
    #[error("coinput did not correspond to msg")]
    MismatchedCoinput,

    /// Coinput was parsed but did not match msg.
    #[error("coinput did not correspond to msg (idx {0})")]
    MismatchedCoinputIdx(usize),

    /// Coinput was parsed and seemed to match msg, but was internally malformed.
    #[error("coinput is internally inconsistent")]
    InconsistentCoinput,

    /// Coinput was parsed and seemed to match msg, but was internally malformed.
    #[error("coinput is internally inconsistent (idx {0})")]
    InconsistentCoinputIdx(usize),

    /// Chain segment provided for EE verification was malformed.
    #[error("provided chain segment malformed")]
    MalformedChainSegment,

    /// Chain segment provided for EE verification does not match pending commits.
    #[error("tried to consume an unexpected chain segment")]
    MismatchedChainSegment,

    /// Tried to verify a chain segment without a waiting commit.
    #[error("tried to consume a chain segment that was not provided")]
    UncommittedChainSegment,

    /// Some computation did not match public state we are constrained by.
    #[error("conflict with external public state")]
    ConflictingPublicState,

    /// If the header or state provided to start verification off with does not
    /// match.
    #[error("mismatched data in current state and whatever")]
    MismatchedCurStateData,

    /// There were some unsatisfied obligations left to deal with in the update
    /// verification state.
    #[error("unsatisfied '{0}' verification obligations")]
    UnsatisfiedObligations(&'static str),
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
