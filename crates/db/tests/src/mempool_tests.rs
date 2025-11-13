use std::collections::HashMap;

use strata_acct_types::{AccountId, VarVec};
use strata_codec::encode_to_vec;
use strata_db_types::traits::MempoolDatabase;
use strata_identifiers::OLTxId;
use strata_ol_chain_types_new::{
    GamTxPayload, OLTransaction, TransactionAttachment, TransactionPayload,
};
use strata_primitives::buf::Buf32;

fn generate_test_tx(seed: u8) -> (OLTxId, Vec<u8>) {
    let gam_payload = GamTxPayload::new(
        AccountId::new([0u8; 32]),
        VarVec::from_vec(vec![seed]).unwrap(),
    );
    let payload = TransactionPayload::GenericAccountMessage(gam_payload);
    let attachment = TransactionAttachment::new_empty();
    let tx = OLTransaction::new(attachment, payload);

    let txid = tx.compute_txid();
    let blob = encode_to_vec(&tx).unwrap();

    (txid, blob)
}

pub fn test_put_and_get_tx_entry(db: &impl MempoolDatabase) {
    let (txid, blob) = generate_test_tx(1);

    db.put_tx_entry(&txid, &blob).unwrap();

    let result = db.get_tx_entry(&txid).unwrap();
    assert_eq!(result, Some(blob));
}

pub fn test_get_nonexistent_tx(db: &impl MempoolDatabase) {
    let txid = OLTxId::from(Buf32::zero());
    let result = db.get_tx_entry(&txid).unwrap();
    assert!(result.is_none());
}

pub fn test_update_existing_tx(db: &impl MempoolDatabase) {
    let (txid, blob1) = generate_test_tx(1);

    db.put_tx_entry(&txid, &blob1).unwrap();

    let (_, blob2) = generate_test_tx(2);
    // Update with a different blob
    db.put_tx_entry(&txid, &blob2).unwrap();

    let result = db.get_tx_entry(&txid).unwrap();
    assert_eq!(result, Some(blob2));
}

pub fn test_get_tx_entries_batch(db: &impl MempoolDatabase) {
    let (txid1, blob1) = generate_test_tx(1);
    let (txid2, blob2) = generate_test_tx(2);
    let (txid3, blob3) = generate_test_tx(3);

    db.put_tx_entry(&txid1, &blob1).unwrap();
    db.put_tx_entry(&txid2, &blob2).unwrap();
    db.put_tx_entry(&txid3, &blob3).unwrap();

    let txids = vec![txid1, txid2, txid3];
    let result = db.get_tx_entries(&txids).unwrap();

    assert_eq!(result.len(), 3);
    assert_eq!(result.get(&txid1), Some(&blob1));
    assert_eq!(result.get(&txid2), Some(&blob2));
    assert_eq!(result.get(&txid3), Some(&blob3));
}

pub fn test_get_tx_entries_partial(db: &impl MempoolDatabase) {
    let (txid1, blob1) = generate_test_tx(1);
    let (txid2, blob2) = generate_test_tx(2);
    let txid_missing = OLTxId::from(Buf32::zero());

    db.put_tx_entry(&txid1, &blob1).unwrap();
    db.put_tx_entry(&txid2, &blob2).unwrap();

    let txids = vec![txid1, txid_missing, txid2];
    let result = db.get_tx_entries(&txids).unwrap();

    assert_eq!(result.len(), 2);
    assert_eq!(result.get(&txid1), Some(&blob1));
    assert_eq!(result.get(&txid2), Some(&blob2));
    assert_eq!(result.get(&txid_missing), None);
}

pub fn test_del_tx_entry(db: &impl MempoolDatabase) {
    let (txid, blob) = generate_test_tx(1);

    db.put_tx_entry(&txid, &blob).unwrap();

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
    let (txid1, blob1) = generate_test_tx(1);
    let (txid2, blob2) = generate_test_tx(2);
    let (txid3, blob3) = generate_test_tx(3);

    db.put_tx_entry(&txid1, &blob1).unwrap();
    db.put_tx_entry(&txid2, &blob2).unwrap();
    db.put_tx_entry(&txid3, &blob3).unwrap();

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
    let (txid1, blob1) = generate_test_tx(1);
    let (txid2, blob2) = generate_test_tx(2);
    let (txid3, blob3) = generate_test_tx(3);

    db.put_tx_entry(&txid1, &blob1).unwrap();
    db.put_tx_entry(&txid2, &blob2).unwrap();
    db.put_tx_entry(&txid3, &blob3).unwrap();

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
