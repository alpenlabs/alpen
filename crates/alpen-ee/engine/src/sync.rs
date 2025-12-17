//! Sync missing blocks in execution engine using payloads stored in sequencer database.

use std::future::Future;

use alloy_primitives::B256;
use alloy_rpc_types_engine::ForkchoiceState;
use alpen_ee_common::{EnginePayload, ExecBlockStorage, ExecutionEngine};
use reth_node_builder::NodeTypesWithDB;
use reth_provider::{
    providers::{BlockchainProvider, ProviderNodeTypes},
    BlockNumReader,
};
use strata_acct_types::Hash;
use tracing::{debug, info};

use crate::SyncError;

/// Interface for checking if blocks exist in the execution layer.
///
/// This abstraction allows for testing without requiring a real Reth provider.
#[cfg_attr(test, mockall::automock)]
trait BlockExistenceChecker {
    /// Check if a block with the given hash exists.
    fn block_exists(&self, blockhash: Hash) -> Result<bool, SyncError>;
}

/// Wrapper around [`BlockchainProvider`] that implements [`BlockExistenceChecker`].
struct RethBlockChecker<'a, N: NodeTypesWithDB + ProviderNodeTypes> {
    provider: &'a BlockchainProvider<N>,
}

impl<'a, N: NodeTypesWithDB + ProviderNodeTypes> RethBlockChecker<'a, N> {
    /// Creates a new Reth block checker.
    fn new(provider: &'a BlockchainProvider<N>) -> Self {
        Self { provider }
    }
}

impl<'a, N: NodeTypesWithDB + ProviderNodeTypes> BlockExistenceChecker for RethBlockChecker<'a, N> {
    fn block_exists(&self, blockhash: Hash) -> Result<bool, SyncError> {
        let b256_hash = B256::from_slice(blockhash.as_ref());
        Ok(self.provider.block_number(b256_hash)?.is_some())
    }
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
    let checker = RethBlockChecker::new(provider);
    sync_chainstate_to_engine_internal(storage, &checker, engine).await
}

