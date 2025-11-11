use std::sync::Arc;

use alpen_ee_common::{EeAccountStateAtBlock, OLBlockOrSlot, Storage, StorageError};
use async_trait::async_trait;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::{OLBlockCommitment, OLBlockId};
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
    blockid_cache: CacheTable<u64, Option<OLBlockId>, DbError>,
    account_state_cache: CacheTable<OLBlockId, Option<EeAccountStateAtBlock>, DbError>,
}

impl EeNodeStorage {
    pub(crate) fn new(pool: ThreadPool, db: Arc<impl EeNodeDb + 'static>) -> Self {
        let ops = ops::Context::new(db).into_ops(pool);
        let blockid_cache = CacheTable::new(64.try_into().unwrap());
        let account_state_cache = CacheTable::new(64.try_into().unwrap());

        Self {
            ops,
            blockid_cache,
            account_state_cache,
        }
    }
}

#[async_trait]
impl Storage for EeNodeStorage {
    /// Get EE account internal state corresponding to a given OL slot.
    async fn ee_account_state(
        &self,
        block_or_slot: OLBlockOrSlot,
    ) -> Result<Option<EeAccountStateAtBlock>, StorageError> {
        let block_id = match block_or_slot {
            OLBlockOrSlot::Block(block_id) => block_id,
            OLBlockOrSlot::Slot(slot) => self
                .blockid_cache
                .get_or_fetch(&slot, || self.ops.get_ol_blockid_chan(slot))
                .await?
                .ok_or(StorageError::StateNotFound(slot))?,
        };

        self.account_state_cache
            .get_or_fetch(&block_id, || self.ops.ee_account_state_chan(block_id))
            .await
            .map_err(Into::into)
    }

    /// Get EE account internal state for the highest slot available.
    async fn best_ee_account_state(&self) -> Result<Option<EeAccountStateAtBlock>, StorageError> {
        self.ops
            .best_ee_account_state_async()
            .await
            .map_err(Into::into)
    }

    /// Store EE account internal state for next slot.
    async fn store_ee_account_state(
        &self,
        ol_block: &OLBlockCommitment,
        ee_account_state: &EeAccountState,
    ) -> Result<(), StorageError> {
        self.ops
            .store_ee_account_state_async(*ol_block, ee_account_state.clone())
            .await?;
        // insertion successful
        // existing cache entries at this location should be purged
        // in case old `None` values are present in them
        self.blockid_cache.purge_async(&ol_block.slot()).await;
        self.account_state_cache.purge_async(ol_block.blkid()).await;

        Ok(())
    }

    /// Remove stored EE internal account state for slots > `to_slot`.
    async fn rollback_ee_account_state(&self, to_slot: u64) -> Result<(), StorageError> {
        self.ops.rollback_ee_account_state_async(to_slot).await?;

        // rollback successful
        // now purge existing entries for slots > to_slot
        self.blockid_cache
            .purge_if_async(|slot| *slot > to_slot)
            .await;
        // purge everything instead of checking individual block_ids
        self.account_state_cache.async_clear().await;

        Ok(())
    }
}
