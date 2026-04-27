use std::sync::Arc;

use strata_db_types::{
    ol_state_index::{
        AccountEpochKey, AccountInboxEntry, AccountUpdateEntry, AccountUpdateRecord,
        BlockIndexingWrites, EpochIndexingData, EpochIndexingWrites, InboxMessageRecord,
    },
    traits::OLStateIndexingDatabase,
    DbResult,
};
use strata_identifiers::{AccountId, Epoch, EpochCommitment};
use threadpool::ThreadPool;

use crate::ops::ol_state_indexing::{Context, OLStateIndexingOps};

// NOTE: cache layer (block-keyed and/or epoch-keyed) is intentionally deferred.
// Add it here when real callers reveal the hot read patterns; the manager is the
// right home for it (DB stays a dumb KV).

/// Database manager for OL state indexing data.
#[expect(
    missing_debug_implementations,
    reason = "Inner types don't have Debug implementation"
)]
pub struct OLStateIndexingManager {
    ops: OLStateIndexingOps,
}

impl OLStateIndexingManager {
    /// Creates a new [`OLStateIndexingManager`].
    pub fn new(pool: ThreadPool, db: Arc<impl OLStateIndexingDatabase + 'static>) -> Self {
        let ops = Context::new(db).into_ops(pool);
        Self { ops }
    }

    pub fn apply_epoch_indexing_blocking(&self, writes: EpochIndexingWrites) -> DbResult<()> {
        self.ops.apply_epoch_indexing_blocking(writes)
    }

    pub async fn apply_epoch_indexing_async(&self, writes: EpochIndexingWrites) -> DbResult<()> {
        self.ops.apply_epoch_indexing_async(writes).await
    }

    pub fn apply_block_indexing_blocking(&self, writes: BlockIndexingWrites) -> DbResult<()> {
        self.ops.apply_block_indexing_blocking(writes)
    }

    pub async fn apply_block_indexing_async(&self, writes: BlockIndexingWrites) -> DbResult<()> {
        self.ops.apply_block_indexing_async(writes).await
    }

    pub fn set_epoch_commitment_blocking(
        &self,
        epoch: Epoch,
        commitment: EpochCommitment,
    ) -> DbResult<()> {
        self.ops.set_epoch_commitment_blocking(epoch, commitment)
    }

    pub async fn set_epoch_commitment_async(
        &self,
        epoch: Epoch,
        commitment: EpochCommitment,
    ) -> DbResult<()> {
        self.ops.set_epoch_commitment_async(epoch, commitment).await
    }

    pub fn get_epoch_indexing_data_blocking(
        &self,
        epoch: Epoch,
    ) -> DbResult<Option<EpochIndexingData>> {
        self.ops.get_epoch_indexing_data_blocking(epoch)
    }

    pub async fn get_epoch_indexing_data_async(
        &self,
        epoch: Epoch,
    ) -> DbResult<Option<EpochIndexingData>> {
        self.ops.get_epoch_indexing_data_async(epoch).await
    }

    pub fn get_account_update_entry_blocking(
        &self,
        key: AccountEpochKey,
    ) -> DbResult<Option<AccountUpdateEntry>> {
        self.ops.get_account_update_entry_blocking(key)
    }

    pub async fn get_account_update_entry_async(
        &self,
        key: AccountEpochKey,
    ) -> DbResult<Option<AccountUpdateEntry>> {
        self.ops.get_account_update_entry_async(key).await
    }

    pub fn get_account_inbox_entry_blocking(
        &self,
        key: AccountEpochKey,
    ) -> DbResult<Option<AccountInboxEntry>> {
        self.ops.get_account_inbox_entry_blocking(key)
    }

    pub async fn get_account_inbox_entry_async(
        &self,
        key: AccountEpochKey,
    ) -> DbResult<Option<AccountInboxEntry>> {
        self.ops.get_account_inbox_entry_async(key).await
    }

    pub fn get_account_creation_epoch_blocking(
        &self,
        account_id: AccountId,
    ) -> DbResult<Option<Epoch>> {
        self.ops.get_account_creation_epoch_blocking(account_id)
    }

    pub async fn get_account_creation_epoch_async(
        &self,
        account_id: AccountId,
    ) -> DbResult<Option<Epoch>> {
        self.ops.get_account_creation_epoch_async(account_id).await
    }

    /// Returns update records for `(epoch, acct)` whose block falls in the
    /// inclusive slot range. Records without `update_meta` (checkpoint-sync
    /// rows) are skipped, since they carry no block commitment to filter on.
    pub async fn get_account_update_records_in_slot_range_async(
        &self,
        key: AccountEpochKey,
        start_slot: u64,
        end_slot: u64,
    ) -> DbResult<Vec<AccountUpdateRecord>> {
        let Some(entry) = self.ops.get_account_update_entry_async(key).await? else {
            return Ok(Vec::new());
        };
        Ok(filter_records_by_slot(
            entry.records(),
            start_slot,
            end_slot,
        ))
    }

    /// Returns inbox writes for `(epoch, acct)` whose block falls in the
    /// inclusive slot range. Records without `block_commitment` are skipped.
    pub async fn get_account_inbox_records_in_slot_range_async(
        &self,
        key: AccountEpochKey,
        start_slot: u64,
        end_slot: u64,
    ) -> DbResult<Vec<InboxMessageRecord>> {
        let Some(entry) = self.ops.get_account_inbox_entry_async(key).await? else {
            return Ok(Vec::new());
        };
        Ok(filter_inbox_by_slot(entry.records(), start_slot, end_slot))
    }
}

fn filter_records_by_slot(
    records: &[AccountUpdateRecord],
    start_slot: u64,
    end_slot: u64,
) -> Vec<AccountUpdateRecord> {
    records
        .iter()
        .filter(|r| {
            r.update_meta()
                .map(|m| {
                    let s = m.block_commitment().slot();
                    s >= start_slot && s <= end_slot
                })
                .unwrap_or(false)
        })
        .cloned()
        .collect()
}

fn filter_inbox_by_slot(
    records: &[InboxMessageRecord],
    start_slot: u64,
    end_slot: u64,
) -> Vec<InboxMessageRecord> {
    records
        .iter()
        .filter(|r| {
            r.block_commitment()
                .map(|c| {
                    let s = c.slot();
                    s >= start_slot && s <= end_slot
                })
                .unwrap_or(false)
        })
        .cloned()
        .collect()
}
