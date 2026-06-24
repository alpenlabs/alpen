//! Alpen EE RPC handler implementation.

use std::{fmt, future::Future, sync::Arc};

use alloy_primitives::B256;
use alpen_ee_common::{ChunkStatus, ChunkStorage, ConsensusHeads, OLBlockOrEpoch, Storage};
use alpen_ee_rpc_api::{
    AlpenEeRpcServer, BlockStatus, BlockStatusResponse, ChunkProofCoverageResponse,
    StaticFeeModelConfig,
};
use async_trait::async_trait;
use jsonrpsee::core::RpcResult;
use reth_node_builder::NodeTypesWithDB;
use reth_provider::{
    providers::{BlockchainProvider, ProviderNodeTypes},
    BlockHashReader, BlockNumReader, ProviderResult,
};
use strata_identifiers::Epoch;
use tokio::sync::watch;

use crate::errors::{
    block_not_found_error, fee_model_config_unavailable_error, internal_error, invalid_params_error,
};

/// Resolve `block_hash` to its canonical block number on `provider`.
///
/// Returns `Ok(Some(n))` only when `block_hash` is the canonical hash at
/// height `n`. A hash that is known to the provider but belongs to an
/// orphaned / non-canonical branch returns `Ok(None)`.
fn fetch_canonical_block_number<N: NodeTypesWithDB + ProviderNodeTypes>(
    provider: &BlockchainProvider<N>,
    block_hash: B256,
) -> ProviderResult<Option<u64>> {
    let Some(block_number) = provider.block_number(block_hash)? else {
        return Ok(None);
    };
    let Some(canonical_hash) = provider.block_hash(block_number)? else {
        return Ok(None);
    };
    if canonical_hash == block_hash {
        Ok(Some(block_number))
    } else {
        Ok(None)
    }
}

fn hash_to_b256(hash: &[u8]) -> B256 {
    B256::from_slice(hash)
}

/// Canonical EE block number of the last block included in `epoch`'s checkpoint.
///
/// Returns an error when the epoch's account state is missing (e.g. pruned) or
/// when its last included block is not canonical on this node.
async fn fetch_epoch_last_alpen_block_number<N: NodeTypesWithDB + ProviderNodeTypes>(
    storage: &Arc<dyn Storage>,
    provider: &BlockchainProvider<N>,
    epoch: Epoch,
) -> RpcResult<u64> {
    let state = storage
        .ee_account_state(OLBlockOrEpoch::Epoch(epoch))
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| internal_error(format!("missing EE account state for epoch {epoch}")))?;

    let last_blkid = state.last_exec_blkid();
    let last_hash = hash_to_b256(last_blkid.as_slice());
    match fetch_canonical_block_number(provider, last_hash) {
        Ok(Some(n)) => Ok(n),
        Ok(None) => Err(internal_error(format!(
            "last block of epoch {epoch} is not canonical"
        ))),
        Err(e) => Err(internal_error(e.to_string())),
    }
}

/// Binary search for the smallest epoch in `[0, frontier_epoch]` whose last
/// included EE block height is at or beyond `target_num`.
///
/// `epoch_last_num` maps an epoch to the canonical EE block height of the last
/// block included in that epoch's checkpoint. The mapping is assumed
/// monotonically non-decreasing, which holds because each successive checkpoint
/// extends the canonical EE chain. The frontier epoch must cover `target_num`;
/// otherwise there is no containing epoch in the search range.
async fn search_containing_epoch<F, Fut>(
    frontier_epoch: Epoch,
    target_num: u64,
    epoch_last_num: F,
) -> RpcResult<Epoch>
where
    F: Fn(Epoch) -> Fut,
    Fut: Future<Output = RpcResult<u64>>,
{
    let frontier_last_num = epoch_last_num(frontier_epoch).await?;
    if frontier_last_num < target_num {
        return Err(internal_error(format!(
            "frontier epoch {frontier_epoch} only covers through block {frontier_last_num}, \
             below target block {target_num}"
        )));
    }

    let mut lo: Epoch = 0;
    let mut hi: Epoch = frontier_epoch;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if epoch_last_num(mid).await? >= target_num {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    Ok(lo)
}

/// RPC handler for [`AlpenEeRpcServer`].
pub struct EeRpcServer<N: NodeTypesWithDB + ProviderNodeTypes> {
    provider: BlockchainProvider<N>,
    consensus_rx: watch::Receiver<ConsensusHeads>,
    chunk_storage: Arc<dyn ChunkStorage>,
    account_storage: Arc<dyn Storage>,
    fee_model_config: Arc<dyn Fn() -> Option<StaticFeeModelConfig> + Send + Sync>,
}

impl<N: NodeTypesWithDB + ProviderNodeTypes> fmt::Debug for EeRpcServer<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EeRpcServer").finish_non_exhaustive()
    }
}

