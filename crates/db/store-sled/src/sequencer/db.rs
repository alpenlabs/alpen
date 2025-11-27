//! Sled implementation of the SequencerDatabase trait.

use strata_db_types::{DbResult, traits::SequencerDatabase};
use strata_ol_chain_types::L2BlockId;

use crate::{
    define_sled_database,
    sequencer::schemas::{ExecPayloadEntry, ExecPayloadSchema},
    utils::{first, to_db_error},
};

define_sled_database!(
    /// Sled database for sequencer-specific data.
    pub struct SequencerDBSled {
        exec_payload_tree: ExecPayloadSchema,
    }
);

impl SequencerDatabase for SequencerDBSled {
    fn put_exec_payload(&self, slot: u64, block_id: L2BlockId, payload: Vec<u8>) -> DbResult<()> {
        let entry = ExecPayloadEntry::new(block_id, payload);
        self.config
            .with_retry((&self.exec_payload_tree,), |(tree,)| {
                tree.insert(&slot, &entry)?;
                Ok(())
            })
            .map_err(to_db_error)
    }

    fn get_exec_payload(&self, slot: u64) -> DbResult<Option<(L2BlockId, Vec<u8>)>> {
        Ok(self
            .exec_payload_tree
            .get(&slot)?
            .map(|entry| (entry.block_id, entry.payload)))
    }

    fn get_last_exec_payload_slot(&self) -> DbResult<Option<u64>> {
        Ok(self.exec_payload_tree.last()?.map(first))
    }

    fn get_exec_payloads_in_range(
        &self,
        start_slot: u64,
        end_slot: u64,
    ) -> DbResult<Vec<(u64, L2BlockId, Vec<u8>)>> {
        let mut results = Vec::new();
        for slot in start_slot..=end_slot {
            if let Some(entry) = self.exec_payload_tree.get(&slot)? {
                results.push((slot, entry.block_id, entry.payload));
            }
        }
        Ok(results)
    }

    fn del_exec_payloads_from_slot(&self, start_slot: u64) -> DbResult<Vec<u64>> {
        // Get the last slot to determine the range to delete
        let last_slot = match self.get_last_exec_payload_slot()? {
            Some(slot) => slot,
            None => return Ok(Vec::new()), // No entries to delete
        };

        if start_slot > last_slot {
            return Ok(Vec::new());
        }

        let deleted = self
            .config
            .with_retry((&self.exec_payload_tree,), |(tree,)| {
                let mut deleted = Vec::new();
                for slot in start_slot..=last_slot {
                    if tree.contains_key(&slot)? {
                        tree.remove(&slot)?;
                        deleted.push(slot);
                    }
                }
                Ok(deleted)
            })
            .map_err(to_db_error)?;

        Ok(deleted)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::SledDbConfig;

    fn setup_db() -> SequencerDBSled {
        let db = sled::Config::new().temporary(true).open().unwrap();
        let sled_db = typed_sled::SledDb::new(db).unwrap();
        let config = SledDbConfig::test();
        SequencerDBSled::new(sled_db.into(), config).unwrap()
    }

    #[test]
    fn test_put_and_get_exec_payload() {
        let db = setup_db();
        let mut arb = ArbitraryGenerator::new();
        let block_id: L2BlockId = arb.generate();
        let payload = vec![1, 2, 3, 4, 5];

        db.put_exec_payload(100, block_id, payload.clone()).unwrap();

        let result = db.get_exec_payload(100).unwrap();
        assert!(result.is_some());
        let (retrieved_id, retrieved_payload) = result.unwrap();
        assert_eq!(retrieved_id, block_id);
        assert_eq!(retrieved_payload, payload);
    }

    #[test]
    fn test_get_last_exec_payload_slot() {
        let db = setup_db();
        let mut arb = ArbitraryGenerator::new();

        // Initially empty
        assert!(db.get_last_exec_payload_slot().unwrap().is_none());

        // Add some entries
        let block_id1: L2BlockId = arb.generate();
        let block_id2: L2BlockId = arb.generate();
        let block_id3: L2BlockId = arb.generate();

        db.put_exec_payload(10, block_id1, vec![1]).unwrap();
        db.put_exec_payload(20, block_id2, vec![2]).unwrap();
        db.put_exec_payload(15, block_id3, vec![3]).unwrap();

        assert_eq!(db.get_last_exec_payload_slot().unwrap(), Some(20));
    }

    #[test]
    fn test_get_exec_payloads_in_range() {
        let db = setup_db();
        let mut arb = ArbitraryGenerator::new();

        let block_id1: L2BlockId = arb.generate();
        let block_id2: L2BlockId = arb.generate();
        let block_id3: L2BlockId = arb.generate();

        db.put_exec_payload(10, block_id1, vec![1]).unwrap();
        db.put_exec_payload(12, block_id2, vec![2]).unwrap();
        db.put_exec_payload(15, block_id3, vec![3]).unwrap();

        let results = db.get_exec_payloads_in_range(10, 13).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, 10);
        assert_eq!(results[1].0, 12);
    }

    #[test]
    fn test_del_exec_payloads_from_slot() {
        let db = setup_db();
        let mut arb = ArbitraryGenerator::new();

        let block_id1: L2BlockId = arb.generate();
        let block_id2: L2BlockId = arb.generate();
        let block_id3: L2BlockId = arb.generate();

        db.put_exec_payload(10, block_id1, vec![1]).unwrap();
        db.put_exec_payload(20, block_id2, vec![2]).unwrap();
        db.put_exec_payload(30, block_id3, vec![3]).unwrap();

        let deleted = db.del_exec_payloads_from_slot(20).unwrap();
        assert_eq!(deleted.len(), 2);
        assert!(deleted.contains(&20));
        assert!(deleted.contains(&30));

        // Verify only slot 10 remains
        assert!(db.get_exec_payload(10).unwrap().is_some());
        assert!(db.get_exec_payload(20).unwrap().is_none());
        assert!(db.get_exec_payload(30).unwrap().is_none());
    }
}
