use std::{error::Error, sync::Arc};

use alpen_ee_common::{
    Batch, BatchId, BatchStatus, Chunk, ChunkId, ChunkStatus, EeAccountStateAtEpoch,
    ExecBlockRecord,
};
use strata_acct_types::Hash;
use strata_db_store_sled::SledDbConfig;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::{EpochCommitment, OLBlockId};
use tracing::{error, warn};
use typed_sled::{error::Error as TSledError, transaction::SledTransactional, SledDb, SledTree};

use super::{AccountStateAtOLEpochSchema, OLBlockAtEpochSchema};
use crate::{
    database::EeNodeDb,
    serialization_types::{
        DBAccountStateAtEpoch, DBBatchId, DBBatchWithStatus, DBChunkId, DBChunkWithStatus,
    },
    sleddb::{
        BatchByIdxSchema, BatchChunksSchema, BatchIdToIdxSchema, ChunkByIdxSchema,
        ChunkIdToIdxSchema, ExecBlockFinalizedSchema, ExecBlockPayloadSchema, ExecBlockSchema,
        ExecBlocksAtHeightSchema,
    },
    DbError, DbResult,
};

fn abort<T>(reason: impl Error + Send + Sync + 'static) -> Result<T, TSledError> {
    Err(TSledError::abort(reason))
}

pub(crate) struct EeNodeDBSled {
    ol_blockid_tree: SledTree<OLBlockAtEpochSchema>,
    account_state_tree: SledTree<AccountStateAtOLEpochSchema>,
    exec_block_tree: SledTree<ExecBlockSchema>,
    exec_blocks_by_height_tree: SledTree<ExecBlocksAtHeightSchema>,
    exec_block_finalized_tree: SledTree<ExecBlockFinalizedSchema>,
    exec_block_payload_tree: SledTree<ExecBlockPayloadSchema>,
    // Batch storage trees
    batch_by_idx_tree: SledTree<BatchByIdxSchema>,
    batch_id_to_idx_tree: SledTree<BatchIdToIdxSchema>,
    chunk_by_idx_tree: SledTree<ChunkByIdxSchema>,
    chunk_id_to_idx_tree: SledTree<ChunkIdToIdxSchema>,
    batch_chunks_tree: SledTree<BatchChunksSchema>,
    config: SledDbConfig,
}

impl EeNodeDBSled {
    pub(crate) fn new(db: Arc<SledDb>, config: SledDbConfig) -> DbResult<Self> {
        Ok(Self {
            ol_blockid_tree: db.get_tree()?,
            account_state_tree: db.get_tree()?,
            exec_block_tree: db.get_tree()?,
            exec_blocks_by_height_tree: db.get_tree()?,
            exec_block_finalized_tree: db.get_tree()?,
            exec_block_payload_tree: db.get_tree()?,
            batch_by_idx_tree: db.get_tree()?,
            batch_id_to_idx_tree: db.get_tree()?,
            chunk_by_idx_tree: db.get_tree()?,
            chunk_id_to_idx_tree: db.get_tree()?,
            batch_chunks_tree: db.get_tree()?,
            config,
        })
    }
}

impl EeNodeDb for EeNodeDBSled {
    fn store_ee_account_state(
        &self,
        ol_epoch: EpochCommitment,
        ee_account_state: EeAccountState,
    ) -> DbResult<()> {
        // ensure null epoch is not persisted
        if ol_epoch.is_null() {
            return Err(DbError::NullOLBlock);
        }

        let epoch = ol_epoch.epoch();

        if let Some((last_epoch, _)) = self.ol_blockid_tree.last()? {
            // existing entries present; next entry must be at last epoch + 1
            if epoch != last_epoch + 1 {
                return Err(DbError::skipped_ol_slot(last_epoch.into(), epoch.into()));
            }
        }
        // else: if db is empty, allow first write at any epoch

        // NOTE: sled currently does not allow to check for db empty or last item inside
        // transaction. There is a potential race condition where this check can be bypassed.

        let blockid = (*ol_epoch.last_blkid()).into();
        let account_state = DBAccountStateAtEpoch::from_parts(
            epoch,
            ol_epoch.last_slot(),
            ee_account_state.clone().into(),
        );

        (&self.ol_blockid_tree, &self.account_state_tree).transaction_with_retry(
            self.config.backoff.as_ref(),
            self.config.retry_count.into(),
            |(ol_blockid_tree, account_state_tree)| {
                // NOTE: Cannot check for last epoch inside txn, so check that expected epoch is
                // empty. This check can still be bypassed by a race with a concurrent deletion.

                if ol_blockid_tree.get(&epoch)?.is_some() {
                    return abort(DbError::TxnFilledOLSlot(epoch.into()))?;
                }

                ol_blockid_tree.insert(&epoch, &blockid)?;
                account_state_tree.insert(&blockid, &account_state)?;

                Ok(())
            },
        )?;

        Ok(())
    }

