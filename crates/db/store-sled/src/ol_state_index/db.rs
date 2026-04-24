//! Sled-backed [`OLStateIndexingDatabase`] implementation.

use std::ops::Bound;

use strata_db_types::{
    DbResult,
    ol_state_index::{
        AccountEpochRecord, BlockIndexingRecord, CommonEpochRecord, EpochIndexingData,
    },
    traits::OLStateIndexingDatabase,
};
use strata_identifiers::{AccountId, Epoch, OLBlockCommitment};

use super::schemas::{
    OLAccountCreationEpochSchema, OLAccountEpochSchema, OLBlockIndexingSchema, OLCommonEpochSchema,
};
use crate::define_sled_database;

define_sled_database!(
    pub struct OLStateIndexingDBSled {
        common_tree: OLCommonEpochSchema,
        account_epoch_tree: OLAccountEpochSchema,
        creation_epoch_tree: OLAccountCreationEpochSchema,
        block_indexing_tree: OLBlockIndexingSchema,
    }
);

// TODO: make apply_epoch_indexing atomic across the four trees. Sled
// transactional views don't expose range iteration, so we currently do the
// block-indexing scan outside any transaction and perform writes
// tree-by-tree. A crash mid-apply can leave partial state that a reader must
// tolerate.
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
        let mut block_index_keys: Vec<(Epoch, OLBlockCommitment)> = Vec::new();
        for item in self
            .block_indexing_tree
            .range((Bound::Included(&start), Bound::Excluded(&end)))?
        {
            let (key, _) = item?;
            block_index_keys.push(key);
        }
        for key in &block_index_keys {
            self.block_indexing_tree.remove(key)?;
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

    fn put_block_indexing(
        &self,
        epoch: Epoch,
        block: OLBlockCommitment,
        record: BlockIndexingRecord,
    ) -> DbResult<()> {
        self.block_indexing_tree.insert(&(epoch, block), &record)?;
        Ok(())
    }

    fn get_epoch_block_indexing(
        &self,
        epoch: Epoch,
    ) -> DbResult<Vec<(OLBlockCommitment, BlockIndexingRecord)>> {
        let start = (epoch, OLBlockCommitment::null());
        let end = (epoch.saturating_add(1), OLBlockCommitment::null());
        let mut out = Vec::new();
        for item in self
            .block_indexing_tree
            .range((Bound::Included(&start), Bound::Excluded(&end)))?
        {
            let ((_, block), rec) = item?;
            out.push((block, rec));
        }
        Ok(out)
    }
}
