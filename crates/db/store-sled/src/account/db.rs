//! Sled-backed account genesis database implementation.

use strata_db_types::{DbError, DbResult, traits::AccountDatabase};
use strata_identifiers::{AccountId, Epoch};
use strata_primitives::OLBlockId;

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
        key: (AccountId, OLBlockId),
        extra_data: Vec<u8>,
    ) -> DbResult<()> {
        self.extra_data_tree.insert(&key, &extra_data)?;
        Ok(())
    }

    fn get_account_extra_data(&self, key: (AccountId, OLBlockId)) -> DbResult<Option<Vec<u8>>> {
        Ok(self.extra_data_tree.get(&key)?)
    }
}
