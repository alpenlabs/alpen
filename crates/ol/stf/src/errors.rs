use strata_acct_types::{AccountId, AcctError};
use strata_codec::CodecError;
use strata_ol_chain_types_new::{Epoch, Slot};
use thiserror::Error;

/// Execution result error.
pub type ExecResult<T> = Result<T, ExecError>;

/// Error from executing/validating the block.
#[derive(Debug, Error)]
pub enum ExecError {
    #[error("header epoch does not match state epoch (header {0}, state {1})")]
    EpochMismatch(Epoch, Epoch),

    /// Signature is invalid, for some purpose.
    #[error("signature for {0} is invalid")]
    SignatureInvalid(&'static str),

    /// Normal balance check fail.
    #[error("tried to underflow a balance")]
    BalanceUnderflow,

    #[error("condition in tx attachment failed")]
    TxConditionCheckFailed,

    /// For like if we'd be skipping blocks in validation somehow.
    #[error("chain integrity invalid")]
    ChainIntegrity,

    #[error("tried to interact with nonexistent account ({0:?})")]
    UnknownAccount(AccountId),

    /// This is used if the target of a snark account update tx is not a snark
    /// account.
    #[error("tx target invalid for tx type")]
    IncorrectTxTargetType,

    /// Used when the block's body doesn't match its header.
    #[error("internal block structure mismatches")]
    BlockStructureMismatch,

    /// The parent blkid field doesn't match the header we're using to verify
    /// the block.
    #[error("parent blkid mismatch")]
    BlockParentMismatch,

    #[error("verifying genesis header with nonnull parent field")]
    GenesisParentNonnull,

    #[error("genesis-looking block has non-zero slot or epoch field")]
    GenesisCoordsNonzero,

    #[error("tried to skip epoch (parent {0}, current {1})")]
    SkipEpochs(Epoch, Epoch),

    #[error("tried to skip too many slots (parent {0}, current {1})")]
    SkipTooManySlots(Slot, Slot),

    #[error("incorrect epoch sequencing (parent {0}, parent terminal {2}, self {1})")]
    IncorrectEpoch(Epoch, Epoch, bool),

    #[error("body inconsistent with header terminal flag")]
    InconsistentBodyTerminality,

    #[error("genesis block was not a terminal")]
    GenesisNonterminal,

    /// Various account errors.
    #[error("acct: {0}")]
    Acct(#[from] AcctError),

    /// Codec error.
    #[error("codec: {0}")]
    Codec(#[from] CodecError),
    // TODO more types
}

impl ExecError {
    pub fn kind(&self) -> ErrorKind {
        // By default, we can assume all errors indicate the block is invalid.
        match self {
            _ => ErrorKind::Correctness,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    /// This is an execution error that should abord validation inconclusively.
    Execution,

    /// This is some correctness error that indicates the block is invalid.
    Correctness,
}
