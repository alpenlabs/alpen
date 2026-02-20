use std::sync::Arc;

use ops::account::{AccountOps, Context};
use strata_db_types::{traits::AccountDatabase, types::AccountExtraDataEntry, DbResult};
use strata_identifiers::{AccountId, Epoch};
use threadpool::ThreadPool;

use crate::ops;

/// Database manager for per-account creation epoch tracking.
#[expect(
    missing_debug_implementations,
    reason = "Inner types don't have Debug implementation"
)]
pub struct AccountManager {
    ops: AccountOps,
}

impl AccountManager {
    /// Creates a new [`AccountGenesisManager`].
    pub fn new(pool: ThreadPool, db: Arc<impl AccountDatabase + 'static>) -> Self {
        let ops = Context::new(db).into_ops(pool);
        Self { ops }
    }

    /// Inserts the creation epoch for an account.
    ///
    /// Fails if the account already has a recorded creation epoch.
    pub fn insert_account_creation_epoch_blocking(
        &self,
        account_id: AccountId,
        epoch: Epoch,
    ) -> DbResult<()> {
        self.ops
            .insert_account_creation_epoch_blocking(account_id, epoch)
    }

    /// Gets the creation epoch for an account, if recorded.
    pub fn get_account_creation_epoch_blocking(
        &self,
        account_id: AccountId,
    ) -> DbResult<Option<Epoch>> {
        self.ops.get_account_creation_epoch_blocking(account_id)
    }

    /// Inserts account extra data by blocking.
    pub fn insert_account_extra_data_blocking(
        &self,
        key: (AccountId, Epoch),
        extra_data: AccountExtraDataEntry,
    ) -> DbResult<()> {
        self.ops.insert_account_extra_data_blocking(key, extra_data)
    }

    /// Inserts account extra data async.
    pub async fn insert_account_extra_data_async(
        &self,
        key: (AccountId, Epoch),
        extra_data: AccountExtraDataEntry,
    ) -> DbResult<()> {
        self.ops
            .insert_account_extra_data_async(key, extra_data)
            .await
    }

    /// Gets account extra data by blocking.
    pub fn get_account_extra_data_blocking(
        &self,
        key: (AccountId, Epoch),
    ) -> DbResult<Option<Vec<AccountExtraDataEntry>>> {
        self.ops.get_account_extra_data_blocking(key)
    }

    /// Gets account extra data async.
    pub async fn get_account_extra_data_async(
        &self,
        key: (AccountId, Epoch),
    ) -> DbResult<Option<Vec<AccountExtraDataEntry>>> {
        self.ops.get_account_extra_data_async(key).await
    }
}
