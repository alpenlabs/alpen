use std::sync::Arc;

use strata_db::{
    DbResult,
    errors::DbError,
    traits::L1WriterDatabase,
    types::{BundledPayloadEntry, IntentEntry},
};
use strata_primitives::buf::Buf32;
use typed_sled::{SledDb, SledTree};

use super::schemas::{IntentIdxSchema, IntentSchema, PayloadSchema};
use crate::{
    SledDbConfig,
    utils::{find_next_available_id, first},
};

#[derive(Debug)]
pub struct L1WriterDBSled {
    payload_tree: SledTree<PayloadSchema>,
    intent_tree: SledTree<IntentSchema>,
    intent_idx_tree: SledTree<IntentIdxSchema>,
    config: SledDbConfig,
}

impl L1WriterDBSled {
    pub fn new(db: Arc<SledDb>, config: SledDbConfig) -> DbResult<Self> {
        Ok(Self {
            payload_tree: db.get_tree()?,
            intent_tree: db.get_tree()?,
            intent_idx_tree: db.get_tree()?,
            config,
        })
    }
}

impl L1WriterDatabase for L1WriterDBSled {
    fn put_payload_entry(&self, idx: u64, entry: BundledPayloadEntry) -> DbResult<()> {
        self.payload_tree.insert(&idx, &entry)?;
        Ok(())
    }

    fn get_payload_entry_by_idx(&self, idx: u64) -> DbResult<Option<BundledPayloadEntry>> {
        Ok(self.payload_tree.get(&idx)?)
    }

    fn get_next_payload_idx(&self) -> DbResult<u64> {
        Ok(self
            .payload_tree
            .last()?
            .map(first)
            .map(|x| x + 1)
            .unwrap_or(0))
    }

    fn put_intent_entry(&self, intent_id: Buf32, intent_entry: IntentEntry) -> DbResult<()> {
        let next_idx = self
            .intent_idx_tree
            .last()?
            .map(first)
            .map(|x| x + 1)
            .unwrap_or(0);
        self.config
            .with_retry((&self.intent_idx_tree, &self.intent_tree), |(iit, it)| {
                let nxt = find_next_available_id(&iit, next_idx)?;
                iit.insert(&nxt, &intent_id)?;
                it.insert(&intent_id, &intent_entry)?;
                Ok(())
            })
    }

    fn get_intent_by_id(&self, id: Buf32) -> DbResult<Option<IntentEntry>> {
        Ok(self.intent_tree.get(&id)?)
    }

    fn get_intent_by_idx(&self, idx: u64) -> DbResult<Option<IntentEntry>> {
        if let Some(id) = self.intent_idx_tree.get(&idx)? {
            self.intent_tree
                .get(&id)?
                .ok_or_else(|| {
                    DbError::Other(format!(
                        "Intent index({idx}) exists but corresponding id does not exist in writer db"
                    ))
                })
                .map(Some)
        } else {
            Ok(None)
        }
    }

    fn get_next_intent_idx(&self) -> DbResult<u64> {
        Ok(self
            .intent_idx_tree
            .last()?
            .map(first)
            .map(|x| x + 1)
            .unwrap_or(0))
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::l1_writer_db_tests;

    use super::*;

    fn setup_db() -> L1WriterDBSled {
        let db = sled::Config::new().temporary(true).open().unwrap();
        let sled_db = SledDb::new(db).unwrap();
        let config = SledDbConfig::new_with_constant_backoff(3, 200);
        L1WriterDBSled::new(sled_db.into(), config).unwrap()
    }

    l1_writer_db_tests!(setup_db());
}
