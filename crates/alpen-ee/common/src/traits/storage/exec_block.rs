use async_trait::async_trait;
use strata_acct_types::Hash;

use super::StorageError;
use crate::{ExecBlockPayload, ExecBlockRecord};

#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
/// Persistence for Exec Blocks
///
/// This expects blocks to be stored as "finalized" or "unfinalized".
/// "finalized" blocks should only ever have a single canonical chain.
/// "unfinalized" blocks may have forks, and all such blocks need to be persisted.
pub trait ExecBlockStorage {
    /// Save block data and payload for a given block hash.
    ///
    /// Blocks are uniquely identified by `ExecBlockRecord::blockhash()`. If a block with the same
    /// hash already exists, the original data is preserved (no overwrite). Succeeds in both cases.
    async fn save_exec_block(
        &self,
        block: ExecBlockRecord,
        payload: ExecBlockPayload,
    ) -> Result<(), StorageError>;

    /// Initialize the finalized chain with a genesis block.
    ///
    /// The block must have been previously saved via `save_exec_block`. Should be idempotent:
    /// calling multiple times with the same hash should succeed. Fails if block doesn't exist.
    async fn init_finalized_chain(&self, hash: Hash) -> Result<(), StorageError>;

    /// Extend the finalized chain by one block.
    ///
    /// The block must have been saved via `save_exec_block` and its parent hash must match the
    /// current best finalized block. Fails if chain is empty, block doesn't exist, or parent
    /// hash doesn't match.
    async fn extend_finalized_chain(&self, hash: Hash) -> Result<(), StorageError>;

    /// Revert the finalized chain to a specified height.
    ///
    /// All blocks above `to_height` are removed from the finalized chain and become unfinalized.
    /// Block data is preserved. Fails if chain is empty.
    async fn revert_finalized_chain(&self, to_height: u64) -> Result<(), StorageError>;

    /// Permanently delete all block data and payloads below the specified height.
    ///
    /// Removes blocks and their payloads at heights < `to_height`. Block data cannot be recovered
    /// after pruning.
    async fn prune_block_data(&self, to_height: u64) -> Result<(), StorageError>;

    /// Get the highest block in the finalized chain.
    ///
    /// Returns `None` if the finalized chain is empty (not yet initialized). Returns the block
    /// with the highest block number that has been finalized.
    async fn best_finalized_block(&self) -> Result<Option<ExecBlockRecord>, StorageError>;

    /// Get the finalized block at a specific height.
    ///
    /// Returns `None` if no block is finalized at the given height. Returns the block
    /// that is finalized at the specified height.
    async fn get_finalized_block_at_height(
        &self,
        height: u64,
    ) -> Result<Option<ExecBlockRecord>, StorageError>;

    /// Get the finalized height of a block, if it exists on the finalized chain.
    ///
    /// Returns `None` if the block doesn't exist, hasn't been finalized, or exists at a height
    /// but a different block hash is finalized at that height (fork case).
    async fn get_finalized_height(&self, hash: Hash) -> Result<Option<u64>, StorageError>;

    /// Get all unfinalized blocks (height > best finalized height).
    ///
    /// Returns block hashes ordered by incrementing height. May include multiple blocks at the
    /// same height (forks). Fails if finalized chain is empty.
    async fn get_unfinalized_blocks(&self) -> Result<Vec<Hash>, StorageError>;

    /// Get block data for a block by its hash.
    ///
    /// Returns `None` if the block doesn't exist. Works for both finalized and unfinalized blocks.
    async fn get_exec_block(&self, hash: Hash) -> Result<Option<ExecBlockRecord>, StorageError>;

    /// Get block payload for a block by its hash.
    ///
    /// Returns `None` if the block doesn't exist. Works for both finalized and unfinalized blocks.
    async fn get_block_payload(&self, hash: Hash)
        -> Result<Option<ExecBlockPayload>, StorageError>;

    /// Delete a single block and its payload by hash.
    ///
    /// Removes the block data, payload, and updates the height index. Returns an error if the
    /// block is in the finalized chain. Returns `Ok(())` if the block doesn't exist (idempotent).
    async fn delete_exec_block(&self, hash: Hash) -> Result<(), StorageError>;
}

