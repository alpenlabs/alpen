use std::sync::Arc;

use rockbound::{OptimisticTransactionDB, SchemaDBOperationsExt};
use strata_db::{chainstate::*, DbError, DbResult};
use strata_ol_chainstate_types::{Chainstate, WriteBatch};
use strata_identifiers::Buf32;

use super::{schemas::*, types::*};
use crate::DbOpsConfig;

#[derive(Debug)]
pub struct ChainstateDb {
    db: Arc<OptimisticTransactionDB>,
    ops: DbOpsConfig,
}

impl ChainstateDb {
    pub fn new(db: Arc<OptimisticTransactionDB>, _ops: DbOpsConfig) -> Self {
        Self { db, ops: _ops }
    }
}

impl ChainstateDatabase for ChainstateDb {
    fn create_new_inst(&self, toplevel: Chainstate) -> DbResult<StateInstanceId> {
        let entry = StateInstanceEntry::new(toplevel);

        let res = self
            .db
            .with_optimistic_txn(self.ops.txn_retry_count(), |txn| {
                // Get the next ID.  It doesn't really matter what the numbers
                // are, since we treat them opaquely.
                let mut inst_iter = txn.iter::<StateInstanceSchema>()?;
                inst_iter.seek_to_last();
                let id = match inst_iter.rev().next() {
                    Some(res) => {
                        let (id, _) = res?.into_tuple();
                        id + 1
                    }
                    None => 0,
                };

                // Actually insert the new state instance.
                txn.put::<StateInstanceSchema>(&id, &entry)?;

                Ok::<_, DbError>(id)
            })
            .map_err(|e| DbError::Other(e.to_string()))?;

        Ok(res)
    }

    fn clone_inst(&self, source_id: StateInstanceId) -> DbResult<StateInstanceId> {
        let res = self
            .db
            .with_optimistic_txn(self.ops.txn_retry_count(), |txn| {
                // Fetch the data we're cloning.
                let entry = txn
                    .get::<StateInstanceSchema>(&source_id)?
                    .ok_or(DbError::MissingStateInstance)?;

                // Get the next ID.  It doesn't really matter what the numbers
                // are, since we treat them opaquely.
                let mut inst_iter = txn.iter::<StateInstanceSchema>()?;
                inst_iter.seek_to_last();
                let new_id = match inst_iter.rev().next() {
                    Some(res) => {
                        let (id, _) = res?.into_tuple();
                        id + 1
                    }

                    // (this should be unreachable but whatever, let's error
                    // here anyways)
                    None => return Err(DbError::MissingStateInstance),
                };

                // Actually insert the new state instance.
                txn.put::<StateInstanceSchema>(&new_id, &entry)?;

                Ok::<_, DbError>(new_id)
            })
            .map_err(|e| DbError::Other(e.to_string()))?;

        Ok(res)
    }

    fn del_inst(&self, id: StateInstanceId) -> DbResult<()> {
        self.db.delete::<StateInstanceSchema>(&id)?;
        Ok(())
    }

    fn get_insts(&self) -> DbResult<Vec<StateInstanceId>> {
        let res = self
            .db
            .with_optimistic_txn(self.ops.txn_retry_count(), |txn| {
                let mut ids = Vec::new();

                // Just iterate over every entry and get the data out.
                //
                // This does a lot of extra effort to parse the state instance
                // entries, but that kinda sucks.
                let mut inst_iter = txn.iter::<StateInstanceSchema>()?;
                while let Some(pair) = inst_iter.next().transpose()? {
                    ids.push(pair.key);
                }

                Ok::<_, DbError>(ids)
            })
            .map_err(|e| DbError::Other(e.to_string()))?;

        Ok(res)
    }

    fn get_inst_root(&self, id: StateInstanceId) -> DbResult<Buf32> {
        self.get_inst_toplevel_state(id)
            .map(|chs| chs.compute_state_root())
    }

    fn get_inst_toplevel_state(&self, id: StateInstanceId) -> DbResult<Chainstate> {
        let entry = self
            .db
            .get::<StateInstanceSchema>(&id)?
            .ok_or(DbError::MissingStateInstance)?;

        Ok(entry.into_toplevel_state())
    }

    fn put_write_batch(&self, id: WriteBatchId, wb: WriteBatch) -> DbResult<()> {
        self.db.put::<WriteBatchSchema>(&id, &wb)?;
        Ok(())
    }

    fn get_write_batch(&self, id: WriteBatchId) -> DbResult<Option<WriteBatch>> {
        Ok(self.db.get::<WriteBatchSchema>(&id)?)
    }

    fn del_write_batch(&self, id: WriteBatchId) -> DbResult<()> {
        self.db.delete::<WriteBatchSchema>(&id)?;
        Ok(())
    }

    fn merge_write_batches(
        &self,
        state_id: StateInstanceId,
        wb_ids: Vec<WriteBatchId>,
    ) -> DbResult<()> {
        // Since we have a really simple state merge concept now, we can just
        // fudge the details on this one.

        self.db
            .with_optimistic_txn(self.ops.txn_retry_count(), |txn| {
                // Load the source state entry to make sure it exists.
                let _inst_entry = txn
                    .get::<StateInstanceSchema>(&state_id)?
                    .ok_or(DbError::MissingStateInstance)?;

                // Just iterate over all the write batch IDs to make sure they
                // exist.
                //
                // Keep the last one so we don't have to read it twice.
                let mut last_wb = None;
                for wb_id in &wb_ids {
                    let wb = txn
                        .get::<WriteBatchSchema>(wb_id)?
                        .ok_or(DbError::MissingWriteBatch(*wb_id))?;

                    // In here we'd apply the write batch, but since we can be
                    // lazy for now we can just write down what it is.
                    last_wb = Some(wb);
                }

                // Applying the last write batch is really simple.
                if let Some(last_wb) = last_wb {
                    let entry = StateInstanceEntry::new(last_wb.into_toplevel());
                    txn.put::<StateInstanceSchema>(&state_id, &entry)?;
                }

                Ok::<_, DbError>(())
            })
            .map_err(|e| DbError::Other(e.to_string()))
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::chain_state_db_tests;

    use super::*;
    use crate::test_utils::get_rocksdb_tmp_instance;

    fn setup_db() -> ChainstateDb {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        ChainstateDb::new(db, db_ops)
    }

    chain_state_db_tests!(setup_db());
}
