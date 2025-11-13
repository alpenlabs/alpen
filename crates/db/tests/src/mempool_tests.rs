use std::collections::HashMap;

use strata_acct_types::AccountId;
use strata_db_types::{traits::MempoolDatabase, types::MempoolTxMetadata};
use strata_identifiers::OLTxId;
use strata_ol_chain_types_new::{OLTransaction, TransactionExtra, TransactionPayload};
use strata_primitives::buf::Buf32;

fn generate_test_tx(seed: u8) -> (OLTxId, OLTransaction, MempoolTxMetadata) {
    let payload = TransactionPayload::GenericAccountMessage {
        target: AccountId::new([0u8; 32]),
        payload: vec![seed],
    };
    let extra = TransactionExtra::default();
    let tx = OLTransaction::new(payload, extra);

    let txid = tx.compute_txid();

    let size_bytes = borsh::to_vec(&tx).unwrap().len();
    let metadata = MempoolTxMetadata {
        size_bytes,
        entry_slot: seed as u64,
        entry_time: seed as u64,
    };

    (txid, tx, metadata)
}

pub fn test_put_and_get_tx_entry(db: &impl MempoolDatabase) {
    let (txid, tx, metadata) = generate_test_tx(1);

    db.put_tx_entry(&txid, &tx, &metadata).unwrap();

    let result = db.get_tx_entry(&txid).unwrap();
    assert_eq!(result, Some((tx, metadata)));
}

pub fn test_get_nonexistent_tx(db: &impl MempoolDatabase) {
    let txid = OLTxId::from(Buf32::zero());
    let result = db.get_tx_entry(&txid).unwrap();
    assert!(result.is_none());
}

pub fn test_update_existing_tx(db: &impl MempoolDatabase) {
    let (txid, tx, metadata) = generate_test_tx(1);

    db.put_tx_entry(&txid, &tx, &metadata).unwrap();

    let new_metadata = MempoolTxMetadata {
        size_bytes: metadata.size_bytes,
        entry_slot: metadata.entry_slot + 1,
        entry_time: metadata.entry_time + 1000,
    };

    db.put_tx_entry(&txid, &tx, &new_metadata).unwrap();

    let result = db.get_tx_entry(&txid).unwrap();
    assert_eq!(result, Some((tx, new_metadata)));
}

pub fn test_get_tx_entries_batch(db: &impl MempoolDatabase) {
    let (txid1, tx1, metadata1) = generate_test_tx(1);
    let (txid2, tx2, metadata2) = generate_test_tx(2);
    let (txid3, tx3, metadata3) = generate_test_tx(3);

    db.put_tx_entry(&txid1, &tx1, &metadata1).unwrap();
    db.put_tx_entry(&txid2, &tx2, &metadata2).unwrap();
    db.put_tx_entry(&txid3, &tx3, &metadata3).unwrap();

    let txids = vec![txid1, txid2, txid3];
    let result = db.get_tx_entries(&txids).unwrap();

    assert_eq!(result.len(), 3);
    assert_eq!(result.get(&txid1), Some(&(tx1, metadata1)));
    assert_eq!(result.get(&txid2), Some(&(tx2, metadata2)));
    assert_eq!(result.get(&txid3), Some(&(tx3, metadata3)));
}

pub fn test_get_tx_entries_partial(db: &impl MempoolDatabase) {
    let (txid1, tx1, metadata1) = generate_test_tx(1);
    let (txid2, tx2, metadata2) = generate_test_tx(2);
    let txid_missing = OLTxId::from(Buf32::zero());

    db.put_tx_entry(&txid1, &tx1, &metadata1).unwrap();
    db.put_tx_entry(&txid2, &tx2, &metadata2).unwrap();

    let txids = vec![txid1, txid_missing, txid2];
    let result = db.get_tx_entries(&txids).unwrap();

    assert_eq!(result.len(), 2);
    assert_eq!(result.get(&txid1), Some(&(tx1, metadata1)));
    assert_eq!(result.get(&txid2), Some(&(tx2, metadata2)));
    assert_eq!(result.get(&txid_missing), None);
}

