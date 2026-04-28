//! Sled-backed [`OLStateIndexingDatabase`] implementation.

use sled::transaction::ConflictableTransactionError;
use strata_db_types::{
    DbError, DbResult,
    ol_state_index::{
        AccountEpochKey, AccountUpdateRecord, EpochIndexingData, InboxMessageRecord, IndexingWrites,
    },
    traits::OLStateIndexingDatabase,
};
use strata_identifiers::{AccountId, Epoch, EpochCommitment, OLBlockCommitment};
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
    fn apply_epoch_indexing(
        &self,
        commitment: EpochCommitment,
        writes: IndexingWrites,
    ) -> DbResult<()> {
        self.config.with_retry(
            (
                &self.epoch_data_tree,
                &self.account_update_tree,
                &self.account_inbox_tree,
                &self.creation_epoch_tree,
            ),
            |(epoch_t, update_t, inbox_t, creation_t): Trees| {
                let epoch = commitment.epoch();
                let common =
                    EpochIndexingData::new(Some(commitment), writes.created_accounts().to_vec());

                for acct in common.created_accounts() {
                    creation_t.insert(acct, &epoch)?;
                }
                epoch_t.insert(&epoch, &common)?;

                for (acct, records) in writes.account_updates() {
                    update_t.insert(&AccountEpochKey::new(epoch, *acct), records)?;
                }
                for (acct, records) in writes.account_inbox() {
                    inbox_t.insert(&AccountEpochKey::new(epoch, *acct), records)?;
                }

                Ok(())
            },
        )
    }

    fn apply_block_indexing(
        &self,
        epoch: Epoch,
        block: OLBlockCommitment,
        writes: IndexingWrites,
    ) -> DbResult<()> {
        self.config.with_retry(
            (
                &self.epoch_data_tree,
                &self.account_update_tree,
                &self.account_inbox_tree,
                &self.creation_epoch_tree,
            ),
            |(epoch_t, update_t, inbox_t, creation_t): Trees| {
                // Always materialize the epoch's common row, even when this
                // block created no accounts. Without this, epochs whose blocks
                // never created accounts would have no row when
                // `set_epoch_commitment` fires at finalization.
                let mut common = epoch_t.get(&epoch)?.unwrap_or_default();
                for acct in writes.created_accounts() {
                    creation_t.insert(acct, &epoch)?;
                    common.push_created_account(*acct);
                }
                epoch_t.insert(&epoch, &common)?;

                for (acct, records) in writes.account_updates() {
                    if records.is_empty() {
                        continue;
                    }
                    let key = AccountEpochKey::new(epoch, *acct);
                    let mut existing = update_t.get(&key)?.unwrap_or_default();
                    if existing.iter().any(|r| {
                        r.update_meta()
                            .is_some_and(|m| *m.block_commitment() == block)
                    }) {
                        return Err(ConflictableTransactionError::Abort(TSledError::abort(
                            DbError::DuplicateBlockIndexing { epoch, block },
                        )));
                    }
                    existing.extend(records.iter().cloned());
                    update_t.insert(&key, &existing)?;
                }

                for (acct, records) in writes.account_inbox() {
                    if records.is_empty() {
                        continue;
                    }
                    let key = AccountEpochKey::new(epoch, *acct);
                    let mut existing = inbox_t.get(&key)?.unwrap_or_default();
                    existing.extend(records.iter().cloned());
                    inbox_t.insert(&key, &existing)?;
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

    fn get_account_update_records(
        &self,
        epoch: Epoch,
        account: AccountId,
    ) -> DbResult<Option<Vec<AccountUpdateRecord>>> {
        Ok(self
            .account_update_tree
            .get(&AccountEpochKey::new(epoch, account))?)
    }

    fn get_account_inbox_records(
        &self,
        epoch: Epoch,
        account: AccountId,
    ) -> DbResult<Option<Vec<InboxMessageRecord>>> {
        Ok(self
            .account_inbox_tree
            .get(&AccountEpochKey::new(epoch, account))?)
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
