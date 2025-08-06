use std::sync::Arc;

use strata_db::{DbResult, errors::DbError, traits::SyncEventDatabase};
use strata_state::sync_event::SyncEvent;
use typed_sled::{SledDb, SledTree, batch::SledBatch};

use super::schemas::{SyncEventSchema, SyncEventWithTimestamp};
use crate::{
    SledDbConfig,
    utils::{find_next_available_id, first},
};

#[derive(Debug)]
pub struct SyncEventDBSled {
    sync_event_tree: SledTree<SyncEventSchema>,
    config: SledDbConfig,
}

impl SyncEventDBSled {
    pub fn new(db: Arc<SledDb>, config: SledDbConfig) -> DbResult<Self> {
        Ok(Self {
            sync_event_tree: db.get_tree()?,
            config,
        })
    }

    fn get_last_key(&self) -> DbResult<Option<u64>> {
        Ok(self.sync_event_tree.last()?.map(first))
    }
}

impl SyncEventDatabase for SyncEventDBSled {
    fn write_sync_event(&self, ev: SyncEvent) -> DbResult<u64> {
        let id = match self.get_last_key()? {
            Some(last_id) => last_id + 1,
            None => 1, // autoincrementing, starting from index 1
        };

        let event = SyncEventWithTimestamp::new(ev);
        let result = self
            .config
            .with_retry((&self.sync_event_tree,), |(se_tree,)| {
                let nxt = find_next_available_id(&se_tree, id)?;
                se_tree.insert(&nxt, &event)?;
                Ok(nxt)
            })?;
        Ok(result)
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

        let mut batch = SledBatch::<SyncEventSchema>::new();
        // Remove events in the specified range
        for id in start_idx..end_idx {
            batch.remove(id)?;
        }
        self.sync_event_tree.apply_batch(batch)?;
        Ok(())
    }

    fn get_last_idx(&self) -> DbResult<Option<u64>> {
        self.get_last_key()
    }

    fn get_sync_event(&self, idx: u64) -> DbResult<Option<SyncEvent>> {
        match self.sync_event_tree.get(&idx)? {
            Some(ev) => Ok(Some(ev.event())),
            None => Ok(None),
        }
    }

    fn get_event_timestamp(&self, idx: u64) -> DbResult<Option<u64>> {
        match self.sync_event_tree.get(&idx)? {
            Some(ev) => Ok(Some(ev.timestamp())),
            None => Ok(None),
        }
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::sync_event_db_tests;

    use super::*;

    fn setup_db() -> SyncEventDBSled {
        let db = sled::Config::new().temporary(true).open().unwrap();
        let sled_db = SledDb::new(db).unwrap();
        let config = SledDbConfig::new_with_constant_backoff(3, 200);
        SyncEventDBSled::new(sled_db.into(), config).unwrap()
    }

    sync_event_db_tests!(setup_db());
}
