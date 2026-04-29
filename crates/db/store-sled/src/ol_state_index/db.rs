//! Sled-backed [`OLStateIndexingDatabase`] implementation.

use std::collections::BTreeSet;

use sled::transaction::ConflictableTransactionError;
use strata_db_types::{
    DbError, DbResult,
    ol_state_index::{
        AccountEpochKey, AccountUpdateRecord, EpochIndexingData, InboxMessageRecord, IndexingWrites,
    },
    traits::OLStateIndexingDatabase,
};
use strata_identifiers::{AccountId, Epoch, EpochCommitment, OLBlockCommitment};
use typed_sled::{SledTree, error::Error as TSledError, tree::SledTransactionalTree};

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

/// Returns the set of account ids that have either an update or an inbox row
/// keyed by `epoch`. Done as a non-transactional scan since
/// [`SledTransactionalTree`] doesn't expose ranged iteration; the actual
/// rollback writes happen inside a transaction afterwards.
fn collect_accounts_in_epoch(
    epoch: Epoch,
    update_tree: &SledTree<OLAccountUpdateEntrySchema>,
    inbox_tree: &SledTree<OLAccountInboxEntrySchema>,
) -> DbResult<BTreeSet<AccountId>> {
    let lo = AccountEpochKey::new(epoch, AccountId::new([0u8; 32]));
    let hi = AccountEpochKey::new(epoch, AccountId::new([0xffu8; 32]));
    let mut out = BTreeSet::new();
    for item in update_tree.range(lo..=hi)? {
        let (key, _) = item?;
        out.insert(key.account_id());
    }
    for item in inbox_tree.range(lo..=hi)? {
        let (key, _) = item?;
        out.insert(key.account_id());
    }
    Ok(out)
}

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
                // Checkpoint-sync has no per-block attribution: writes the
                // whole epoch atomically, so block-rollback can't undo
                // individual blocks. Mark with `None` so per-block rollback
                // is a no-op against these entries.
                let created: Vec<(AccountId, Option<OLBlockCommitment>)> = writes
                    .created_accounts()
                    .iter()
                    .map(|acct| (*acct, None))
                    .collect();
                // Checkpoint-sync has no per-block high-water mark.
                let common = EpochIndexingData::new(Some(commitment), created, None);

                for (acct, _) in common.created_accounts() {
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
                // Read the common row first and gate on the high-water mark
                // BEFORE touching any other tree. This catches duplicate /
                // out-of-order applies regardless of which write families
                // are populated this call.
                let mut common = epoch_t.get(&epoch)?.unwrap_or_default();
                if common
                    .last_applied_block()
                    .is_some_and(|prev| block.slot() <= prev.slot())
                {
                    return Err(ConflictableTransactionError::Abort(TSledError::abort(
                        DbError::DuplicateBlockIndexing { epoch, block },
                    )));
                }
                common.set_last_applied_block(block);

                for acct in writes.created_accounts() {
                    creation_t.insert(acct, &epoch)?;
                    common.push_created_account(*acct, Some(block));
                }
                epoch_t.insert(&epoch, &common)?;

                for (acct, records) in writes.account_updates() {
                    if records.is_empty() {
                        continue;
                    }
                    let key = AccountEpochKey::new(epoch, *acct);
                    let mut existing = update_t.get(&key)?.unwrap_or_default();
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

    fn rollback_to_block(&self, epoch: Epoch, block: OLBlockCommitment) -> DbResult<()> {
        let cutoff_slot = block.slot();
        // Pre-scan affected accounts via the non-transactional trees.
        // SledTransactionalTree has no range API, so we collect keys here and
        // act on them inside the transaction below.
        let affected =
            collect_accounts_in_epoch(epoch, &self.account_update_tree, &self.account_inbox_tree)?;

        self.config.with_retry(
            (
                &self.epoch_data_tree,
                &self.account_update_tree,
                &self.account_inbox_tree,
                &self.creation_epoch_tree,
            ),
            |(epoch_t, update_t, inbox_t, creation_t): Trees| {
                // Filter records past the cutoff out of update + inbox rows;
                // delete the row when nothing is left.
                for acct in &affected {
                    let key = AccountEpochKey::new(epoch, *acct);
                    if let Some(records) = update_t.get(&key)? {
                        let kept: Vec<AccountUpdateRecord> = records
                            .into_iter()
                            .filter(|r| {
                                r.update_meta()
                                    .is_none_or(|m| m.block_commitment().slot() <= cutoff_slot)
                            })
                            .collect();
                        if kept.is_empty() {
                            update_t.remove(&key)?;
                        } else {
                            update_t.insert(&key, &kept)?;
                        }
                    }
                    if let Some(records) = inbox_t.get(&key)? {
                        let kept: Vec<InboxMessageRecord> = records
                            .into_iter()
                            .filter(|r| {
                                r.block_commitment().is_none_or(|c| c.slot() <= cutoff_slot)
                            })
                            .collect();
                        if kept.is_empty() {
                            inbox_t.remove(&key)?;
                        } else {
                            inbox_t.insert(&key, &kept)?;
                        }
                    }
                }

                // Drop creators past the cutoff from the common row, reset
                // the high-water mark if it was past the cutoff, and remove
                // creation_epoch entries for any dropped creators.
                if let Some(mut common) = epoch_t.get(&epoch)? {
                    let dropped = common.drop_created_after_slot(cutoff_slot);
                    let prev_high_water = common.last_applied_block().copied();
                    common.clear_last_applied_block_after_slot(cutoff_slot);
                    let high_water_changed =
                        prev_high_water != common.last_applied_block().copied();
                    if !dropped.is_empty() || high_water_changed {
                        epoch_t.insert(&epoch, &common)?;
                        for acct in dropped {
                            creation_t.remove(&acct)?;
                        }
                    }
                }

                Ok(())
            },
        )
    }

    fn rollback_to_epoch(&self, epoch: Epoch) -> DbResult<()> {
        // Enumerate epochs strictly greater than `epoch` to drop.
        let mut epochs_to_drop: Vec<Epoch> = Vec::new();
        for item in self.epoch_data_tree.range((epoch + 1)..)? {
            let (e, _) = item?;
            epochs_to_drop.push(e);
        }
        // Per-epoch, collect affected accounts via update/inbox trees, since
        // an epoch can have account rows even if the common row is missing
        // (defensive — should not happen in practice).
        let mut per_epoch: Vec<(Epoch, BTreeSet<AccountId>)> =
            Vec::with_capacity(epochs_to_drop.len());
        for e in &epochs_to_drop {
            let affected =
                collect_accounts_in_epoch(*e, &self.account_update_tree, &self.account_inbox_tree)?;
            per_epoch.push((*e, affected));
        }

        self.config.with_retry(
            (
                &self.epoch_data_tree,
                &self.account_update_tree,
                &self.account_inbox_tree,
                &self.creation_epoch_tree,
            ),
            |(epoch_t, update_t, inbox_t, creation_t): Trees| {
                for (e, affected) in &per_epoch {
                    for acct in affected {
                        let key = AccountEpochKey::new(*e, *acct);
                        update_t.remove(&key)?;
                        inbox_t.remove(&key)?;
                    }
                    if let Some(common) = epoch_t.get(e)? {
                        for (acct, _) in common.created_accounts() {
                            creation_t.remove(acct)?;
                        }
                        epoch_t.remove(e)?;
                    }
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