impl<N: NodeTypesWithDB + ProviderNodeTypes> EeRpcServer<N> {
    pub fn new(
        provider: BlockchainProvider<N>,
        consensus_rx: watch::Receiver<ConsensusHeads>,
        chunk_storage: Arc<dyn ChunkStorage>,
        account_storage: Arc<dyn Storage>,
        fee_model_config: Arc<dyn Fn() -> Option<StaticFeeModelConfig> + Send + Sync>,
    ) -> Self {
        Self {
            provider,
            consensus_rx,
            chunk_storage,
            account_storage,
            fee_model_config,
        }
    }

    /// Resolves the OL checkpoint epoch that contains the canonical EE block at
    /// `target_num`, searching epochs in `[0, frontier_epoch]`.
    ///
    /// The OL tracker records, for each epoch, the last EE block included in that
    /// epoch's checkpoint. That last-block height is monotonically non-decreasing
    /// in the epoch number, so a binary search finds the smallest epoch whose last
    /// included block is at or beyond `target_num` — i.e. the epoch that contains
    /// the block. `frontier_epoch` is the confirmed or finalized frontier already
    /// verified to cover `target_num`, so its last included block is at or beyond
    /// `target_num` and the search is well defined.
    async fn containing_epoch(&self, target_num: u64, frontier_epoch: Epoch) -> RpcResult<Epoch> {
        let storage = self.account_storage.clone();
        let provider = self.provider.clone();
        search_containing_epoch(frontier_epoch, target_num, move |epoch| {
            let storage = storage.clone();
            let provider = provider.clone();
            async move { fetch_epoch_last_alpen_block_number(&storage, &provider, epoch).await }
        })
        .await
    }
}

