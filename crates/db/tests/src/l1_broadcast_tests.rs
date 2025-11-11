use bitcoin::hashes::Hash;
use strata_db_types::{
    traits::L1BroadcastDatabase,
    types::{L1TxEntry, L1TxStatus},
};
use strata_primitives::buf::Buf32;
use strata_test_utils_btc::get_test_bitcoin_txs;

pub fn test_get_last_tx_entry(db: &impl L1BroadcastDatabase) {
    for _ in 0..2 {
        let (txid, txentry) = generate_l1_tx_entry();

        let _ = db.put_tx_entry(txid, txentry.clone()).unwrap();
        let last_entry = db.get_last_tx_entry().unwrap();

        assert_eq!(last_entry, Some(txentry));
    }
}

pub fn test_add_tx_new_entry(db: &impl L1BroadcastDatabase) {
    let (txid, txentry) = generate_l1_tx_entry();

    let idx = db.put_tx_entry(txid, txentry.clone()).unwrap();

    assert_eq!(idx, Some(0));

    let stored_entry = db.get_tx_entry(idx.unwrap()).unwrap();
    assert_eq!(stored_entry, Some(txentry));
}

pub fn test_put_tx_existing_entry(db: &impl L1BroadcastDatabase) {
    let (txid, txentry) = generate_l1_tx_entry();

    let _ = db.put_tx_entry(txid, txentry.clone()).unwrap();

    // Update the same txid
    let result = db.put_tx_entry(txid, txentry);

    assert!(result.is_ok());
}

pub fn test_update_tx_entry(db: &impl L1BroadcastDatabase) {
    let (txid, txentry) = generate_l1_tx_entry();

    // Attempt to update non-existing index
    let result = db.put_tx_entry_by_idx(0, txentry.clone());
    assert!(result.is_err());

    // Add and then update the entry by index
    let idx = db.put_tx_entry(txid, txentry.clone()).unwrap();

    let mut updated_txentry = txentry;
    updated_txentry.status = L1TxStatus::Finalized { confirmations: 1 };

    db.put_tx_entry_by_idx(idx.unwrap(), updated_txentry.clone())
        .unwrap();

    let stored_entry = db.get_tx_entry(idx.unwrap()).unwrap();
    assert_eq!(stored_entry, Some(updated_txentry));
}

pub fn test_get_txentry_by_idx(db: &impl L1BroadcastDatabase) {
    // Test non-existing entry
    let result = db.get_tx_entry(0);
    assert!(result.is_err());

    let (txid, txentry) = generate_l1_tx_entry();

    let idx = db.put_tx_entry(txid, txentry.clone()).unwrap();

    let stored_entry = db.get_tx_entry(idx.unwrap()).unwrap();
    assert_eq!(stored_entry, Some(txentry));
}

pub fn test_get_next_txidx(db: &impl L1BroadcastDatabase) {
    let next_txidx = db.get_next_tx_idx().unwrap();
    assert_eq!(next_txidx, 0, "The next txidx is 0 in the beginning");

    let (txid, txentry) = generate_l1_tx_entry();

    let idx = db.put_tx_entry(txid, txentry.clone()).unwrap();

    let next_txidx = db.get_next_tx_idx().unwrap();

    assert_eq!(next_txidx, idx.unwrap() + 1);
}

pub fn test_del_tx_entry_single(db: &impl L1BroadcastDatabase) {
    let (txid, txentry) = generate_l1_tx_entry();

    // Insert tx entry
    db.put_tx_entry(txid, txentry.clone())
        .expect("test: insert");

    // Verify it exists
    assert!(db.get_tx_entry_by_id(txid).expect("test: get").is_some());

    // Delete it
    let deleted = db.del_tx_entry(txid).expect("test: delete");
    assert!(
        deleted,
        "Should return true when deleting existing tx entry"
    );

    // Verify it's gone
    assert!(db
        .get_tx_entry_by_id(txid)
        .expect("test: get after delete")
        .is_none());

    // Delete again should return false
    let deleted_again = db.del_tx_entry(txid).expect("test: delete again");
    assert!(
        !deleted_again,
        "Should return false when deleting non-existent tx entry"
    );
}

