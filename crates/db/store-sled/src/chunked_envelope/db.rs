use strata_db_types::{
    traits::L1ChunkedEnvelopeDatabase, types::ChunkedEnvelopeEntry, DbResult,
};

use super::schemas::ChunkedEnvelopeSchema;
use crate::{define_sled_database, utils::first};

define_sled_database!(
    pub struct L1ChunkedEnvelopeDBSled {
        entry_tree: ChunkedEnvelopeSchema,
    }
);

impl L1ChunkedEnvelopeDatabase for L1ChunkedEnvelopeDBSled {
    fn put_chunked_envelope_entry(&self, idx: u64, entry: ChunkedEnvelopeEntry) -> DbResult<()> {
        self.entry_tree.insert(&idx, &entry)?;
        Ok(())
    }

    fn get_chunked_envelope_entry(&self, idx: u64) -> DbResult<Option<ChunkedEnvelopeEntry>> {
        Ok(self.entry_tree.get(&idx)?)
    }

    fn get_next_chunked_envelope_idx(&self) -> DbResult<u64> {
        Ok(self
            .entry_tree
            .last()?
            .map(first)
            .map(|x| x + 1)
            .unwrap_or(0))
    }

    fn del_chunked_envelope_entry(&self, idx: u64) -> DbResult<bool> {
        let exists = self.entry_tree.contains_key(&idx)?;
        if exists {
            self.entry_tree.remove(&idx)?;
        }
        Ok(exists)
    }

    fn del_chunked_envelope_entries_from_idx(&self, start_idx: u64) -> DbResult<Vec<u64>> {
        let last_idx = self.entry_tree.last()?.map(first);
        let Some(last_idx) = last_idx else {
            return Ok(Vec::new());
        };

        if start_idx > last_idx {
            return Ok(Vec::new());
        }

        let deleted_indices = self.config.with_retry((&self.entry_tree,), |(tree,)| {
            let mut deleted = Vec::new();
            for idx in start_idx..=last_idx {
                if tree.contains_key(&idx)? {
                    tree.remove(&idx)?;
                    deleted.push(idx);
                }
            }
            Ok(deleted)
        })?;
        Ok(deleted_indices)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::l1_chunked_envelope_db_tests;

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(L1ChunkedEnvelopeDBSled, l1_chunked_envelope_db_tests);
}
