use thiserror::Error;

use crate::id::AcctTypeId;

pub type AcctResult<T> = Result<T, AcctError>;

/// Account related error types.
#[derive(Debug, Error)]
pub enum AcctError {
    /// When we mismatch uses of types.
    #[error("tried to use {0} as {1}")]
    MismatchedType(AcctTypeId, AcctTypeId),

    /// Issue decoding an account's type state.
    #[error("decode {0} account state")]
    DecodeState(AcctTypeId),
}
