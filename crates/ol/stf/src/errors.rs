use strata_acct_types::{AccountId, AcctError};
use thiserror::Error;

/// Execution result error.
pub type ExecResult<T> = Result<T, ExecError>;

/// Error from executing/validating the block.
#[derive(Debug, Error)]
pub enum ExecError {
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

    /// Various account errors.
    #[error("acct: {0}")]
    Acct(#[from] AcctError),
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
