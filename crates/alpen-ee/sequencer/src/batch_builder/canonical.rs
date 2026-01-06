//! Canonical chain reader trait for batch builder reorg detection.

use std::sync::Arc;

use alpen_ee_common::ExecBlockStorage;
use alpen_ee_exec_chain::ExecChainHandle;
use async_trait::async_trait;
use eyre::Result;
use strata_acct_types::Hash;

/// Checks if blocks are on the canonical chain (finalized or unfinalized canonical).
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub(crate) trait CanonicalChainReader: Send + Sync {
    /// Returns `true` if the block is on the canonical chain.
    async fn is_canonical(&self, hash: Hash) -> Result<bool>;

    /// Returns `true` if the block is finalized.
    async fn is_finalized(&self, hash: Hash) -> Result<bool>;
}

/// Implementation of `CanonicalChainReader` using `ExecChainHandle` and `ExecBlockStorage`.
#[expect(dead_code, reason = "todo")]
pub(crate) struct ExecChainCanonicalReader<S> {
    exec_chain: Arc<ExecChainHandle>,
    block_storage: Arc<S>,
}

impl<S> ExecChainCanonicalReader<S> {
    #[allow(unused, clippy::allow_attributes, reason = "todo")]
    pub(crate) fn new(exec_chain: Arc<ExecChainHandle>, block_storage: Arc<S>) -> Self {
        Self {
            exec_chain,
            block_storage,
        }
    }
}

#[async_trait]
impl<S: ExecBlockStorage> CanonicalChainReader for ExecChainCanonicalReader<S> {
    async fn is_canonical(&self, hash: Hash) -> Result<bool> {
        // Check finalized first (avoids async query to exec_chain)
        if self
            .block_storage
            .get_finalized_height(hash)
            .await?
            .is_some()
        {
            return Ok(true);
        }
        // Check unfinalized canonical chain
        self.exec_chain.is_canonical(hash).await
    }

    async fn is_finalized(&self, _hash: Hash) -> Result<bool> {
        unimplemented!()
    }
}
