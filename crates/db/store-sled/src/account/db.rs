//! Sled-backed account genesis database implementation.

use strata_db_types::{DbResult, traits::AccountDatabase, types::AccountExtraDataEntry};
use strata_identifiers::{AccountId, Epoch};

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
        self.genesis_tree.insert(&account_id, &epoch)?;
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
        // Replace the existing entry
        let curr = self.extra_data_tree.get(&key)?;
        self.extra_data_tree
            .compare_and_swap(key, curr, Some(extra_data))?;
        Ok(())
    }

    fn get_account_extra_data(
        &self,
        key: (AccountId, Epoch),
    ) -> DbResult<Option<AccountExtraDataEntry>> {
        Ok(self.extra_data_tree.get(&key)?)
    }
}