    fn rollback_ee_account_state(&self, to_epoch: u32) -> DbResult<()> {
        let Some((max_epoch, _)) = self.ol_blockid_tree.last()? else {
            warn!("called rollback_ee_account_state on empty db");
            return Ok(());
        };

        let Some((min_epoch, _)) = self.ol_blockid_tree.first()? else {
            error!("database should not be empty!!!");
            return Ok(());
        };

        // NOTE: how large can a sled txn get? If there are limits, should chunk this deletion.

        let min_epoch = min_epoch.max(to_epoch + 1);
        (&self.ol_blockid_tree, &self.account_state_tree).transaction_with_retry(
            self.config.backoff.as_ref(),
            self.config.retry_count.into(),
            |(ol_blockid_tree, account_state_tree)| {
                // NOTE: Cannot check for last epoch inside txn, so check that next expected epoch
                // is empty. This check can still be bypassed by a race with
                // inserting new state.
                if ol_blockid_tree.get(&(max_epoch + 1))?.is_some() {
                    abort(DbError::TxnExpectEmptyOLSlot((max_epoch + 1).into()))?;
                }

                for epoch in (min_epoch..=max_epoch).rev() {
                    let Some(blockid) = ol_blockid_tree.take(&epoch)? else {
                        warn!(%epoch, "expected block to exist in db");
                        // Even if the epoch does not exist for some reason, we are trying to remove
                        // it so its ok. But this may leave orphan account
                        // state entries in the db. Will need an orphan
                        // cleanup util or task if this turns out to be an issue.
                        continue;
                    };
                    account_state_tree.remove(&blockid)?;
                }

                Ok(())
            },
        )?;

        Ok(())
    }

    fn get_ol_blockid(&self, epoch: u32) -> DbResult<Option<OLBlockId>> {
        let block_id = self.ol_blockid_tree.get(&epoch)?;
        Ok(block_id.map(Into::into))
    }

    fn ee_account_state(&self, block_id: OLBlockId) -> DbResult<Option<EeAccountStateAtEpoch>> {
        let block_id = block_id.into();
        let Some(account_state) = self.account_state_tree.get(&block_id)? else {
            return Ok(None);
        };

        let (epoch, slot, account_state) = account_state.into_parts();

        let ol_epoch = EpochCommitment::new(epoch, slot, block_id.into());

        Ok(Some(EeAccountStateAtEpoch::new(
            ol_epoch,
            account_state.into(),
        )))
    }

    fn best_ee_account_state(&self) -> DbResult<Option<EeAccountStateAtEpoch>> {
        let Some((_, block_id)) = self.ol_blockid_tree.last()? else {
            return Ok(None);
        };

        let Some(account_state) = self.account_state_tree.get(&block_id)? else {
            return Err(DbError::MissingAccountState(block_id.into()));
        };

        let (epoch, slot, account_state) = account_state.into_parts();

        let ol_epoch = EpochCommitment::new(epoch, slot, block_id.into());

        Ok(Some(EeAccountStateAtEpoch::new(
            ol_epoch,
            account_state.into(),
        )))
    }

