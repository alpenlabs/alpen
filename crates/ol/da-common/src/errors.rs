use strata_da_framework::DaError as FrameworkDaError;
use strata_identifiers::AccountSerial;
use strata_predicate::PredicateError;
use thiserror::Error;

pub type DaResult<T> = Result<T, DaError>;

#[derive(Debug, Error)]
pub enum DaError {
    #[error("DA framework failure: {0}")]
    FrameworkError(#[from] FrameworkDaError),

    #[error("invalid predicate key: {0}")]
    InvalidPredicateKey(#[from] PredicateError),

    #[error("invalid state diff: {0}")]
    InvalidStateDiff(&'static str),

    #[error("invalid ledger diff: {0}")]
    InvalidLedgerDiff(&'static str),

    #[error("unknown serial {0:?}")]
    UnknownSerial(AccountSerial),

    #[error("{0}")]
    Other(&'static str),
}
