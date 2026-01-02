use std::sync::Arc;

use sled::transaction::ConflictableTransactionResult;
use strata_db_types::DbResult;
use typed_sled::{
    error::Error,
    transaction::{Backoff, ConstantBackoff, SledTransactional},
};

use crate::utils::to_db_error;

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
    pub fn with_retry<Trees, F, R>(&self, trees: Trees, f: F) -> DbResult<R>
    where
        Trees: SledTransactional,
        F: Fn(Trees::View) -> ConflictableTransactionResult<R, Error>,
    {
        trees
            .transaction_with_retry(self.backoff.as_ref(), self.retry_count.into(), f)
            .map_err(to_db_error)
    }
}
