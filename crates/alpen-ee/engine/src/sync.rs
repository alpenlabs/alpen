//! Sync missing blocks in execution engine using payloads stored in sequencer database.

use std::future::Future;

use alloy_primitives::B256;
use alloy_rpc_types_engine::ForkchoiceState;
use alpen_ee_common::{EnginePayload, ExecBlockStorage, ExecutionEngine, ExecutionEngineError};
use reth_node_builder::NodeTypesWithDB;
use reth_provider::{
    providers::{BlockchainProvider, ProviderNodeTypes},
    BlockNumReader, ProviderError,
};
use strata_acct_types::Hash;
use thiserror::Error;
use tracing::{debug, info};

/// Errors that can occur during chainstate sync.
#[derive(Debug, Error)]
pub enum SyncError {
    /// Missing exec block at height.
    #[error("missing exec block at height {0}")]
    MissingExecBlock(u64),

    /// Missing block payload for specified block hash.
    #[error("missing block payload for hash {0:?}")]
    MissingBlockPayload(Hash),

    /// Block was reported as unfinalized but not found in storage.
    #[error("unfinalized block {0:?} not found in storage")]
    UnfinalizedBlockNotFound(Hash),

    /// Finalized chain is empty.
    #[error("finalized chain is empty")]
    EmptyFinalizedChain,

