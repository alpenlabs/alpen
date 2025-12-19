//! Sled database implementation for OL state storage.

use strata_acct_types::{AccountId, Mmr64};
use strata_db_types::{DbError, DbResult, OLFinalizedState, OLStateDatabase, OLWriteBatch};
use strata_primitives::{buf::Buf32, l1::L1Height, l2::OLBlockId};
use strata_snark_acct_types::MessageEntry;

use super::schemas::{
    FinalizedStateSchema, InboxMessageKey, InboxMessageSchema, ManifestEntrySchema,
    ManifestMmrSchema, SlotWriteBatchSchema,
};
use crate::define_sled_database;

/// MMR capacity as log2 (2^64 leaves max)
const MMR_CAP_LOG2: usize = 64;

define_sled_database!(
    pub struct OLStateDBSled {
        slot_write_batch_tree: SlotWriteBatchSchema,
        finalized_state_tree: FinalizedStateSchema,
        manifest_entry_tree: ManifestEntrySchema,
        manifest_mmr_tree: ManifestMmrSchema,
        inbox_message_tree: InboxMessageSchema,
    }
);

impl OLStateDBSled {
    /// Load the MMR from database, or create a new one if not present
    fn load_or_create_mmr(&self) -> DbResult<Mmr64> {
        match self.manifest_mmr_tree.get(&())? {
            Some(compact) => Ok(Mmr64::from_compact(&compact)),
            None => Ok(Mmr64::new(MMR_CAP_LOG2)),
        }
    }

    /// Save the MMR to database
    fn save_mmr(&self, mmr: &Mmr64) -> DbResult<()> {
        let compact = mmr.to_compact();
        self.manifest_mmr_tree.insert(&(), &compact)?;
        Ok(())
    }
}

impl OLStateDatabase for OLStateDBSled {
    fn put_slot_write_batch(&self, slot_blkid: OLBlockId, wb: OLWriteBatch) -> DbResult<()> {
        self.slot_write_batch_tree.insert(&slot_blkid, &wb)?;
        Ok(())
    }

    fn get_slot_write_batch(&self, slot_blkid: OLBlockId) -> DbResult<Option<OLWriteBatch>> {
        Ok(self.slot_write_batch_tree.get(&slot_blkid)?)
    }

    fn put_finalized_state(&self, state: OLFinalizedState) -> DbResult<()> {
        self.finalized_state_tree.insert(&(), &state)?;
        Ok(())
    }

    fn get_finalized_state(&self) -> DbResult<Option<OLFinalizedState>> {
        Ok(self.finalized_state_tree.get(&())?)
    }

    fn append_manifest_entry(&self, height: L1Height, manifest_hash: Buf32) -> DbResult<()> {
        // Store the entry in the manifest entry tree (for height-based lookups if needed)
        self.config
            .with_retry((&self.manifest_entry_tree,), |(entry_tree,)| {
                let mut entries = entry_tree.get(&height)?.unwrap_or_default();
                entries.push(manifest_hash);
                entry_tree.insert(&height, &entries)?;
                Ok(())
            })?;

        // Load MMR, append leaf, save
        let mut mmr = self.load_or_create_mmr()?;
        mmr.add_leaf(manifest_hash.0)
            .map_err(|e| DbError::Other(format!("MMR add_leaf failed: {e}")))?;
        self.save_mmr(&mmr)?;

        Ok(())
    }

    fn get_manifest_mmr_root(&self) -> DbResult<Buf32> {
        let mmr = self.load_or_create_mmr()?;
        if mmr.num_entries() == 0 {
            return Err(DbError::Other("No MMR root found".into()));
        }
        Ok(Buf32(
            mmr.peaks_slice()
                .last()
                .expect("MMR must have at least one peak")
                .0,
        ))
    }

    fn put_inbox_message(
        &self,
        acct_id: AccountId,
        msg_idx: u64,
        entry: MessageEntry,
    ) -> DbResult<()> {
        let key = InboxMessageKey::new(acct_id, msg_idx);
        self.inbox_message_tree.insert(&key, &entry)?;
        Ok(())
    }

    fn get_inbox_messages(
        &self,
        acct_id: AccountId,
        from_idx: u64,
        count: u32,
    ) -> DbResult<Vec<MessageEntry>> {
        let mut messages = Vec::new();

        for idx in from_idx..(from_idx + count as u64) {
            let key = InboxMessageKey::new(acct_id, idx);
            if let Some(entry) = self.inbox_message_tree.get(&key)? {
                messages.push(entry);
            } else {
                // Stop when we encounter a missing message
                break;
            }
        }

        Ok(messages)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use super::*;

    fn _setup_db() -> OLStateDBSled {
        let db = sled::Config::new().temporary(true).open().unwrap();
        let sled_db = typed_sled::SledDb::new(db).unwrap();
        let config = crate::SledDbConfig::test();
        OLStateDBSled::new(sled_db.into(), config).unwrap()
    }

    #[test]
    #[ignore] // TODO: Implement comprehensive tests
    fn test_write_batch_storage() {
        let _db = _setup_db();
        // Add basic tests here
    }

    #[test]
    #[ignore] // TODO: Implement comprehensive tests
    fn test_finalized_state_storage() {
        let _db = _setup_db();
        // Add basic tests here
    }

    #[test]
    #[ignore] // TODO: Implement comprehensive tests
    fn test_manifest_entry_append() {
        let _db = _setup_db();
        // Add basic tests here
    }

    #[test]
    #[ignore] // TODO: Implement comprehensive tests
    fn test_inbox_messages() {
        let _db = _setup_db();
        // Add basic tests here
    }
}
