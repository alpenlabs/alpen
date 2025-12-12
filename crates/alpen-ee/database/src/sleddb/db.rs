use std::sync::Arc;

use alpen_ee_common::{EeAccountStateAtEpoch, ExecBlockRecord};
use strata_acct_types::Hash;
use strata_db_store_sled::SledDbConfig;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::{EpochCommitment, OLBlockId};
use tracing::{error, warn};
use typed_sled::{error::Error as TSledError, transaction::SledTransactional, SledDb, SledTree};

use super::{AccountStateAtOLEpochSchema, OLBlockAtEpochSchema};
use crate::{
    database::EeNodeDb,
    serialization_types::DBAccountStateAtEpoch,
    sleddb::{
        ExecBlockFinalizedSchema, ExecBlockPayloadSchema, ExecBlockSchema, ExecBlocksAtHeightSchema,
    },
    DbError, DbResult,
};

fn abort<T>(reason: impl std::error::Error + Send + Sync + 'static) -> Result<T, TSledError> {
    Err(TSledError::abort(reason))
}

pub(crate) struct EeNodeDBSled {
    ol_blockid_tree: SledTree<OLBlockAtEpochSchema>,
    account_state_tree: SledTree<AccountStateAtOLEpochSchema>,
    exec_block_tree: SledTree<ExecBlockSchema>,
    exec_blocks_by_height_tree: SledTree<ExecBlocksAtHeightSchema>,
    exec_block_finalized_tree: SledTree<ExecBlockFinalizedSchema>,
    exec_block_payload_tree: SledTree<ExecBlockPayloadSchema>,
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
}
