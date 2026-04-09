//! Alpen EE RPC handler implementation for block-status methods.

use std::{error::Error, fmt, num::NonZeroU64, sync::Arc};

use alloy_primitives::B256;
use alpen_ee_common::{BatchStorage, ConsensusHeads, ExecBlockStorage, StorageError};
use alpen_ee_rpc_api::{AlpenEeRpcServer, BlockStatus, BlockStatusResponse};
use async_trait::async_trait;
use jsonrpsee::core::RpcResult;
use strata_acct_types::Hash;
use tokio::sync::watch;

use crate::errors::{block_not_found_error, frontier_unavailable_error, internal_error};

/// RPC handler for [`AlpenEeRpcServer`].
#[derive(Debug)]
pub struct EeRpcServer<S> {
    storage: Arc<S>,
    consensus_rx: watch::Receiver<ConsensusHeads>,
}

/// Batch lookup result for an execution block hash.
enum BlockBatchLookup {
    /// Block hash does not exist in execution storage.
    BlockNotFound,

    /// Block exists but no batch covers it yet.
    NoBatchCoverage,

    /// Block is covered by the genesis marker batch (idx 0).
    CoveredGenesis,

    /// Block is covered by a non-genesis batch index.
    Covered(NonZeroU64),
}

/// Errors while resolving a consensus frontier hash to batch coverage.
#[derive(Debug)]
enum FrontierResolveError {
    /// Frontier block exists but no batch covers it yet.
    NoBatchCoverage(Hash),

    /// Frontier block hash does not exist in execution storage.
    MissingExecBlock(Hash),

    /// Storage failure while resolving frontier coverage.
    Storage(StorageError),
}

impl fmt::Display for FrontierResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBatchCoverage(hash) => {
                write!(f, "frontier block {hash} not covered by any batch")
            }
            Self::MissingExecBlock(hash) => {
                write!(f, "frontier block {hash} missing in execution storage")
            }
            Self::Storage(err) => {
                write!(f, "storage error while resolving frontier batch: {err}")
            }
        }
    }
}

impl Error for FrontierResolveError {}

impl<S> EeRpcServer<S> {
    pub fn new(storage: Arc<S>, consensus_rx: watch::Receiver<ConsensusHeads>) -> Self {
        Self {
            storage,
            consensus_rx,
        }
    }
}

impl<S: BatchStorage + ExecBlockStorage> EeRpcServer<S> {
    /// Resolves a frontier head hash to its batch index.
    ///
    /// Returns `None` if the hash is zero (uninitialized frontier).
    async fn resolve_frontier_batch_idx(
        &self,
        head_hash: Hash,
    ) -> Result<Option<u64>, FrontierResolveError> {
        if head_hash.is_zero() {
            return Ok(None);
        }

        let lookup = self
            .find_batch_for_block(head_hash)
            .await
            .map_err(FrontierResolveError::Storage)?;

        match lookup {
            BlockBatchLookup::CoveredGenesis => Ok(Some(0)),
            BlockBatchLookup::Covered(idx) => Ok(Some(idx.get())),
            BlockBatchLookup::NoBatchCoverage => {
                Err(FrontierResolveError::NoBatchCoverage(head_hash))
            }
            BlockBatchLookup::BlockNotFound => {
                Err(FrontierResolveError::MissingExecBlock(head_hash))
            }
        }
    }

    /// Resolves a block hash to its block number and finds the covering batch.
    async fn find_batch_for_block(
        &self,
        target_hash: Hash,
    ) -> Result<BlockBatchLookup, StorageError> {
        let Some(target_record) = self.storage.get_exec_block(target_hash).await? else {
            return Ok(BlockBatchLookup::BlockNotFound);
        };

        let batch_idx = self
            .find_batch_by_number(target_record.blocknum(), target_hash)
            .await?;

        Ok(match batch_idx {
            Some(0) => BlockBatchLookup::CoveredGenesis,
            Some(idx) => BlockBatchLookup::Covered(
                NonZeroU64::new(idx)
                    .expect("batch index must be non-zero when mapped to Covered variant"),
            ),
            None => BlockBatchLookup::NoBatchCoverage,
        })
    }

    /// Binary-searches `BatchStorage` for the batch containing `target_blocknum` and `target_hash`.
    ///
    /// Returns `None` if the block is not yet covered by any batch.
    async fn find_batch_by_number(
        &self,
        target_blocknum: u64,
        target_hash: Hash,
    ) -> Result<Option<u64>, StorageError> {
        let Some((latest_batch, _)) = self.storage.get_latest_batch().await? else {
            return Ok(None);
        };

        let mut lo: u64 = 0;
        let mut hi: u64 = latest_batch.idx();
        let mut candidate_idx: Option<u64> = None;

        while lo <= hi {
            let mid = lo + (hi - lo) / 2;
            let Some((batch, _)) = self.storage.get_batch_by_idx(mid).await? else {
                break;
            };

            if batch.last_blocknum() >= target_blocknum {
                candidate_idx = Some(mid);
                if mid == 0 {
                    break;
                }
                hi = mid - 1;
            } else {
                lo = mid + 1;
            }
        }

        let Some(idx) = candidate_idx else {
            return Ok(None);
        };

        let Some((batch, _)) = self.storage.get_batch_by_idx(idx).await? else {
            return Ok(None);
        };

        if batch.last_block() == target_hash || batch.inner_blocks().contains(&target_hash) {
            Ok(Some(idx))
        } else {
            Ok(None)
        }
    }
}

#[async_trait]
impl<S> AlpenEeRpcServer for EeRpcServer<S>
where
    S: BatchStorage + ExecBlockStorage + Send + Sync + 'static,
{
    async fn get_block_status(&self, block_hash: B256) -> RpcResult<BlockStatusResponse> {
        // This handler is registered only when running in sequencer mode (see `main.rs`).
        let hash = Hash::from(block_hash.0);

        let target_lookup = self
            .find_batch_for_block(hash)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let target_batch_idx = match target_lookup {
            BlockBatchLookup::BlockNotFound => return Err(block_not_found_error()),
            BlockBatchLookup::NoBatchCoverage => {
                return Ok(BlockStatusResponse {
                    status: BlockStatus::Pending,
                })
            }
            // Batch 0 is a storage marker, not a real update submitted to OL.
            BlockBatchLookup::CoveredGenesis => {
                return Ok(BlockStatusResponse {
                    status: BlockStatus::Finalized,
                })
            }
            BlockBatchLookup::Covered(idx) => idx.get(),
        };

        let heads = self.consensus_rx.borrow().clone();

        let finalized_batch_idx = self
            .resolve_frontier_batch_idx(*heads.finalized())
            .await
            .map_err(frontier_unavailable_error)?;

        if finalized_batch_idx.is_some_and(|idx| target_batch_idx <= idx) {
            return Ok(BlockStatusResponse {
                status: BlockStatus::Finalized,
            });
        }

        let confirmed_batch_idx = self
            .resolve_frontier_batch_idx(*heads.confirmed())
            .await
            .map_err(frontier_unavailable_error)?;

        if confirmed_batch_idx.is_some_and(|idx| target_batch_idx <= idx) {
            return Ok(BlockStatusResponse {
                status: BlockStatus::Confirmed,
            });
        }

        Ok(BlockStatusResponse {
            status: BlockStatus::Pending,
        })
    }
}
