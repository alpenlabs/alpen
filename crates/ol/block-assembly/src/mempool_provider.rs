//! Mempool provider trait for block assembly.

use std::sync::Arc;

use async_trait::async_trait;
use strata_identifiers::OLTxId;
use strata_ol_chain_types_new::OLTransaction;
use strata_ol_mempool::{MempoolHandle, MempoolTxInvalidReason};

use crate::{BlockAssemblyError, BlockAssemblyResult};

/// Provider for mempool transactions.
#[async_trait]
pub trait MempoolProvider: Send + Sync + 'static {
    /// Gets [`OLTransaction`] entries from mempool.
    ///
    /// Returns up to `limit` transactions in priority order with their [`OLTxId`] values.
    async fn get_transactions(
        &self,
        limit: usize,
    ) -> BlockAssemblyResult<Vec<(OLTxId, OLTransaction)>>;

    /// Reports invalid transactions to mempool by providing IDs and reasons for being invalid.
    async fn report_invalid_transactions(
        &self,
        txs: &[(OLTxId, MempoolTxInvalidReason)],
    ) -> BlockAssemblyResult<()>;
}

/// Mempool provider implementation backed by [`MempoolHandle`].
#[expect(
    missing_debug_implementations,
    reason = "MempoolHandle does not implement Debug"
)]
pub struct MempoolProviderImpl {
    mempool_handle: Arc<MempoolHandle>,
}

impl MempoolProviderImpl {
    /// Create a new mempool provider.
    pub fn new(mempool_handle: Arc<MempoolHandle>) -> Self {
        Self { mempool_handle }
    }
}

#[async_trait]
impl MempoolProvider for MempoolProviderImpl {
    async fn get_transactions(
        &self,
        limit: usize,
    ) -> BlockAssemblyResult<Vec<(OLTxId, OLTransaction)>> {
        self.mempool_handle
            .get_transactions(limit)
            .await
            .map_err(BlockAssemblyError::Mempool)
    }

    async fn report_invalid_transactions(
        &self,
        txs: &[(OLTxId, MempoolTxInvalidReason)],
    ) -> BlockAssemblyResult<()> {
        self.mempool_handle
            .report_invalid_transactions(txs.to_vec())
            .await
            .map_err(BlockAssemblyError::Mempool)
    }
}
