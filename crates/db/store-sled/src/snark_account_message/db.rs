use strata_acct_types::AccountId;
use strata_db_types::{DbResult, traits::SnarkAccountMessageDatabase};
use strata_snark_acct_types::{MessageEntry, MessageEntryProof};

use super::schemas::{MessageEntryKey, MessageEntryProofSchema, MessageEntrySchema};
use crate::define_sled_database;

define_sled_database!(
    pub struct SnarkAccountMessageDBSled {
        message_entry_tree: MessageEntrySchema,
        message_proof_tree: MessageEntryProofSchema,
    }
);

impl SnarkAccountMessageDatabase for SnarkAccountMessageDBSled {
    fn get_message_entry(
        &self,
        account_id: AccountId,
        index: u64,
    ) -> DbResult<Option<MessageEntry>> {
        let key = MessageEntryKey { account_id, index };
        Ok(self.message_entry_tree.get(&key)?)
    }

    fn get_message_proof(
        &self,
        account_id: AccountId,
        index: u64,
    ) -> DbResult<Option<MessageEntryProof>> {
        let key = MessageEntryKey { account_id, index };
        Ok(self.message_proof_tree.get(&key)?)
    }

    fn put_message_entry(
        &self,
        account_id: AccountId,
        index: u64,
        entry: MessageEntry,
    ) -> DbResult<()> {
        let key = MessageEntryKey { account_id, index };
        self.message_entry_tree.insert(&key, &entry)?;
        Ok(())
    }

    fn put_message_proof(
        &self,
        account_id: AccountId,
        index: u64,
        proof: MessageEntryProof,
    ) -> DbResult<()> {
        let key = MessageEntryKey { account_id, index };
        self.message_proof_tree.insert(&key, &proof)?;
        Ok(())
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::snark_account_message_db_tests;

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(SnarkAccountMessageDBSled, snark_account_message_db_tests);
}
