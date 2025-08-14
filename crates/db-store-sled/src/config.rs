use std::sync::Arc;

use sled::transaction::ConflictableTransactionResult;
use strata_db::{DbError, DbResult};
use typed_sled::transaction::{Backoff, ConstantBackoff, SledTransactional};

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

    /// Execute a transaction with retry logic using this config's settings
    pub fn with_retry<Trees, F, R>(&self, trees: Trees, f: F) -> DbResult<R>
    where
        Trees: SledTransactional,
        F: Fn(Trees::View) -> ConflictableTransactionResult<R, typed_sled::error::Error>,
    {
        trees
            .transaction_with_retry(self.backoff.as_ref(), self.retry_count.into(), f)
            .map_err(|e| DbError::Other(format!("{:?}", e)))
    }
}
