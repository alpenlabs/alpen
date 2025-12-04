use std::sync::Arc;

use strata_acct_types::AccountId;
use strata_db_types::{traits::SnarkAccountMessageDatabase, DbResult};
use strata_snark_acct_types::{MessageEntry, MessageEntryProof};

/// Manager for snark account message database operations
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct SnarkAccountMessageManager {
    db: Arc<dyn SnarkAccountMessageDatabase>,
}

impl SnarkAccountMessageManager {
    /// Create new instance of [`SnarkAccountMessageManager`]
    pub fn new(db: Arc<impl SnarkAccountMessageDatabase + 'static>) -> Self {
        Self { db }
    }

    /// Retrieve a message entry by account ID and index
    pub fn get_message_entry(
        &self,
        account_id: AccountId,
        index: u64,
    ) -> DbResult<Option<MessageEntry>> {
        self.db.get_message_entry(account_id, index)
    }

    /// Retrieve a message entry proof by account ID and index
    pub fn get_message_proof(
        &self,
        account_id: AccountId,
        index: u64,
    ) -> DbResult<Option<MessageEntryProof>> {
        self.db.get_message_proof(account_id, index)
    }

    /// Store a message entry
    pub fn put_message_entry(
        &self,
        account_id: AccountId,
        index: u64,
        entry: MessageEntry,
    ) -> DbResult<()> {
        self.db.put_message_entry(account_id, index, entry)
    }

    /// Store a message entry proof
    pub fn put_message_proof(
        &self,
        account_id: AccountId,
        index: u64,
        proof: MessageEntryProof,
    ) -> DbResult<()> {
        self.db.put_message_proof(account_id, index, proof)
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload, RawMerkleProof};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::traits::DatabaseBackend;
    use strata_snark_acct_types::{MessageEntry, MessageEntryProof};

    use super::*;

    fn create_test_message_entry(account_id: AccountId, epoch: u32, value: u64) -> MessageEntry {
        let payload = MsgPayload::new(BitcoinAmount::from_sat(value), vec![1, 2, 3]);
        MessageEntry::new(account_id, epoch, payload)
    }

    fn create_test_message_entry_proof(entry: MessageEntry) -> MessageEntryProof {
        let raw_proof = RawMerkleProof::new(vec![[1u8; 32], [2u8; 32]]);
        MessageEntryProof::new(entry, raw_proof)
    }

    #[test]
    fn test_snark_account_message_manager_basic_operations() {
        let db = get_test_sled_backend();
        let manager = SnarkAccountMessageManager::new(db.snark_account_message_db());

        let account_id = AccountId::new([1u8; 32]);
        let index = 0u64;
        let entry = create_test_message_entry(account_id, 1, 100);

        // Test put and get message entry
        manager
            .put_message_entry(account_id, index, entry.clone())
            .unwrap();
        let result = manager.get_message_entry(account_id, index).unwrap();
        assert_eq!(result, Some(entry));

        // Test put and get message proof
        let entry2 = create_test_message_entry(account_id, 2, 200);
        let proof = create_test_message_entry_proof(entry2);
        manager
            .put_message_proof(account_id, index, proof.clone())
            .unwrap();
        let result = manager.get_message_proof(account_id, index).unwrap();
        assert_eq!(result, Some(proof));
    }
}