pub fn test_del_tx_entry(db: &impl MempoolDatabase) {
    let (txid, tx, metadata) = generate_test_tx(1);

    db.put_tx_entry(&txid, &tx, &metadata).unwrap();

    assert!(db.get_tx_entry(&txid).unwrap().is_some());

    db.del_tx_entry(&txid).unwrap();

    assert!(db.get_tx_entry(&txid).unwrap().is_none());
}

pub fn test_del_nonexistent_tx(db: &impl MempoolDatabase) {
    let txid = OLTxId::from(Buf32::zero());
    let result = db.del_tx_entry(&txid);
    assert!(result.is_ok());
}

pub fn test_del_tx_entries_batch(db: &impl MempoolDatabase) {
    let (txid1, tx1, metadata1) = generate_test_tx(1);
    let (txid2, tx2, metadata2) = generate_test_tx(2);
    let (txid3, tx3, metadata3) = generate_test_tx(3);

    db.put_tx_entry(&txid1, &tx1, &metadata1).unwrap();
    db.put_tx_entry(&txid2, &tx2, &metadata2).unwrap();
    db.put_tx_entry(&txid3, &tx3, &metadata3).unwrap();

    let txids_to_delete = vec![txid1, txid3];
    db.del_tx_entries(&txids_to_delete).unwrap();

    assert!(db.get_tx_entry(&txid1).unwrap().is_none());
    assert!(db.get_tx_entry(&txid2).unwrap().is_some());
    assert!(db.get_tx_entry(&txid3).unwrap().is_none());
}

pub fn test_get_all_tx_ids_empty(db: &impl MempoolDatabase) {
    let result = db.get_all_tx_ids().unwrap();
    assert!(result.is_empty());
}

pub fn test_get_all_tx_ids(db: &impl MempoolDatabase) {
    let (txid1, tx1, metadata1) = generate_test_tx(1);
    let (txid2, tx2, metadata2) = generate_test_tx(2);
    let (txid3, tx3, metadata3) = generate_test_tx(3);

    db.put_tx_entry(&txid1, &tx1, &metadata1).unwrap();
    db.put_tx_entry(&txid2, &tx2, &metadata2).unwrap();
    db.put_tx_entry(&txid3, &tx3, &metadata3).unwrap();

    let result = db.get_all_tx_ids().unwrap();

    assert_eq!(result.len(), 3);

    let result_set: HashMap<OLTxId, ()> = result.into_iter().map(|txid| (txid, ())).collect();
    assert!(result_set.contains_key(&txid1));
    assert!(result_set.contains_key(&txid2));
    assert!(result_set.contains_key(&txid3));
}

#[macro_export]
macro_rules! mempool_db_tests {
    ($setup_expr:expr) => {
        #[test]
        fn test_put_and_get_tx_entry() {
            let db = $setup_expr;
            $crate::mempool_tests::test_put_and_get_tx_entry(&db);
        }

        #[test]
        fn test_get_nonexistent_tx() {
            let db = $setup_expr;
            $crate::mempool_tests::test_get_nonexistent_tx(&db);
        }

        #[test]
        fn test_update_existing_tx() {
            let db = $setup_expr;
            $crate::mempool_tests::test_update_existing_tx(&db);
        }

        #[test]
        fn test_get_tx_entries_batch() {
            let db = $setup_expr;
            $crate::mempool_tests::test_get_tx_entries_batch(&db);
        }

        #[test]
        fn test_get_tx_entries_partial() {
            let db = $setup_expr;
            $crate::mempool_tests::test_get_tx_entries_partial(&db);
        }

        #[test]
        fn test_del_tx_entry() {
            let db = $setup_expr;
            $crate::mempool_tests::test_del_tx_entry(&db);
        }

        #[test]
        fn test_del_nonexistent_tx() {
            let db = $setup_expr;
            $crate::mempool_tests::test_del_nonexistent_tx(&db);
        }

        #[test]
        fn test_del_tx_entries_batch() {
            let db = $setup_expr;
            $crate::mempool_tests::test_del_tx_entries_batch(&db);
        }

        #[test]
        fn test_get_all_tx_ids_empty() {
            let db = $setup_expr;
            $crate::mempool_tests::test_get_all_tx_ids_empty(&db);
        }

        #[test]
        fn test_get_all_tx_ids() {
            let db = $setup_expr;
            $crate::mempool_tests::test_get_all_tx_ids(&db);
        }
    };
}
