use async_trait::async_trait;
use strata_acct_types::Hash;

use super::StorageError;
use crate::ExecBlockRecord;

#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
/// Persistence for Exec Blocks
///
/// This expects blocks to be stored as "finalized" or "unfinalized"
/// "finalized" blocks should only ever have a single canonical chain
/// "unfinalized" blocks may have forks, and all such blocks need to be persisted.
pub trait ExecBlockStorage {
    /// Save block data and payload for a given block hash.
    ///
    /// Uniquely identified by `ExecBlockRecord::blockhash()`, if it exists then data will NOT be
    /// overwritten.
    async fn save_exec_block(
        &self,
        block: ExecBlockRecord,
        payload: Vec<u8>,
    ) -> Result<(), StorageError>;

    /// Insert first block to local view of canonical finalized chain (ie. genesis block)
    ///
    /// If finalized chain is not empty, this will check that block 0 is expected genesis hash.
    async fn init_finalized_chain(&self, hash: Hash) -> Result<(), StorageError>;

    /// Extend local view of canonical finalized chain with specified block hash.
    /// Finalized chain must not be empty.
    async fn extend_finalized_chain(&self, hash: Hash) -> Result<(), StorageError>;

    /// Revert local view of canonical finalized chain to specified height.
    async fn revert_finalized_chain(&self, to_height: u64) -> Result<(), StorageError>;

    /// Remove all block data below specified height.
    async fn prune_block_data(&self, to_height: u64) -> Result<(), StorageError>;

    /// Get exec block for the highest blocknum available in the local view of canonical chain.
    async fn best_finalized_block(&self) -> Result<Option<ExecBlockRecord>, StorageError>;

    /// Get height of block if it exists in local view of canonical chain.
    async fn get_finalized_height(&self, hash: Hash) -> Result<Option<u64>, StorageError>;

    /// Get all blocks in db with height > finalized height.
    /// The blockhashes should be ordered by incrementing height.
    async fn get_unfinalized_blocks(&self) -> Result<Vec<Hash>, StorageError>;

    /// Get block data for a specified block, if it exists.
    async fn get_exec_block(&self, hash: Hash) -> Result<Option<ExecBlockRecord>, StorageError>;

    /// Get block payload for a specified block, if it exists.
    async fn get_block_payload(&self, hash: Hash) -> Result<Option<Vec<u8>>, StorageError>;
}
