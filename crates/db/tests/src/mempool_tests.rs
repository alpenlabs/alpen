use strata_db_types::{traits::MempoolDatabase, types::MempoolTxData};
use strata_identifiers::{Buf32, OLTxId};

pub fn test_put_and_get_tx(db: &impl MempoolDatabase) {
    let txid = OLTxId::from(Buf32::from([1u8; 32]));
    let tx_bytes = vec![1, 2, 3, 4, 5];
    let timestamp_micros = 1_000_000;

    // Put transaction
    let data = MempoolTxData::new(txid, tx_bytes.clone(), timestamp_micros);
    db.put_tx(data).unwrap();

    // Get transaction
    let result = db.get_tx(txid).unwrap();
    assert!(result.is_some());
    let retrieved = result.unwrap();
    assert_eq!(retrieved.tx_bytes, tx_bytes);
    assert_eq!(retrieved.timestamp_micros, timestamp_micros);
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
    let timestamp_micros = 1_000_000;

    // Put transaction
    let data = MempoolTxData::new(txid, tx_bytes.clone(), timestamp_micros);
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
    assert_eq!(db.get_all_txs().unwrap().len(), 0);

    (1u8..=3).for_each(|i| {
        db.put_tx(MempoolTxData::new(
            OLTxId::from(Buf32::from([i; 32])),
            vec![i; 10], // creates vec with 10 copies of i
            (i as u64) * 1_000_000,
        ))
        .unwrap();
    });

    let all_txs = db.get_all_txs().unwrap();
    assert_eq!(all_txs.len(), 3);

    // Verify each transaction exists
    for i in 1u8..=3 {
        assert!(all_txs
            .iter()
            .any(|tx| tx.txid == OLTxId::from(Buf32::from([i; 32]))
                && tx.tx_bytes == vec![i; 10]
                && tx.timestamp_micros == (i as u64) * 1_000_000));
    }
}

pub fn test_overwrite_tx(db: &impl MempoolDatabase) {
    let txid = OLTxId::from(Buf32::from([1u8; 32]));
    let tx_bytes_1 = vec![1, 2, 3];
    let timestamp_micros_1 = 1_000_000;

    let tx_bytes_2 = vec![4, 5, 6, 7];
    let timestamp_micros_2 = 2_000_000;

    // Put first version
    db.put_tx(MempoolTxData::new(
        txid,
        tx_bytes_1.clone(),
        timestamp_micros_1,
    ))
    .unwrap();

    // Overwrite with second version
    db.put_tx(MempoolTxData::new(
        txid,
        tx_bytes_2.clone(),
        timestamp_micros_2,
    ))
    .unwrap();

    // Get should return second version
    let result = db.get_tx(txid).unwrap();
    assert!(result.is_some());
    let retrieved = result.unwrap();
    assert_eq!(retrieved.tx_bytes, tx_bytes_2);
    assert_eq!(retrieved.timestamp_micros, timestamp_micros_2);
}

pub fn test_empty_tx_bytes(db: &impl MempoolDatabase) {
    let txid = OLTxId::from(Buf32::from([1u8; 32]));
    let tx_bytes = vec![];
    let timestamp_micros = 1_000_000;

    // Put transaction with empty bytes
    db.put_tx(MempoolTxData::new(txid, tx_bytes.clone(), timestamp_micros))
        .unwrap();

    // Get transaction
    let result = db.get_tx(txid).unwrap();
    assert!(result.is_some());
    let retrieved = result.unwrap();
    assert_eq!(retrieved.tx_bytes, tx_bytes);
    assert_eq!(retrieved.timestamp_micros, timestamp_micros);
}

pub fn test_large_tx_bytes(db: &impl MempoolDatabase) {
    let txid = OLTxId::from(Buf32::from([1u8; 32]));
    let tx_bytes = vec![0x42; 1_000_000]; // 1 MB transaction
    let timestamp_micros = 1_000_000;

    // Put large transaction
    db.put_tx(MempoolTxData::new(txid, tx_bytes.clone(), timestamp_micros))
        .unwrap();

    // Get transaction
    let result = db.get_tx(txid).unwrap();
    assert!(result.is_some());
    let retrieved = result.unwrap();
    assert_eq!(retrieved.tx_bytes, tx_bytes);
    assert_eq!(retrieved.timestamp_micros, timestamp_micros);
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
