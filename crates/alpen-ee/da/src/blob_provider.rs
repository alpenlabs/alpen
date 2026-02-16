//! [`DaBlobSource`] implementation backed by Reth state diffs.

use std::{fmt, sync::Arc};

use alloy_primitives::B256;
use alpen_ee_common::{BatchId, BatchStorage, DaBlob, DaBlobSource, HeaderSummaryProvider};
use alpen_reth_db::{EeDaContext, StateDiffProvider};
use alpen_reth_statediff::BatchBuilder;
use async_trait::async_trait;
use tracing::*;

/// [`DaBlobSource`] that builds encoded DA blobs from Reth state diffs.
///
/// For each batch, it:
/// 1. Retrieves the block range from [`BatchStorage`].
/// 2. Fetches per-block [`BlockStateChanges`](alpen_reth_statediff::BlockStateChanges) from the
///    [`StateDiffProvider`].
/// 3. Aggregates them into a [`BatchStateDiff`](alpen_reth_statediff::BatchStateDiff) via
///    [`BatchBuilder`].
/// 4. Reads the last block's header to build
///    [`EvmHeaderSummary`](alpen_ee_common::EvmHeaderSummary).
/// 5. Returns the assembled [`DaBlob`].
pub struct StateDiffBlobProvider<S, D, H> {
    batch_storage: Arc<S>,
    state_diff_provider: Arc<D>,
    header_summary: Arc<H>,
    da_ctx: Arc<dyn EeDaContext + Send + Sync>,
}

impl<S, D, H> fmt::Debug for StateDiffBlobProvider<S, D, H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StateDiffBlobProvider")
            .finish_non_exhaustive()
    }
}

impl<S, D, H> StateDiffBlobProvider<S, D, H> {
    pub fn new(
        batch_storage: Arc<S>,
        state_diff_provider: Arc<D>,
        header_summary: Arc<H>,
        da_ctx: Arc<dyn EeDaContext + Send + Sync>,
    ) -> Self {
        Self {
            batch_storage,
            state_diff_provider,
            header_summary,
            da_ctx,
        }
    }

    /// Returns `true` if the bytecode has not been published in a prior batch
    /// and therefore still needs to be included in the DA blob.
    ///
    /// On DB errors the bytecode is conservatively kept — duplicates are safe,
    /// missing data is not.
    fn bytecode_needs_publish(&self, code_hash: &B256) -> bool {
        match self.da_ctx.is_code_hash_published(code_hash) {
            Ok(published) => !published,
            Err(e) => {
                warn!(?code_hash, error = %e, "failed to check published status, keeping bytecode");
                true
            }
        }
    }
}

#[async_trait]
impl<S, D, H> DaBlobSource for StateDiffBlobProvider<S, D, H>
where
    S: BatchStorage,
    D: StateDiffProvider + Send + Sync,
    H: HeaderSummaryProvider,
{
    async fn get_blob(&self, batch_id: BatchId) -> eyre::Result<DaBlob> {
        // 1. Look up the batch to get its block range.
        let (batch, _status) = self
            .batch_storage
            .get_batch_by_id(batch_id)
            .await?
            .ok_or_else(|| eyre::eyre!("batch {batch_id:?} not found in storage"))?;

        // 2. Aggregate per-block diffs via BatchBuilder.
        let mut builder = BatchBuilder::new();
        let mut block_count = 0u64;

        for block_hash in batch.blocks_iter() {
            // Convert Hash (Buf32) -> B256 for the StateDiffProvider interface.
            let b256 = B256::from(block_hash.0);

            match self.state_diff_provider.get_state_diff_by_hash(b256) {
                Ok(Some(block_diff)) => {
                    builder.apply_block(&block_diff);
                    block_count += 1;
                }
                Ok(None) => {
                    warn!(?block_hash, "no state diff for block, skipping");
                }
                Err(e) => {
                    warn!(?block_hash, error = %e, "failed to fetch state diff for block");
                    return Err(eyre::eyre!(
                        "failed to fetch state diff for block {block_hash:?}: {e}"
                    ));
                }
            }
        }

        let expected_count = batch.blocks_iter().count() as u64;
        eyre::ensure!(
            block_count == expected_count,
            "state diff count mismatch for batch {batch_id:?}: got {block_count}, expected {expected_count}"
        );

        // 3. Build the aggregate diff and filter already-published bytecodes.
        let mut state_diff = builder.build();

        let before = state_diff.deployed_bytecodes.len();
        state_diff
            .deployed_bytecodes
            .retain(|hash, _| self.bytecode_needs_publish(hash));
        let deduped = before - state_diff.deployed_bytecodes.len();

        info!(
            ?batch_id,
            block_count,
            is_empty = state_diff.is_empty(),
            deduped,
            "built DA blob for batch"
        );

        // 4. Read the last block's header for chain-reconstruction metadata.
        let evm_header = self.header_summary.header_summary(batch.last_blocknum())?;

        // 5. Construct the DaBlob with metadata.
        Ok(DaBlob {
            batch_id,
            evm_header,
            state_diff,
        })
    }

    async fn are_state_diffs_ready(&self, batch_id: BatchId) -> eyre::Result<bool> {
        // Look up the batch to get its block range.
        let (batch, _status) = self
            .batch_storage
            .get_batch_by_id(batch_id)
            .await?
            .ok_or_else(|| eyre::eyre!("batch {batch_id:?} not found in storage"))?;

        // Check if all blocks have state diffs available.
        for block_hash in batch.blocks_iter() {
            let b256 = B256::from(block_hash.0);
            match self.state_diff_provider.get_state_diff_by_hash(b256) {
                Ok(Some(_)) => {
                    // State diff exists for this block
                }
                Ok(None) => {
                    // State diff not yet available
                    debug!(?block_hash, "state diff not available for block");
                    return Ok(false);
                }
                Err(e) => {
                    warn!(?block_hash, error = %e, "failed to check state diff for block");
                    return Err(eyre::eyre!(
                        "failed to check state diff for block {block_hash:?}: {e}"
                    ));
                }
            }
        }

        Ok(true)
    }
}
