use std::sync::Arc;

use strata_db_types::{
    ol_state_index::{AccountUpdateRecord, EpochIndexingData, InboxMessageRecord, IndexingWrites},
    traits::OLStateIndexingDatabase,
    DbResult,
};
use strata_identifiers::{AccountId, Epoch, EpochCommitment, OLBlockCommitment};
use threadpool::ThreadPool;

use crate::ops::ol_state_indexing::{Context, OLStateIndexingOps};

// NOTE: A cache layer (block-keyed and/or epoch-keyed) can be added later as required.

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

    pub fn apply_epoch_indexing_blocking(
        &self,
        commitment: EpochCommitment,
        writes: IndexingWrites,
    ) -> DbResult<()> {
        self.ops.apply_epoch_indexing_blocking(commitment, writes)
    }

    pub async fn apply_epoch_indexing_async(
        &self,
        commitment: EpochCommitment,
        writes: IndexingWrites,
    ) -> DbResult<()> {
        self.ops
            .apply_epoch_indexing_async(commitment, writes)
            .await
    }

    pub fn apply_block_indexing_blocking(
        &self,
        epoch: Epoch,
        block: OLBlockCommitment,
        writes: IndexingWrites,
    ) -> DbResult<()> {
        self.ops.apply_block_indexing_blocking(epoch, block, writes)
    }

    pub async fn apply_block_indexing_async(
        &self,
        epoch: Epoch,
        block: OLBlockCommitment,
        writes: IndexingWrites,
    ) -> DbResult<()> {
        self.ops
            .apply_block_indexing_async(epoch, block, writes)
            .await
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

    pub fn get_account_update_records_blocking(
        &self,
        epoch: Epoch,
        account: AccountId,
    ) -> DbResult<Option<Vec<AccountUpdateRecord>>> {
        self.ops.get_account_update_records_blocking(epoch, account)
    }

    pub async fn get_account_update_records_async(
        &self,
        epoch: Epoch,
        account: AccountId,
    ) -> DbResult<Option<Vec<AccountUpdateRecord>>> {
        self.ops
            .get_account_update_records_async(epoch, account)
            .await
    }

    pub fn get_account_inbox_records_blocking(
        &self,
        epoch: Epoch,
        account: AccountId,
    ) -> DbResult<Option<Vec<InboxMessageRecord>>> {
        self.ops.get_account_inbox_records_blocking(epoch, account)
    }

    pub async fn get_account_inbox_records_async(
        &self,
        epoch: Epoch,
        account: AccountId,
    ) -> DbResult<Option<Vec<InboxMessageRecord>>> {
        self.ops
            .get_account_inbox_records_async(epoch, account)
            .await
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
}