    fn save_exec_block(&self, block: ExecBlockRecord, payload: Vec<u8>) -> DbResult<()> {
        let hash = block.blockhash();
        let height = block.blocknum();
        let db_block = block.into();
        (
            &self.exec_block_tree,
            &self.exec_block_payload_tree,
            &self.exec_blocks_by_height_tree,
        )
            .transaction_with_retry(
                self.config.backoff.as_ref(),
                self.config.retry_count.into(),
                |(block_tree, payload_tree, blocks_by_height_tree)| {
                    // Check if block already exists; if so, preserve original data (no overwrite)
                    let block_exists = block_tree.get(&hash)?.is_some();

                    if !block_exists {
                        // save block data
                        block_tree.insert(&hash, &db_block)?;
                        // save payload
                        payload_tree.insert(&hash, &payload)?;
                    }

                    // Update blocks by height.
                    let mut hashes_at_height =
                        blocks_by_height_tree.get(&height)?.unwrap_or_default();
                    // dedupe, just in case
                    if !hashes_at_height.contains(&hash) {
                        hashes_at_height.push(hash);
                        blocks_by_height_tree.insert(&height, &hashes_at_height)?;
                    } else {
                        // Block was absent in `exec_block_tree`, but its hash was tracked in `exec_blocks_by_height_tree`.
                        // The db state is inconsistent, although this particular case is harmless.
                        // Log this inconsistency for further investigation anyway.
                        warn!(blockhash = ?hash, "Inconsistent DB state; blockhash present exec_blocks_by_height_tree without corresponding entry in exec_block_tree");
                    }

                    Ok(())
                },
            )
            .map_err(Into::into)
    }

    fn init_finalized_chain(&self, hash: Hash) -> DbResult<()> {
        // 1. Check if chain is already initialized (check genesis at height 0)
        if let Some(existing_genesis_hash) = self.exec_block_finalized_tree.get(&0)? {
            if existing_genesis_hash == hash {
                // Already initialized with the same genesis block - idempotent success
                return Ok(());
            }
            // Chain is already initialized with a different genesis block
            return Err(DbError::FinalizedExecChainGenesisBlockMismatch);
        }

        // 2. Check that the requested block exists.
        let block = self
            .get_exec_block(hash)?
            .ok_or(DbError::MissingExecBlock(hash))?;

        // 3. Insert the block hash as finalized at height 0.
        let height = block.blocknum();
        if height != 0 {
            return Err(DbError::Other(format!(
                "init_finalized_chain called with non-genesis block at height {}",
                height
            )));
        }
        self.exec_block_finalized_tree.insert(&height, &hash)?;

        Ok(())
    }

    fn extend_finalized_chain(&self, hash: Hash) -> DbResult<()> {
        let block = self
            .get_exec_block(hash)?
            .ok_or(DbError::MissingExecBlock(hash))?;

        let (last_finalized_height, last_finalized_blockhash) = self
            .exec_block_finalized_tree
            .last()?
            .ok_or(DbError::FinalizedExecChainEmpty)?;

        if block.parent_blockhash() != last_finalized_blockhash {
            // does not extend chain
            return Err(DbError::ExecBlockDoesNotExtendChain(hash));
        }

        (&self.exec_block_finalized_tree,).transaction_with_retry(
            self.config.backoff.as_ref(),
            self.config.retry_count.into(),
            |(finalized_tree,)| {
                // NOTE: Cannot check for last entry inside txn, so have to do this. CANNOT retry if
                // finalized block has changed in a race.

                // ensure finalized block has not changed
                if finalized_tree.get(&last_finalized_height)? != Some(last_finalized_blockhash) {
                    abort(DbError::TxnExpectFinalized(
                        last_finalized_height,
                        last_finalized_blockhash,
                    ))?;
                }
                let next_height = last_finalized_height + 1;
                // ensure next block height is empty
                if finalized_tree.get(&next_height)?.is_some() {
                    abort(DbError::TxnExpectEmptyFinalized(last_finalized_height + 1))?;
                }

                // las finalized block has not changed and can extend
                finalized_tree.insert(&next_height, &hash)?;

                Ok(())
            },
        )?;

        Ok(())
    }