    /// Storage error.
    #[error("storage error: {0}")]
    Storage(#[from] alpen_ee_common::StorageError),

    /// Alpen's execution engine error.
    #[error("alpen engine error: {0}")]
    Engine(#[from] ExecutionEngineError),

    /// Reth `Provider` error.
    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),

    /// Payload deserialization error.
    #[error("payload deserialization error: {0}")]
    PayloadDeserialization(String),
}

/// Syncs missing blocks in Alpen's execution engine using payloads stored in sequencer database.
///
/// Compares the finalized chain in the sequencer's database with the blocks present in Reth. If
/// Reth is missing blocks, they are submitted using stored payloads.
///
/// # Arguments
///
/// - `storage` - Sequencer's block storage containing canonical chain and payloads
/// - `provider` - Reth blockchain provider to check which blocks exist
/// - `engine` - Execution engine to submit missing payloads
///
/// # Returns
///
/// `Ok(())` if sync completed successfully, or an error if sync failed.
// TODO: retry on network errors
pub async fn sync_chainstate_to_engine<N, E, S>(
    storage: &S,
    provider: &BlockchainProvider<N>,
    engine: &E,
) -> Result<(), SyncError>
where
    N: NodeTypesWithDB + ProviderNodeTypes,
    E: ExecutionEngine,
    S: ExecBlockStorage,
{
    // Get the best finalized block from sequencer's database
    let Some(best_finalized) = storage.best_finalized_block().await? else {
        return Err(SyncError::EmptyFinalizedChain);
    };

    // Get the latest height of the finalized chain
    let latest_height = best_finalized.blocknum();

    info!(
        latest_height = %latest_height,
        latest_hash = ?best_finalized.blockhash(),
        "starting chainstate sync check"
    );

    // Start the binary search to find the last known block in Alpen's execution engine.
    // Build list of finalized block hashes by height
    let mut canonical_chain = Vec::new();
    for height in 0..=latest_height {
        let Some(block) = storage.get_finalized_block_at_height(height).await? else {
            return Err(SyncError::MissingExecBlock(height));
        };
        canonical_chain.push(block.blockhash());
    }

    let earliest_idx = 0;
    let latest_idx = canonical_chain.len().saturating_sub(1);

    info!(total_blocks = %canonical_chain.len(), "searching for last known block in engine");

    // Find the last block in the canonical chain that exists in Reth
    let sync_from_idx = find_last_match((earliest_idx, latest_idx), |idx| {
        let blockhash = canonical_chain[idx];
        check_block_exists_in_reth(blockhash, provider)
    })?
    .map(|idx| idx + 1) // sync from next block
    .unwrap_or(0); // sync from genesis

    if sync_from_idx > canonical_chain.len() {
        info!("all blocks already in engine, no sync needed");
        return Ok(());
    }

    // Calculate the number of blocks to sync
    let blocks_to_sync = canonical_chain.len() - sync_from_idx;
    info!(
        sync_from_height = %sync_from_idx,
        total_blocks = %canonical_chain.len(),
        blocks_to_sync = %blocks_to_sync,
        "syncing missing blocks to engine"
    );

    // Sync all blocks from sync_from_idx onwards
    for (idx, &blockhash) in canonical_chain.iter().enumerate().skip(sync_from_idx) {
        debug!(height = %idx, ?blockhash, "syncing block");

        // Get the payload for this block
        let Some(payload) = storage.get_block_payload(blockhash).await? else {
            return Err(SyncError::MissingBlockPayload(blockhash));
        };

        // Deserialize and submit the payload
        let engine_payload = <E::TEnginePayload as EnginePayload>::from_bytes(payload.as_bytes())
            .map_err(|e| SyncError::PayloadDeserialization(e.to_string()))?;

        engine.submit_payload(engine_payload).await?;

        // Update fork choice to mark this block as the new head
        let forkchoice_state = ForkchoiceState {
            head_block_hash: B256::from_slice(&blockhash),
            safe_block_hash: B256::from_slice(&blockhash),
            finalized_block_hash: if let Some(prev) = prev_blockhash {
                B256::from_slice(&prev)
            } else {
                B256::from_slice(&blockhash)
            },
        };
        engine.update_consensus_state(forkchoice_state).await?;

        debug!(height = %height, ?blockhash, "block synced successfully");
        prev_blockhash = Some(blockhash);
    }

    info!(blocks_synced = %blocks_to_sync, "finalized chainstate sync completed");

    // Sync unfinalized blocks (blocks above best finalized height)
    sync_unfinalized_blocks(storage, checker, engine, &best_finalized).await?;

    Ok(())
}

/// Checks if a block exists in Reth's database.
fn check_block_exists_in_reth<N: NodeTypesWithDB + ProviderNodeTypes>(
    blockhash: Hash,
    provider: &BlockchainProvider<N>,
) -> Result<bool, SyncError> {
    let b256_hash = B256::from_slice(&blockhash);
    Ok(provider.block_number(b256_hash)?.is_some())
}

/// Binary search to find the last index where the predicate returns true.
///
/// Assumes the predicate returns `true` for a contiguous range starting from index `0`,
/// and `false` for all indices after that range.
///
/// The predicate is async to allow fetching data on-demand during the search,
/// resulting in O(log n) fetches instead of requiring all data upfront.
async fn find_last_match<F, Fut>(
    range: (usize, usize),
    predicate: F,
) -> Result<Option<usize>, SyncError>
where
    F: Fn(usize) -> Fut,
    Fut: Future<Output = Result<bool, SyncError>>,
{
    let (mut left, mut right) = range;

    // Handle empty range
    if left > right {
        return Ok(None);
    }

    // Check the leftmost value first
    if !predicate(left).await? {
        return Ok(None); // If the leftmost value is false, no values can be true
    }

    let mut best_match = None;

    // Proceed with binary search
    while left <= right {
        let mid = left + (right - left) / 2;

        if predicate(mid).await? {
            best_match = Some(mid); // Update best match
            left = mid + 1; // Continue searching in the right half
        } else {
            if mid == 0 {
                break;
            }
            right = mid - 1; // Search in the left half
        }
    }

    Ok(best_match)
}

/// Sync unfinalized blocks to the execution engine.
///
/// Unfinalized blocks are blocks that have been saved but not yet finalized.
/// These may include forks. Each block is checked against Reth and synced if missing.
async fn sync_unfinalized_blocks<C, E, S>(
    storage: &S,
    checker: &C,
    engine: &E,
    best_finalized: &alpen_ee_common::ExecBlockRecord,
) -> Result<(), SyncError>
where
    C: BlockExistenceChecker,
    E: ExecutionEngine,
    S: ExecBlockStorage,
{
    info!("checking unfinalized blocks");

    let unfinalized_hashes = storage.get_unfinalized_blocks().await?;
    if unfinalized_hashes.is_empty() {
        info!("no unfinalized blocks to sync");
        return Ok(());
    }

    info!(count = %unfinalized_hashes.len(), "found unfinalized blocks");

    let best_finalized_hash = best_finalized.blockhash();

    for hash in unfinalized_hashes {
        // Check if block exists in Reth
        if checker.block_exists(hash)? {
            continue; // Skip if already present
        }

        debug!(?hash, "syncing unfinalized block");

        // Get block metadata for logging
        let Some(block) = storage.get_exec_block(hash).await? else {
            return Err(SyncError::UnfinalizedBlockNotFound(hash));
        };

        // Get and submit payload
        let Some(payload) = storage.get_block_payload(hash).await? else {
            return Err(SyncError::MissingBlockPayload(hash));
        };

        let engine_payload = <E::TEnginePayload as EnginePayload>::from_bytes(payload.as_bytes())
            .map_err(|e| SyncError::PayloadDeserialization(e.to_string()))?;

        engine.submit_payload(engine_payload).await?;

        // For unfinalized blocks, update forkchoice with head=hash,
        // finalized=best_finalized
        let forkchoice_state = ForkchoiceState {
            head_block_hash: B256::from_slice(&hash),
            safe_block_hash: B256::from_slice(&hash),
            finalized_block_hash: B256::from_slice(&best_finalized_hash),
        };
        engine.update_consensus_state(forkchoice_state).await?;

        debug!(height = %block.blocknum(), ?hash, "unfinalized block synced successfully");
    }

    info!("unfinalized blocks sync completed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_last_match() {
        // find match
        assert!(matches!(
            find_last_match((0, 5), |idx| Ok(idx < 3)),
            Ok(Some(2))
        ));
        // found no match
        assert!(matches!(find_last_match((0, 5), |_| Ok(false)), Ok(None)));
        // got error
        assert!(matches!(
            find_last_match((0, 5), |_| Err(SyncError::EmptyFinalizedChain)),
            Err(SyncError::EmptyFinalizedChain)
        ));
    }
}
