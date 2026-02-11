use strata_db_types::{
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
    }
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

    fn put_intent_entry(&self, intent_id: Buf32, intent_entry: IntentEntry) -> DbResult<u64> {
        let next_idx = self
            .intent_idx_tree
            .last()?
            .map(first)
            .map(|x| x + 1)
            .unwrap_or(0);
        let idx =
            self.config
                .with_retry((&self.intent_idx_tree, &self.intent_tree), |(iit, it)| {
                    let nxt = find_next_available_id(&iit, next_idx)?;
                    iit.insert(&nxt, &intent_id)?;
                    it.insert(&intent_id, &intent_entry)?;
                    Ok(nxt)
                })?;
        Ok(idx)
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

    fn del_payload_entry(&self, idx: u64) -> DbResult<bool> {
        let exists = self.payload_tree.contains_key(&idx)?;
        if exists {
            self.payload_tree.remove(&idx)?;
        }
        Ok(exists)
    }

    fn del_payload_entries_from_idx(&self, start_idx: u64) -> DbResult<Vec<u64>> {
        let last_idx = self.payload_tree.last()?.map(first);
        let Some(last_idx) = last_idx else {
            return Ok(Vec::new());
        };

        if start_idx > last_idx {
            return Ok(Vec::new());
        }

        let deleted_indices = self.config.with_retry((&self.payload_tree,), |(ptree,)| {
            let mut deleted_indices = Vec::new();
            for idx in start_idx..=last_idx {
                if ptree.contains_key(&idx)? {
                    ptree.remove(&idx)?;
                    deleted_indices.push(idx);
                }
            }
            Ok(deleted_indices)
        })?;
        Ok(deleted_indices)
    }

    fn del_intent_entry(&self, id: Buf32) -> DbResult<bool> {
        let exists = self.intent_tree.contains_key(&id)?;
        if !exists {
            return Ok(false);
        }

        // Get the last index to know the range to scan
        let last_idx = self.intent_idx_tree.last()?.map(first);

        // Delete both the intent entry and its index mapping
        self.config
            .with_retry((&self.intent_idx_tree, &self.intent_tree), |(iit, it)| {
                // Find ALL index entries pointing to this ID by scanning IntentIdxSchema
                // Note: IDs are not unique, multiple indices can point to the same ID
                let mut indices_to_delete = Vec::new();
                if let Some(last_idx) = last_idx {
                    for idx in 0..=last_idx {
                        if let Some(intent_id) = iit.get(&idx)?
                            && intent_id == id
                        {
                            indices_to_delete.push(idx);
                        }
                    }
                }

                // Delete all index mappings found
                for idx in indices_to_delete {
                    iit.remove(&idx)?;
                }

                // Delete the intent entry
                it.remove(&id)?;
                Ok(())
            })?;

        Ok(true)
    }

    fn del_intent_entries_from_idx(&self, start_idx: u64) -> DbResult<Vec<u64>> {
        let last_idx = self.intent_idx_tree.last()?.map(first);
        let Some(last_idx) = last_idx else {
            return Ok(Vec::new());
        };

        if start_idx > last_idx {
            return Ok(Vec::new());
        }

        self.config
            .with_retry((&self.intent_idx_tree, &self.intent_tree), |(iit, it)| {
                let mut deleted_indices = Vec::new();
                for idx in start_idx..=last_idx {
                    if let Some(intent_id) = iit.get(&idx)? {
                        iit.remove(&idx)?;
                        it.remove(&intent_id)?;
                        deleted_indices.push(idx);
                    }
                }
                Ok(deleted_indices)
            })
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