    fn revert_finalized_chain(&self, to_height: u64) -> DbResult<()> {
        // Get the current best finalized height
        let Some((current_height, _)) = self.exec_block_finalized_tree.last()? else {
            // Chain is empty
            return Err(DbError::FinalizedExecChainEmpty);
        };

        // If already at or below target height, nothing to do
        if current_height <= to_height {
            return Ok(());
        }

        // Collect heights to remove outside the transaction
        let heights_to_remove: Vec<u64> = ((to_height + 1)..=current_height).collect();

        // Remove finalized entries for heights > to_height
        (&self.exec_block_finalized_tree,).transaction_with_retry(
            self.config.backoff.as_ref(),
            self.config.retry_count.into(),
            |(finalized_tree,)| {
                // Remove all entries above to_height
                for height in &heights_to_remove {
                    finalized_tree.remove(height)?;
                }

                Ok(())
            },
        )?;

        Ok(())
    }

    fn prune_block_data(&self, to_height: u64) -> DbResult<()> {
        // Collect all data to prune outside the transaction
        let mut hashes_to_prune = Vec::new();
        let mut heights_to_remove = Vec::new();

        // Iterate through all heights in the blocks_by_height_tree
        for entry in self.exec_blocks_by_height_tree.iter() {
            let (height, hashes) = entry?;
            if height < to_height {
                hashes_to_prune.extend(hashes);
                heights_to_remove.push(height);
            }
        }

        // Delete block data, payloads, and height index entries in a transaction
        (
            &self.exec_block_tree,
            &self.exec_block_payload_tree,
            &self.exec_blocks_by_height_tree,
        )
            .transaction_with_retry(
                self.config.backoff.as_ref(),
                self.config.retry_count.into(),
                |(block_tree, payload_tree, blocks_by_height_tree)| {
                    // Remove block data and payloads
                    for hash in &hashes_to_prune {
                        block_tree.remove(hash)?;
                        payload_tree.remove(hash)?;
                    }

                    // Remove height index entries for heights < to_height
                    for height in &heights_to_remove {
                        blocks_by_height_tree.remove(height)?;
                    }

                    Ok(())
                },
            )?;

        Ok(())
    }

    fn best_finalized_block(&self) -> DbResult<Option<ExecBlockRecord>> {
        let Some((_, best_blockhash)) = self.exec_block_finalized_tree.last()? else {
            return Ok(None);
        };

        self.get_exec_block(best_blockhash)
    }

    fn get_finalized_block_at_height(&self, height: u64) -> DbResult<Option<ExecBlockRecord>> {
        let Some(blockhash) = self.exec_block_finalized_tree.get(&height)? else {
            return Ok(None);
        };

        self.get_exec_block(blockhash)
    }

    fn get_finalized_height(&self, hash: Hash) -> DbResult<Option<u64>> {
        // get block data
        let Some(height) = self.exec_block_tree.get(&hash)?.map(|block| block.blocknum) else {
            return Ok(None);
        };

        // check if block is in finalized chain
        let Some(finalized_blockhash) = self.exec_block_finalized_tree.get(&height)? else {
            // this height has not been finalized yet
            return Ok(None);
        };

        if finalized_blockhash != hash {
            // block does not lie on finalized chain.
            return Ok(None);
        }

        Ok(Some(height))
    }

    fn get_unfinalized_blocks(&self) -> DbResult<Vec<Hash>> {
        // 1. Get height of last finalized block
        let (finalized_height, _) = self
            .exec_block_finalized_tree
            .last()?
            .ok_or(DbError::FinalizedExecChainEmpty)?;

        // 2. get all blocks for height > finalized_height
        let Some((last_unfinalized_height, _)) = self.exec_blocks_by_height_tree.last()? else {
            // `exec_block_finalized_tree` is not empty, but `exec_blocks_by_height_tree` is empty.
            // This should not be possible normally, but it is safe to ignore.
            warn!("exec_blocks_by_height_tree is empty");
            return Ok(Vec::new());
        };
        let mut unfinalized_hashes = Vec::new();
        for height in (finalized_height + 1)..=last_unfinalized_height {
            let Some(mut blockhashes) = self.exec_blocks_by_height_tree.get(&height)? else {
                continue;
            };
            unfinalized_hashes.append(&mut blockhashes);
        }

        Ok(unfinalized_hashes)
    }

