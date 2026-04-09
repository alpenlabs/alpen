//! Sled-backed account genesis database implementation.

use strata_db_types::{DbResult, traits::AccountDatabase, types::AccountExtraData};
use strata_identifiers::{AccountId, Epoch};
use strata_primitives::nonempty_vec::NonEmptyVec;

use super::schemas::{AccountExtraDataSchema, AccountGenesisSchema};
use crate::define_sled_database;

define_sled_database!(
    pub struct AccountDBSled {
        genesis_tree: AccountGenesisSchema,
        extra_data_tree: AccountExtraDataSchema,
    }
);

impl AccountDatabase for AccountDBSled {
    fn insert_account_creation_epoch(&self, account_id: AccountId, epoch: Epoch) -> DbResult<()> {
        self.genesis_tree.insert(&account_id, &epoch)?;
        Ok(())
    }

    fn get_account_creation_epoch(&self, account_id: AccountId) -> DbResult<Option<Epoch>> {
        Ok(self.genesis_tree.get(&account_id)?)
    }

    fn put_account_extra_data(
        &self,
        key: (AccountId, Epoch),
        extra_data: NonEmptyVec<AccountExtraData>,
    ) -> DbResult<()> {
        // Replace the existing entry
        let curr = self.extra_data_tree.get(&key)?;
        self.extra_data_tree
            .compare_and_swap(key, curr, Some(extra_data))?;
        Ok(())
    }

    fn append_account_extra_data(
        &self,
        key: (AccountId, Epoch),
        extra_data: AccountExtraData,
    ) -> DbResult<()> {
        let curr = self.extra_data_tree.get(&key)?;
        let new = match curr.clone() {
            Some(mut curr) => {
                curr.push(extra_data);
                curr
            }
            None => NonEmptyVec::new(extra_data),
        };
        self.extra_data_tree
            .compare_and_swap(key, curr, Some(new))?;
        Ok(())
    }

    fn get_account_extra_data(
        &self,
        key: (AccountId, Epoch),
    ) -> DbResult<Option<NonEmptyVec<AccountExtraData>>> {
        Ok(self.extra_data_tree.get(&key)?)
    }
}
