use strata_db::{
    DbResult,
    errors::DbError,
    traits::L1WriterDatabase,
    types::{BundledPayloadEntry, IntentEntry},
};
use strata_primitives::buf::Buf32;

use super::schemas::{IntentIdxSchema, IntentSchema, PayloadSchema};
use crate::{
    define_sled_database,
    utils::{find_next_available_id, first},
};

define_sled_database!(
    pub struct L1WriterDBSled {
        payload_tree: PayloadSchema,
        intent_tree: IntentSchema,
        intent_idx_tree: IntentIdxSchema,
    }, config: config
);

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
    use crate::sled_db_test_setup;

    sled_db_test_setup!(L1WriterDBSled, l1_writer_db_tests);
}