    fn get_exec_block(&self, hash: Hash) -> DbResult<Option<ExecBlockRecord>> {
        let Some(db_block) = self.exec_block_tree.get(&hash)? else {
            return Ok(None);
        };

        let block = db_block
            .try_into()
            .map_err(|err| DbError::Other(format!("Failed to decode block: {err:?}")))?;

        Ok(Some(block))
    }

    fn get_block_payload(&self, hash: Hash) -> DbResult<Option<Vec<u8>>> {
        self.exec_block_payload_tree.get(&hash).map_err(Into::into)
    }

    fn delete_exec_block(&self, hash: Hash) -> DbResult<()> {
        // Get block height if it exists
        let Some(height) = self.exec_block_tree.get(&hash)?.map(|block| block.blocknum) else {
            // Block doesn't exist - idempotent success
            return Ok(());
        };

        // Check if this block is in the finalized chain
        if let Some(finalized_hash) = self.exec_block_finalized_tree.get(&height)? {
            if finalized_hash == hash {
                return Err(DbError::CannotDeleteFinalizedBlock(hash));
            }
        }

        // Delete the block, payload, and update height index
        (
            &self.exec_block_tree,
            &self.exec_block_payload_tree,
            &self.exec_blocks_by_height_tree,
        )
            .transaction_with_retry(
                self.config.backoff.as_ref(),
                self.config.retry_count.into(),
                |(block_tree, payload_tree, blocks_by_height_tree)| {
                    // Remove block data and payload
                    block_tree.remove(&hash)?;
                    payload_tree.remove(&hash)?;

                    // Update the height index to remove this hash
                    if let Some(mut hashes_at_height) = blocks_by_height_tree.get(&height)? {
                        hashes_at_height.retain(|&h| h != hash);
                        if hashes_at_height.is_empty() {
                            blocks_by_height_tree.remove(&height)?;
                        } else {
                            blocks_by_height_tree.insert(&height, &hashes_at_height)?;
                        }
                    }

                    Ok(())
                },
            )?;

        Ok(())
    }

    // Batch storage implementations

    fn save_genesis_batch(&self, batch: Batch) -> DbResult<()> {
        // If any batches exist, this is a noop
        if self.batch_by_idx_tree.first()?.is_some() {
            return Ok(());
        }

        let idx = batch.idx();
        let batch_id: DBBatchId = batch.id().into();
        let db_batch = DBBatchWithStatus::new(batch, BatchStatus::Sealed);

        (&self.batch_by_idx_tree, &self.batch_id_to_idx_tree).transaction_with_retry(
            self.config.backoff.as_ref(),
            self.config.retry_count.into(),
            |(batch_tree, id_to_idx_tree)| {
                // Check again inside transaction that no batches exist at idx 0 (genesis)
                if batch_tree.get(&0u64)?.is_some() {
                    return Ok(());
                }

                batch_tree.insert(&idx, &db_batch)?;
                id_to_idx_tree.insert(&batch_id, &idx)?;

                Ok(())
            },
        )?;

        Ok(())
    }

    fn save_next_batch(&self, batch: Batch) -> DbResult<()> {
        let idx = batch.idx();
        let batch_id: DBBatchId = batch.id().into();
        let db_batch = DBBatchWithStatus::new(batch, BatchStatus::Sealed);

        (&self.batch_by_idx_tree, &self.batch_id_to_idx_tree).transaction_with_retry(
            self.config.backoff.as_ref(),
            self.config.retry_count.into(),
            |(batch_tree, id_to_idx_tree)| {
                // Insert the batch
                batch_tree.insert(&idx, &db_batch)?;
                id_to_idx_tree.insert(&batch_id, &idx)?;

                Ok(())
            },
        )?;

        Ok(())
    }

