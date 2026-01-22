use thiserror::Error;

#[derive(Debug, Error)]
pub enum EnvError {
    /// Codec error during encoding or decoding.
    #[error("codec error")]
    Codec(#[from] strata_codec::CodecError),

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

    /// For use when a there's state entries that the partial state doesn't have
    /// information about that was referenced by some operation in processing a
    /// block, so we can't check if the block is valid or not.
    #[error("provided partial state insufficient for block being executed")]
    InsufficientPartialState,

    /// There was an invalid block within a segment, for some reason.
    #[error("invalid block")]
    InvalidBlock,

    /// There was a tx that was invalid in a block, for some reason.
    #[error("invalid tx in a block")]
    InvalidBlockTx,

    /// A deposit has an invalid destination address.
    #[error("invalid deposit address: {0}")]
    InvalidDepositAddress(strata_acct_types::SubjectId),

    #[error("blocks in a chunk did not match the chunk's attested io")]
    InconsistentChunkIo,
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
