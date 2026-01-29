//! Sled implementation of L1DaBlobDatabase.

use strata_db_types::{
    errors::DbError,
    traits::L1DaBlobDatabase,
    types::{DaBlobEntry, DaBlobStatusDb, DaChunkEntry},
    DbResult,
};
use strata_primitives::buf::Buf32;

use super::schemas::{DaBlobSchema, DaChunkSchema, DaLastChunkWtxidSchema};
use crate::define_sled_database;

define_sled_database!(
    /// Sled implementation for DA blob storage.
    pub struct L1DaBlobDBSled {
        blob_tree: DaBlobSchema,
        chunk_tree: DaChunkSchema,
        last_wtxid_tree: DaLastChunkWtxidSchema,
    }
);

impl L1DaBlobDBSled {
    fn get_next_chunk_idx(&self) -> DbResult<u64> {
        match self.chunk_tree.last()? {
            Some((idx, _)) => Ok(idx + 1),
            None => Ok(0),
        }
    }
}

impl L1DaBlobDatabase for L1DaBlobDBSled {
    fn put_da_blob(&self, blob_id: &Buf32, entry: DaBlobEntry) -> DbResult<()> {
        self.blob_tree.insert(blob_id, &entry)?;
        Ok(())
    }

    fn get_da_blob(&self, blob_id: &Buf32) -> DbResult<Option<DaBlobEntry>> {
        Ok(self.blob_tree.get(blob_id)?)
    }

    fn update_da_blob_status(&self, blob_id: &Buf32, status: DaBlobStatusDb) -> DbResult<()> {
        if let Some(mut entry) = self.blob_tree.get(blob_id)? {
            entry.status = status;
            self.blob_tree.insert(blob_id, &entry)?;
            Ok(())
        } else {
            Err(DbError::Other(format!(
                "DA blob not found: {:?}",
                blob_id
            )))
        }
    }

    fn del_da_blob(&self, blob_id: &Buf32) -> DbResult<bool> {
        let old_item = self.blob_tree.get(blob_id)?;
        let exists = old_item.is_some();
        if exists {
            self.blob_tree.compare_and_swap(*blob_id, old_item, None)?;
        }
        Ok(exists)
    }

    fn get_pending_da_blobs(&self) -> DbResult<Vec<DaBlobEntry>> {
        let mut pending = Vec::new();
        for entry_result in self.blob_tree.iter() {
            let (_, entry) = entry_result?;
            match entry.status {
                DaBlobStatusDb::Pending | DaBlobStatusDb::CommitConfirmed { .. } => {
                    pending.push(entry);
                }
                _ => {}
            }
        }
        Ok(pending)
    }

    fn put_da_chunk(&self, idx: u64, entry: DaChunkEntry) -> DbResult<()> {
        self.chunk_tree.insert(&idx, &entry)?;
        Ok(())
    }

    fn get_da_chunk(&self, idx: u64) -> DbResult<Option<DaChunkEntry>> {
        Ok(self.chunk_tree.get(&idx)?)
    }

    fn get_next_da_chunk_idx(&self) -> DbResult<u64> {
        self.get_next_chunk_idx()
    }

    fn del_da_chunk(&self, idx: u64) -> DbResult<bool> {
        let old_item = self.chunk_tree.get(&idx)?;
        let exists = old_item.is_some();
        if exists {
            self.chunk_tree.compare_and_swap(idx, old_item, None)?;
        }
        Ok(exists)
    }

    fn put_da_last_chunk_wtxid(&self, tag: [u8; 4], wtxid: [u8; 32]) -> DbResult<()> {
        self.last_wtxid_tree.insert(&tag, &wtxid)?;
        Ok(())
    }