    fn update_batch_status(&self, batch_id: BatchId, status: BatchStatus) -> DbResult<()> {
        let db_batch_id: DBBatchId = batch_id.into();

        // Look up idx by id (outside transaction is fine - idx mapping is stable)
        let Some(idx) = self.batch_id_to_idx_tree.get(&db_batch_id)? else {
            return Err(DbError::BatchNotFound(batch_id));
        };

        // Use transaction for read-modify-write to prevent race conditions
        (&self.batch_by_idx_tree,).transaction_with_retry(
            self.config.backoff.as_ref(),
            self.config.retry_count.into(),
            |(batch_tree,)| {
                let Some(current) = batch_tree.get(&idx)? else {
                    abort(DbError::BatchNotFound(batch_id))?
                };

                let parts_result: Result<(Batch, BatchStatus), _> = current.into_parts();
                let (batch, _old_status) = parts_result
                    .map_err(|e| TSledError::abort(DbError::BatchDeserialize(e.to_string())))?;
                let updated = DBBatchWithStatus::new(batch, status.clone());

                batch_tree.insert(&idx, &updated)?;
                Ok(())
            },
        )?;

        Ok(())
    }

    fn revert_batches(&self, to_idx: u64) -> DbResult<()> {
        // Get highest idx
        let Some((max_idx, _)) = self.batch_by_idx_tree.last()? else {
            // Empty, nothing to do
            return Ok(());
        };

        if max_idx <= to_idx {
            // Nothing to revert
            return Ok(());
        }

        // Collect batch ids to remove
        let mut batch_ids_to_remove = Vec::new();
        for idx in (to_idx + 1)..=max_idx {
            if let Some(db_batch) = self.batch_by_idx_tree.get(&idx)? {
                let parts_result: Result<(Batch, BatchStatus), _> = db_batch.into_parts();
                if let Ok((batch, _)) = parts_result {
                    batch_ids_to_remove.push((idx, DBBatchId::from(batch.id())));
                }
            }
        }

        (
            &self.batch_by_idx_tree,
            &self.batch_id_to_idx_tree,
            &self.batch_chunks_tree,
        )
            .transaction_with_retry(
                self.config.backoff.as_ref(),
                self.config.retry_count.into(),
                |(batch_tree, id_to_idx_tree, batch_chunks_tree)| {
                    for (idx, batch_id) in &batch_ids_to_remove {
                        batch_tree.remove(idx)?;
                        id_to_idx_tree.remove(batch_id)?;
                        batch_chunks_tree.remove(batch_id)?;
                    }
                    Ok(())
                },
            )?;

        Ok(())
    }

    fn get_batch_by_id(&self, batch_id: BatchId) -> DbResult<Option<(Batch, BatchStatus)>> {
        let db_batch_id: DBBatchId = batch_id.into();

        let Some(idx) = self.batch_id_to_idx_tree.get(&db_batch_id)? else {
            return Ok(None);
        };

        self.get_batch_by_idx(idx)
    }

    fn get_batch_by_idx(&self, idx: u64) -> DbResult<Option<(Batch, BatchStatus)>> {
        let Some(db_batch) = self.batch_by_idx_tree.get(&idx)? else {
            return Ok(None);
        };

        let parts_result: Result<(Batch, BatchStatus), _> = db_batch.into_parts();
        let (batch, status) = parts_result.map_err(|e| DbError::BatchDeserialize(e.to_string()))?;

        Ok(Some((batch, status)))
    }

    fn get_latest_batch(&self) -> DbResult<Option<(Batch, BatchStatus)>> {
        let Some((idx, _)) = self.batch_by_idx_tree.last()? else {
            return Ok(None);
        };

        self.get_batch_by_idx(idx)
    }

    // Chunk storage implementations

    fn save_next_chunk(&self, chunk: Chunk) -> DbResult<()> {
        let idx = chunk.idx();
        let chunk_id: DBChunkId = chunk.id().into();
        let db_chunk = DBChunkWithStatus::new(chunk, ChunkStatus::ProvingNotStarted);

        (&self.chunk_by_idx_tree, &self.chunk_id_to_idx_tree).transaction_with_retry(
            self.config.backoff.as_ref(),
            self.config.retry_count.into(),
            |(chunk_tree, id_to_idx_tree)| {
                chunk_tree.insert(&idx, &db_chunk)?;
                id_to_idx_tree.insert(&chunk_id, &idx)?;

                Ok(())
            },
        )?;

        Ok(())
    }

