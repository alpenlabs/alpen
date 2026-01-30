//! Block template types for OL block assembly.

use strata_identifiers::OLTxId;
pub use strata_ol_block_template_types::{
    BlockCompletionData, BlockGenerationConfig, BlockTemplate, FullBlockTemplate,
};
use strata_ol_mempool::MempoolTxInvalidReason;

/// Type alias for a failed mempool transaction with failure reason.
pub(crate) type FailedMempoolTx = (OLTxId, MempoolTxInvalidReason);

/// Result of block template generation including the template and any failed transactions.
#[derive(Debug, Clone)]
pub(crate) struct BlockTemplateResult {
    template: FullBlockTemplate,
    failed_txs: Vec<FailedMempoolTx>,
}

impl BlockTemplateResult {
    /// Create a new block template result.
    pub(crate) fn new(template: FullBlockTemplate, failed_txs: Vec<FailedMempoolTx>) -> Self {
        Self {
            template,
            failed_txs,
        }
    }

    /// Returns the block template.
    #[cfg_attr(not(test), expect(dead_code, reason = "used in tests"))]
    pub(crate) fn template(&self) -> &FullBlockTemplate {
        &self.template
    }

    /// Consumes self and returns the template.
    #[cfg_attr(not(test), expect(dead_code, reason = "used in tests"))]
    pub(crate) fn into_template(self) -> FullBlockTemplate {
        self.template
    }

    /// Consumes self and returns both components.
    pub(crate) fn into_parts(self) -> (FullBlockTemplate, Vec<FailedMempoolTx>) {
        (self.template, self.failed_txs)
    }
}
