//! Alpen EE RPC handler implementation.

use std::{fmt, sync::Arc};

use alloy_primitives::B256;
use alpen_ee_common::{Batch, BatchStatus, BatchStorage, Chunk, ChunkStatus, ConsensusHeads};
use alpen_ee_rpc_api::{
    AlpenEeProofPipelineRpcServer, AlpenEeRpcServer, BlockStatus, BlockStatusResponse,
    ProofPipelineBatch, ProofPipelineBatchStatus, ProofPipelineChunk, ProofPipelineChunkStatus,
    ProofPipelineStatusResponse,
};
use async_trait::async_trait;
use jsonrpsee::core::RpcResult;
use reth_node_builder::NodeTypesWithDB;
use reth_provider::{
    providers::{BlockchainProvider, ProviderNodeTypes},
    BlockHashReader, BlockNumReader, ProviderResult,
};
use tokio::sync::watch;

use crate::errors::{block_not_found_error, internal_error};

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

/// RPC handler for [`AlpenEeRpcServer`].
///
/// Resolves block status by combining Reth's canonical-chain lookup with the
/// `OLTracker`-derived [`ConsensusHeads`]. Works on both sequencer and fullnode
/// because neither dependency is sequencer-specific.
pub struct EeRpcServer<N: NodeTypesWithDB + ProviderNodeTypes> {
    provider: BlockchainProvider<N>,
    consensus_rx: watch::Receiver<ConsensusHeads>,
    batch_storage: Arc<dyn BatchStorage>,
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

    fn block_number_for_hash(&self, block_hash: B256) -> RpcResult<Option<u64>> {
        self.provider
            .block_number(block_hash)
            .map_err(|e| internal_error(e.to_string()))
    }
}

impl<N: NodeTypesWithDB + ProviderNodeTypes> fmt::Debug for EeRpcServer<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EeRpcServer").finish_non_exhaustive()
    }
}

fn convert_batch_status(status: BatchStatus) -> (ProofPipelineBatchStatus, Option<String>) {
    match status {
        BatchStatus::Genesis => (ProofPipelineBatchStatus::Genesis, None),
        BatchStatus::Sealed => (ProofPipelineBatchStatus::Sealed, None),
        BatchStatus::DaPending { .. } => (ProofPipelineBatchStatus::DaPending, None),
        BatchStatus::DaComplete { .. } => (ProofPipelineBatchStatus::DaComplete, None),
        BatchStatus::ProofPending { .. } => (ProofPipelineBatchStatus::ProofPending, None),
        BatchStatus::ProofReady { proof, .. } => (
            ProofPipelineBatchStatus::ProofReady,
            Some(proof.to_string()),
        ),
    }
}

fn convert_chunk_status(status: ChunkStatus) -> (ProofPipelineChunkStatus, Option<String>) {
    match status {
        ChunkStatus::ProvingNotStarted => (ProofPipelineChunkStatus::ProvingNotStarted, None),
        ChunkStatus::ProofPending(_) => (ProofPipelineChunkStatus::ProofPending, None),
        ChunkStatus::ProofReady(proof) => (
            ProofPipelineChunkStatus::ProofReady,
            Some(proof.to_string()),
        ),
    }
}

fn batch_response(batch: Batch, status: BatchStatus) -> ProofPipelineBatch {
    let (status, proof) = convert_batch_status(status);
    ProofPipelineBatch {
        idx: batch.idx(),
        last_block: batch.last_block().to_string(),
        last_block_number: batch.last_blocknum(),
        status,
        proof,
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
        let finalized_b256 = B256::from_slice(heads.finalized().as_slice());
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
        let confirmed_b256 = B256::from_slice(heads.confirmed().as_slice());
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
}

#[async_trait]
impl<N> AlpenEeProofPipelineRpcServer for EeRpcServer<N>
where
    N: NodeTypesWithDB + ProviderNodeTypes + Send + Sync + 'static,
{
    async fn get_proof_pipeline_status(&self) -> RpcResult<ProofPipelineStatusResponse> {
        let latest_batch = self
            .batch_storage
            .get_latest_batch()
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let latest_proof_ready_batch = match latest_batch.as_ref() {
            Some((batch, _)) => {
                let mut idx = batch.idx();
                let mut found = None;
                loop {
                    let entry = self
                        .batch_storage
                        .get_batch_by_idx(idx)
                        .await
                        .map_err(|e| internal_error(e.to_string()))?;
                    if let Some((candidate, status @ BatchStatus::ProofReady { .. })) = entry {
                        found = Some(batch_response(candidate, status));
                        break;
                    }
                    if idx == 0 {
                        break;
                    }
                    idx -= 1;
                }
                found
            }
            None => None,
        };

        let latest_chunk = self
            .batch_storage
            .get_latest_chunk()
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let latest_chunk = match latest_chunk {
            Some((chunk, status)) => Some(self.chunk_response(chunk, status)?),
            None => None,
        };

        let latest_proof_ready_chunk = match latest_chunk.as_ref() {
            Some(chunk) => {
                let mut idx = chunk.idx;
                let mut found = None;
                loop {
                    let entry = self
                        .batch_storage
                        .get_chunk_by_idx(idx)
                        .await
                        .map_err(|e| internal_error(e.to_string()))?;
                    if let Some((candidate, status @ ChunkStatus::ProofReady(_))) = entry {
                        found = Some(self.chunk_response(candidate, status)?);
                        break;
                    }
                    if idx == 0 {
                        break;
                    }
                    idx -= 1;
                }
                found
            }
            None => None,
        };

        Ok(ProofPipelineStatusResponse {
            latest_batch: latest_batch.map(|(batch, status)| batch_response(batch, status)),
            latest_proof_ready_batch,
            latest_chunk,
            latest_proof_ready_chunk,
        })
    }
}

impl<N> EeRpcServer<N>
where
    N: NodeTypesWithDB + ProviderNodeTypes + Send + Sync + 'static,
{
    fn chunk_response(&self, chunk: Chunk, status: ChunkStatus) -> RpcResult<ProofPipelineChunk> {
        let (status, proof) = convert_chunk_status(status);
        Ok(ProofPipelineChunk {
            idx: chunk.idx(),
            last_block: chunk.last_block().to_string(),
            last_block_number: self
                .block_number_for_hash(B256::from_slice(chunk.last_block().as_slice()))?,
            status,
            proof,
        })
    }
}
