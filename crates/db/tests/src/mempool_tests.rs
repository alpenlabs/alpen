use strata_db_types::{traits::MempoolDatabase, types::MempoolTxData};
use strata_identifiers::{Buf32, OLTxId};

pub fn test_put_and_get_tx(db: &impl MempoolDatabase) {
    let txid = OLTxId::from(Buf32::from([1u8; 32]));
    let tx_bytes = vec![1, 2, 3, 4, 5];
    let first_seen_slot = 100;
    let insertion_id = 1;

    // Put transaction
    let data = MempoolTxData::new(txid, tx_bytes.clone(), first_seen_slot, insertion_id);
    db.put_tx(data).unwrap();

    // Get transaction
    let result = db.get_tx(txid).unwrap();
    assert!(result.is_some());
    let retrieved = result.unwrap();
    assert_eq!(retrieved.tx_bytes, tx_bytes);
    assert_eq!(retrieved.first_seen_slot, first_seen_slot);
    assert_eq!(retrieved.insertion_id, insertion_id);
}

pub fn test_get_nonexistent_tx(db: &impl MempoolDatabase) {
    let txid = OLTxId::from(Buf32::from([1u8; 32]));

    // Try to get non-existent transaction
    let result = db.get_tx(txid).unwrap();
    assert!(result.is_none());
}

pub fn test_del_tx(db: &impl MempoolDatabase) {
    let txid = OLTxId::from(Buf32::from([1u8; 32]));
    let tx_bytes = vec![1, 2, 3, 4, 5];
    let first_seen_slot = 100;
    let insertion_id = 1;

    // Put transaction
    let data = MempoolTxData::new(txid, tx_bytes.clone(), first_seen_slot, insertion_id);
    db.put_tx(data).unwrap();

    // Verify it exists
    assert!(db.get_tx(txid).unwrap().is_some());

    // Delete transaction
    let existed = db.del_tx(txid).unwrap();
    assert!(existed);

    // Verify it's gone
    assert!(db.get_tx(txid).unwrap().is_none());

    // Delete again should return false
    let existed = db.del_tx(txid).unwrap();
    assert!(!existed);
}

pub fn test_get_all_txs(db: &impl MempoolDatabase) {
    // Initially empty
    let all_txs = db.get_all_txs().unwrap();
    assert_eq!(all_txs.len(), 0);

    // Add multiple transactions
    let tx1_id = OLTxId::from(Buf32::from([1u8; 32]));
    let tx1_bytes = vec![1, 2, 3];
    let tx1_slot = 100;
    let tx1_insertion = 1;

    let tx2_id = OLTxId::from(Buf32::from([2u8; 32]));
    let tx2_bytes = vec![4, 5, 6];
    let tx2_slot = 200;
    let tx2_insertion = 2;

    let tx3_id = OLTxId::from(Buf32::from([3u8; 32]));
    let tx3_bytes = vec![7, 8, 9];
    let tx3_slot = 300;
    let tx3_insertion = 3;

    db.put_tx(MempoolTxData::new(
        tx1_id,
        tx1_bytes.clone(),
        tx1_slot,
        tx1_insertion,
    ))
    .unwrap();
    db.put_tx(MempoolTxData::new(
        tx2_id,
        tx2_bytes.clone(),
        tx2_slot,
        tx2_insertion,
    ))
    .unwrap();
    db.put_tx(MempoolTxData::new(
        tx3_id,
        tx3_bytes.clone(),
        tx3_slot,
        tx3_insertion,
    ))
    .unwrap();

    // Get all transactions
    let all_txs = db.get_all_txs().unwrap();
    assert_eq!(all_txs.len(), 3);

    // Verify contents (order not guaranteed)
    let mut found_tx1 = false;
    let mut found_tx2 = false;
    let mut found_tx3 = false;

    for tx in all_txs {
        if tx.txid == tx1_id {
            assert_eq!(tx.tx_bytes, tx1_bytes);
            assert_eq!(tx.first_seen_slot, tx1_slot);
            assert_eq!(tx.insertion_id, tx1_insertion);
            found_tx1 = true;
        } else if tx.txid == tx2_id {
            assert_eq!(tx.tx_bytes, tx2_bytes);
            assert_eq!(tx.first_seen_slot, tx2_slot);
            assert_eq!(tx.insertion_id, tx2_insertion);
            found_tx2 = true;
        } else if tx.txid == tx3_id {
            assert_eq!(tx.tx_bytes, tx3_bytes);
            assert_eq!(tx.first_seen_slot, tx3_slot);
            assert_eq!(tx.insertion_id, tx3_insertion);
            found_tx3 = true;
        } else {
            panic!("Unexpected transaction ID");
        }
    }

    assert!(found_tx1);
    assert!(found_tx2);
    assert!(found_tx3);
}