    fn get_da_last_chunk_wtxid(&self, tag: [u8; 4]) -> DbResult<Option<[u8; 32]>> {
        Ok(self.last_wtxid_tree.get(&tag)?)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::SledDbConfig;

    fn setup_db() -> L1DaBlobDBSled {
        let db = sled::Config::new().temporary(true).open().unwrap();
        let sled_db = typed_sled::SledDb::new(db).unwrap();
        let config = SledDbConfig::test();
        L1DaBlobDBSled::new(sled_db.into(), config).unwrap()
    }

    #[test]
    fn test_blob_crud() {
        let db = setup_db();
        let blob_id = Buf32::zero();
        let entry = DaBlobEntry::new_pending(
            blob_id,
            [0x42; 32],
            1000,
            3,
            *b"EEDA",
            Buf32::zero(),
        );

        // Create
        db.put_da_blob(&blob_id, entry.clone()).unwrap();

        // Read
        let retrieved = db.get_da_blob(&blob_id).unwrap().unwrap();
        assert_eq!(retrieved.blob_hash, [0x42; 32]);
        assert_eq!(retrieved.total_chunks, 3);

        // Update status
        db.update_da_blob_status(&blob_id, DaBlobStatusDb::AllRevealsConfirmed)
            .unwrap();
        let updated = db.get_da_blob(&blob_id).unwrap().unwrap();
        assert_eq!(updated.status, DaBlobStatusDb::AllRevealsConfirmed);

        // Delete
        let deleted = db.del_da_blob(&blob_id).unwrap();
        assert!(deleted);
        assert!(db.get_da_blob(&blob_id).unwrap().is_none());
    }

    #[test]
    fn test_chunk_crud() {
        let db = setup_db();

        let chunk = DaChunkEntry::new_unsigned(
            Buf32::zero(),
            0,
            3,
            [0x42; 32],
            [0; 32],
            Buf32::zero(),
            Buf32::zero(),
        );

        // Get next index (should be 0)
        let idx = db.get_next_da_chunk_idx().unwrap();
        assert_eq!(idx, 0);

        // Insert
        db.put_da_chunk(idx, chunk.clone()).unwrap();

        // Next index should be 1
        let next_idx = db.get_next_da_chunk_idx().unwrap();
        assert_eq!(next_idx, 1);

        // Read
        let retrieved = db.get_da_chunk(idx).unwrap().unwrap();
        assert_eq!(retrieved.chunk_index, 0);

        // Delete
        let deleted = db.del_da_chunk(idx).unwrap();
        assert!(deleted);
    }

    #[test]
    fn test_last_chunk_wtxid() {
        let db = setup_db();
        let tag = *b"EEDA";
        let wtxid = [0x42; 32];

        // Initially none
        assert!(db.get_da_last_chunk_wtxid(tag).unwrap().is_none());

        // Set
        db.put_da_last_chunk_wtxid(tag, wtxid).unwrap();

        // Get
        let retrieved = db.get_da_last_chunk_wtxid(tag).unwrap().unwrap();
        assert_eq!(retrieved, wtxid);
    }

    #[test]
    fn test_get_pending_blobs() {
        let db = setup_db();

        // Create some blobs with different statuses
        let blob1 = Buf32([1; 32]);
        let blob2 = Buf32([2; 32]);
        let blob3 = Buf32([3; 32]);

        db.put_da_blob(
            &blob1,
            DaBlobEntry::new_pending(blob1, [1; 32], 100, 1, *b"EEDA", Buf32::zero()),
        )
        .unwrap();

        let mut entry2 =
            DaBlobEntry::new_pending(blob2, [2; 32], 200, 2, *b"EEDA", Buf32::zero());
        entry2.status = DaBlobStatusDb::CommitConfirmed {
            reveals_confirmed: 1,
        };
        db.put_da_blob(&blob2, entry2).unwrap();

        let mut entry3 =
            DaBlobEntry::new_pending(blob3, [3; 32], 300, 3, *b"EEDA", Buf32::zero());
        entry3.status = DaBlobStatusDb::Finalized;
        db.put_da_blob(&blob3, entry3).unwrap();

        // Should only get pending and commit_confirmed
        let pending = db.get_pending_da_blobs().unwrap();
        assert_eq!(pending.len(), 2);
    }
}
