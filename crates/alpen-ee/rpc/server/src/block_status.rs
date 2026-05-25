//! Alpen EE RPC handler implementation.

use std::{fmt, sync::Arc};

use alloy_primitives::B256;
use alpen_ee_common::{BatchStorage, ChunkStatus, ConsensusHeads};
use alpen_ee_rpc_api::{
    AlpenEeRpcServer, BlockStatus, BlockStatusResponse, ChunkProofCoverageResponse,
    ChunkProofRange, ChunkProofStatus,
};
use async_trait::async_trait;
use jsonrpsee::core::RpcResult;
use reth_node_builder::NodeTypesWithDB;
use reth_provider::{
    providers::{BlockchainProvider, ProviderNodeTypes},
    BlockHashReader, BlockNumReader, ProviderResult,
};
use tokio::sync::watch;

use crate::errors::{block_not_found_error, internal_error, invalid_params_error};

/// Resolve `block_hash` to its canonical block number on `provider`.
///
/// Returns `Ok(Some(n))` only when `block_hash` is the canonical hash at
/// height `n`. A hash that is known to the provider but belongs to an
/// orphaned / non-canonical branch returns `Ok(None)`.
fn canonical_block_number<N: NodeTypesWithDB + ProviderNodeTypes>(
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

fn hash_to_rpc_string(hash: &[u8]) -> String {
    hash_to_b256(hash).to_string()
}

fn chunk_proof_status(status: &ChunkStatus) -> (ChunkProofStatus, Option<String>) {
    match status {
        ChunkStatus::ProvingNotStarted => (ChunkProofStatus::NotStarted, None),
        ChunkStatus::ProofPending(_) => (ChunkProofStatus::Pending, None),
        ChunkStatus::ProofReady(proof_id) => (
            ChunkProofStatus::ProofReady,
            Some(hash_to_rpc_string(proof_id.as_slice())),
        ),
    }
}

fn update_coverage(next_uncovered: &mut u64, end_block: u64, range: &ChunkProofRange) -> bool {
    if range.status != ChunkProofStatus::ProofReady || range.end_block < *next_uncovered {
        return false;
    }

    if range.start_block > *next_uncovered {
        return false;
    }

    if range.end_block >= end_block {
        *next_uncovered = end_block.saturating_add(1);
        return true;
    }

    *next_uncovered = range.end_block + 1;
    false
}

/// RPC handler for [`AlpenEeRpcServer`].
pub struct EeRpcServer<N: NodeTypesWithDB + ProviderNodeTypes> {
    provider: BlockchainProvider<N>,
    consensus_rx: watch::Receiver<ConsensusHeads>,
    batch_storage: Arc<dyn BatchStorage>,
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
        batch_storage: Arc<dyn BatchStorage>,
    ) -> Self {
        Self {
            provider,
            consensus_rx,
            batch_storage,
        }
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
        let target_num = match canonical_block_number(&self.provider, block_hash) {
            Ok(Some(n)) => n,
            Ok(None) => return Err(block_not_found_error()),
            Err(e) => return Err(internal_error(e.to_string())),
        };

        // Preserve genesis semantics: block 0 is always considered finalized.
        if target_num == 0 {
            return Ok(BlockStatusResponse {
                status: BlockStatus::Finalized,
            });
        }

        let heads = self.consensus_rx.borrow().clone();

        // Finalized check: skip when the head is unset or not canonical on
        // this node (transient during sync / reorg — OLTracker may still be
        // tracking a fork that Reth hasn't reorged to).
        let finalized_b256 = hash_to_b256(heads.finalized().as_slice());
        if !finalized_b256.is_zero() {
            match canonical_block_number(&self.provider, finalized_b256) {
                Ok(Some(fin_num)) if target_num <= fin_num => {
                    return Ok(BlockStatusResponse {
                        status: BlockStatus::Finalized,
                    });
                }
                Ok(_) => {}
                Err(e) => return Err(internal_error(e.to_string())),
            }
        }

        // Confirmed check.
        let confirmed_b256 = hash_to_b256(heads.confirmed().as_slice());
        if !confirmed_b256.is_zero() {
            match canonical_block_number(&self.provider, confirmed_b256) {
                Ok(Some(conf_num)) if target_num <= conf_num => {
                    return Ok(BlockStatusResponse {
                        status: BlockStatus::Confirmed,
                    });
                }
                Ok(_) => {}
                Err(e) => return Err(internal_error(e.to_string())),
            }
        }

        Ok(BlockStatusResponse {
            status: BlockStatus::Pending,
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
            .batch_storage
            .get_latest_chunk()
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let Some((latest_chunk, _)) = latest_chunk else {
            return Ok(ChunkProofCoverageResponse {
                start_block,
                end_block,
                covered: false,
                first_uncovered_block: Some(start_block),
                ranges: Vec::new(),
            });
        };

        let mut ranges = Vec::new();
        let mut next_uncovered = start_block;
        let mut covered = false;

        for chunk_idx in 0..=latest_chunk.idx() {
            let Some((chunk, status)) = self
                .batch_storage
                .get_chunk_by_idx(chunk_idx)
                .await
                .map_err(|e| internal_error(e.to_string()))?
            else {
                continue;
            };

            let prev_block_num = match canonical_block_number(
                &self.provider,
                hash_to_b256(chunk.prev_block().as_slice()),
            ) {
                Ok(Some(n)) => n,
                Ok(None) => continue,
                Err(e) => return Err(internal_error(e.to_string())),
            };
            let last_block_num = match canonical_block_number(
                &self.provider,
                hash_to_b256(chunk.last_block().as_slice()),
            ) {
                Ok(Some(n)) => n,
                Ok(None) => continue,
                Err(e) => return Err(internal_error(e.to_string())),
            };

            let chunk_start_block = prev_block_num.saturating_add(1);
            if last_block_num < start_block {
                continue;
            }
            if chunk_start_block > end_block {
                break;
            }

            let (status, proof_id) = chunk_proof_status(&status);
            let range = ChunkProofRange {
                chunk_index: chunk.idx(),
                start_block: chunk_start_block,
                end_block: last_block_num,
                status,
                proof_id,
                prev_block: hash_to_rpc_string(chunk.prev_block().as_slice()),
                last_block: hash_to_rpc_string(chunk.last_block().as_slice()),
            };

            if update_coverage(&mut next_uncovered, end_block, &range) {
                covered = true;
            }
            ranges.push(range);
        }

        Ok(ChunkProofCoverageResponse {
            start_block,
            end_block,
            covered,
            first_uncovered_block: (!covered).then_some(next_uncovered),
            ranges,
        })
    }
}