pub fn test_overwrite_tx(db: &impl MempoolDatabase) {
    let txid = OLTxId::from(Buf32::from([1u8; 32]));
    let tx_bytes_1 = vec![1, 2, 3];
    let slot_1 = 100;
    let insertion_1 = 1;

    let tx_bytes_2 = vec![4, 5, 6, 7];
    let slot_2 = 200;
    let insertion_2 = 2;

    // Put first version
    db.put_tx(MempoolTxData::new(
        txid,
        tx_bytes_1.clone(),
        slot_1,
        insertion_1,
    ))
    .unwrap();

    // Overwrite with second version
    db.put_tx(MempoolTxData::new(
        txid,
        tx_bytes_2.clone(),
        slot_2,
        insertion_2,
    ))
    .unwrap();

    // Get should return second version
    let result = db.get_tx(txid).unwrap();
    assert!(result.is_some());
    let retrieved = result.unwrap();
    assert_eq!(retrieved.tx_bytes, tx_bytes_2);
    assert_eq!(retrieved.first_seen_slot, slot_2);
    assert_eq!(retrieved.insertion_id, insertion_2);
}

pub fn test_empty_tx_bytes(db: &impl MempoolDatabase) {
    let txid = OLTxId::from(Buf32::from([1u8; 32]));
    let tx_bytes = vec![];
    let first_seen_slot = 100;
    let insertion_id = 1;

    // Put transaction with empty bytes
    db.put_tx(MempoolTxData::new(
        txid,
        tx_bytes.clone(),
        first_seen_slot,
        insertion_id,
    ))
    .unwrap();

    // Get transaction
    let result = db.get_tx(txid).unwrap();
    assert!(result.is_some());
    let retrieved = result.unwrap();
    assert_eq!(retrieved.tx_bytes, tx_bytes);
    assert_eq!(retrieved.first_seen_slot, first_seen_slot);
    assert_eq!(retrieved.insertion_id, insertion_id);
}

pub fn test_large_tx_bytes(db: &impl MempoolDatabase) {
    let txid = OLTxId::from(Buf32::from([1u8; 32]));
    let tx_bytes = vec![0x42; 1_000_000]; // 1 MB transaction
    let first_seen_slot = 100;
    let insertion_id = 1;

    // Put large transaction
    db.put_tx(MempoolTxData::new(
        txid,
        tx_bytes.clone(),
        first_seen_slot,
        insertion_id,
    ))
    .unwrap();

    // Get transaction
    let result = db.get_tx(txid).unwrap();
    assert!(result.is_some());
    let retrieved = result.unwrap();
    assert_eq!(retrieved.tx_bytes, tx_bytes);
    assert_eq!(retrieved.first_seen_slot, first_seen_slot);
    assert_eq!(retrieved.insertion_id, insertion_id);
}

#[macro_export]
macro_rules! mempool_db_tests {
    ($setup_expr:expr) => {
        #[test]
        fn test_put_and_get_tx() {
            let db = $setup_expr;
            $crate::mempool_tests::test_put_and_get_tx(&db);
        }

        #[test]
        fn test_get_nonexistent_tx() {
            let db = $setup_expr;
            $crate::mempool_tests::test_get_nonexistent_tx(&db);
        }

        #[test]
        fn test_del_tx() {
            let db = $setup_expr;
            $crate::mempool_tests::test_del_tx(&db);
        }

        #[test]
        fn test_get_all_txs() {
            let db = $setup_expr;
            $crate::mempool_tests::test_get_all_txs(&db);
        }

        #[test]
        fn test_overwrite_tx() {
            let db = $setup_expr;
            $crate::mempool_tests::test_overwrite_tx(&db);
        }

        #[test]
        fn test_empty_tx_bytes() {
            let db = $setup_expr;
            $crate::mempool_tests::test_empty_tx_bytes(&db);
        }

        #[test]
        fn test_large_tx_bytes() {
            let db = $setup_expr;
            $crate::mempool_tests::test_large_tx_bytes(&db);
        }
    };
}
