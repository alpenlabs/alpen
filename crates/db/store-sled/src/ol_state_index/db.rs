//! Sled-backed [`OLStateIndexingDatabase`] implementation.

use sled::transaction::ConflictableTransactionError;
use strata_db_types::{
    DbError, DbResult,
    ol_state_index::{
        AccountEpochKey, AccountInboxEntry, AccountUpdateEntry, BlockIndexingWrites,
        EpochIndexingData, EpochIndexingWrites,
    },
    traits::OLStateIndexingDatabase,
};
use strata_identifiers::{AccountId, Epoch, EpochCommitment};
use typed_sled::{error::Error as TSledError, tree::SledTransactionalTree};

use super::schemas::{
    OLAccountCreationEpochSchema, OLAccountInboxEntrySchema, OLAccountUpdateEntrySchema,
    OLEpochIndexingDataSchema,
};
use crate::define_sled_database;

define_sled_database!(
    pub struct OLStateIndexingDBSled {
        epoch_data_tree: OLEpochIndexingDataSchema,
        account_update_tree: OLAccountUpdateEntrySchema,
        account_inbox_tree: OLAccountInboxEntrySchema,
        creation_epoch_tree: OLAccountCreationEpochSchema,
    }
);

type Trees = (
    SledTransactionalTree<OLEpochIndexingDataSchema>,
    SledTransactionalTree<OLAccountUpdateEntrySchema>,
    SledTransactionalTree<OLAccountInboxEntrySchema>,
    SledTransactionalTree<OLAccountCreationEpochSchema>,
);

impl OLStateIndexingDatabase for OLStateIndexingDBSled {
    fn apply_epoch_indexing(&self, writes: EpochIndexingWrites) -> DbResult<()> {
        self.config.with_retry(
            (
                &self.epoch_data_tree,
                &self.account_update_tree,
                &self.account_inbox_tree,
                &self.creation_epoch_tree,
            ),
            |(epoch_t, update_t, inbox_t, creation_t): Trees| {
                let epoch = writes.epoch;
                for acct in writes.common.created_accounts() {
                    creation_t.insert(acct, &epoch)?;
                }
                epoch_t.insert(&epoch, &writes.common)?;

                for (acct, entry) in &writes.account_updates {
                    update_t.insert(&AccountEpochKey::new(epoch, *acct), entry)?;
                }
                for (acct, entry) in &writes.account_inbox {
                    inbox_t.insert(&AccountEpochKey::new(epoch, *acct), entry)?;
                }

                Ok(())
            },
        )
    }

    fn apply_block_indexing(&self, writes: BlockIndexingWrites) -> DbResult<()> {
        self.config.with_retry(
            (
                &self.epoch_data_tree,
                &self.account_update_tree,
                &self.account_inbox_tree,
                &self.creation_epoch_tree,
            ),
            |(epoch_t, update_t, inbox_t, creation_t): Trees| {
                let epoch = writes.epoch;

                if !writes.created_accounts.is_empty() {
                    let mut common = epoch_t.get(&epoch)?.unwrap_or_default();
                    for acct in &writes.created_accounts {
                        creation_t.insert(acct, &epoch)?;
                        common.push_created_account(*acct);
                    }
                    epoch_t.insert(&epoch, &common)?;
                }

                for (acct, records) in &writes.account_updates {
                    if records.is_empty() {
                        continue;
                    }
                    let key = AccountEpochKey::new(epoch, *acct);
                    let mut entry = update_t.get(&key)?.unwrap_or_default();
                    if entry.records().iter().any(|r| {
                        r.update_meta()
                            .is_some_and(|m| *m.block_commitment() == writes.block)
                    }) {
                        return Err(ConflictableTransactionError::Abort(TSledError::abort(
                            DbError::DuplicateBlockIndexing {
                                epoch,
                                block: writes.block,
                            },
                        )));
                    }
                    entry.extend(records.iter().cloned());
                    update_t.insert(&key, &entry)?;
                }

                for (acct, records) in &writes.account_inbox_writes {
                    if records.is_empty() {
                        continue;
                    }
                    let key = AccountEpochKey::new(epoch, *acct);
                    let mut entry = inbox_t.get(&key)?.unwrap_or_default();
                    entry.extend(records.iter().cloned());
                    inbox_t.insert(&key, &entry)?;
                }

                Ok(())
            },
        )
    }

    fn set_epoch_commitment(&self, epoch: Epoch, commitment: EpochCommitment) -> DbResult<()> {
        self.config.with_retry(
            (&self.epoch_data_tree,),
            |(epoch_t,): (SledTransactionalTree<OLEpochIndexingDataSchema>,)| {
                let Some(mut common) = epoch_t.get(&epoch)? else {
                    return Err(ConflictableTransactionError::Abort(TSledError::abort(
                        DbError::Other(format!("no epoch indexing data for epoch {epoch}")),
                    )));
                };
                common.set_epoch_commitment(commitment);
                epoch_t.insert(&epoch, &common)?;
                Ok(())
            },
        )
    }

    fn get_epoch_indexing_data(&self, epoch: Epoch) -> DbResult<Option<EpochIndexingData>> {
        Ok(self.epoch_data_tree.get(&epoch)?)
    }

    fn get_account_update_entry(
        &self,
        key: AccountEpochKey,
    ) -> DbResult<Option<AccountUpdateEntry>> {
        Ok(self.account_update_tree.get(&key)?)
    }

    fn get_account_inbox_entry(&self, key: AccountEpochKey) -> DbResult<Option<AccountInboxEntry>> {
        Ok(self.account_inbox_tree.get(&key)?)
    }

    fn get_account_creation_epoch(&self, acct: AccountId) -> DbResult<Option<Epoch>> {
        Ok(self.creation_epoch_tree.get(&acct)?)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::ol_state_indexing_db_tests;

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(OLStateIndexingDBSled, ol_state_indexing_db_tests);
}
