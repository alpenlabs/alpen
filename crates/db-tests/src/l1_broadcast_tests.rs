use bitcoin::hashes::Hash;
use strata_db::{
    traits::L1BroadcastDatabase,
    types::{L1TxEntry, L1TxStatus},
};
use strata_primitives::buf::Buf32;
use strata_test_utils::bitcoin::get_test_bitcoin_txs;

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
    };
}
