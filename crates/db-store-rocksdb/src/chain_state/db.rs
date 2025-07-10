use std::sync::Arc;

use rockbound::{OptimisticTransactionDB, SchemaBatch, SchemaDBOperationsExt};
use strata_db::{errors::DbError, traits::*, DbResult};
use strata_state::{
    id::L2BlockId,
    state_op::{WriteBatch, WriteBatchEntry},
};

use super::schemas::WriteBatchSchema;
use crate::{
    utils::{get_first_idx, get_last_idx},
    DbOpsConfig,
};

#[derive(Debug)]
pub struct ChainstateDb {
    db: Arc<OptimisticTransactionDB>,
    _ops: DbOpsConfig,
}

impl ChainstateDb {
    pub fn new(db: Arc<OptimisticTransactionDB>, ops: DbOpsConfig) -> Self {
        Self { db, _ops: ops }
    }

    fn get_first_idx(&self) -> DbResult<Option<u64>> {
        get_first_idx::<WriteBatchSchema>(&self.db)
    }

    fn get_last_idx(&self) -> DbResult<Option<u64>> {
        get_last_idx::<WriteBatchSchema>(&self.db)
    }
}

impl ChainstateDatabase for ChainstateDb {
    fn write_genesis_state(
        &self,
        toplevel: strata_state::chain_state::Chainstate,
        blockid: L2BlockId,
    ) -> DbResult<()> {
        let genesis_key = 0;

        // This should only ever be called once.
        if self.get_first_idx()?.is_some() || self.get_last_idx()?.is_some() {
            return Err(DbError::OverwriteStateUpdate(genesis_key));
        }

        let mut batch = SchemaBatch::new();

        let genesis_wb = WriteBatch::new_replace(toplevel);
        batch.put::<WriteBatchSchema>(&genesis_key, &WriteBatchEntry::new(genesis_wb, blockid))?;

        self.db.write_schemas(batch)?;

        Ok(())
    }

    fn put_write_batch(&self, idx: u64, writebatch: WriteBatchEntry) -> DbResult<()> {
        if self.db.get::<WriteBatchSchema>(&idx)?.is_some() {
            return Err(DbError::OverwriteStateUpdate(idx));
        }

        // Make sure we always have a contiguous range of batches.
        // FIXME this *could* be a race condition / TOCTOU issue, but we're only
        // going to be writing from a single thread anyways so it should be fine
        match self.get_last_idx()? {
            Some(last_idx) => {
                if idx != last_idx + 1 {
                    return Err(DbError::OooInsert("Chainstate", idx));
                }
            }
            None => return Err(DbError::NotBootstrapped),
        }

        // TODO maybe do this in a tx to make sure we don't race/TOCTOU it
        self.db.put::<WriteBatchSchema>(&idx, &writebatch)?;

        #[cfg(test)]
        eprintln!("db inserted index {idx}");

        Ok(())
    }

    fn get_write_batch(&self, idx: u64) -> DbResult<Option<WriteBatchEntry>> {
        Ok(self.db.get::<WriteBatchSchema>(&idx)?)
    }

    fn purge_entries_before(&self, before_idx: u64) -> DbResult<()> {
        let first_idx = match self.get_first_idx()? {
            Some(idx) => idx,
            None => return Err(DbError::NotBootstrapped),
        };

        if first_idx > before_idx {
            return Err(DbError::MissingL2State(before_idx));
        }

        let mut del_batch = SchemaBatch::new();
        for idx in first_idx..before_idx {
            del_batch.delete::<WriteBatchSchema>(&idx)?;
        }
        self.db.write_schemas(del_batch)?;

        Ok(())
    }

    fn rollback_writes_to(&self, new_tip_idx: u64) -> DbResult<()> {
        let last_idx = match self.get_last_idx()? {
            Some(idx) => idx,
            None => return Err(DbError::NotBootstrapped),
        };

        let first_idx = match self.get_first_idx()? {
            Some(idx) => idx,
            None => return Err(DbError::NotBootstrapped),
        };

        // In this case, we'd still be before the rollback idx.
        if last_idx < new_tip_idx {
            return Err(DbError::RevertAboveCurrent(new_tip_idx, last_idx));
        }

        // In this case, we'd have to roll back past the first idx.
        if first_idx > new_tip_idx {
            return Err(DbError::MissingL2State(new_tip_idx));
        }

        let mut del_batch = SchemaBatch::new();
        for idx in new_tip_idx + 1..=last_idx {
            del_batch.delete::<WriteBatchSchema>(&idx)?;
        }
        self.db.write_schemas(del_batch)?;

        Ok(())
    }

    fn get_earliest_write_idx(&self) -> DbResult<u64> {
        self.get_first_idx()?.ok_or(DbError::NotBootstrapped)
    }

    fn get_last_write_idx(&self) -> DbResult<u64> {
        self.get_last_idx()?.ok_or(DbError::NotBootstrapped)
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
