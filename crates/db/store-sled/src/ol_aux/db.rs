//! OL auxiliary data database implementation.

use strata_db_types::{DbResult, traits::InboxMessageDatabase};
use strata_identifiers::AccountId;
use strata_snark_acct_types::MessageEntry;

use super::schemas::{InboxMessageKey, InboxMessageSchema};
use crate::define_sled_database;

define_sled_database!(
    /// Sled database for OL auxiliary data (inbox messages).
    pub struct OLAuxDBSled {
        inbox_tree: InboxMessageSchema,
    }
);

impl InboxMessageDatabase for OLAuxDBSled {
    fn put_inbox_message(
        &self,
        account_id: AccountId,
        index: u64,
        entry: MessageEntry,
    ) -> DbResult<()> {
        let key = InboxMessageKey::new(account_id, index);
        self.inbox_tree.insert(&key, &entry)?;
        Ok(())
    }

    fn get_inbox_message(
        &self,
        account_id: AccountId,
        index: u64,
    ) -> DbResult<Option<MessageEntry>> {
        let key = InboxMessageKey::new(account_id, index);
        Ok(self.inbox_tree.get(&key)?)
    }

    fn del_inbox_message(&self, account_id: AccountId, index: u64) -> DbResult<()> {
        let key = InboxMessageKey::new(account_id, index);
        self.inbox_tree.remove(&key)?;
        Ok(())
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_acct_types::{BitcoinAmount, MsgPayload};

    use super::*;
    use crate::test_utils::get_test_sled_backend;

    fn test_account_id(n: u8) -> AccountId {
        AccountId::from([n; 32])
    }

    fn test_message_entry(source_n: u8, epoch: u32) -> MessageEntry {
        let source = test_account_id(source_n);
        let payload = MsgPayload::new(BitcoinAmount::from_sat(100), vec![1, 2, 3]);
        MessageEntry::new(source, epoch, payload)
    }

    #[test]
    fn test_put_and_get_inbox_message() {
        let backend = get_test_sled_backend();
        let db = backend.inbox_message_db();

        let account_id = test_account_id(1);
        let entry = test_message_entry(2, 5);

        db.put_inbox_message(account_id, 0, entry.clone()).unwrap();

        let retrieved = db.get_inbox_message(account_id, 0).unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.incl_epoch(), entry.incl_epoch());
    }

    #[test]
    fn test_get_nonexistent_message() {
        let backend = get_test_sled_backend();
        let db = backend.inbox_message_db();

        let account_id = test_account_id(1);
        let result = db.get_inbox_message(account_id, 999).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_multiple_messages_per_account() {
        let backend = get_test_sled_backend();
        let db = backend.inbox_message_db();

        let account_id = test_account_id(1);
        let entry1 = test_message_entry(2, 1);
        let entry2 = test_message_entry(3, 2);
        let entry3 = test_message_entry(4, 3);

        db.put_inbox_message(account_id, 0, entry1.clone()).unwrap();
        db.put_inbox_message(account_id, 1, entry2.clone()).unwrap();
        db.put_inbox_message(account_id, 2, entry3.clone()).unwrap();

        assert_eq!(
            db.get_inbox_message(account_id, 0)
                .unwrap()
                .unwrap()
                .incl_epoch(),
            1
        );
        assert_eq!(
            db.get_inbox_message(account_id, 1)
                .unwrap()
                .unwrap()
                .incl_epoch(),
            2
        );
        assert_eq!(
            db.get_inbox_message(account_id, 2)
                .unwrap()
                .unwrap()
                .incl_epoch(),
            3
        );
    }

    #[test]
    fn test_delete_inbox_message() {
        let backend = get_test_sled_backend();
        let db = backend.inbox_message_db();

        let account_id = test_account_id(1);
        let entry = test_message_entry(2, 5);

        db.put_inbox_message(account_id, 0, entry).unwrap();
        assert!(db.get_inbox_message(account_id, 0).unwrap().is_some());

        db.del_inbox_message(account_id, 0).unwrap();
        assert!(db.get_inbox_message(account_id, 0).unwrap().is_none());
    }

    #[test]
    fn test_messages_different_accounts() {
        let backend = get_test_sled_backend();
        let db = backend.inbox_message_db();

        let account1 = test_account_id(1);
        let account2 = test_account_id(2);
        let entry1 = test_message_entry(10, 100);
        let entry2 = test_message_entry(20, 200);

        db.put_inbox_message(account1, 0, entry1.clone()).unwrap();
        db.put_inbox_message(account2, 0, entry2.clone()).unwrap();

        let retrieved1 = db.get_inbox_message(account1, 0).unwrap().unwrap();
        let retrieved2 = db.get_inbox_message(account2, 0).unwrap().unwrap();

        assert_eq!(retrieved1.incl_epoch(), 100);
        assert_eq!(retrieved2.incl_epoch(), 200);
    }
}
