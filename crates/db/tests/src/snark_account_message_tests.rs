use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload, RawMerkleProof};
use strata_db_types::traits::SnarkAccountMessageDatabase;
use strata_snark_acct_types::{MessageEntry, MessageEntryProof};

fn generate_test_message_entry(account_id: AccountId, epoch: u32, value: u64) -> MessageEntry {
    let payload = MsgPayload::new(BitcoinAmount::from_sat(value), vec![1, 2, 3]);
    MessageEntry::new(account_id, epoch, payload)
}

fn generate_test_message_entry_proof(entry: MessageEntry) -> MessageEntryProof {
    // Create a dummy raw merkle proof for testing
    let raw_proof = RawMerkleProof::new(vec![[1u8; 32], [2u8; 32]]);
    MessageEntryProof::new(entry, raw_proof)
}

pub fn test_put_and_get_message_entry(db: &impl SnarkAccountMessageDatabase) {
    let account_id = AccountId::new([1u8; 32]);
    let index = 0u64;
    let entry = generate_test_message_entry(account_id, 1, 100);

    db.put_message_entry(account_id, index, entry.clone())
        .unwrap();

    let result = db.get_message_entry(account_id, index).unwrap();
    assert_eq!(result, Some(entry));
}

pub fn test_get_nonexistent_message_entry(db: &impl SnarkAccountMessageDatabase) {
    let account_id = AccountId::new([2u8; 32]);
    let index = 0u64;

    let result = db.get_message_entry(account_id, index).unwrap();
    assert!(result.is_none());
}

pub fn test_update_existing_message_entry(db: &impl SnarkAccountMessageDatabase) {
    let account_id = AccountId::new([3u8; 32]);
    let index = 0u64;
    let entry1 = generate_test_message_entry(account_id, 1, 100);

    db.put_message_entry(account_id, index, entry1.clone())
        .unwrap();

    let entry2 = generate_test_message_entry(account_id, 2, 200);
    db.put_message_entry(account_id, index, entry2.clone())
        .unwrap();

    let result = db.get_message_entry(account_id, index).unwrap();
    assert_eq!(result, Some(entry2));
}

pub fn test_put_and_get_message_proof(db: &impl SnarkAccountMessageDatabase) {
    let account_id = AccountId::new([4u8; 32]);
    let index = 0u64;
    let entry = generate_test_message_entry(account_id, 1, 100);
    let proof = generate_test_message_entry_proof(entry);

    db.put_message_proof(account_id, index, proof.clone())
        .unwrap();

    let result = db.get_message_proof(account_id, index).unwrap();
    assert_eq!(result, Some(proof));
}

pub fn test_get_nonexistent_message_proof(db: &impl SnarkAccountMessageDatabase) {
    let account_id = AccountId::new([5u8; 32]);
    let index = 0u64;

    let result = db.get_message_proof(account_id, index).unwrap();
    assert!(result.is_none());
}

pub fn test_update_existing_message_proof(db: &impl SnarkAccountMessageDatabase) {
    let account_id = AccountId::new([6u8; 32]);
    let index = 0u64;
    let entry1 = generate_test_message_entry(account_id, 1, 100);
    let proof1 = generate_test_message_entry_proof(entry1);

    db.put_message_proof(account_id, index, proof1.clone())
        .unwrap();

    let entry2 = generate_test_message_entry(account_id, 2, 200);
    let proof2 = generate_test_message_entry_proof(entry2);
    db.put_message_proof(account_id, index, proof2.clone())
        .unwrap();

    let result = db.get_message_proof(account_id, index).unwrap();
    assert_eq!(result, Some(proof2));
}

pub fn test_multiple_accounts_and_indices(db: &impl SnarkAccountMessageDatabase) {
    let account_id1 = AccountId::new([10u8; 32]);
    let account_id2 = AccountId::new([20u8; 32]);
    let entry1 = generate_test_message_entry(account_id1, 1, 100);
    let entry2 = generate_test_message_entry(account_id1, 1, 200);
    let entry3 = generate_test_message_entry(account_id2, 1, 200);

    db.put_message_entry(account_id1, 0, entry1.clone())
        .unwrap();
    db.put_message_entry(account_id1, 1, entry2.clone())
        .unwrap();
    db.put_message_entry(account_id2, 0, entry3.clone())
        .unwrap();

    let result1 = db.get_message_entry(account_id1, 0).unwrap();
    assert_eq!(result1, Some(entry1));

    let result2 = db.get_message_entry(account_id1, 1).unwrap();
    assert_eq!(result2, Some(entry2));

    let result3 = db.get_message_entry(account_id2, 0).unwrap();
    assert_eq!(result3, Some(entry3));
}

#[macro_export]
macro_rules! snark_account_message_db_tests {
    ($setup_expr:expr) => {
        #[test]
        fn test_put_and_get_message_entry() {
            let db = $setup_expr;
            $crate::snark_account_message_tests::test_put_and_get_message_entry(&db);
        }

        #[test]
        fn test_get_nonexistent_message_entry() {
            let db = $setup_expr;
            $crate::snark_account_message_tests::test_get_nonexistent_message_entry(&db);
        }

        #[test]
        fn test_update_existing_message_entry() {
            let db = $setup_expr;
            $crate::snark_account_message_tests::test_update_existing_message_entry(&db);
        }

        #[test]
        fn test_put_and_get_message_proof() {
            let db = $setup_expr;
            $crate::snark_account_message_tests::test_put_and_get_message_proof(&db);
        }

        #[test]
        fn test_get_nonexistent_message_proof() {
            let db = $setup_expr;
            $crate::snark_account_message_tests::test_get_nonexistent_message_proof(&db);
        }

        #[test]
        fn test_update_existing_message_proof() {
            let db = $setup_expr;
            $crate::snark_account_message_tests::test_update_existing_message_proof(&db);
        }

        #[test]
        fn test_multiple_accounts_and_indices() {
            let db = $setup_expr;
            $crate::snark_account_message_tests::test_multiple_accounts_and_indices(&db);
        }
    };
}
