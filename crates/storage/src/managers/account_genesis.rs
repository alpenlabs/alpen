use std::sync::Arc;

use ops::account_genesis::{AccountGenesisOps, Context};
use strata_db_types::{traits::AccountGenesisDatabase, DbResult};
use strata_identifiers::{AccountId, Epoch};
use threadpool::ThreadPool;

use crate::ops;

/// Database manager for per-account creation epoch tracking.
#[expect(
    missing_debug_implementations,
    reason = "Inner types don't have Debug implementation"
)]
pub struct AccountGenesisManager {
    ops: AccountGenesisOps,
}

impl AccountGenesisManager {
    /// Creates a new [`AccountGenesisManager`].
    pub fn new(pool: ThreadPool, db: Arc<impl AccountGenesisDatabase + 'static>) -> Self {
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
}
