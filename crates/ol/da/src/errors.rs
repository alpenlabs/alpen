use strata_acct_types::AccountSerial;
use thiserror::Error;

pub type DaResult<T> = Result<T, DaError>;

#[derive(Debug, Error)]
pub enum DaError {
    #[error("unknown serial {0:?}")]
    UnknownSerial(AccountSerial),

    #[error("{0}")]
    Other(&'static str),
}