/// Macro to instantiate all ExecBlockStorage tests for a given storage setup.
#[cfg(feature = "test-utils")]
#[macro_export]
macro_rules! exec_block_storage_tests {
    ($setup_expr:expr) => {
        #[tokio::test]
        async fn test_save_and_get_exec_block() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_save_and_get_exec_block(&storage).await;
        }

        #[tokio::test]
        async fn test_save_duplicate_block_hash() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_save_duplicate_block_hash(&storage).await;
        }

        #[tokio::test]
        async fn test_init_finalized_chain() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_init_finalized_chain(&storage).await;
        }

        #[tokio::test]
        async fn test_init_finalized_chain_twice() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_init_finalized_chain_twice(&storage).await;
        }

        #[tokio::test]
        async fn test_init_finalized_chain_missing_block() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_init_finalized_chain_missing_block(&storage)
                .await;
        }

        #[tokio::test]
        async fn test_init_finalized_chain_different_genesis() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_init_finalized_chain_different_genesis(
                &storage,
            )
            .await;
        }

        #[tokio::test]
        async fn test_init_finalized_chain_after_extend() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_init_finalized_chain_after_extend(&storage)
                .await;
        }

        #[tokio::test]
        async fn test_extend_finalized_chain() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_extend_finalized_chain(&storage).await;
        }

        #[tokio::test]
        async fn test_extend_before_init() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_extend_before_init(&storage).await;
        }

        #[tokio::test]
        async fn test_extend_with_wrong_parent() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_extend_with_wrong_parent(&storage).await;
        }

        #[tokio::test]
        async fn test_extend_missing_block() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_extend_missing_block(&storage).await;
        }

        #[tokio::test]
        async fn test_best_finalized_block() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_best_finalized_block(&storage).await;
        }

        #[tokio::test]
        async fn test_best_finalized_block_empty() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_best_finalized_block_empty(&storage).await;
        }

        #[tokio::test]
        async fn test_get_finalized_height() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_get_finalized_height(&storage).await;
        }

        #[tokio::test]
        async fn test_get_finalized_height_unfinalized_block() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_get_finalized_height_unfinalized_block(
                &storage,
            )
            .await;
        }

        #[tokio::test]
        async fn test_get_finalized_height_fork_block() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_get_finalized_height_fork_block(&storage)
                .await;
        }

        #[tokio::test]
        async fn test_get_unfinalized_blocks() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_get_unfinalized_blocks(&storage).await;
        }

        #[tokio::test]
        async fn test_get_unfinalized_blocks_with_forks() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_get_unfinalized_blocks_with_forks(&storage)
                .await;
        }

        #[tokio::test]
        async fn test_get_unfinalized_blocks_empty_chain() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_get_unfinalized_blocks_empty_chain(&storage)
                .await;
        }

        #[tokio::test]
        async fn test_finalized_chain_sequence() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_finalized_chain_sequence(&storage).await;
        }

        #[tokio::test]
        async fn test_revert_finalized_chain() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_revert_finalized_chain(&storage).await;
        }

        #[tokio::test]
        async fn test_prune_block_data() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_prune_block_data(&storage).await;
        }

        #[tokio::test]
        async fn test_delete_unfinalized_block() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_delete_unfinalized_block(&storage).await;
        }

        #[tokio::test]
        async fn test_delete_finalized_block_fails() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_delete_finalized_block_fails(&storage).await;
        }

        #[tokio::test]
        async fn test_delete_nonexistent_block() {
            let storage = $setup_expr;
            $crate::exec_block_storage_test_fns::test_delete_nonexistent_block(&storage).await;
        }
    };
}

#[cfg(feature = "test-utils")]
pub mod exec_block_storage_test_fns {
    use strata_acct_types::{BitcoinAmount, Hash};
    use strata_ee_acct_types::EeAccountState;
    use strata_ee_chain_types::{BlockInputs, BlockOutputs, ExecBlockCommitment, ExecBlockPackage};
    use strata_identifiers::{Buf32, OLBlockCommitment, OLBlockId};

    use super::*;
    use crate::ExecBlockRecord;

    /// Helper to create a test block hash from a u8 value
    fn hash_from_u8(value: u8) -> Hash {
        Hash::from(Buf32::new([value; 32]))
    }

    /// Helper to create a test OL block commitment
    fn create_ol_block(slot: u64, value: u8) -> OLBlockCommitment {
        OLBlockCommitment::new(slot, OLBlockId::from(Buf32::new([value; 32])))
    }

    /// Helper to create a minimal valid ExecBlockPackage for testing
    fn create_package(exec_block_id: Hash, raw_block_encoded_hash: Hash) -> ExecBlockPackage {
        ExecBlockPackage::new(
            ExecBlockCommitment::new(exec_block_id, raw_block_encoded_hash),
            BlockInputs::new_empty(),
            BlockOutputs::new_empty(),
        )
    }

