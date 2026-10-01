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
    use typed_sled::{CodecError, SledDb};

    use super::*;
    use crate::ol::schemas::OLBlockStatusSchema;

    #[test]
    fn storage_io_categories_survive_every_adapter_path() {
        for kind in [
            ErrorKind::Interrupted,
            ErrorKind::WouldBlock,
            ErrorKind::PermissionDenied,
            ErrorKind::InvalidData,
            ErrorKind::StorageFull,
            ErrorKind::Other,
        ] {
            let io_error = || SledError::Io(IoError::new(kind, "injected storage failure"));
            for error in [
                conv_sled_err(Error::SledError(io_error())),
                conv_sled_err(Error::TransactionError(
                    UnabortableTransactionError::Storage(io_error()),
                )),
                tx_error_to_db_error(TransactionError::Storage(io_error())),
                tx_error_to_db_error(TransactionError::Abort(Error::SledError(io_error()))),
            ] {
                assert!(matches!(
                    &error,
                    DbError::Io { kind: actual, message }
                        if *actual == kind && message == "injected storage failure"
                ));
                let retryable = matches!(kind, ErrorKind::Interrupted | ErrorKind::WouldBlock);
                assert_eq!(error.is_retryable(), retryable);
                assert_eq!(
                    DbError::RetriesExhausted {
                        attempts: 3,
                        last_error: Box::new(error),
                    }
                    .is_retryable(),
                    retryable
                );
            }
        }
    }

    #[test]
    fn typed_transaction_failures_preserve_retry_policy() {
        let db = SledDb::new(sled::Config::new().temporary(true).open().unwrap()).unwrap();
        let tree = db.get_tree::<OLBlockStatusSchema>().unwrap();
        let config = SledDbConfig::new_with_constant_backoff(0, 0);

        // Exercise typed-sled's abort conversion inside a real transaction and the production
        // config adapter. Only the failure is injected; no FCM storage stub is involved.
        for kind in [ErrorKind::WouldBlock, ErrorKind::PermissionDenied] {
            let result: DbResult<()> = config.with_retry((&tree,), |_| {
                Err(
                    Error::TransactionError(UnabortableTransactionError::Storage(SledError::Io(
                        IoError::new(kind, "injected transaction failure"),
                    )))
                    .into(),
                )
            });
            let error = result.unwrap_err();
            assert!(matches!(&error, DbError::Io { kind: actual, .. } if *actual == kind));
            assert_eq!(error.is_retryable(), kind == ErrorKind::WouldBlock);
        }

        let conflict: DbResult<()> = config.with_retry((&tree,), |_| {
            Err(Error::TransactionError(UnabortableTransactionError::Conflict).into())
        });
        assert!(matches!(conflict.unwrap_err(), DbError::Busy));
    }

    #[test]
    fn permanent_failures_and_domain_aborts_remain_fatal() {
        let codec_error = tx_error_to_db_error(TransactionError::Abort(Error::CodecError(
            CodecError::InvalidKeyLength {
                schema: "test",
                expected: 32,
                actual: 1,
            },
        )));
        assert!(matches!(codec_error, DbError::CodecError(_)));
        assert!(!codec_error.is_retryable());

        for error in [
            SledError::Corruption { at: None, bt: () },
            SledError::Unsupported("unsupported operation".into()),
            SledError::ReportableBug("internal failure".into()),
        ] {
            assert!(!conv_sled_err(Error::SledError(error.clone())).is_retryable());
            assert!(!tx_error_to_db_error(TransactionError::Storage(error)).is_retryable());
        }
        let corruption = conv_sled_storage_err(SledError::Corruption { at: None, bt: () });
        assert!(matches!(corruption, DbError::Corruption(_)));

        let domain = tx_error_to_db_error(TransactionError::Abort(Error::abort(
            DbError::InvalidArgument,
        )));
        assert!(matches!(domain, DbError::InvalidArgument));
        assert!(!domain.is_retryable());
        let unknown = tx_error_to_db_error(TransactionError::Abort(Error::abort(IoError::other(
            "unknown abort",
        ))));
        assert!(matches!(unknown, DbError::Other(_)));
        assert!(!unknown.is_retryable());
    }
}
