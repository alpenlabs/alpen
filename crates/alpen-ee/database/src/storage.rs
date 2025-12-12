use std::{num::NonZeroUsize, sync::Arc};

use alpen_ee_common::{
    EeAccountStateAtEpoch, ExecBlockPayload, ExecBlockRecord, ExecBlockStorage, OLBlockOrEpoch,
    Storage, StorageError,
};
use async_trait::async_trait;
use strata_acct_types::Hash;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::{EpochCommitment, OLBlockId};
use strata_storage_common::cache::CacheTable;
use threadpool::ThreadPool;

use crate::{
    database::{ops, EeNodeDb},
    DbError,
};

/// Storage implementation for EE node with caching.
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct EeNodeStorage {
    ops: ops::EeNodeOps,
    blockid_cache: CacheTable<u32, Option<OLBlockId>, DbError>,
    account_state_cache: CacheTable<OLBlockId, Option<EeAccountStateAtEpoch>, DbError>,
}

impl EeNodeStorage {
    pub(crate) fn new(pool: ThreadPool, db: Arc<impl EeNodeDb + 'static>) -> Self {
        let ops = ops::Context::new(db).into_ops(pool);
        let blockid_cache = CacheTable::new(NonZeroUsize::new(64).expect("64 is always NonZero"));
        let account_state_cache =
            CacheTable::new(NonZeroUsize::new(64).expect("64 is always NonZero"));

        Self {
            ops,
            blockid_cache,
            account_state_cache,
        }
    }
}

#[async_trait]
impl Storage for EeNodeStorage {
    /// Get EE account internal state corresponding to a given OL epoch.
    async fn ee_account_state(
        &self,
        block_or_epoch: OLBlockOrEpoch,
    ) -> Result<Option<EeAccountStateAtEpoch>, StorageError> {
        let block_id = match block_or_epoch {
            OLBlockOrEpoch::TerminalBlock(block_id) => block_id,
            OLBlockOrEpoch::Epoch(epoch) => self
                .blockid_cache
                .get_or_fetch(&epoch, || self.ops.get_ol_blockid_chan(epoch))
                .await?
                .ok_or(StorageError::StateNotFound(epoch.into()))?,
        };

        self.account_state_cache
            .get_or_fetch(&block_id, || self.ops.ee_account_state_chan(block_id))
            .await
            .map_err(Into::into)
    }

    /// Get EE account internal state for the highest epoch available.
    async fn best_ee_account_state(&self) -> Result<Option<EeAccountStateAtEpoch>, StorageError> {
        self.ops
            .best_ee_account_state_async()
            .await
            .map_err(Into::into)
    }

    /// Store EE account internal state for next epoch.
    async fn store_ee_account_state(
        &self,
        ol_epoch: &EpochCommitment,
        ee_account_state: &EeAccountState,
    ) -> Result<(), StorageError> {
        self.ops
            .store_ee_account_state_async(*ol_epoch, ee_account_state.clone())
            .await?;
        // insertion successful
        // existing cache entries at this location should be purged
        // in case old `None` values are present in them
        self.blockid_cache.purge_async(&ol_epoch.epoch()).await;
        self.account_state_cache
            .purge_async(ol_epoch.last_blkid())
            .await;

        Ok(())
    }

    /// Remove stored EE internal account state for epochs > `to_epoch`.
    async fn rollback_ee_account_state(&self, to_epoch: u32) -> Result<(), StorageError> {
        self.ops.rollback_ee_account_state_async(to_epoch).await?;

        // rollback successful
        // now purge existing entries for epochs > to_epoch
        self.blockid_cache
            .purge_if_async(|epoch| *epoch > to_epoch)
            .await;
        // purge everything instead of checking individual block_ids
        self.account_state_cache.async_clear().await;

        Ok(())
    }
}

#[async_trait]
impl ExecBlockStorage for EeNodeStorage {
    /// Save block data and payload for a given block hash
    async fn save_exec_block(
        &self,
        block: ExecBlockRecord,
        payload: ExecBlockPayload,
    ) -> Result<(), StorageError> {
        self.ops
            .save_exec_block_async(block, payload.to_bytes())
            .await
            .map_err(Into::into)
    }

    /// Insert first block to local view of canonical finalized chain (ie. genesis block)
    async fn init_finalized_chain(&self, hash: Hash) -> Result<(), StorageError> {
        self.ops
            .init_finalized_chain_async(hash)
            .await
            .map_err(Into::into)
    }

    /// Extend local view of canonical chain with specified block hash
    async fn extend_finalized_chain(&self, hash: Hash) -> Result<(), StorageError> {
        self.ops
            .extend_finalized_chain_async(hash)
            .await
            .map_err(Into::into)
    }

    /// Revert local view of canonical chain to specified height
    async fn revert_finalized_chain(&self, to_height: u64) -> Result<(), StorageError> {
        self.ops
            .revert_finalized_chain_async(to_height)
            .await
            .map_err(Into::into)
    }

    /// Remove all block data below specified height
    async fn prune_block_data(&self, to_height: u64) -> Result<(), StorageError> {
        self.ops
            .prune_block_data_async(to_height)
            .await
            .map_err(Into::into)
    }

    /// Get exec block for the highest blocknum available in the local view of canonical chain.
    async fn best_finalized_block(&self) -> Result<Option<ExecBlockRecord>, StorageError> {
        self.ops
            .best_finalized_block_async()
            .await
            .map_err(Into::into)
    }

    /// Get height of block if it exists in local view of canonical chain.
    async fn get_finalized_height(&self, hash: Hash) -> Result<Option<u64>, StorageError> {
        self.ops
            .get_finalized_height_async(hash)
            .await
            .map_err(Into::into)
    }

    /// Get all blocks in db with height > finalized height
    async fn get_unfinalized_blocks(&self) -> Result<Vec<Hash>, StorageError> {
        self.ops
            .get_unfinalized_blocks_async()
            .await
            .map_err(Into::into)
    }

    /// Get block data for a specified block, if it exits.
    async fn get_exec_block(&self, hash: Hash) -> Result<Option<ExecBlockRecord>, StorageError> {
        self.ops
            .get_exec_block_async(hash)
            .await
            .map_err(Into::into)
    }

    /// Get block payload for a specified block, if it exists.
    async fn get_block_payload(
        &self,
        hash: Hash,
    ) -> Result<Option<ExecBlockPayload>, StorageError> {
        self.ops
            .get_block_payload_async(hash)
            .await
            .map(|maybe_bytes| maybe_bytes.map(ExecBlockPayload::from_bytes))
            .map_err(Into::into)
    }

    /// Delete a single block and its payload by hash.
    async fn delete_exec_block(&self, hash: Hash) -> Result<(), StorageError> {
        self.ops
            .delete_exec_block_async(hash)
            .await
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use alpen_ee_common::{exec_block_storage_tests, storage_tests};
    use strata_db_store_sled::SledDbConfig;
    use typed_sled::SledDb;

    use super::*;
    use crate::sleddb::EeNodeDBSled;

    fn setup_storage() -> EeNodeStorage {
        // Create a temporary sled database
        let db = sled::Config::new().temporary(true).open().unwrap();
        let sled_db = SledDb::new(db).unwrap();
        let config = SledDbConfig::test();

        let ee_node_db = EeNodeDBSled::new(Arc::new(sled_db), config).unwrap();
        let pool = threadpool::ThreadPool::new(4);

        EeNodeStorage::new(pool, Arc::new(ee_node_db))
    }

    storage_tests!(setup_storage());
    exec_block_storage_tests!(setup_storage());
}
