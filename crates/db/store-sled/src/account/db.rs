//! Sled-backed account genesis database implementation.

use strata_db_types::{DbError, DbResult, traits::AccountDatabase, types::AccountExtraDataEntry};
use strata_identifiers::{AccountId, Epoch};
use strata_primitives::nonempty_vec::NonEmptyVec;

use super::schemas::{AccountExtraDataSchema, AccountGenesisSchema};
use crate::define_sled_database;

define_sled_database!(
    pub struct AccountGenesisDBSled {
        genesis_tree: AccountGenesisSchema,
        extra_data_tree: AccountExtraDataSchema,
    }
);

impl AccountDatabase for AccountGenesisDBSled {
    fn insert_account_creation_epoch(&self, account_id: AccountId, epoch: Epoch) -> DbResult<()> {
        if self.genesis_tree.get(&account_id)?.is_some() {
            return Err(DbError::EntryAlreadyExists);
        }
        self.genesis_tree
            .compare_and_swap(account_id, None, Some(epoch))?;
        Ok(())
    }

    fn get_account_creation_epoch(&self, account_id: AccountId) -> DbResult<Option<Epoch>> {
        Ok(self.genesis_tree.get(&account_id)?)
    }

    fn insert_account_extra_data(
        &self,
        key: (AccountId, Epoch),
        extra_data: AccountExtraDataEntry,
    ) -> DbResult<()> {
        // Append to existing list of entries
        let curr = self.extra_data_tree.get(&key)?;
        let new = if let Some(ref d) = curr {
            let mut new = d.clone();
            new.push(extra_data);
            new
        } else {
            NonEmptyVec::new(extra_data)
        };
        self.extra_data_tree
            .compare_and_swap(key, curr, Some(new))?;
        Ok(())
    }

    fn get_account_extra_data(
        &self,
        key: (AccountId, Epoch),
    ) -> DbResult<Option<NonEmptyVec<AccountExtraDataEntry>>> {
        Ok(self.extra_data_tree.get(&key)?)
    }
}
