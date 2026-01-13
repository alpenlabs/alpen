use std::sync::Arc;

use strata_asm_common::AsmManifest;
use strata_common::instrumentation::components;
use strata_db_types::{traits::L1Database, DbError, DbResult};
use strata_primitives::l1::L1BlockId;
use threadpool::ThreadPool;
use tracing::{error, instrument};

use crate::{cache::CacheTable, ops};

/// Caching manager of L1 block data
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct L1BlockManager {
    ops: ops::l1::L1DataOps,
    manifest_cache: CacheTable<L1BlockId, Option<AsmManifest>>,
    blockheight_cache: CacheTable<u64, Option<L1BlockId>>,
}

impl L1BlockManager {
    /// Create new instance of [`L1BlockManager`]
    pub fn new(pool: ThreadPool, db: Arc<impl L1Database + 'static>) -> Self {
        let ops = ops::l1::Context::new(db).into_ops(pool);
        let manifest_cache = CacheTable::new(64.try_into().unwrap());
        let blockheight_cache = CacheTable::new(64.try_into().unwrap());
        Self {
            ops,
            manifest_cache,
            blockheight_cache,
        }
    }

    /// Save an [`AsmManifest`] to database. Does not add block to tracked canonical chain.
    #[instrument(
        skip(self, manifest),
        fields(
            component = components::STORAGE_L1,
            block_id = %manifest.blkid(),
            height = manifest.height(),
        )
    )]
    pub fn put_block_data(&self, manifest: AsmManifest) -> DbResult<()> {
        let blockid = manifest.blkid();
        self.manifest_cache.purge_blocking(blockid);
        self.ops.put_block_data_blocking(manifest)
    }

    /// Save an [`AsmManifest`] to database. Does not add block to tracked canonical chain.
    #[instrument(
        skip(self, manifest),
        fields(
            component = components::STORAGE_L1,
            block_id = %manifest.blkid(),
            height = manifest.height(),
        )
    )]
    pub async fn put_block_data_async(&self, manifest: AsmManifest) -> DbResult<()> {
        let blockid = manifest.blkid();
        self.manifest_cache.purge_async(blockid).await;
        self.ops.put_block_data_async(manifest).await
    }

    /// Append [`L1BlockId`] to tracked canonical chain at the specified height.
    // Note: In the new architecture, btcio stores chain tracking data first,
    // then the ASM worker stores manifests asynchronously.
    #[instrument(
        skip(self),
        fields(
            component = components::STORAGE_L1,
            block_id = %blockid,
            height = height,
        )
    )]
    pub fn extend_canonical_chain(&self, blockid: &L1BlockId, height: u64) -> DbResult<()> {
        if let Some((tip_height, _tip_blockid)) = self.get_canonical_chain_tip()? {
            if height != tip_height + 1 {
                error!(expected = %(tip_height + 1), got = %height, "attempted to extend canonical chain out of order");
                return Err(DbError::OooInsert("l1block", height));
            }

            // Note: Chain continuity validation happens in the ASM STF's PoW verification
        };

        self.ops
            .set_canonical_chain_entry_blocking(height, *blockid)
    }

    /// Append [`L1BlockId`] to tracked canonical chain at the specified height.
    // Note: In the new architecture, btcio stores chain tracking data first,
    // then the ASM worker stores manifests asynchronously.
    #[instrument(
        skip(self),
        fields(
            component = components::STORAGE_L1,
            block_id = %blockid,
            height = height,
        )
    )]
    pub async fn extend_canonical_chain_async(
        &self,
        blockid: &L1BlockId,
        height: u64,
    ) -> DbResult<()> {
        if let Some((tip_height, _tip_blockid)) = self.get_canonical_chain_tip_async().await? {
            if height != tip_height + 1 {
                error!(expected = %(tip_height + 1), got = %height, "attempted to extend canonical chain out of order");
                return Err(DbError::OooInsert("l1block", height));
            }

            // Note: Chain continuity validation happens in the ASM STF's PoW verification
        };

        self.ops
            .set_canonical_chain_entry_async(height, *blockid)
            .await
    }

    /// Reverts tracked canonical chain to `height`.
    /// `height` must be less than tracked canonical chain height.
    #[instrument(
        skip(self),
        fields(
            component = components::STORAGE_L1,
            revert_to_height = height,
        )
    )]
    pub fn revert_canonical_chain(&self, height: u64) -> DbResult<()> {
        let Some((tip_height, _)) = self.ops.get_canonical_chain_tip_blocking()? else {
            // no chain to revert
            // but clear cache anyway for sanity
            self.blockheight_cache.blocking_clear();
            return Err(DbError::L1CanonicalChainEmpty);
        };

        if height > tip_height {
            return Err(DbError::L1InvalidRevertHeight(height, tip_height));
        }

        // clear item from cache for range height +1..=tip_height
        self.blockheight_cache
            .purge_if_blocking(|h| height < *h && *h <= tip_height);

        self.ops
            .remove_canonical_chain_entries_blocking(height + 1, tip_height)
    }

    /// Reverts tracked canonical chain to `height`.
    /// `height` must be less than tracked canonical chain height.
    #[instrument(
        skip(self),
        fields(
            component = components::STORAGE_L1,
            revert_to_height = height,
        )
    )]
    pub async fn revert_canonical_chain_async(&self, height: u64) -> DbResult<()> {
        let Some((tip_height, _)) = self.ops.get_canonical_chain_tip_async().await? else {
            // no chain to revert
            // but clear cache anyway for sanity
            self.blockheight_cache.blocking_clear();

            return Err(DbError::L1CanonicalChainEmpty);
        };

        if height > tip_height {
            return Err(DbError::L1InvalidRevertHeight(height, tip_height));
        }

        // clear item from cache for range height +1..=tip_height
        self.blockheight_cache
            .purge_if_async(|h| height < *h && *h <= tip_height)
            .await;

        self.ops
            .remove_canonical_chain_entries_async(height + 1, tip_height)
            .await
    }

    // Get tracked canonical chain tip height and blockid.
    pub fn get_canonical_chain_tip(&self) -> DbResult<Option<(u64, L1BlockId)>> {
        self.ops.get_canonical_chain_tip_blocking()
    }

    // Get tracked canonical chain tip height and blockid.
    pub async fn get_canonical_chain_tip_async(&self) -> DbResult<Option<(u64, L1BlockId)>> {
        self.ops.get_canonical_chain_tip_async().await
    }

    // Get tracked canonical chain tip height.
    pub fn get_chain_tip_height(&self) -> DbResult<Option<u64>> {
        Ok(self.get_canonical_chain_tip()?.map(|(height, _)| height))
    }

    // Get tracked canonical chain tip height.
    pub async fn get_chain_tip_height_async(&self) -> DbResult<Option<u64>> {
        Ok(self
            .get_canonical_chain_tip_async()
            .await?
            .map(|(height, _)| height))
    }

    // Get [`AsmManifest`] for given [`L1BlockId`].
    pub fn get_block_manifest(&self, blockid: &L1BlockId) -> DbResult<Option<AsmManifest>> {
        self.manifest_cache
            .get_or_fetch_blocking(blockid, || self.ops.get_block_manifest_blocking(*blockid))
    }

    // Get [`AsmManifest`] for given [`L1BlockId`].
    pub async fn get_block_manifest_async(
        &self,
        blockid: &L1BlockId,
    ) -> DbResult<Option<AsmManifest>> {
        self.manifest_cache
            .get_or_fetch(blockid, || self.ops.get_block_manifest_chan(*blockid))
            .await
    }

    // Get [`AsmManifest`] at `height` in tracked canonical chain.
    pub fn get_block_manifest_at_height(&self, height: u64) -> DbResult<Option<AsmManifest>> {
        let Some(blockid) = self.get_canonical_blockid_at_height(height)? else {
            return Ok(None);
        };

        self.get_block_manifest(&blockid)
    }

    // Get [`AsmManifest`] at `height` in tracked canonical chain.
    pub async fn get_block_manifest_at_height_async(
        &self,
        height: u64,
    ) -> DbResult<Option<AsmManifest>> {
        let Some(blockid) = self.get_canonical_blockid_at_height_async(height).await? else {
            return Ok(None);
        };

        self.get_block_manifest_async(&blockid).await
    }

    // Get [`L1BlockId`] at `height` in tracked canonical chain.
    pub fn get_canonical_blockid_at_height(&self, height: u64) -> DbResult<Option<L1BlockId>> {
        self.blockheight_cache.get_or_fetch_blocking(&height, || {
            self.ops.get_canonical_blockid_at_height_blocking(height)
        })
    }

    // Get [`L1BlockId`] at `height` in tracked canonical chain.
    pub async fn get_canonical_blockid_at_height_async(
        &self,
        height: u64,
    ) -> DbResult<Option<L1BlockId>> {
        self.blockheight_cache
            .get_or_fetch(&height, || {
                self.ops.get_canonical_blockid_at_height_chan(height)
            })
            .await
    }

    pub fn get_canonical_blockid_range(
        &self,
        start_idx: u64,
        end_idx: u64,
    ) -> DbResult<Vec<L1BlockId>> {
        self.ops
            .get_canonical_blockid_range_blocking(start_idx, end_idx)
    }

    pub async fn get_canonical_blockid_range_async(
        &self,
        start_idx: u64,
        end_idx: u64,
    ) -> DbResult<Vec<L1BlockId>> {
        self.ops
            .get_canonical_blockid_range_async(start_idx, end_idx)
            .await
    }
}
