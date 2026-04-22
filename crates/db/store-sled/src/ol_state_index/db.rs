//! Sled-backed [`OLStateIndexingDatabase`] implementation.

use std::ops::Bound;

use strata_db_types::{
    DbResult,
    ol_state_index::{
        AccountEpochRecord, CommonEpochRecord, EpochIndexingData, PerBlockStagingRecord,
    },
    traits::OLStateIndexingDatabase,
};
use strata_identifiers::{AccountId, Epoch, OLBlockCommitment};

use super::schemas::{
    OLAccountCreationEpochSchema, OLAccountEpochSchema, OLCommonEpochSchema, OLIndexStagingSchema,
};
use crate::define_sled_database;

define_sled_database!(
    pub struct OLStateIndexingDBSled {
        common_tree: OLCommonEpochSchema,
        account_epoch_tree: OLAccountEpochSchema,
        creation_epoch_tree: OLAccountCreationEpochSchema,
        staging_tree: OLIndexStagingSchema,
    }
);

impl OLStateIndexingDBSled {
    /// Persists a per-block staging record.
    pub fn put_block_staging(
        &self,
        epoch: Epoch,
        block: OLBlockCommitment,
        record: PerBlockStagingRecord,
    ) -> DbResult<()> {
        self.staging_tree.insert(&(epoch, block), &record)?;
        Ok(())
    }

    /// Returns all staging records for an epoch, in (block commitment) key order.
    pub fn get_epoch_staging(
        &self,
        epoch: Epoch,
    ) -> DbResult<Vec<(OLBlockCommitment, PerBlockStagingRecord)>> {
        let start = (epoch, OLBlockCommitment::null());
        let end = (epoch.saturating_add(1), OLBlockCommitment::null());
        let mut out = Vec::new();
        for item in self
            .staging_tree
            .range((Bound::Included(&start), Bound::Excluded(&end)))?
        {
            let ((_, block), rec) = item?;
            out.push((block, rec));
        }
        Ok(out)
    }
}

// TODO: make apply_epoch_indexing atomic across the four trees. Sled
// transactional views don't expose range iteration, so we currently do the
// staging scan outside any transaction and perform writes tree-by-tree. A
// crash mid-apply can leave partial state that a reader must tolerate.
impl OLStateIndexingDatabase for OLStateIndexingDBSled {
    fn apply_epoch_indexing(&self, data: EpochIndexingData) -> DbResult<()> {
        let epoch = data.epoch();

        self.common_tree.insert(&epoch, data.common())?;

        for acct in data.common().accounts_created() {
            self.creation_epoch_tree.insert(acct, &epoch)?;
        }

        for (acct, record) in data.accounts() {
            self.account_epoch_tree.insert(&(*acct, epoch), record)?;
        }

        let start = (epoch, OLBlockCommitment::null());
        let end = (epoch.saturating_add(1), OLBlockCommitment::null());
        let mut staging_keys: Vec<(Epoch, OLBlockCommitment)> = Vec::new();
        for item in self
            .staging_tree
            .range((Bound::Included(&start), Bound::Excluded(&end)))?
        {
            let (key, _) = item?;
            staging_keys.push(key);
        }
        for key in &staging_keys {
            self.staging_tree.remove(key)?;
        }

        Ok(())
    }

    fn get_common_epoch_record(&self, epoch: Epoch) -> DbResult<Option<CommonEpochRecord>> {
        Ok(self.common_tree.get(&epoch)?)
    }

    fn get_account_epoch_record(
        &self,
        acct: AccountId,
        epoch: Epoch,
    ) -> DbResult<Option<AccountEpochRecord>> {
        Ok(self.account_epoch_tree.get(&(acct, epoch))?)
    }

    fn get_account_creation_epoch(&self, acct: AccountId) -> DbResult<Option<Epoch>> {
        Ok(self.creation_epoch_tree.get(&acct)?)
    }
}
