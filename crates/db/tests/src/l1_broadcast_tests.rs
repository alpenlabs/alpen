use bitcoin::{consensus::deserialize, hashes::Hash, Transaction};
use strata_db_types::{
    traits::L1BroadcastDatabase,
    types::{L1TxEntry, L1TxStatus},
};
use strata_primitives::buf::Buf32;

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
    updated_txentry.status = L1TxStatus::Finalized {
        confirmations: 1,
        block_hash: Buf32::zero(),
        block_height: 100,
    };

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

fn get_test_bitcoin_txs() -> Vec<Transaction> {
    let tx_hex = [
        "0200000000010176f29f18c5fc677ad6dd6c9309f6b9112f83cb95889af21da4be7fbfe22d1d220000000000fdffffff0300e1f505000000002200203946555814a18ccc94ef4991fb6af45278425e6a0a2cfc2bf4cf9c47515c56ff0000000000000000176a1500e0e78c8201d91f362c2ad3bb6f8e6f31349454663b1010240100000022512012d77c9ae5fdca5a3ab0b17a29b683fd2690f5ad56f6057a000ec42081ac89dc0247304402205de15fbfb413505a3563608dad6a73eb271b4006a4156eeb62d1eacca5efa10b02201eb71b975304f3cbdc664c6dd1c07b93ac826603309b3258cb92cfd201bb8792012102f55f96fd587a706a7b5e7312c4e9d755a65b3dad9945d65598bca34c9e961db400000000",
        "02000000000101f4f2e8830d2948b5e980e739e61b23f048d03d4af81588bf5da4618406c495aa0000000000fdffffff02969e0700000000002200203946555814a18ccc94ef4991fb6af45278425e6a0a2cfc2bf4cf9c47515c56ff60f59000000000001600148d0499ec043b1921a608d24690b061196e57c927040047304402203875f7b610f8783d5f5c163118eeec1a23473dd33b53c8ea584c7d28a82b209b022034b6814344b79826a348e23cc19ff06ed2df23850b889557552e376bf9e32c560147304402200f647dad3c137ff98d7da7a302345c82a57116a3d0e6a3719293bbb421cb0abe02201c04a1e808f5bab3595f77985af91aeaf61e9e042c9ac97d696e0f4b020cb54b0169522102dba8352965522ff44538dde37d793b3b4ece54e07759ade5f648aa396165d2962103c0683712773b725e7fe4809cbc90c9e0b890c45e5e24a852a4c472d1b6e9fd482103bf56f172d0631a7f8ae3ef648ad43a816ad01de4137ba89ebc33a2da8c48531553ae00000000",
        "02000000000101f4f2e8830d2948b5e980e739e61b23f048d03d4af81588bf5da4618406c495aa0200000000ffffffff0380969800000000002200203946555814a18ccc94ef4991fb6af45278425e6a0a2cfc2bf4cf9c47515c56ff0000000000000000176a15006e1a916a60b93a545f2370f2a36d2f807fb3d675588b693a000000001600149fafc79c72d1c4d917a360f32bdc68755402ef670247304402203c813ad8918366ce872642368b57b78e78e03b1a1eafe16ec8f3c9268b4fc050022018affe880963f18bfc0338f1e54c970185aa90f8c36a52ac935fe76cb885d726012102fa9b81d082a98a46d0857d62e6c9afe9e1bf40f9f0cbf361b96241c9d6fb064b00000000",
        "02000000000101d8acf0a647b7d5d1d0ee83360158d5bf01146d3762c442defd7985476b02aa6b0100000000fdffffff030065cd1d000000002200203946555814a18ccc94ef4991fb6af45278425e6a0a2cfc2bf4cf9c47515c56ff0000000000000000176a1500e0e78c8201d91f362c2ad3bb6f8e6f3134945466aec19dd00000000022512040718748dbca6dea8ac6b6f0b177014f0826478f1613c2b489e738db7ecdf3610247304402207cfc5cd87ec83687c9ac2bd921e96b8a58710f15d77bc7624da4fb29fe589dab0220437b74ed8e8f9d3084269edfb8641bf27246b0e5476667918beba73025c7a2c501210249a34cfbb6163b1b6ca2fff63fd1f8a802fb1999fa7930b2febe5a711f713dd900000000",
        "0200000000010176f29f18c5fc677ad6dd6c9309f6b9112f83cb95889af21da4be7fbfe22d1d220000000000fdffffff0300e1f505000000002200203946555814a18ccc94ef4991fb6af45278425e6a0a2cfc2bf4cf9c47515c56ff0000000000000000176a1500e0e78c8201d91f362c2ad3bb6f8e6f31349454663b1010240100000022512012d77c9ae5fdca5a3ab0b17a29b683fd2690f5ad56f6057a000ec42081ac89dc0247304402205de15fbfb413505a3563608dad6a73eb271b4006a4156eeb62d1eacca5efa10b02201eb71b975304f3cbdc664c6dd1c07b93ac826603309b3258cb92cfd201bb8792012102f55f96fd587a706a7b5e7312c4e9d755a65b3dad9945d65598bca34c9e961db400000000",
    ];

    tx_hex
        .iter()
        .map(|encoded| {
            deserialize(&hex::decode(encoded).expect("valid test tx hex"))
                .expect("valid test tx bytes")
        })
        .collect()
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
