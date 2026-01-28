use strata_snark_acct_runtime::ProgramError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EnvError {
    /// Malformed extra data.
    #[error("malformed extra data")]
    MalformedExtraData,

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

    #[error("mismatched intermediate state")]
    MismatchedIntermediateState,

    #[error("mismatched terminal state")]
    MismatchedTerminalState,

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

    /// Codec error during encoding or decoding.
    #[error("codec: {0}")]
    Codec(#[from] strata_codec::CodecError),
}

pub type EnvResult<T> = Result<T, EnvError>;

impl From<EnvError> for ProgramError<EnvError> {
    fn from(value: EnvError) -> Self {
        match value {
            // Pass codec errors through unchanged.
            EnvError::Codec(e) => ProgramError::Codec(e),
            _ => ProgramError::Internal(value),
        }
    }
}

pub type EnvProgramResult<T> = Result<T, ProgramError<EnvError>>;

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
