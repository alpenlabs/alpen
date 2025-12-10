use strata_identifiers::AccountId;
use thiserror::Error;

use crate::{AccountTypeId, RawAccountTypeId};

pub type AcctResult<T> = Result<T, AcctError>;

/// Account related error types.
// leaving this abbreviated because it's used a lot
#[derive(Debug, Error)]
pub enum AcctError {
    /// When we mismatch uses of types.
    ///
    /// (real acct type, asked type)
    #[error("tried to use {0} as {1}")]
    MismatchedType(AccountTypeId, AccountTypeId),

    /// Issue decoding an account's type state.
    #[error("decode {0} account state")]
    DecodeState(AccountTypeId),

    #[error("tried to create account with existing ID ({0:?})")]
    AccountIdExists(AccountId),

    #[error("tried to access account that does not exist ({0:?})")]
    MissingExpectedAccount(AccountId),

    #[error("operation not supported in this context")]
    Unsupported,
}
