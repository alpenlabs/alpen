use std::sync::Arc;

use rockbound::{OptimisticTransactionDB, SchemaBatch, SchemaDBOperationsExt};
use strata_db::{errors::DbError, traits::SyncEventDatabase, DbResult};
use strata_state::sync_event::SyncEvent;

use super::schemas::{SyncEventSchema, SyncEventWithTimestamp};
use crate::{sequence::get_next_id_opts, DbOpsConfig};

#[derive(Debug)]
pub struct SyncEventDb {
    db: Arc<OptimisticTransactionDB>,
    ops: DbOpsConfig,
}

impl SyncEventDb {
    // NOTE: db is expected to open all the column families defined in STORE_COLUMN_FAMILIES.
    // FIXME: Make it better/generic.
    pub fn new(db: Arc<OptimisticTransactionDB>, ops: DbOpsConfig) -> Self {
        Self { db, ops }
    }

    fn get_last_key(&self) -> DbResult<Option<u64>> {
        let mut iterator = self.db.iter::<SyncEventSchema>()?;
        iterator.seek_to_last();
        match iterator.rev().next() {
            Some(res) => {
                let (tip, _) = res?.into_tuple();
                Ok(Some(tip))
            }
            None => Ok(None),
        }
    }
}

impl SyncEventDatabase for SyncEventDb {
    fn write_sync_event(&self, ev: SyncEvent) -> DbResult<u64> {
        self.db
            .with_optimistic_txn(
                rockbound::TransactionRetry::Count(self.ops.retry_count),
                move |txn| {
                    // autoincrementing, starting from index 1
                    let id = get_next_id_opts::<SyncEventSchema, OptimisticTransactionDB>(
                        txn,
                        |v| v + 1,
                        1,
                    )?;
                    let event = SyncEventWithTimestamp::new(ev.clone());
                    txn.put::<SyncEventSchema>(&id, &event)?;
                    Ok::<_, anyhow::Error>(id)
                },
            )
            .map_err(|err| DbError::TransactionError(err.to_string()))
    }

    fn clear_sync_event_range(&self, start_idx: u64, end_idx: u64) -> DbResult<()> {
        if start_idx >= end_idx {
            return Err(DbError::Other(
                "start_idx must be less than end_idx".to_string(),
            ));
        }

        match self.get_last_key()? {
            Some(last_key) => {
                if end_idx > last_key {
                    return Err(DbError::Other(
                        "end_idx must be less than or equal to last_key".to_string(),
                    ));
                }
            }
            None => return Err(DbError::Other("cannot clear empty db".to_string())),
        }

        let iterator = self.db.iter::<SyncEventSchema>()?;

        // TODO: determine if the expectation behaviour for this is to clear early events or clear
        // late events. The implementation is based for early events
        let mut batch = SchemaBatch::new();

        for res in iterator {
            let (id, _) = res?.into_tuple();
            if id >= end_idx {
                break;
            }

            if id >= start_idx {
                batch.delete::<SyncEventSchema>(&id)?;
            }
        }
        self.db.write_schemas(batch)?;
        Ok(())
    }

    fn get_last_idx(&self) -> DbResult<Option<u64>> {
        self.get_last_key()
    }

    fn get_sync_event(&self, idx: u64) -> DbResult<Option<SyncEvent>> {
        let event = self.db.get::<SyncEventSchema>(&idx)?;
        match event {
            Some(ev) => Ok(Some(ev.event())),
            None => Ok(None),
        }
    }

    fn get_event_timestamp(&self, idx: u64) -> DbResult<Option<u64>> {
        let event = self.db.get::<SyncEventSchema>(&idx)?;
        match event {
            Some(ev) => Ok(Some(ev.timestamp())),
            None => Ok(None),
        }
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::sync_event_tests;

    use super::*;
    use crate::test_utils::get_rocksdb_tmp_instance;

    fn setup_db() -> SyncEventDb {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        SyncEventDb::new(db, db_ops)
    }

    #[test]
    fn test_get_sync_event() {
        let db = setup_db();
        sync_event_tests::test_get_sync_event(&db);
    }

    #[test]
    fn test_get_last_idx_1() {
        let db = setup_db();
        sync_event_tests::test_get_last_idx_1(&db);
    }

    #[test]
    fn test_get_timestamp() {
        let db = setup_db();
        sync_event_tests::test_get_timestamp(&db);
    }

    #[test]
    fn test_clear_sync_event() {
        let db = setup_db();
        sync_event_tests::test_clear_sync_event(&db);
    }

    #[test]
    fn test_clear_sync_event_2() {
        let db = setup_db();
        sync_event_tests::test_clear_sync_event_2(&db);
    }

    #[test]
    fn test_get_last_idx_2() {
        let db = setup_db();
        sync_event_tests::test_get_last_idx_2(&db);
    }
}