pub fn test_del_tx_entries_from_idx(db: &impl L1BroadcastDatabase) {
    let txs = get_test_bitcoin_txs();

    // Generate different tx entries
    let txid1: Buf32 = txs[0].compute_txid().as_raw_hash().to_byte_array().into();
    let txid2: Buf32 = txs[1].compute_txid().as_raw_hash().to_byte_array().into();
    let txid3: Buf32 = txs[2].compute_txid().as_raw_hash().to_byte_array().into();
    let txid4: Buf32 = txs[3].compute_txid().as_raw_hash().to_byte_array().into();

    let txentry1 = L1TxEntry::from_tx(&txs[0]);
    let txentry2 = L1TxEntry::from_tx(&txs[1]);
    let txentry3 = L1TxEntry::from_tx(&txs[2]);
    let txentry4 = L1TxEntry::from_tx(&txs[3]);

    // Insert tx entries - they will get consecutive indices
    db.put_tx_entry(txid1, txentry1).expect("test: insert 1");
    db.put_tx_entry(txid2, txentry2).expect("test: insert 2");
    db.put_tx_entry(txid3, txentry3).expect("test: insert 3");
    db.put_tx_entry(txid4, txentry4).expect("test: insert 4");

    // Verify all exist by getting tx by idx
    assert!(db.get_tx_entry(0).expect("test: get idx 0").is_some());
    assert!(db.get_tx_entry(1).expect("test: get idx 1").is_some());
    assert!(db.get_tx_entry(2).expect("test: get idx 2").is_some());
    assert!(db.get_tx_entry(3).expect("test: get idx 3").is_some());

    // Delete from index 2 onwards
    let deleted_indices = db
        .del_tx_entries_from_idx(2)
        .expect("test: delete from idx 2");
    assert_eq!(deleted_indices, vec![2, 3], "Should delete indices 2 and 3");

    // Verify indices 0 and 1 still exist, indices 2 and 3 are gone
    assert!(db.get_tx_entry(0).expect("test: get idx 0 after").is_some());
    assert!(db.get_tx_entry(1).expect("test: get idx 1 after").is_some());
    assert!(
        db.get_tx_entry(2).is_err(),
        "Should error when getting deleted index 2"
    );
    assert!(
        db.get_tx_entry(3).is_err(),
        "Should error when getting deleted index 3"
    );

    // Also verify the tx entries themselves are gone
    assert!(db
        .get_tx_entry_by_id(txid3)
        .expect("test: get id 3")
        .is_none());
    assert!(db
        .get_tx_entry_by_id(txid4)
        .expect("test: get id 4")
        .is_none());
}

pub fn test_del_tx_entries_empty_database(db: &impl L1BroadcastDatabase) {
    // Delete from empty database should return empty vec
    let deleted_indices = db
        .del_tx_entries_from_idx(0)
        .expect("test: delete from empty");
    assert!(
        deleted_indices.is_empty(),
        "Should return empty vec for empty database"
    );
}

// Helper function to generate L1TxEntry
fn generate_l1_tx_entry() -> (Buf32, L1TxEntry) {
    let txns = get_test_bitcoin_txs();
    let txid = txns[0].compute_txid().as_raw_hash().to_byte_array().into();
    let txentry = L1TxEntry::from_tx(&txns[0]);
    (txid, txentry)
}

#[macro_export]
macro_rules! l1_broadcast_db_tests {
    ($setup_expr:expr) => {
        #[test]
        fn test_get_last_tx_entry() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_get_last_tx_entry(&db);
        }

        #[test]
        fn test_add_tx_new_entry() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_add_tx_new_entry(&db);
        }

        #[test]
        fn test_put_tx_existing_entry() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_put_tx_existing_entry(&db);
        }

        #[test]
        fn test_update_tx_entry() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_update_tx_entry(&db);
        }

        #[test]
        fn test_get_txentry_by_idx() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_get_txentry_by_idx(&db);
        }

        #[test]
        fn test_get_next_txidx() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_get_next_txidx(&db);
        }

        #[test]
        fn test_del_tx_entry_single() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_del_tx_entry_single(&db);
        }

        #[test]
        fn test_del_tx_entries_from_idx() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_del_tx_entries_from_idx(&db);
        }

        #[test]
        fn test_del_tx_entries_empty_database() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_del_tx_entries_empty_database(&db);
        }
    };
}