#[async_trait]
impl<N> AlpenEeRpcServer for EeRpcServer<N>
where
    N: NodeTypesWithDB + ProviderNodeTypes + Send + Sync + 'static,
{
    async fn get_block_status(&self, block_hash: B256) -> RpcResult<BlockStatusResponse> {
        // Resolve target to a canonical block number. `block_number` alone
        // does not distinguish canonical blocks from orphaned ones stored in
        // the DB, so round-trip through `block_hash(number)` to verify.
        let target_num = match fetch_canonical_block_number(&self.provider, block_hash) {
            Ok(Some(n)) => n,
            Ok(None) => return Err(block_not_found_error()),
            Err(e) => return Err(internal_error(e.to_string())),
        };

        // Preserve genesis semantics: block 0 is always considered finalized
        // and belongs to the genesis epoch.
        if target_num == 0 {
            return Ok(BlockStatusResponse {
                status: BlockStatus::Finalized,
                checkpoint_epoch: Some(0),
            });
        }

        let heads = self.consensus_rx.borrow().clone();

        // Finalized check: skip when the head is unset or not canonical on
        // this node (transient during sync / reorg — OLTracker may still be
        // tracking a fork that Reth hasn't reorged to).
        let finalized_b256 = hash_to_b256(heads.finalized().as_slice());
        if !finalized_b256.is_zero() {
            match fetch_canonical_block_number(&self.provider, finalized_b256) {
                Ok(Some(fin_num)) if target_num <= fin_num => {
                    let epoch = self
                        .containing_epoch(target_num, heads.finalized_epoch())
                        .await?;
                    return Ok(BlockStatusResponse {
                        status: BlockStatus::Finalized,
                        checkpoint_epoch: Some(epoch),
                    });
                }
                Ok(_) => {}
                Err(e) => return Err(internal_error(e.to_string())),
            }
        }

        // Confirmed check.
        let confirmed_b256 = hash_to_b256(heads.confirmed().as_slice());
        if !confirmed_b256.is_zero() {
            match fetch_canonical_block_number(&self.provider, confirmed_b256) {
                Ok(Some(conf_num)) if target_num <= conf_num => {
                    let epoch = self
                        .containing_epoch(target_num, heads.confirmed_epoch())
                        .await?;
                    return Ok(BlockStatusResponse {
                        status: BlockStatus::Confirmed,
                        checkpoint_epoch: Some(epoch),
                    });
                }
                Ok(_) => {}
                Err(e) => return Err(internal_error(e.to_string())),
            }
        }

        Ok(BlockStatusResponse {
            status: BlockStatus::Pending,
            checkpoint_epoch: None,
        })
    }

    async fn get_chunk_proof_coverage(
        &self,
        start_block: u64,
        end_block: u64,
    ) -> RpcResult<ChunkProofCoverageResponse> {
        if start_block == 0 || start_block > end_block {
            return Err(invalid_params_error(
                "start_block must be non-zero and less than or equal to end_block",
            ));
        }

        let latest_chunk = self
            .chunk_storage
            .get_latest_chunk()
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let Some((latest_chunk, _)) = latest_chunk else {
            return Ok(ChunkProofCoverageResponse {
                start_block,
                end_block,
                covered: false,
                first_uncovered_block: Some(start_block),
            });
        };

        let mut first_uncovered_block = start_block;

        for chunk_idx in 0..=latest_chunk.idx() {
            let Some((chunk, status)) = self
                .chunk_storage
                .get_chunk_by_idx(chunk_idx)
                .await
                .map_err(|e| internal_error(e.to_string()))?
            else {
                continue;
            };

            let prev_block_num = match fetch_canonical_block_number(
                &self.provider,
                hash_to_b256(chunk.prev_block().as_slice()),
            ) {
                Ok(Some(n)) => n,
                Ok(None) => continue,
                Err(e) => return Err(internal_error(e.to_string())),
            };
            let last_block_num = match fetch_canonical_block_number(
                &self.provider,
                hash_to_b256(chunk.last_block().as_slice()),
            ) {
                Ok(Some(n)) => n,
                Ok(None) => continue,
                Err(e) => return Err(internal_error(e.to_string())),
            };

            let chunk_start_block = prev_block_num.saturating_add(1);
            if last_block_num < first_uncovered_block {
                continue;
            }
            if chunk_start_block > end_block {
                break;
            }
            if chunk_start_block > first_uncovered_block
                || !matches!(status, ChunkStatus::ProofReady(_))
            {
                continue;
            }
            if last_block_num >= end_block {
                return Ok(ChunkProofCoverageResponse {
                    start_block,
                    end_block,
                    covered: true,
                    first_uncovered_block: None,
                });
            }
            first_uncovered_block = last_block_num + 1;
        }

        Ok(ChunkProofCoverageResponse {
            start_block,
            end_block,
            covered: false,
            first_uncovered_block: Some(first_uncovered_block),
        })
    }

    async fn get_fee_model_config(&self) -> RpcResult<StaticFeeModelConfig> {
        (self.fee_model_config)().ok_or_else(fee_model_config_unavailable_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the epoch binary search against a synthetic, monotonically
    /// non-decreasing `epoch -> last included block height` mapping.
    async fn search(heights: &[u64], target_num: u64) -> Epoch {
        let frontier_epoch = (heights.len() - 1) as Epoch;
        search_containing_epoch(frontier_epoch, target_num, |epoch| {
            let height = heights[epoch as usize];
            async move { Ok(height) }
        })
        .await
        .expect("search should succeed")
    }

    #[tokio::test]
    async fn finds_smallest_covering_epoch() {
        // epoch:                0   1   2    3
        // last block height:    5  10  10   20
        // Epoch 2 includes no new blocks (height unchanged from epoch 1).
        let heights = [5u64, 10, 10, 20];

        // Blocks 1..=5 were first included in epoch 0.
        assert_eq!(search(&heights, 1).await, 0);
        assert_eq!(search(&heights, 5).await, 0);

        // Blocks 6..=10 were first included in epoch 1, not the later empty epoch 2.
        assert_eq!(search(&heights, 6).await, 1);
        assert_eq!(search(&heights, 10).await, 1);

        // Blocks 11..=20 were included in epoch 3.
        assert_eq!(search(&heights, 11).await, 3);
        assert_eq!(search(&heights, 20).await, 3);
    }

    #[tokio::test]
    async fn single_epoch_frontier() {
        let heights = [7u64];
        assert_eq!(search(&heights, 1).await, 0);
        assert_eq!(search(&heights, 7).await, 0);
    }

    #[tokio::test]
    async fn propagates_lookup_error() {
        let result = search_containing_epoch(3, 10, |_epoch| async move {
            Err(internal_error("boom".to_string()))
        })
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn errors_when_frontier_does_not_cover_target() {
        let heights = [3u64, 5, 7];
        let result = search_containing_epoch(2, 10, |epoch| {
            let height = heights[epoch as usize];
            async move { Ok(height) }
        })
        .await;

        assert!(result.is_err());
    }
}
