use std::sync::Arc;

use alpen_ee_common::EeAccountStateAtBlock;
use strata_db_store_sled::SledDbConfig;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::{OLBlockCommitment, OLBlockId};
use tracing::{error, warn};
use typed_sled::{error::Error as TSledError, transaction::SledTransactional, SledDb, SledTree};

use super::{AccountStateAtOlBlockSchema, OlBlockAtSlotSchema};
use crate::{
    database::EeNodeDb,
    serialization_types::DBAccountStateAtSlot,
    sleddb::{
        ExecBlockCanonicalSchema, ExecBlockPayloadSchema, ExecBlockSchema, ExecBlocksAtHeightSchema,
    },
    DbError, DbResult,
};

fn abort<T>(reason: impl std::error::Error + Send + Sync + 'static) -> Result<T, TSledError> {
    Err(TSledError::abort(reason))
}

#[expect(dead_code, reason = "wip")]
pub(crate) struct EeNodeDBSled {
    ol_blockid_tree: SledTree<OlBlockAtSlotSchema>,
    account_state_tree: SledTree<AccountStateAtOlBlockSchema>,
    exec_block_tree: SledTree<ExecBlockSchema>,
    exec_blocks_by_height_tree: SledTree<ExecBlocksAtHeightSchema>,
    exec_block_canonical_tree: SledTree<ExecBlockCanonicalSchema>,
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
            exec_block_canonical_tree: db.get_tree()?,
            exec_block_payload_tree: db.get_tree()?,
            config,
        })
    }
}

#[expect(unused, reason = "wip")]
impl EeNodeDb for EeNodeDBSled {
    fn store_ee_account_state(
        &self,
        ol_block: OLBlockCommitment,
        ee_account_state: EeAccountState,
    ) -> DbResult<()> {
        // ensure null block is not persisted
        if ol_block.is_null() {
            return Err(DbError::NullOlBlock);
        }

        let slot = ol_block.slot();

        if let Some((last_slot, _)) = self.ol_blockid_tree.last()? {
            // existing entries present; next entry must be at last slot + 1
            if slot != last_slot + 1 {
                return Err(DbError::skipped_ol_slot(last_slot + 1, slot));
            }
        }
        // else: if db is empty, allow first write at any slot

        // NOTE: sled currently does not allow to check for db empty or last item inside
        // transaction. There is a potential race condition where this check can be bypassed.

        let blockid = (*ol_block.blkid()).into();
        let account_state = DBAccountStateAtSlot::from_parts(slot, ee_account_state.clone().into());

        (&self.ol_blockid_tree, &self.account_state_tree).transaction_with_retry(
            self.config.backoff.as_ref(),
            self.config.retry_count.into(),
            |(ol_blockid_tree, account_state_tree)| {
                // NOTE: Cannot check for last slot inside txn, so check that expected slot is
                // empty. This check can still be bypassed by a race with a concurrent deletion.

                if ol_blockid_tree.get(&slot)?.is_some() {
                    return abort(DbError::TxnFilledOlSlot(slot))?;
                }

                ol_blockid_tree.insert(&slot, &blockid)?;
                account_state_tree.insert(&blockid, &account_state)?;

                Ok(())
            },
        )?;

        Ok(())
    }

    fn rollback_ee_account_state(&self, to_slot: u64) -> DbResult<()> {
        let Some((max_slot, _)) = self.ol_blockid_tree.last()? else {
            warn!("called rollback_ee_account_state on empty db");
            return Ok(());
        };

        let Some((min_slot, _)) = self.ol_blockid_tree.first()? else {
            error!("database should not be empty!!!");
            return Ok(());
        };

        // NOTE: how large can a sled txn get? If there are limits, should chunk this deletion.

        let min_slot = min_slot.max(to_slot + 1);
        (&self.ol_blockid_tree, &self.account_state_tree).transaction_with_retry(
            self.config.backoff.as_ref(),
            self.config.retry_count.into(),
            |(ol_blockid_tree, account_state_tree)| {
                // NOTE: Cannot check for last slot inside txn, so check that next expected slot is
                // empty. This check can still be bypassed by a race with inserting new state.
                if ol_blockid_tree.get(&(max_slot + 1))?.is_some() {
                    abort(DbError::TxnExpectEmptyOlSlot(max_slot + 1))?;
                }

                for slot in (min_slot..=max_slot).rev() {
                    let Some(blockid) = ol_blockid_tree.take(&slot)? else {
                        warn!("expected block to exist in db: slot = {}", slot);
                        // Even if the slot does not exist for some reason, we are trying to remove
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

    fn get_ol_blockid(&self, slot: u64) -> DbResult<Option<OLBlockId>> {
        let block_id = self.ol_blockid_tree.get(&slot)?;
        Ok(block_id.map(Into::into))
    }

    fn ee_account_state(&self, block_id: OLBlockId) -> DbResult<Option<EeAccountStateAtBlock>> {
        let block_id = block_id.into();
        let Some(account_state) = self.account_state_tree.get(&block_id)? else {
            return Ok(None);
        };

        let (slot, account_state) = account_state.into_parts();

        let ol_block = OLBlockCommitment::new(slot, block_id.into());

        Ok(Some(EeAccountStateAtBlock::new(
            ol_block,
            account_state.into(),
        )))
    }

    fn best_ee_account_state(&self) -> DbResult<Option<EeAccountStateAtBlock>> {
        let Some((_, block_id)) = self.ol_blockid_tree.last()? else {
            return Ok(None);
        };

        let Some(account_state) = self.account_state_tree.get(&block_id)? else {
            return Err(DbError::MissingAccountState(block_id.into()));
        };

        let (slot, account_state) = account_state.into_parts();

        let ol_block = OLBlockCommitment::new(slot, block_id.into());

        Ok(Some(EeAccountStateAtBlock::new(
            ol_block,
            account_state.into(),
        )))
    }

    fn save_exec_block(
        &self,
        block: alpen_ee_common::ExecBlockRecord,
        payload: Vec<u8>,
    ) -> DbResult<()> {
        todo!()
    }

    fn extend_finalized_chain(&self, hash: strata_acct_types::Hash) -> DbResult<()> {
        todo!()
    }

    fn revert_finalized_chain(&self, to_height: u64) -> DbResult<()> {
        todo!()
    }

    fn prune_block_data(&self, to_height: u64) -> DbResult<()> {
        todo!()
    }

    fn best_finalized_block(&self) -> DbResult<Option<alpen_ee_common::ExecBlockRecord>> {
        todo!()
    }

    fn get_finalized_height(&self, hash: strata_acct_types::Hash) -> DbResult<Option<u64>> {
        todo!()
    }

    fn get_unfinalized_blocks(&self) -> DbResult<Vec<strata_acct_types::Hash>> {
        todo!()
    }

    fn get_exec_block(
        &self,
        hash: strata_acct_types::Hash,
    ) -> DbResult<Option<alpen_ee_common::ExecBlockRecord>> {
        todo!()
    }

    fn get_block_payload(&self, hash: strata_acct_types::Hash) -> DbResult<Option<Vec<u8>>> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::ee_node_db_tests;

    fn setup_db() -> EeNodeDBSled {
        // Create a temporary sled database
        let db = sled::Config::new().temporary(true).open().unwrap();
        let sled_db = SledDb::new(db).unwrap();
        let config = SledDbConfig::test();

        EeNodeDBSled::new(Arc::new(sled_db), config).unwrap()
    }

    ee_node_db_tests!(setup_db());
}
