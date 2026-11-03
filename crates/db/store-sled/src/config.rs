use std::sync::Arc;

use sled::transaction::{ConflictableTransactionResult, TransactionError};
use strata_db_types::{DbError, DbResult};
use tracing::instrument;
use typed_sled::error::Error;
use typed_sled::transaction::{Backoff, ConstantBackoff, SledTransactional};

use crate::instrumentation::components;
use crate::utils::{conv_sled_err, conv_sled_storage_err};

/// Flattens a `TransactionError<typed_sled::Error>` into a `DbError`,
/// preserving aborted `DbError` variants via [`conv_sled_err`].
fn tx_error_to_db_error(err: TransactionError<Error>) -> DbError {
    match err {
        TransactionError::Abort(e) => conv_sled_err(e),
        TransactionError::Storage(e) => conv_sled_storage_err(e),
    }
}

// Configuration constants
pub(crate) const DEFAULT_RETRY_COUNT: u16 = 3;
pub(crate) const DEFAULT_RETRY_DELAY_MS: u64 = 150;
pub(crate) const TEST_RETRY_DELAY_MS: u64 = 50; // Faster for tests

/// database operations configuration
#[derive(Debug, Clone)]
pub struct SledDbConfig {
    pub retry_count: u16,
    pub backoff: Arc<dyn Backoff>,
}

impl SledDbConfig {
    pub fn new(retry_count: u16, backoff: Arc<dyn Backoff>) -> Self {
        Self {
            retry_count,
            backoff,
        }
    }

    pub fn new_with_constant_backoff(retry_count: u16, delay: u64) -> Self {
        let const_backoff = ConstantBackoff::new(delay);
        Self {
            retry_count,
            backoff: Arc::new(const_backoff),
        }
    }

    /// Create production configuration with default values
    pub fn production() -> Self {
        Self::new_with_constant_backoff(DEFAULT_RETRY_COUNT, DEFAULT_RETRY_DELAY_MS)
    }

    /// Create test configuration with faster retry delays
    pub fn test() -> Self {
        Self::new_with_constant_backoff(DEFAULT_RETRY_COUNT, TEST_RETRY_DELAY_MS)
    }

    /// Execute a transaction with retry logic using this config's settings
    #[instrument(
        level = "debug",
        skip_all,
        fields(
            component = components::DB_SLED_TRANSACTION,
            max_retries = self.retry_count,
        )
    )]
    pub fn with_retry<Trees, F, R>(&self, trees: Trees, f: F) -> DbResult<R>
    where
        Trees: SledTransactional,
        F: Fn(Trees::View) -> ConflictableTransactionResult<R, Error>,
    {
        trees
            .transaction_with_retry(self.backoff.as_ref(), self.retry_count.into(), f)
            .map_err(tx_error_to_db_error)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Error as IoError, ErrorKind};

    use sled::Error as SledError;
    use sled::transaction::UnabortableTransactionError;

    use super::*;

    #[test]
    fn io_classification_survives_direct_and_transactional_storage_errors() {
        for (kind, retryable) in [
            (ErrorKind::Interrupted, true),
            (ErrorKind::WouldBlock, true),
            (ErrorKind::TimedOut, true),
            (ErrorKind::PermissionDenied, false),
            (ErrorKind::NotFound, false),
            (ErrorKind::InvalidData, false),
            (ErrorKind::StorageFull, false),
            (ErrorKind::Other, false),
        ] {
            let storage = || SledError::Io(IoError::new(kind, "storage failed"));
            for error in [
                conv_sled_err(Error::SledError(storage())),
                conv_sled_err(Error::TransactionError(
                    UnabortableTransactionError::Storage(storage()),
                )),
                tx_error_to_db_error(TransactionError::Storage(storage())),
            ] {
                assert_eq!(error.is_retryable(), retryable, "{kind:?}");
                assert!(matches!(error, DbError::Io { kind: actual, message }
                    if actual == kind && message == "storage failed"));
            }
        }
    }

    #[test]
    fn wrapping_domain_errors_preserves_the_retry_policy() {
        for error in [
            DbError::Busy,
            DbError::CodecError("invalid stored value".into()),
            DbError::WorkerFailedStrangely("worker panicked".into()),
            DbError::Other("inconsistent database".into()),
            DbError::InvalidArgument,
        ] {
            let retryable = matches!(error, DbError::Busy);
            for converted in [
                conv_sled_err(Error::abort(error.clone())),
                tx_error_to_db_error(TransactionError::Abort(Error::abort(error.clone()))),
            ] {
                assert_eq!(converted.is_retryable(), retryable);
                assert_eq!(converted.to_string(), error.to_string());
            }

            let exhausted = DbError::RetriesExhausted {
                attempts: 3,
                last_error: Box::new(error),
            };
            assert_eq!(exhausted.is_retryable(), retryable);
        }

        let storage = SledError::ReportableBug("storage invariant violated".into());
        let error = tx_error_to_db_error(TransactionError::Storage(storage));
        assert!(!error.is_retryable());
    }
}