/// Internal sync function that accepts an abstracted block checker.
///
/// This allows for testing without requiring a real Reth provider.
async fn sync_chainstate_to_engine_internal<C, E, S>(
    storage: &S,
    checker: &C,
    engine: &E,
) -> Result<(), SyncError>
where
    C: BlockExistenceChecker,
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

    let total_blocks = latest_height + 1; // 0-indexed heights

    info!(total_blocks = %total_blocks, "searching for last known block in engine");

    // Find the last block in the canonical chain that exists in Reth using binary search.
    // Fetches blocks on-demand during the search (O(log n).
    let sync_from_height = find_last_match((0, latest_height as usize), |height| async move {
        let Some(block) = storage.get_finalized_block_at_height(height as u64).await? else {
            return Err(SyncError::MissingExecBlock(height as u64));
        };
        checker.block_exists(block.blockhash())
    })
    .await?
    .map(|height| height + 1) // sync from next block
    .unwrap_or(0); // sync from genesis

    if sync_from_height as u64 > latest_height {
        info!("all finalized blocks already in engine");
        // Still need to check unfinalized blocks
        sync_unfinalized_blocks(storage, checker, engine, &best_finalized).await?;
        return Ok(());
    }

    // Calculate the number of blocks to sync
    let blocks_to_sync = total_blocks as usize - sync_from_height;
    info!(
        %sync_from_height,
        %total_blocks,
        %blocks_to_sync,
        "syncing missing blocks to engine"
    );

    // Track the previous block hash for forkchoice updates
    let mut prev_blockhash: Option<Hash> = if sync_from_height > 0 {
        let prev_block = storage
            .get_finalized_block_at_height((sync_from_height - 1) as u64)
            .await?
            .ok_or(SyncError::MissingExecBlock((sync_from_height - 1) as u64))?;
        Some(prev_block.blockhash())
    } else {
        None
    };

    // Sync all blocks from sync_from_height onwards
    for height in sync_from_height..=(latest_height as usize) {
        let Some(block) = storage.get_finalized_block_at_height(height as u64).await? else {
            return Err(SyncError::MissingExecBlock(height as u64));
        };
        let blockhash = block.blockhash();

        debug!(height = %height, ?blockhash, "syncing block");

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
            head_block_hash: B256::from_slice(blockhash.as_ref()),
            safe_block_hash: B256::from_slice(blockhash.as_ref()),
            finalized_block_hash: if let Some(ref prev) = prev_blockhash {
                B256::from_slice(prev.as_ref())
            } else {
                B256::from_slice(blockhash.as_ref())
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
            head_block_hash: B256::from_slice(hash.as_ref()),
            safe_block_hash: B256::from_slice(hash.as_ref()),
            finalized_block_hash: B256::from_slice(best_finalized_hash.as_ref()),
        };
        engine.update_consensus_state(forkchoice_state).await?;

        debug!(height = %block.blocknum(), ?hash, "unfinalized block synced successfully");
    }

    info!("unfinalized blocks sync completed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use alpen_ee_common::{
        EnginePayload, ExecBlockPayload, ExecBlockRecord, ExecutionEngineError,
        MockExecBlockStorage, StorageError,
    };
    use async_trait::async_trait;
    use strata_acct_types::BitcoinAmount;
    use strata_ee_acct_types::EeAccountState;
    use strata_ee_chain_types::{BlockInputs, BlockOutputs, ExecBlockCommitment, ExecBlockPackage};

    use super::*;

    // =========================================================================
    // Mock Types
    // =========================================================================

    /// Mock payload type for testing.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct MockPayload {
        blocknum: u64,
        blockhash: Hash,
        data: Vec<u8>,
    }

    impl MockPayload {
        /// Creates a new mock payload.
        fn new(blocknum: u64, blockhash: Hash) -> Self {
            Self {
                blocknum,
                blockhash,
                data: vec![blocknum as u8],
            }
        }
    }

    /// Mock payload error.
    #[derive(Debug, thiserror::Error)]
    #[error("mock payload error: {0}")]
    struct MockPayloadError(String);

    impl EnginePayload for MockPayload {
        type Error = MockPayloadError;

        fn blocknum(&self) -> u64 {
            self.blocknum
        }

        fn blockhash(&self) -> Hash {
            self.blockhash
        }

        fn withdrawal_intents(&self) -> &[alpen_reth_node::WithdrawalIntent] {
            &[]
        }

        fn to_bytes(&self) -> Result<Vec<u8>, Self::Error> {
            Ok(self.data.clone())
        }

        fn from_bytes(bytes: &[u8]) -> Result<Self, Self::Error> {
            if bytes.is_empty() {
                return Err(MockPayloadError("empty bytes".to_string()));
            }
            let blocknum = bytes[0] as u64;
            let mut hash_bytes = [0u8; 32];
            hash_bytes[0] = bytes[0];
            Ok(Self {
                blocknum,
                blockhash: Hash::from(hash_bytes),
                data: bytes.to_vec(),
            })
        }
    }

    /// Mock execution engine for testing.
    #[derive(Default, Clone)]
    struct MockExecutionEngine {
        /// Payloads submitted to the engine.
        submitted_payloads: Arc<Mutex<Vec<MockPayload>>>,

        /// Forkchoice updates applied to the engine.
        forkchoice_updates: Arc<Mutex<Vec<ForkchoiceState>>>,

        /// Result of the last payload submission.
        submit_result: Arc<Mutex<Option<String>>>,

        /// Result of the last forkchoice update.
        forkchoice_result: Arc<Mutex<Option<String>>>,
    }

    impl MockExecutionEngine {
        /// Creates a new mock execution engine.
        fn new() -> Self {
            Self::default()
        }

        fn set_submit_error(&self, msg: &str) {
            *self.submit_result.lock().unwrap() = Some(msg.to_string());
        }

        fn set_forkchoice_error(&self, msg: &str) {
            *self.forkchoice_result.lock().unwrap() = Some(msg.to_string());
        }

        fn submitted_payloads(&self) -> Vec<MockPayload> {
            self.submitted_payloads.lock().unwrap().clone()
        }

        fn forkchoice_updates(&self) -> Vec<ForkchoiceState> {
            self.forkchoice_updates.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ExecutionEngine for MockExecutionEngine {
        type TEnginePayload = MockPayload;

        async fn submit_payload(
            &self,
            payload: Self::TEnginePayload,
        ) -> Result<(), ExecutionEngineError> {
            if let Some(error) = self.submit_result.lock().unwrap().as_ref() {
                return Err(ExecutionEngineError::InvalidPayload(error.clone()));
            }
            self.submitted_payloads.lock().unwrap().push(payload);
            Ok(())
        }

        async fn update_consensus_state(
            &self,
            state: ForkchoiceState,
        ) -> Result<(), ExecutionEngineError> {
            if let Some(error) = self.forkchoice_result.lock().unwrap().as_ref() {
                return Err(ExecutionEngineError::ForkChoiceUpdate(error.clone()));
            }
            self.forkchoice_updates.lock().unwrap().push(state);
            Ok(())
        }
    }

    // =========================================================================
    // Test Helpers
    // =========================================================================

    /// Creates a hash from a single byte value.
    fn hash_from_u8(value: u8) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[0] = value;
        Hash::from(bytes)
    }

    /// Gets the first byte of a hash.
    /// Works with both `[u8; 32]` and `Buf32`.
    fn hash_first_byte(hash: &Hash) -> u8 {
        // Note: For [u8; 32], as_ref() returns &[u8].
        // For Buf32, as_ref() returns &[u8; 32] which coerces to &[u8].
        let bytes: &[u8] = hash.as_ref();
        bytes[0]
    }

    /// Creates a test ExecBlockRecord with proper parent-child relationship.
    fn create_exec_block(blocknum: u64, parent_hash: Hash, block_hash: Hash) -> ExecBlockRecord {
        use strata_identifiers::{Buf32, OLBlockCommitment, OLBlockId};

        let package = ExecBlockPackage::new(
            ExecBlockCommitment::new(block_hash, block_hash),
            BlockInputs::new_empty(),
            BlockOutputs::new_empty(),
        );
        let account_state = EeAccountState::new(block_hash, BitcoinAmount::ZERO, vec![], vec![]);

        // Create OL block commitment
        let mut ol_block_bytes = [0u8; 32];
        ol_block_bytes[0] = blocknum as u8;
        let ol_block =
            OLBlockCommitment::new(blocknum * 10, OLBlockId::from(Buf32::new(ol_block_bytes)));
        let timestamp_ms = 1_000_000 + blocknum * 1_000;

        ExecBlockRecord::new(
            package,
            account_state,
            blocknum,
            ol_block,
            timestamp_ms,
            parent_hash,
        )
    }

    /// Creates a chain of exec blocks with sequential hashes.
    fn create_exec_block_chain(heights: &[u64]) -> Vec<ExecBlockRecord> {
        let mut blocks = Vec::new();
        let mut parent_hash = Hash::default();

        for &height in heights {
            let block_hash = hash_from_u8(height as u8);
            let block = create_exec_block(height, parent_hash, block_hash);
            blocks.push(block);
            parent_hash = block_hash;
        }

        blocks
    }

    /// Sets up mock storage with a chain of blocks.
    fn setup_mock_storage_with_chain(
        mock_storage: &mut MockExecBlockStorage,
        chain: Vec<ExecBlockRecord>,
    ) {
        // Setup best_finalized_block
        let best = chain.last().cloned();
        mock_storage
            .expect_best_finalized_block()
            .returning(move || Ok(best.clone()));

        // Setup get_finalized_block_at_height
        let chain_for_height = chain.clone();
        mock_storage
            .expect_get_finalized_block_at_height()
            .returning(move |height| {
                chain_for_height
                    .iter()
                    .find(|b| b.blocknum() == height)
                    .cloned()
                    .map(Some)
                    .ok_or_else(|| StorageError::database("block not found"))
            });

        // Setup get_block_payload
        mock_storage
            .expect_get_block_payload()
            .returning(move |hash| {
                let blocknum = hash_first_byte(&hash) as u64;
                let payload = MockPayload::new(blocknum, hash);
                Ok(Some(ExecBlockPayload::from_bytes(
                    payload.to_bytes().unwrap(),
                )))
            });

        // Setup get_unfinalized_blocks - by default, return empty (all blocks are finalized)
        mock_storage
            .expect_get_unfinalized_blocks()
            .returning(|| Ok(vec![]));

        // Setup get_exec_block
        let chain_for_exec = chain.clone();
        mock_storage.expect_get_exec_block().returning(move |hash| {
            Ok(chain_for_exec
                .iter()
                .find(|b| b.blockhash() == hash)
                .cloned())
        });
    }

    /// Sets up mock block checker to return true for blocks in known_heights.
    fn setup_mock_checker_with_known_blocks(
        checker: &mut MockBlockExistenceChecker,
        known_heights: &[u64],
    ) {
        let known = known_heights.to_vec();
        checker.expect_block_exists().returning(move |hash| {
            let height = hash_first_byte(&hash) as u64;
            Ok(known.contains(&height))
        });
    }

    // =========================================================================
    // Unit Tests
    // =========================================================================

    #[tokio::test]
    async fn test_find_last_match() {
        // find match
        assert!(matches!(
            find_last_match((0, 5), |idx| async move { Ok(idx < 3) }).await,
            Ok(Some(2))
        ));
        // found no match
        assert!(matches!(
            find_last_match((0, 5), |_| async move { Ok(false) }).await,
            Ok(None)
        ));
        // got error
        assert!(matches!(
            find_last_match(
                (0, 5),
                |_| async move { Err(SyncError::EmptyFinalizedChain) }
            )
            .await,
            Err(SyncError::EmptyFinalizedChain)
        ));
    }

    // =========================================================================
    // Integration Tests
    // =========================================================================

    mod sync_chainstate_to_engine_tests {
        use super::*;

        #[tokio::test]
        async fn test_no_sync_needed_when_all_blocks_exist() {
            // Scenario: Reth has all blocks (0-5), no sync needed
            // Local storage:  [0, 1, 2, 3, 4, 5]
            // Reth:           [0, 1, 2, 3, 4, 5]
            // Expected:       No payloads submitted

            let chain = create_exec_block_chain(&[0, 1, 2, 3, 4, 5]);
            let mut mock_storage = MockExecBlockStorage::new();
            let mut mock_checker = MockBlockExistenceChecker::new();
            let mock_engine = MockExecutionEngine::new();

            setup_mock_storage_with_chain(&mut mock_storage, chain);
            setup_mock_checker_with_known_blocks(&mut mock_checker, &[0, 1, 2, 3, 4, 5]);

            let result =
                sync_chainstate_to_engine_internal(&mock_storage, &mock_checker, &mock_engine)
                    .await;

            assert!(result.is_ok());
            assert_eq!(mock_engine.submitted_payloads().len(), 0);
            assert_eq!(mock_engine.forkchoice_updates().len(), 0);
        }

        #[tokio::test]
        async fn test_syncs_all_blocks_from_genesis() {
            // Scenario: Reth is empty, sync entire chain
            // Local storage:  [0, 1, 2, 3]
            // Reth:           []
            // Expected:       Sync blocks 0-3

            let chain = create_exec_block_chain(&[0, 1, 2, 3]);
            let mut mock_storage = MockExecBlockStorage::new();
            let mut mock_checker = MockBlockExistenceChecker::new();
            let mock_engine = MockExecutionEngine::new();

            setup_mock_storage_with_chain(&mut mock_storage, chain.clone());
            setup_mock_checker_with_known_blocks(&mut mock_checker, &[]);

            let result =
                sync_chainstate_to_engine_internal(&mock_storage, &mock_checker, &mock_engine)
                    .await;

            assert!(result.is_ok());
            assert_eq!(mock_engine.submitted_payloads().len(), 4);
            assert_eq!(mock_engine.forkchoice_updates().len(), 4);

            // Verify payloads were submitted in order
            let payloads = mock_engine.submitted_payloads();
            for (i, payload) in payloads.iter().enumerate() {
                assert_eq!(payload.blocknum(), i as u64);
            }
        }

        #[tokio::test]
        async fn test_syncs_missing_tail_blocks() {
            // Scenario: Reth has blocks 0-3, storage has 0-5
            // Local storage:  [0, 1, 2, 3, 4, 5]
            // Reth:           [0, 1, 2, 3]
            // Expected:       Sync blocks 4-5

            let chain = create_exec_block_chain(&[0, 1, 2, 3, 4, 5]);
            let mut mock_storage = MockExecBlockStorage::new();
            let mut mock_checker = MockBlockExistenceChecker::new();
            let mock_engine = MockExecutionEngine::new();

            setup_mock_storage_with_chain(&mut mock_storage, chain);
            setup_mock_checker_with_known_blocks(&mut mock_checker, &[0, 1, 2, 3]);

            let result =
                sync_chainstate_to_engine_internal(&mock_storage, &mock_checker, &mock_engine)
                    .await;

            assert!(result.is_ok());
            assert_eq!(mock_engine.submitted_payloads().len(), 2);

            // Verify only blocks 4 and 5 were synced
            let payloads = mock_engine.submitted_payloads();
            assert_eq!(payloads[0].blocknum(), 4);
            assert_eq!(payloads[1].blocknum(), 5);
        }

        #[tokio::test]
        async fn test_returns_error_when_finalized_chain_empty() {
            // Scenario: Storage has no finalized blocks
            // Expected:       EmptyFinalizedChain error

            let mut mock_storage = MockExecBlockStorage::new();
            let mock_checker = MockBlockExistenceChecker::new();
            let mock_engine = MockExecutionEngine::new();

            mock_storage
                .expect_best_finalized_block()
                .returning(|| Ok(None));

            let result =
                sync_chainstate_to_engine_internal(&mock_storage, &mock_checker, &mock_engine)
                    .await;

            assert!(matches!(result, Err(SyncError::EmptyFinalizedChain)));
        }

        #[tokio::test]
        async fn test_returns_error_when_block_missing_at_height() {
            // Scenario: Gap in storage chain (missing block at height 2)
            // Local storage:  [0, 1, _, 3, 4] (block 2 missing)
            // Expected:       MissingExecBlock error

            let chain = create_exec_block_chain(&[0, 1, 3, 4]);
            let mut mock_storage = MockExecBlockStorage::new();
            let mut mock_checker = MockBlockExistenceChecker::new();
            let mock_engine = MockExecutionEngine::new();

            // Best block is at height 4
            let chain_for_best = chain.clone();
            mock_storage
                .expect_best_finalized_block()
                .returning(move || Ok(Some(chain_for_best[3].clone())));

            // Heights 0, 1, 3, 4 exist, but 2 is missing
            let chain_for_height = chain.clone();
            mock_storage
                .expect_get_finalized_block_at_height()
                .returning(move |height| {
                    if height == 2 {
                        Ok(None)
                    } else {
                        Ok(chain_for_height
                            .iter()
                            .find(|b| b.blocknum() == height)
                            .cloned())
                    }
                });

            // Setup checker for blocks that will be accessed before error
            setup_mock_checker_with_known_blocks(&mut mock_checker, &[0, 1, 3, 4]);

            let result =
                sync_chainstate_to_engine_internal(&mock_storage, &mock_checker, &mock_engine)
                    .await;

            assert!(matches!(result, Err(SyncError::MissingExecBlock(2))));
        }

        #[tokio::test]
        async fn test_returns_error_when_payload_missing() {
            // Scenario: Block exists but payload is missing
            // Local storage:  [0, 1, 2] (block 1 has no payload)
            // Reth:           [0]
            // Expected:       MissingBlockPayload error

            let chain = create_exec_block_chain(&[0, 1, 2]);
            let mut mock_storage = MockExecBlockStorage::new();
            let mut mock_checker = MockBlockExistenceChecker::new();
            let mock_engine = MockExecutionEngine::new();

            // Setup best and height queries
            let best = chain.last().cloned();
            mock_storage
                .expect_best_finalized_block()
                .returning(move || Ok(best.clone()));

            let chain_for_height = chain.clone();
            mock_storage
                .expect_get_finalized_block_at_height()
                .returning(move |height| {
                    Ok(chain_for_height
                        .iter()
                        .find(|b| b.blocknum() == height)
                        .cloned())
                });

            // Payload for block 1 is missing
            mock_storage
                .expect_get_block_payload()
                .returning(move |hash| {
                    if hash_first_byte(&hash) == 1 {
                        Ok(None)
                    } else {
                        let blocknum = hash_first_byte(&hash) as u64;
                        let payload = MockPayload::new(blocknum, hash);
                        Ok(Some(ExecBlockPayload::from_bytes(
                            payload.to_bytes().unwrap(),
                        )))
                    }
                });

            setup_mock_checker_with_known_blocks(&mut mock_checker, &[0]);

            let result =
                sync_chainstate_to_engine_internal(&mock_storage, &mock_checker, &mock_engine)
                    .await;

            assert!(matches!(
                result,
                Err(SyncError::MissingBlockPayload(hash)) if hash_first_byte(&hash) == 1
            ));
        }

        #[tokio::test]
        async fn test_propagates_storage_error() {
            // Scenario: Storage returns error during chain iteration
            // Expected:       Storage error propagated

            let chain = create_exec_block_chain(&[0, 1, 2]);
            let mut mock_storage = MockExecBlockStorage::new();
            let mock_checker = MockBlockExistenceChecker::new();
            let mock_engine = MockExecutionEngine::new();

            mock_storage
                .expect_best_finalized_block()
                .returning(move || Ok(Some(chain[2].clone())));

            mock_storage
                .expect_get_finalized_block_at_height()
                .returning(|_| Err(StorageError::database("disk error")));

            let result =
                sync_chainstate_to_engine_internal(&mock_storage, &mock_checker, &mock_engine)
                    .await;

            assert!(matches!(result, Err(SyncError::Storage(_))));
        }

        #[tokio::test]
        async fn test_propagates_engine_error_on_submit() {
            // Scenario: Engine fails on submit_payload
            // Local storage:  [0, 1, 2]
            // Reth:           [0]
            // Expected:       Engine error propagated

            let chain = create_exec_block_chain(&[0, 1, 2]);
            let mut mock_storage = MockExecBlockStorage::new();
            let mut mock_checker = MockBlockExistenceChecker::new();
            let mock_engine = MockExecutionEngine::new();

            setup_mock_storage_with_chain(&mut mock_storage, chain);
            setup_mock_checker_with_known_blocks(&mut mock_checker, &[0]);

            // Engine fails on submit
            mock_engine.set_submit_error("invalid payload");

            let result =
                sync_chainstate_to_engine_internal(&mock_storage, &mock_checker, &mock_engine)
                    .await;

            assert!(matches!(result, Err(SyncError::Engine(_))));
        }

        #[tokio::test]
        async fn test_propagates_engine_error_on_forkchoice() {
            // Scenario: Engine fails on update_consensus_state
            // Local storage:  [0, 1]
            // Reth:           [0]
            // Expected:       Engine error propagated

            let chain = create_exec_block_chain(&[0, 1]);
            let mut mock_storage = MockExecBlockStorage::new();
            let mut mock_checker = MockBlockExistenceChecker::new();
            let mock_engine = MockExecutionEngine::new();

            setup_mock_storage_with_chain(&mut mock_storage, chain);
            setup_mock_checker_with_known_blocks(&mut mock_checker, &[0]);

            // Engine fails on forkchoice update
            mock_engine.set_forkchoice_error("forkchoice failed");

            let result =
                sync_chainstate_to_engine_internal(&mock_storage, &mock_checker, &mock_engine)
                    .await;

            assert!(matches!(result, Err(SyncError::Engine(_))));
        }

        #[tokio::test]
        async fn test_propagates_provider_error() {
            // Scenario: Block checker returns error
            // Expected:       Provider error propagated

            let chain = create_exec_block_chain(&[0, 1, 2]);
            let mut mock_storage = MockExecBlockStorage::new();
            let mut mock_checker = MockBlockExistenceChecker::new();
            let mock_engine = MockExecutionEngine::new();

            setup_mock_storage_with_chain(&mut mock_storage, chain);

            mock_checker
                .expect_block_exists()
                .returning(|_| Err(SyncError::EmptyFinalizedChain));

            let result =
                sync_chainstate_to_engine_internal(&mock_storage, &mock_checker, &mock_engine)
                    .await;

            // The checker returns EmptyFinalizedChain error which gets propagated
            assert!(matches!(result, Err(SyncError::EmptyFinalizedChain)));
        }

        #[tokio::test]
        async fn test_single_block_chain() {
            // Scenario: Chain has only genesis block
            // Local storage:  [0]
            // Reth:           []
            // Expected:       Sync genesis block

            let chain = create_exec_block_chain(&[0]);
            let mut mock_storage = MockExecBlockStorage::new();
            let mut mock_checker = MockBlockExistenceChecker::new();
            let mock_engine = MockExecutionEngine::new();

            setup_mock_storage_with_chain(&mut mock_storage, chain);
            setup_mock_checker_with_known_blocks(&mut mock_checker, &[]);

            let result =
                sync_chainstate_to_engine_internal(&mock_storage, &mock_checker, &mock_engine)
                    .await;

            assert!(result.is_ok());
            assert_eq!(mock_engine.submitted_payloads().len(), 1);
            assert_eq!(mock_engine.forkchoice_updates().len(), 1);

            // For genesis, finalized should equal head
            let forkchoice = &mock_engine.forkchoice_updates()[0];
            assert_eq!(forkchoice.head_block_hash, forkchoice.finalized_block_hash);
        }

        #[tokio::test]
        async fn test_forkchoice_state_correctness() {
            // Scenario: Verify forkchoice state is set correctly
            // Local storage:  [0, 1, 2]
            // Reth:           []
            // Expected:       Correct head/safe/finalized for each block

            let chain = create_exec_block_chain(&[0, 1, 2]);
            let mut mock_storage = MockExecBlockStorage::new();
            let mut mock_checker = MockBlockExistenceChecker::new();
            let mock_engine = MockExecutionEngine::new();

            setup_mock_storage_with_chain(&mut mock_storage, chain.clone());
            setup_mock_checker_with_known_blocks(&mut mock_checker, &[]);

            let result =
                sync_chainstate_to_engine_internal(&mock_storage, &mock_checker, &mock_engine)
                    .await;

            assert!(result.is_ok());

            let forkchoice_updates = mock_engine.forkchoice_updates();
            assert_eq!(forkchoice_updates.len(), 3);

            // Block 0: head=0, safe=0, finalized=0
            let fc0 = &forkchoice_updates[0];
            assert_eq!(
                fc0.head_block_hash,
                B256::from_slice(hash_from_u8(0).as_ref())
            );
            assert_eq!(
                fc0.safe_block_hash,
                B256::from_slice(hash_from_u8(0).as_ref())
            );
            assert_eq!(
                fc0.finalized_block_hash,
                B256::from_slice(hash_from_u8(0).as_ref())
            );

            // Block 1: head=1, safe=1, finalized=0
            let fc1 = &forkchoice_updates[1];
            assert_eq!(
                fc1.head_block_hash,
                B256::from_slice(hash_from_u8(1).as_ref())
            );
            assert_eq!(
                fc1.safe_block_hash,
                B256::from_slice(hash_from_u8(1).as_ref())
            );
            assert_eq!(
                fc1.finalized_block_hash,
                B256::from_slice(hash_from_u8(0).as_ref())
            );

            // Block 2: head=2, safe=2, finalized=1
            let fc2 = &forkchoice_updates[2];
            assert_eq!(
                fc2.head_block_hash,
                B256::from_slice(hash_from_u8(2).as_ref())
            );
            assert_eq!(
                fc2.safe_block_hash,
                B256::from_slice(hash_from_u8(2).as_ref())
            );
            assert_eq!(
                fc2.finalized_block_hash,
                B256::from_slice(hash_from_u8(1).as_ref())
            );
        }

        #[tokio::test]
        async fn test_syncs_unfinalized_blocks() {
            // Scenario: Finalized chain is complete, but unfinalized blocks exist and need sync
            // Local storage:  [0, 1, 2] finalized, [3, 4] unfinalized
            // Reth:           [0, 1, 2] (missing unfinalized 3, 4)
            // Expected:       Sync unfinalized blocks 3, 4

            let finalized_chain = create_exec_block_chain(&[0, 1, 2]);
            let mut unfinalized_chain = create_exec_block_chain(&[0, 1, 2, 3, 4]);
            // Extract unfinalized blocks (3, 4)
            let unfinalized_blocks: Vec<_> = unfinalized_chain.drain(3..).collect();

            let mut mock_storage = MockExecBlockStorage::new();
            let mut mock_checker = MockBlockExistenceChecker::new();
            let mock_engine = MockExecutionEngine::new();

            // Setup best_finalized_block
            let best = finalized_chain.last().cloned();
            mock_storage
                .expect_best_finalized_block()
                .returning(move || Ok(best.clone()));

            // Setup get_finalized_block_at_height
            let chain_for_height = finalized_chain.clone();
            mock_storage
                .expect_get_finalized_block_at_height()
                .returning(move |height| {
                    Ok(chain_for_height
                        .iter()
                        .find(|b| b.blocknum() == height)
                        .cloned())
                });

            // Setup get_block_payload
            mock_storage
                .expect_get_block_payload()
                .returning(move |hash| {
                    let blocknum = hash_first_byte(&hash) as u64;
                    let payload = MockPayload::new(blocknum, hash);
                    Ok(Some(ExecBlockPayload::from_bytes(
                        payload.to_bytes().unwrap(),
                    )))
                });

            // Setup get_unfinalized_blocks - return blocks 3 and 4
            let unfinalized_hashes: Vec<Hash> =
                unfinalized_blocks.iter().map(|b| b.blockhash()).collect();
            mock_storage
                .expect_get_unfinalized_blocks()
                .returning(move || Ok(unfinalized_hashes.clone()));

            // Setup get_exec_block for unfinalized blocks
            let all_blocks: Vec<_> = finalized_chain
                .iter()
                .chain(unfinalized_blocks.iter())
                .cloned()
                .collect();
            mock_storage.expect_get_exec_block().returning(move |hash| {
                Ok(all_blocks.iter().find(|b| b.blockhash() == hash).cloned())
            });

            // Reth has finalized blocks 0-2, missing unfinalized 3, 4
            setup_mock_checker_with_known_blocks(&mut mock_checker, &[0, 1, 2]);

            let result =
                sync_chainstate_to_engine_internal(&mock_storage, &mock_checker, &mock_engine)
                    .await;

            assert!(result.is_ok());
            // Should have synced 2 unfinalized blocks
            assert_eq!(mock_engine.submitted_payloads().len(), 2);

            // Verify unfinalized blocks were synced
            let payloads = mock_engine.submitted_payloads();
            assert_eq!(payloads[0].blocknum(), 3);
            assert_eq!(payloads[1].blocknum(), 4);

            // Verify forkchoice state for unfinalized blocks uses best finalized as
            // finalized_block_hash
            let forkchoice_updates = mock_engine.forkchoice_updates();
            let best_finalized_hash = hash_from_u8(2);
            for fc in forkchoice_updates.iter() {
                assert_eq!(
                    fc.finalized_block_hash,
                    B256::from_slice(best_finalized_hash.as_ref())
                );
            }
        }

        #[tokio::test]
        async fn test_no_unfinalized_sync_needed() {
            // Scenario: Unfinalized blocks already exist in Reth
            // Local storage:  [0, 1, 2] finalized, [3] unfinalized
            // Reth:           [0, 1, 2, 3] (all present)
            // Expected:       No payloads submitted

            let finalized_chain = create_exec_block_chain(&[0, 1, 2]);
            let mut unfinalized_chain = create_exec_block_chain(&[0, 1, 2, 3]);
            let unfinalized_blocks: Vec<_> = unfinalized_chain.drain(3..).collect();

            let mut mock_storage = MockExecBlockStorage::new();
            let mut mock_checker = MockBlockExistenceChecker::new();
            let mock_engine = MockExecutionEngine::new();

            // Setup best_finalized_block
            let best = finalized_chain.last().cloned();
            mock_storage
                .expect_best_finalized_block()
                .returning(move || Ok(best.clone()));

            // Setup get_finalized_block_at_height
            let chain_for_height = finalized_chain.clone();
            mock_storage
                .expect_get_finalized_block_at_height()
                .returning(move |height| {
                    Ok(chain_for_height
                        .iter()
                        .find(|b| b.blocknum() == height)
                        .cloned())
                });

            // Setup get_block_payload
            mock_storage
                .expect_get_block_payload()
                .returning(move |hash| {
                    let blocknum = hash_first_byte(&hash) as u64;
                    let payload = MockPayload::new(blocknum, hash);
                    Ok(Some(ExecBlockPayload::from_bytes(
                        payload.to_bytes().unwrap(),
                    )))
                });

            // Setup get_unfinalized_blocks - return block 3
            let unfinalized_hashes: Vec<Hash> =
                unfinalized_blocks.iter().map(|b| b.blockhash()).collect();
            mock_storage
                .expect_get_unfinalized_blocks()
                .returning(move || Ok(unfinalized_hashes.clone()));

            // Setup get_exec_block for unfinalized blocks
            let all_blocks: Vec<_> = finalized_chain
                .iter()
                .chain(unfinalized_blocks.iter())
                .cloned()
                .collect();
            mock_storage.expect_get_exec_block().returning(move |hash| {
                Ok(all_blocks.iter().find(|b| b.blockhash() == hash).cloned())
            });

            // Reth has all blocks including unfinalized
            setup_mock_checker_with_known_blocks(&mut mock_checker, &[0, 1, 2, 3]);

            let result =
                sync_chainstate_to_engine_internal(&mock_storage, &mock_checker, &mock_engine)
                    .await;

            assert!(result.is_ok());
            // No blocks should be synced (all exist in Reth)
            assert_eq!(mock_engine.submitted_payloads().len(), 0);
            assert_eq!(mock_engine.forkchoice_updates().len(), 0);
        }

        #[tokio::test]
        async fn test_mixed_finalized_and_unfinalized_sync() {
            // Scenario: Both finalized and unfinalized blocks need sync
            // Local storage:  [0, 1, 2] finalized, [3] unfinalized
            // Reth:           [0, 1] (missing finalized 2 and unfinalized 3)
            // Expected:       Sync finalized block 2, then unfinalized block 3

            let finalized_chain = create_exec_block_chain(&[0, 1, 2]);
            let mut unfinalized_chain = create_exec_block_chain(&[0, 1, 2, 3]);
            let unfinalized_blocks: Vec<_> = unfinalized_chain.drain(3..).collect();

            let mut mock_storage = MockExecBlockStorage::new();
            let mut mock_checker = MockBlockExistenceChecker::new();
            let mock_engine = MockExecutionEngine::new();

            // Setup best_finalized_block
            let best = finalized_chain.last().cloned();
            mock_storage
                .expect_best_finalized_block()
                .returning(move || Ok(best.clone()));

            // Setup get_finalized_block_at_height
            let chain_for_height = finalized_chain.clone();
            mock_storage
                .expect_get_finalized_block_at_height()
                .returning(move |height| {
                    Ok(chain_for_height
                        .iter()
                        .find(|b| b.blocknum() == height)
                        .cloned())
                });

            // Setup get_block_payload
            mock_storage
                .expect_get_block_payload()
                .returning(move |hash| {
                    let blocknum = hash_first_byte(&hash) as u64;
                    let payload = MockPayload::new(blocknum, hash);
                    Ok(Some(ExecBlockPayload::from_bytes(
                        payload.to_bytes().unwrap(),
                    )))
                });

            // Setup get_unfinalized_blocks - return block 3
            let unfinalized_hashes: Vec<Hash> =
                unfinalized_blocks.iter().map(|b| b.blockhash()).collect();
            mock_storage
                .expect_get_unfinalized_blocks()
                .returning(move || Ok(unfinalized_hashes.clone()));

            // Setup get_exec_block for all blocks
            let all_blocks: Vec<_> = finalized_chain
                .iter()
                .chain(unfinalized_blocks.iter())
                .cloned()
                .collect();
            mock_storage.expect_get_exec_block().returning(move |hash| {
                Ok(all_blocks.iter().find(|b| b.blockhash() == hash).cloned())
            });

            // Reth has blocks 0-1, missing finalized 2 and unfinalized 3
            setup_mock_checker_with_known_blocks(&mut mock_checker, &[0, 1]);

            let result =
                sync_chainstate_to_engine_internal(&mock_storage, &mock_checker, &mock_engine)
                    .await;

            assert!(result.is_ok());
            // Should have synced finalized block 2 and unfinalized block 3
            assert_eq!(mock_engine.submitted_payloads().len(), 2);

            let payloads = mock_engine.submitted_payloads();
            assert_eq!(payloads[0].blocknum(), 2); // finalized
            assert_eq!(payloads[1].blocknum(), 3); // unfinalized
        }
    }
}