    /// Helper to create an EeAccountState with a specific block hash
    fn create_account_state(blockhash: Hash) -> EeAccountState {
        EeAccountState::new(blockhash, BitcoinAmount::ZERO, Vec::new(), Vec::new())
    }

    /// Helper to create a test ExecBlockRecord with proper parent-child relationship
    pub fn create_exec_block(
        blocknum: u64,
        parent_hash: Hash,
        block_hash: Hash,
        ol_slot: u64,
    ) -> ExecBlockRecord {
        let package = create_package(block_hash, block_hash);
        let account_state = create_account_state(block_hash);
        let ol_block = create_ol_block(ol_slot, blocknum as u8);
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

    /// Test saving and retrieving a single exec block
    pub async fn test_save_and_get_exec_block(storage: &impl ExecBlockStorage) {
        let hash = hash_from_u8(1);
        let parent_hash = hash_from_u8(0);
        let block = create_exec_block(1, parent_hash, hash, 100);
        let payload = ExecBlockPayload::from_bytes(vec![1, 2, 3, 4]);

        // Save the block
        storage
            .save_exec_block(block.clone(), payload.clone())
            .await
            .unwrap();

        // Retrieve the block
        let retrieved_block = storage.get_exec_block(hash).await.unwrap().unwrap();
        assert_eq!(retrieved_block.blockhash(), hash);
        assert_eq!(retrieved_block.blocknum(), 1);
        assert_eq!(retrieved_block.parent_blockhash(), parent_hash);

        // Retrieve the payload
        let retrieved_payload = storage.get_block_payload(hash).await.unwrap().unwrap();
        assert_eq!(retrieved_payload, payload);
    }

    /// Test saving a block with duplicate hash (should not overwrite per docs)
    pub async fn test_save_duplicate_block_hash(storage: &impl ExecBlockStorage) {
        let hash = hash_from_u8(1);
        let parent_hash = hash_from_u8(0);
        let block1 = create_exec_block(1, parent_hash, hash, 100);
        let payload1 = ExecBlockPayload::from_bytes(vec![1, 2, 3]);

        // Save first block
        storage
            .save_exec_block(block1, payload1.clone())
            .await
            .unwrap();

        // Try to save second block with same hash but different data
        let block2 = create_exec_block(1, parent_hash, hash, 101);
        let payload2 = ExecBlockPayload::from_bytes(vec![4, 5, 6]);

        // Should succeed but not overwrite
        storage.save_exec_block(block2, payload2).await.unwrap();

        // Original payload should still be there
        let retrieved_payload = storage.get_block_payload(hash).await.unwrap().unwrap();
        assert_eq!(retrieved_payload, payload1);
    }

    /// Test initializing finalized chain with genesis block
    pub async fn test_init_finalized_chain(storage: &impl ExecBlockStorage) {
        let genesis_hash = hash_from_u8(0);
        let genesis_block = create_exec_block(0, Hash::default(), genesis_hash, 0);

        // Save genesis block first
        storage
            .save_exec_block(genesis_block.clone(), ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();

        // Initialize finalized chain
        storage.init_finalized_chain(genesis_hash).await.unwrap();

        // Genesis should be the best finalized block
        let best = storage.best_finalized_block().await.unwrap().unwrap();
        assert_eq!(best.blockhash(), genesis_hash);
        assert_eq!(best.blocknum(), 0);

        // Genesis should have finalized height 0
        let height = storage
            .get_finalized_height(genesis_hash)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(height, 0);
    }

    /// Test initializing finalized chain twice (should be idempotent/succeed per requirements)
    pub async fn test_init_finalized_chain_twice(storage: &impl ExecBlockStorage) {
        let genesis_hash = hash_from_u8(0);
        let genesis_block = create_exec_block(0, Hash::default(), genesis_hash, 0);

        // Save genesis block
        storage
            .save_exec_block(genesis_block, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();

        // First init
        storage.init_finalized_chain(genesis_hash).await.unwrap();

        // Second init with same hash - should succeed (idempotent)
        storage.init_finalized_chain(genesis_hash).await.unwrap();

        // Verify genesis is still the best finalized block
        let best = storage.best_finalized_block().await.unwrap().unwrap();
        assert_eq!(best.blockhash(), genesis_hash);
        assert_eq!(best.blocknum(), 0);
    }

    /// Test initializing finalized chain with non-existent block
    pub async fn test_init_finalized_chain_missing_block(storage: &impl ExecBlockStorage) {
        let missing_hash = hash_from_u8(99);

        // Should fail because block doesn't exist
        let result = storage.init_finalized_chain(missing_hash).await;
        assert!(result.is_err());
    }

    /// Test initializing finalized chain with a different genesis hash after already initialized
    pub async fn test_init_finalized_chain_different_genesis(storage: &impl ExecBlockStorage) {
        let genesis_hash_a = hash_from_u8(0);
        let genesis_hash_b = hash_from_u8(1);

        // Create two different genesis blocks
        let genesis_block_a = create_exec_block(0, Hash::default(), genesis_hash_a, 0);
        let genesis_block_b = create_exec_block(0, Hash::default(), genesis_hash_b, 0);

        // Save both blocks
        storage
            .save_exec_block(genesis_block_a, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();
        storage
            .save_exec_block(genesis_block_b, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();

        // Initialize with first genesis
        storage.init_finalized_chain(genesis_hash_a).await.unwrap();

        // Try to initialize with different genesis - should fail
        let result = storage.init_finalized_chain(genesis_hash_b).await;
        assert!(result.is_err());
    }

    /// Test initializing finalized chain after chain has been extended beyond genesis.
    /// Should succeed (idempotent) if the genesis hash matches, making no changes.
    pub async fn test_init_finalized_chain_after_extend(storage: &impl ExecBlockStorage) {
        let genesis_hash = hash_from_u8(0);
        let block1_hash = hash_from_u8(1);

        // Create genesis and block 1
        let genesis_block = create_exec_block(0, Hash::default(), genesis_hash, 0);
        let block1 = create_exec_block(1, genesis_hash, block1_hash, 1);

        // Save both blocks
        storage
            .save_exec_block(genesis_block, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();
        storage
            .save_exec_block(block1, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();

        // Initialize and extend chain
        storage.init_finalized_chain(genesis_hash).await.unwrap();
        storage.extend_finalized_chain(block1_hash).await.unwrap();

        // Try to init again with same genesis - should succeed (idempotent)
        storage.init_finalized_chain(genesis_hash).await.unwrap();

        // Chain should still be at block 1 (no changes made)
        let best = storage.best_finalized_block().await.unwrap().unwrap();
        assert_eq!(best.blockhash(), block1_hash);
        assert_eq!(best.blocknum(), 1);
    }

    /// Test extending finalized chain
    pub async fn test_extend_finalized_chain(storage: &impl ExecBlockStorage) {
        // Create and save genesis
        let hash0 = hash_from_u8(0);
        let block0 = create_exec_block(0, Hash::default(), hash0, 0);
        storage
            .save_exec_block(block0, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();
        storage.init_finalized_chain(hash0).await.unwrap();

        // Create and save block 1 (child of genesis)
        let hash1 = hash_from_u8(1);
        let block1 = create_exec_block(1, hash0, hash1, 1);
        storage
            .save_exec_block(block1, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();

        // Extend chain
        storage.extend_finalized_chain(hash1).await.unwrap();

        // Block 1 should now be best finalized
        let best = storage.best_finalized_block().await.unwrap().unwrap();
        assert_eq!(best.blockhash(), hash1);
        assert_eq!(best.blocknum(), 1);
    }

    /// Test extending before initializing
    pub async fn test_extend_before_init(storage: &impl ExecBlockStorage) {
        let hash = hash_from_u8(1);
        let block = create_exec_block(1, hash_from_u8(0), hash, 1);
        storage
            .save_exec_block(block, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();

        // Should fail because chain is not initialized
        let result = storage.extend_finalized_chain(hash).await;
        assert!(result.is_err());
    }

    /// Test extending with wrong parent hash
    pub async fn test_extend_with_wrong_parent(storage: &impl ExecBlockStorage) {
        // Initialize with genesis
        let hash0 = hash_from_u8(0);
        let block0 = create_exec_block(0, Hash::default(), hash0, 0);
        storage
            .save_exec_block(block0, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();
        storage.init_finalized_chain(hash0).await.unwrap();

        // Create block with wrong parent
        let hash1 = hash_from_u8(1);
        let wrong_parent = hash_from_u8(99);
        let block1 = create_exec_block(1, wrong_parent, hash1, 1);
        storage
            .save_exec_block(block1, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();

        // Should fail because parent doesn't match
        let result = storage.extend_finalized_chain(hash1).await;
        assert!(result.is_err());
    }

    /// Test extending with non-existent block
    pub async fn test_extend_missing_block(storage: &impl ExecBlockStorage) {
        // Initialize with genesis
        let hash0 = hash_from_u8(0);
        let block0 = create_exec_block(0, Hash::default(), hash0, 0);
        storage
            .save_exec_block(block0, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();
        storage.init_finalized_chain(hash0).await.unwrap();

        // Try to extend with non-existent block
        let missing_hash = hash_from_u8(99);
        let result = storage.extend_finalized_chain(missing_hash).await;
        assert!(result.is_err());
    }

    /// Test getting best finalized block
    pub async fn test_best_finalized_block(storage: &impl ExecBlockStorage) {
        // Build a chain: 0 -> 1 -> 2
        let hash0 = hash_from_u8(0);
        let hash1 = hash_from_u8(1);
        let hash2 = hash_from_u8(2);

        let block0 = create_exec_block(0, Hash::default(), hash0, 0);
        let block1 = create_exec_block(1, hash0, hash1, 1);
        let block2 = create_exec_block(2, hash1, hash2, 2);

        storage
            .save_exec_block(block0, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();
        storage
            .save_exec_block(block1, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();
        storage
            .save_exec_block(block2, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();

        storage.init_finalized_chain(hash0).await.unwrap();
        storage.extend_finalized_chain(hash1).await.unwrap();
        storage.extend_finalized_chain(hash2).await.unwrap();

        // Best should be block 2
        let best = storage.best_finalized_block().await.unwrap().unwrap();
        assert_eq!(best.blockhash(), hash2);
        assert_eq!(best.blocknum(), 2);
    }

    /// Test best finalized block on empty chain
    pub async fn test_best_finalized_block_empty(storage: &impl ExecBlockStorage) {
        // Should return None for empty chain
        let best = storage.best_finalized_block().await.unwrap();
        assert!(best.is_none());
    }

    /// Test getting finalized height
    pub async fn test_get_finalized_height(storage: &impl ExecBlockStorage) {
        // Create chain: 0 -> 1 -> 2
        let hash0 = hash_from_u8(0);
        let hash1 = hash_from_u8(1);
        let hash2 = hash_from_u8(2);

        let block0 = create_exec_block(0, Hash::default(), hash0, 0);
        let block1 = create_exec_block(1, hash0, hash1, 1);
        let block2 = create_exec_block(2, hash1, hash2, 2);

        storage
            .save_exec_block(block0, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();
        storage
            .save_exec_block(block1, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();
        storage
            .save_exec_block(block2, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();

        storage.init_finalized_chain(hash0).await.unwrap();
        storage.extend_finalized_chain(hash1).await.unwrap();

        // Block 0 and 1 should have finalized heights
        assert_eq!(
            storage.get_finalized_height(hash0).await.unwrap().unwrap(),
            0
        );
        assert_eq!(
            storage.get_finalized_height(hash1).await.unwrap().unwrap(),
            1
        );

        // Block 2 is saved but not finalized, should return None
        assert!(storage.get_finalized_height(hash2).await.unwrap().is_none());
    }

    /// Test getting finalized height for unfinalized block
    pub async fn test_get_finalized_height_unfinalized_block(storage: &impl ExecBlockStorage) {
        // Initialize with genesis
        let hash0 = hash_from_u8(0);
        let block0 = create_exec_block(0, Hash::default(), hash0, 0);
        storage
            .save_exec_block(block0, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();
        storage.init_finalized_chain(hash0).await.unwrap();

        // Save block 1 but don't finalize it
        let hash1 = hash_from_u8(1);
        let block1 = create_exec_block(1, hash0, hash1, 1);
        storage
            .save_exec_block(block1, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();

        // Should return None for unfinalized block
        let height = storage.get_finalized_height(hash1).await.unwrap();
        assert!(height.is_none());
    }

    /// Test getting finalized height for fork case (different block at same height)
    pub async fn test_get_finalized_height_fork_block(storage: &impl ExecBlockStorage) {
        // Initialize with genesis
        let hash0 = hash_from_u8(0);
        let block0 = create_exec_block(0, Hash::default(), hash0, 0);
        storage
            .save_exec_block(block0, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();
        storage.init_finalized_chain(hash0).await.unwrap();

        // Finalize block 1a
        let hash1a = hash_from_u8(1);
        let block1a = create_exec_block(1, hash0, hash1a, 1);
        storage
            .save_exec_block(block1a, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();
        storage.extend_finalized_chain(hash1a).await.unwrap();

        // Save block 1b - a fork at the same height but different hash
        let hash1b = hash_from_u8(11); // Different hash, same height
        let block1b = create_exec_block(1, hash0, hash1b, 2);
        storage
            .save_exec_block(block1b, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();

        // Block 1a should return height 1 (it's finalized)
        assert_eq!(
            storage.get_finalized_height(hash1a).await.unwrap().unwrap(),
            1
        );

        // Block 1b should return None (different block is finalized at that height)
        assert!(storage
            .get_finalized_height(hash1b)
            .await
            .unwrap()
            .is_none());
    }

    /// Test getting unfinalized blocks
    pub async fn test_get_unfinalized_blocks(storage: &impl ExecBlockStorage) {
        // Create chain: 0 (finalized) -> 1 (finalized) -> 2 (unfinalized) -> 3 (unfinalized)
        let hash0 = hash_from_u8(0);
        let hash1 = hash_from_u8(1);
        let hash2 = hash_from_u8(2);
        let hash3 = hash_from_u8(3);

        let block0 = create_exec_block(0, Hash::default(), hash0, 0);
        let block1 = create_exec_block(1, hash0, hash1, 1);
        let block2 = create_exec_block(2, hash1, hash2, 2);
        let block3 = create_exec_block(3, hash2, hash3, 3);

        storage
            .save_exec_block(block0, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();
        storage
            .save_exec_block(block1, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();
        storage
            .save_exec_block(block2, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();
        storage
            .save_exec_block(block3, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();

        // Finalize only blocks 0 and 1
        storage.init_finalized_chain(hash0).await.unwrap();
        storage.extend_finalized_chain(hash1).await.unwrap();

        // Should return blocks 2 and 3
        let unfinalized = storage.get_unfinalized_blocks().await.unwrap();
        assert_eq!(unfinalized.len(), 2);
        assert!(unfinalized.contains(&hash2));
        assert!(unfinalized.contains(&hash3));
    }

    /// Test getting unfinalized blocks with forks at same height
    pub async fn test_get_unfinalized_blocks_with_forks(storage: &impl ExecBlockStorage) {
        // Create chain: 0 (finalized) -> 1 (finalized) -> 2a, 2b (both unfinalized forks)
        let hash0 = hash_from_u8(0);
        let hash1 = hash_from_u8(1);
        let hash2a = hash_from_u8(2);
        let hash2b = hash_from_u8(22); // Different hash, same height

        let block0 = create_exec_block(0, Hash::default(), hash0, 0);
        let block1 = create_exec_block(1, hash0, hash1, 1);
        let block2a = create_exec_block(2, hash1, hash2a, 2);
        let block2b = create_exec_block(2, hash1, hash2b, 3); // Same height, different parent

        storage
            .save_exec_block(block0, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();
        storage
            .save_exec_block(block1, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();
        storage
            .save_exec_block(block2a, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();
        storage
            .save_exec_block(block2b, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();

        // Finalize only blocks 0 and 1
        storage.init_finalized_chain(hash0).await.unwrap();
        storage.extend_finalized_chain(hash1).await.unwrap();

        // Should return both fork blocks
        let unfinalized = storage.get_unfinalized_blocks().await.unwrap();
        assert_eq!(unfinalized.len(), 2);
        assert!(unfinalized.contains(&hash2a));
        assert!(unfinalized.contains(&hash2b));
    }

    /// Test getting unfinalized blocks on empty chain
    pub async fn test_get_unfinalized_blocks_empty_chain(storage: &impl ExecBlockStorage) {
        // Try to get unfinalized blocks without initializing the chain
        let result = storage.get_unfinalized_blocks().await;

        // Should fail because finalized chain is empty
        assert!(result.is_err());
    }

    /// Test complete finalized chain sequence
    pub async fn test_finalized_chain_sequence(storage: &impl ExecBlockStorage) {
        // Build and finalize a chain of 5 blocks
        let hashes: Vec<Hash> = (0..5).map(hash_from_u8).collect();

        // Save genesis
        let block0 = create_exec_block(0, Hash::default(), hashes[0], 0);
        storage
            .save_exec_block(block0, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();
        storage.init_finalized_chain(hashes[0]).await.unwrap();

        // Save and finalize blocks 1-4
        for i in 1..5usize {
            let block = create_exec_block(i as u64, hashes[i - 1], hashes[i], i as u64);
            storage
                .save_exec_block(block, ExecBlockPayload::from_bytes(vec![]))
                .await
                .unwrap();
            storage.extend_finalized_chain(hashes[i]).await.unwrap();
        }

        // Verify all blocks are finalized with correct heights
        for (i, hash) in hashes.iter().enumerate() {
            let height = storage.get_finalized_height(*hash).await.unwrap().unwrap();
            assert_eq!(height, i as u64);
        }

        // Best should be last block
        let best = storage.best_finalized_block().await.unwrap().unwrap();
        assert_eq!(best.blockhash(), hashes[4]);
        assert_eq!(best.blocknum(), 4);

        // Should be no unfinalized blocks
        let unfinalized = storage.get_unfinalized_blocks().await.unwrap();
        assert!(unfinalized.is_empty());
    }

    /// Test reverting finalized chain
    pub async fn test_revert_finalized_chain(storage: &impl ExecBlockStorage) {
        // Build chain: 0 -> 1 -> 2 -> 3
        let hashes: Vec<Hash> = (0..4).map(hash_from_u8).collect();

        let block0 = create_exec_block(0, Hash::default(), hashes[0], 0);
        storage
            .save_exec_block(block0, ExecBlockPayload::from_bytes(vec![]))
            .await
            .unwrap();
        storage.init_finalized_chain(hashes[0]).await.unwrap();

        for i in 1..4usize {
            let block = create_exec_block(i as u64, hashes[i - 1], hashes[i], i as u64);
            storage
                .save_exec_block(block, ExecBlockPayload::from_bytes(vec![]))
                .await
                .unwrap();
            storage.extend_finalized_chain(hashes[i]).await.unwrap();
        }

        // Verify all blocks are finalized before revert
        for hash in hashes.iter().take(4) {
            assert!(storage.get_finalized_height(*hash).await.unwrap().is_some());
        }

        // Revert to height 1
        storage.revert_finalized_chain(1).await.unwrap();

        // Best finalized should be block 1
        let best = storage.best_finalized_block().await.unwrap().unwrap();
        assert_eq!(best.blockhash(), hashes[1]);
        assert_eq!(best.blocknum(), 1);

        // Blocks 0 and 1 should still be finalized
        assert_eq!(
            storage.get_finalized_height(hashes[0]).await.unwrap(),
            Some(0)
        );
        assert_eq!(
            storage.get_finalized_height(hashes[1]).await.unwrap(),
            Some(1)
        );

        // Blocks 2 and 3 should no longer be finalized
        assert_eq!(storage.get_finalized_height(hashes[2]).await.unwrap(), None);
        assert_eq!(storage.get_finalized_height(hashes[3]).await.unwrap(), None);

        // Blocks 2 and 3 should be in unfinalized list
        let unfinalized = storage.get_unfinalized_blocks().await.unwrap();
        assert!(unfinalized.contains(&hashes[2]));
        assert!(unfinalized.contains(&hashes[3]));

        // Block data should still exist (not deleted)
        assert!(storage.get_exec_block(hashes[2]).await.unwrap().is_some());
        assert!(storage.get_exec_block(hashes[3]).await.unwrap().is_some());
    }

    /// Test pruning block data
    pub async fn test_prune_block_data(storage: &impl ExecBlockStorage) {
        // Build chain: 0 -> 1 -> 2 -> 3 -> 4
        let hashes: Vec<Hash> = (0..5).map(hash_from_u8).collect();
        let payloads: Vec<_> = (0..5)
            .map(|i| ExecBlockPayload::from_bytes(vec![i as u8; 10]))
            .collect();

        let block0 = create_exec_block(0, Hash::default(), hashes[0], 0);
        storage
            .save_exec_block(block0, payloads[0].clone())
            .await
            .unwrap();
        storage.init_finalized_chain(hashes[0]).await.unwrap();

        for i in 1..5usize {
            let block = create_exec_block(i as u64, hashes[i - 1], hashes[i], i as u64);
            storage
                .save_exec_block(block, payloads[i].clone())
                .await
                .unwrap();
            storage.extend_finalized_chain(hashes[i]).await.unwrap();
        }

        // Verify all blocks and payloads exist before pruning
        for hash in hashes.iter().take(5) {
            assert!(storage.get_exec_block(*hash).await.unwrap().is_some());
            assert!(storage.get_block_payload(*hash).await.unwrap().is_some());
        }

        // Prune blocks below height 3
        storage.prune_block_data(3).await.unwrap();

        // Blocks 0, 1, 2 should be deleted
        assert!(storage.get_exec_block(hashes[0]).await.unwrap().is_none());
        assert!(storage.get_exec_block(hashes[1]).await.unwrap().is_none());
        assert!(storage.get_exec_block(hashes[2]).await.unwrap().is_none());

        // Payloads for blocks 0, 1, 2 should also be deleted
        assert!(storage
            .get_block_payload(hashes[0])
            .await
            .unwrap()
            .is_none());
        assert!(storage
            .get_block_payload(hashes[1])
            .await
            .unwrap()
            .is_none());
        assert!(storage
            .get_block_payload(hashes[2])
            .await
            .unwrap()
            .is_none());

        // Blocks 3 and 4 should still exist
        assert!(storage.get_exec_block(hashes[3]).await.unwrap().is_some());
        assert!(storage.get_exec_block(hashes[4]).await.unwrap().is_some());

        // Payloads for blocks 3 and 4 should still exist
        assert_eq!(
            storage.get_block_payload(hashes[3]).await.unwrap().unwrap(),
            payloads[3]
        );
        assert_eq!(
            storage.get_block_payload(hashes[4]).await.unwrap().unwrap(),
            payloads[4]
        );

        // Finalized chain should still be intact (pruning doesn't affect finalization status)
        let best = storage.best_finalized_block().await.unwrap().unwrap();
        assert_eq!(best.blockhash(), hashes[4]);
        assert_eq!(best.blocknum(), 4);
    }

    /// Test deleting an unfinalized block
    pub async fn test_delete_unfinalized_block(storage: &impl ExecBlockStorage) {
        // Initialize finalized chain with genesis
        let hash0 = hash_from_u8(0);
        let block0 = create_exec_block(0, Hash::default(), hash0, 0);
        storage
            .save_exec_block(block0, ExecBlockPayload::from_bytes(vec![0]))
            .await
            .unwrap();
        storage.init_finalized_chain(hash0).await.unwrap();

        // Save and finalize block 1
        let hash1 = hash_from_u8(1);
        let block1 = create_exec_block(1, hash0, hash1, 1);
        storage
            .save_exec_block(block1, ExecBlockPayload::from_bytes(vec![1]))
            .await
            .unwrap();
        storage.extend_finalized_chain(hash1).await.unwrap();

        // Save unfinalized block 2
        let hash2 = hash_from_u8(2);
        let block2 = create_exec_block(2, hash1, hash2, 2);
        let payload2 = ExecBlockPayload::from_bytes(vec![2, 2, 2]);
        storage
            .save_exec_block(block2.clone(), payload2.clone())
            .await
            .unwrap();

        // Verify block 2 exists
        assert!(storage.get_exec_block(hash2).await.unwrap().is_some());
        assert!(storage.get_block_payload(hash2).await.unwrap().is_some());

        // Delete the unfinalized block
        storage.delete_exec_block(hash2).await.unwrap();

        // Verify block 2 is deleted
        assert!(storage.get_exec_block(hash2).await.unwrap().is_none());
        assert!(storage.get_block_payload(hash2).await.unwrap().is_none());

        // Verify finalized blocks still exist
        assert!(storage.get_exec_block(hash0).await.unwrap().is_some());
        assert!(storage.get_exec_block(hash1).await.unwrap().is_some());
    }

    /// Test attempting to delete a finalized block (should fail)
    pub async fn test_delete_finalized_block_fails(storage: &impl ExecBlockStorage) {
        // Initialize finalized chain with genesis
        let hash0 = hash_from_u8(0);
        let block0 = create_exec_block(0, Hash::default(), hash0, 0);
        storage
            .save_exec_block(block0, ExecBlockPayload::from_bytes(vec![0]))
            .await
            .unwrap();
        storage.init_finalized_chain(hash0).await.unwrap();

        // Save and finalize block 1
        let hash1 = hash_from_u8(1);
        let block1 = create_exec_block(1, hash0, hash1, 1);
        storage
            .save_exec_block(block1, ExecBlockPayload::from_bytes(vec![1]))
            .await
            .unwrap();
        storage.extend_finalized_chain(hash1).await.unwrap();

        // Attempt to delete finalized block 0 - should fail
        let result = storage.delete_exec_block(hash0).await;
        assert!(result.is_err());

        // Attempt to delete finalized block 1 - should fail
        let result = storage.delete_exec_block(hash1).await;
        assert!(result.is_err());

        // Verify both blocks still exist
        assert!(storage.get_exec_block(hash0).await.unwrap().is_some());
        assert!(storage.get_exec_block(hash1).await.unwrap().is_some());
    }

    /// Test deleting a non-existent block (should be idempotent)
    pub async fn test_delete_nonexistent_block(storage: &impl ExecBlockStorage) {
        let nonexistent_hash = hash_from_u8(99);

        // Delete should succeed even though block doesn't exist
        storage.delete_exec_block(nonexistent_hash).await.unwrap();

        // Verify it still doesn't exist
        assert!(storage
            .get_exec_block(nonexistent_hash)
            .await
            .unwrap()
            .is_none());
    }
}