    fn update_chunk_status(&self, chunk_id: ChunkId, status: ChunkStatus) -> DbResult<()> {
        let db_chunk_id: DBChunkId = chunk_id.into();

        // Look up idx by id (outside transaction is fine - idx mapping is stable)
        let Some(idx) = self.chunk_id_to_idx_tree.get(&db_chunk_id)? else {
            return Err(DbError::ChunkNotFound(chunk_id));
        };

        // Use transaction for read-modify-write to prevent race conditions
        (&self.chunk_by_idx_tree,).transaction_with_retry(
            self.config.backoff.as_ref(),
            self.config.retry_count.into(),
            |(chunk_tree,)| {
                let Some(current) = chunk_tree.get(&idx)? else {
                    abort(DbError::ChunkNotFound(chunk_id))?
                };

                let (chunk, _old_status) = current.into_parts();
                let updated = DBChunkWithStatus::new(chunk, status.clone());

                chunk_tree.insert(&idx, &updated)?;
                Ok(())
            },
        )?;

        Ok(())
    }

    fn revert_chunks_from(&self, from_idx: u64) -> DbResult<()> {
        // Get highest idx
        let Some((max_idx, _)) = self.chunk_by_idx_tree.last()? else {
            // Empty, nothing to do
            return Ok(());
        };

        if max_idx < from_idx {
            // Nothing to revert
            return Ok(());
        }

        // Collect chunk ids to remove
        let mut chunk_ids_to_remove = Vec::new();
        for idx in from_idx..=max_idx {
            if let Some(db_chunk) = self.chunk_by_idx_tree.get(&idx)? {
                let parts: (Chunk, ChunkStatus) = db_chunk.into_parts();
                let (chunk, _) = parts;
                chunk_ids_to_remove.push((idx, DBChunkId::from(chunk.id())));
            }
        }

        (&self.chunk_by_idx_tree, &self.chunk_id_to_idx_tree).transaction_with_retry(
            self.config.backoff.as_ref(),
            self.config.retry_count.into(),
            |(chunk_tree, id_to_idx_tree)| {
                for (idx, chunk_id) in &chunk_ids_to_remove {
                    chunk_tree.remove(idx)?;
                    id_to_idx_tree.remove(chunk_id)?;
                }
                Ok(())
            },
        )?;

        Ok(())
    }

    fn get_chunk_by_id(&self, chunk_id: ChunkId) -> DbResult<Option<(Chunk, ChunkStatus)>> {
        let db_chunk_id: DBChunkId = chunk_id.into();

        let Some(idx) = self.chunk_id_to_idx_tree.get(&db_chunk_id)? else {
            return Ok(None);
        };

        self.get_chunk_by_idx(idx)
    }

    fn get_chunk_by_idx(&self, idx: u64) -> DbResult<Option<(Chunk, ChunkStatus)>> {
        let Some(db_chunk) = self.chunk_by_idx_tree.get(&idx)? else {
            return Ok(None);
        };

        let parts: (Chunk, ChunkStatus) = db_chunk.into_parts();
        let (chunk, status) = parts;

        Ok(Some((chunk, status)))
    }

    fn get_latest_chunk(&self) -> DbResult<Option<(Chunk, ChunkStatus)>> {
        let Some((idx, _)) = self.chunk_by_idx_tree.last()? else {
            return Ok(None);
        };

        self.get_chunk_by_idx(idx)
    }

    fn set_batch_chunks(&self, batch_id: BatchId, chunks: Vec<ChunkId>) -> DbResult<()> {
        let db_batch_id: DBBatchId = batch_id.into();
        let db_chunks: Vec<DBChunkId> = chunks.into_iter().map(Into::into).collect();

        self.batch_chunks_tree.insert(&db_batch_id, &db_chunks)?;

        Ok(())
    }
}
