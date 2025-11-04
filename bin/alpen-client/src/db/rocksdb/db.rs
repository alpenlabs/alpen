use std::sync::Arc;

use rockbound::{
    utils::{get_first, get_last},
    OptimisticTransactionDB, SchemaDBOperationsExt, TransactionCtx, TransactionRetry,
};
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::{OLBlockCommitment, OLBlockId};
use tracing::{error, warn};

use super::schema::{AccountStateAtOlBlockSchema, OlBlockAtSlotSchema};
use crate::{
    db::{
        database::EeNodeDb,
        serialization_types::{DBAccountStateAtSlot, DBOLBlockId},
        DbError, DbResult,
    },
    traits::storage::EeAccountStateAtBlock,
};

struct DbConfig {
    retry_count: u16,
}

pub(crate) struct EeNodeRocksDb {
    db: Arc<OptimisticTransactionDB>,
    config: DbConfig,
}

impl EeNodeRocksDb {
    pub(crate) fn new(db: Arc<OptimisticTransactionDB>, retry_count: u16) -> Self {
        Self {
            db,
            config: DbConfig { retry_count },
        }
    }

    fn with_optimistic_txn<F, R>(&self, cb: F) -> DbResult<R>
    where
        F: FnMut(&TransactionCtx<'_, OptimisticTransactionDB>) -> DbResult<R>,
    {
        let retry = TransactionRetry::Count(self.config.retry_count);
        self.db.with_optimistic_txn(retry, cb).map_err(Into::into)
    }
}

impl EeNodeDb for EeNodeRocksDb {
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

        if let Some((last_slot, _)) = get_last::<OlBlockAtSlotSchema>(self.db.as_ref())? {
            // existing entries present; next entry must be at last slot + 1
            if slot != last_slot + 1 {
                return Err(DbError::skipped_ol_slot(last_slot + 1, slot));
            }
        }
        // else: if db is empty, allow first write at any slot

        // NOTE: sled currently does not allow to check for db empty or last item inside
        // transaction. There is a potential race condition where this check can be bypassed.

        let blockid: DBOLBlockId = (*ol_block.blkid()).into();
        let account_state = DBAccountStateAtSlot::from_parts(slot, ee_account_state.clone().into());

        self.with_optimistic_txn(|txn| {
            // NOTE: Cannot check for last slot inside txn, so check that expected slot is
            // empty. This check can still be bypassed by a race with a concurrent deletion.
            if txn.get_for_update::<OlBlockAtSlotSchema>(&slot)?.is_some() {
                Err(DbError::TxnFilledOlSlot(slot))?;
            }

            txn.put::<OlBlockAtSlotSchema>(&slot, &blockid)?;
            txn.put::<AccountStateAtOlBlockSchema>(&blockid, &account_state)?;

            Ok(())
        })?;

        Ok(())
    }

    fn rollback_ee_account_state(&self, to_slot: u64) -> DbResult<()> {
        let Some((max_slot, _)) = get_last::<OlBlockAtSlotSchema>(self.db.as_ref())? else {
            warn!("called rollback_ee_account_state on empty db");
            return Ok(());
        };

        let Some((min_slot, _)) = get_first::<OlBlockAtSlotSchema>(self.db.as_ref())? else {
            error!("database should not be empty!!!");
            return Ok(());
        };

        let min_slot = min_slot.max(to_slot + 1);

        self.with_optimistic_txn(|txn| {
            // NOTE: Cannot check for last slot inside txn, so check that next expected slot is
            // empty. This check can still be bypassed by a race with inserting new state.
            if txn
                .get_for_update::<OlBlockAtSlotSchema>(&(max_slot + 1))?
                .is_some()
            {
                Err(DbError::TxnExpectEmptyOlSlot(max_slot + 1))?;
            }

            for slot in (min_slot..=max_slot).rev() {
                let Some(blockid) = txn.get_for_update::<OlBlockAtSlotSchema>(&slot)? else {
                    warn!("expected block to exist in db: slot = {}", slot);
                    // Even if the slot does not exist for some reason, we are trying to remove
                    // it so its ok. But this may leave orphan account
                    // state entries in the db. Will need an orphan
                    // cleanup util or task if this turns out to be an issue.
                    continue;
                };
                txn.delete::<OlBlockAtSlotSchema>(&slot)?;
                txn.delete::<AccountStateAtOlBlockSchema>(&blockid)?;
            }

            Ok(())
        })?;

        Ok(())
    }

    fn get_ol_blockid(&self, slot: u64) -> DbResult<Option<OLBlockId>> {
        let block_id = self.db.get::<OlBlockAtSlotSchema>(&slot)?;
        Ok(block_id.map(Into::into))
    }

    fn ee_account_state(&self, block_id: OLBlockId) -> DbResult<Option<EeAccountStateAtBlock>> {
        let block_id: DBOLBlockId = block_id.into();
        let Some(account_state) = self.db.get::<AccountStateAtOlBlockSchema>(&block_id)? else {
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
        let Some((_, block_id)) = get_last::<OlBlockAtSlotSchema>(self.db.as_ref())? else {
            return Ok(None);
        };

        let Some(account_state) = self.db.get::<AccountStateAtOlBlockSchema>(&block_id)? else {
            return Err(DbError::MissingAccountState(block_id.into()));
        };

        let (slot, account_state) = account_state.into_parts();

        let ol_block = OLBlockCommitment::new(slot, block_id.into());

        Ok(Some(EeAccountStateAtBlock::new(
            ol_block,
            account_state.into(),
        )))
    }
}
