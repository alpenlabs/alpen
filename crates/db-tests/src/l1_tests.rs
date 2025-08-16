use strata_db::traits::L1Database;
use strata_primitives::l1::{L1BlockManifest, L1Tx, L1TxProof, L1TxRef, ProtocolOperation};
use strata_test_utils::ArbitraryGenerator;

pub fn test_insert_into_empty_db(db: &impl L1Database) {
    let mut arb = ArbitraryGenerator::new_with_size(1 << 12);
    let idx = 1;

    // TODO maybe tweak this to make it a bit more realistic?
    let txs: Vec<L1Tx> = (0..10)
        .map(|i| {
            let proof = L1TxProof::new(i as u32, arb.generate());
            let parsed_tx: ProtocolOperation = arb.generate();
            L1Tx::new(proof, arb.generate(), vec![parsed_tx])
        })
        .collect();
    let mf = L1BlockManifest::new(arb.generate(), txs.clone(), arb.generate(), arb.generate());

    // Insert block data
    let res = db.put_block_data(mf.clone());
    assert!(res.is_ok(), "put should work but got: {}", res.unwrap_err());
    let res = db.set_canonical_chain_entry(idx, *mf.blkid());
    assert!(res.is_ok(), "put should work but got: {}", res.unwrap_err());

    // insert another block with arbitrary id
    let idx = 200_011;
    let txs: Vec<L1Tx> = (0..10)
        .map(|i| {
            let proof = L1TxProof::new(i as u32, arb.generate());
            let parsed_tx: ProtocolOperation = arb.generate();
            L1Tx::new(proof, arb.generate(), vec![parsed_tx])
        })
        .collect();
    let mf = L1BlockManifest::new(arb.generate(), txs.clone(), arb.generate(), arb.generate());

    // Insert block data
    let res = db.put_block_data(mf.clone());
    assert!(res.is_ok(), "put should work but got: {}", res.unwrap_err());
    let res = db.set_canonical_chain_entry(idx, *mf.blkid());
    assert!(res.is_ok(), "put should work but got: {}", res.unwrap_err());
}

pub fn test_insert_into_canonical_chain(db: &impl L1Database) {
    let heights = vec![1, 2, 5000, 1000, 1002, 999];
    let mut blockids = Vec::new();
    for height in &heights {
        let mut arb = ArbitraryGenerator::new();
        let txs: Vec<L1Tx> = (0..10).map(|_| arb.generate()).collect();
        let mf = L1BlockManifest::new(arb.generate(), txs, arb.generate(), arb.generate());
        let blockid = *mf.blkid();
        db.put_block_data(mf).unwrap();
        assert!(db.set_canonical_chain_entry(*height, blockid).is_ok());
        blockids.push(blockid);
    }

    for (height, expected_blockid) in heights.into_iter().zip(blockids) {
        assert!(matches!(
            db.get_canonical_blockid_at_height(height),
            Ok(Some(blockid)) if blockid == expected_blockid
        ));
    }
}

pub fn test_remove_canonical_chain_range(db: &impl L1Database) {
    // First insert a couple of manifests
    let num_txs = 10;
    let start_height = 1;
    let end_height = 10;
    for h in start_height..=end_height {
        insert_block_data(h, db, num_txs);
    }

    let remove_start_height = 5;
    let remove_end_height = 15;
    assert!(db
        .remove_canonical_chain_entries(remove_start_height, remove_end_height)
        .is_ok());

    // all removed items are gone from canonical chain
    for h in remove_start_height..=remove_end_height {
        assert!(matches!(db.get_canonical_blockid_at_height(h), Ok(None)));
    }
    // everything else is retained
    for h in start_height..remove_start_height {
        assert!(matches!(db.get_canonical_blockid_at_height(h), Ok(Some(_))));
    }
}

pub fn test_get_block_data(db: &impl L1Database) {
    let idx = 1;

    // insert
    let (mf, txs) = insert_block_data(idx, db, 10);

    // fetch non existent block
    let non_idx = 200;
    let observed_blockid = db
        .get_canonical_blockid_at_height(non_idx)
        .expect("Could not fetch from db");
    assert_eq!(observed_blockid, None);

    // fetch and check, existent block
    let blockid = db
        .get_canonical_blockid_at_height(idx)
        .expect("Could not fetch from db")
        .expect("Expected block missing");
    let observed_mf = db
        .get_block_manifest(blockid)
        .expect("Could not fetch from db");
    assert_eq!(observed_mf, Some(mf));

    // Fetch txs
    for (i, tx) in txs.iter().enumerate() {
        let tx_from_db = db
            .get_tx((blockid, i as u32).into())
            .expect("Can't fetch from db")
            .unwrap();
        assert_eq!(*tx, tx_from_db, "Txns should match at index {i}");
    }
}

pub fn test_get_tx(db: &impl L1Database) {
    let idx = 1; // block number
                 // Insert a block
    let (mf, txns) = insert_block_data(idx, db, 10);
    let blockid = mf.blkid();
    let txidx: u32 = 3; // some tx index
    assert!(txns.len() > txidx as usize);
    let tx_ref: L1TxRef = (*blockid, txidx).into();
    let tx = db.get_tx(tx_ref);
    assert!(tx.as_ref().unwrap().is_some());
    let tx = tx.unwrap().unwrap().clone();
    assert_eq!(
        tx,
        *txns.get(txidx as usize).unwrap(),
        "Should fetch correct transaction"
    );
    // Check txn at different index. It should not match
    assert_ne!(
        tx,
        *txns.get(txidx as usize + 1).unwrap(),
        "Txn at different index should not match"
    );
}

pub fn test_get_chain_tip(db: &impl L1Database) {
    assert_eq!(
        db.get_canonical_chain_tip().unwrap(),
        None,
        "chain tip of empty db should be unset"
    );

    // Insert some block data
    let num_txs = 10;
    insert_block_data(1, db, num_txs);
    assert!(matches!(
        db.get_canonical_chain_tip().unwrap(),
        Some((1, _))
    ));
    insert_block_data(2, db, num_txs);
    assert!(matches!(
        db.get_canonical_chain_tip().unwrap(),
        Some((2, _))
    ));
}

pub fn test_get_block_txs(db: &impl L1Database) {
    let num_txs = 10;
    insert_block_data(1, db, num_txs);
    insert_block_data(2, db, num_txs);
    insert_block_data(3, db, num_txs);

    let blockid = db.get_canonical_blockid_at_height(2).unwrap().unwrap();
    let block_txs = db.get_block_txs(blockid).unwrap().unwrap();
    let expected: Vec<_> = (0..num_txs).map(|i| (blockid, i as u32).into()).collect(); // 10 because insert_block_data inserts 10 txs
    assert_eq!(block_txs, expected);
}

pub fn test_get_blockid_invalid_range(db: &impl L1Database) {
    let num_txs = 10;
    let _ = insert_block_data(1, db, num_txs);
    let _ = insert_block_data(2, db, num_txs);
    let _ = insert_block_data(3, db, num_txs);

    let range = db.get_canonical_blockid_range(3, 1).unwrap();
    assert_eq!(range.len(), 0);
}

pub fn test_get_blockid_range(db: &impl L1Database) {
    let num_txs = 10;
    let (mf1, _) = insert_block_data(1, db, num_txs);
    let (mf2, _) = insert_block_data(2, db, num_txs);
    let (mf3, _) = insert_block_data(3, db, num_txs);

    let range = db.get_canonical_blockid_range(1, 4).unwrap();
    assert_eq!(range.len(), 3);
    for (exp, obt) in vec![mf1, mf2, mf3].iter().zip(range) {
        assert_eq!(*exp.blkid(), obt);
    }
}

pub fn test_get_txs_fancy(db: &impl L1Database) {
    let num_txs = 3;
    let total_num_blocks = 4;

    let mut l1_txs = Vec::with_capacity(total_num_blocks);
    for i in 0..total_num_blocks {
        let (mf, block_txs) = insert_block_data(i as u64, db, num_txs);
        l1_txs.push((*mf.blkid(), block_txs));
    }

    let (latest_idx, _) = db
        .get_canonical_chain_tip()
        .expect("should not error")
        .expect("should have latest");

    assert_eq!(
        latest_idx,
        (total_num_blocks - 1) as u64,
        "the latest index must match the total number of blocks inserted"
    );

    for (blockid, block_txs) in l1_txs.iter() {
        for (i, exp_tx) in block_txs.iter().enumerate() {
            let real_tx = db
                .get_tx(L1TxRef::from((*blockid, i as u32)))
                .expect("test: database failed")
                .expect("test: missing expected tx");

            assert_eq!(
                &real_tx, exp_tx,
                "tx mismatch in block {blockid} at idx {i}"
            );
        }
    }

    // get past the final index.
    let (latest_idx, _) = db
        .get_canonical_chain_tip()
        .expect("should not error")
        .expect("should have latest");
    let expected_latest = (total_num_blocks - 1) as u64;

    assert_eq!(
        latest_idx, expected_latest,
        "test: wrong latest block number",
    );
}

// Helper function to insert block data
fn insert_block_data(
    height: u64,
    db: &impl L1Database,
    num_txs: usize,
) -> (L1BlockManifest, Vec<L1Tx>) {
    let mut arb = ArbitraryGenerator::new_with_size(1 << 12);

    // TODO maybe tweak this to make it a bit more realistic?
    let txs: Vec<L1Tx> = (0..num_txs)
        .map(|i| {
            let proof = L1TxProof::new(i as u32, arb.generate());
            let parsed_tx: ProtocolOperation = arb.generate();
            L1Tx::new(proof, arb.generate(), vec![parsed_tx])
        })
        .collect();
    let mf = L1BlockManifest::new(arb.generate(), txs.clone(), arb.generate(), arb.generate());

    // Insert block data
    let res = db.put_block_data(mf.clone());
    assert!(res.is_ok(), "put should work but got: {}", res.unwrap_err());
    let res = db.set_canonical_chain_entry(height, *mf.blkid());
    assert!(res.is_ok(), "put should work but got: {}", res.unwrap_err());

    (mf, txs)
}

#[macro_export]
macro_rules! l1_db_tests {
    ($setup_expr:expr) => {
        #[test]
        fn test_insert_into_empty_db() {
            let db = $setup_expr;
            $crate::l1_tests::test_insert_into_empty_db(&db);
        }

        #[test]
        fn test_insert_into_canonical_chain() {
            let db = $setup_expr;
            $crate::l1_tests::test_insert_into_canonical_chain(&db);
        }

        #[test]
        fn test_remove_canonical_chain_range() {
            let db = $setup_expr;
            $crate::l1_tests::test_remove_canonical_chain_range(&db);
        }

        #[test]
        fn test_get_block_data() {
            let db = $setup_expr;
            $crate::l1_tests::test_get_block_data(&db);
        }

        #[test]
        fn test_get_tx() {
            let db = $setup_expr;
            $crate::l1_tests::test_get_tx(&db);
        }

        #[test]
        fn test_get_chain_tip() {
            let db = $setup_expr;
            $crate::l1_tests::test_get_chain_tip(&db);
        }

        #[test]
        fn test_get_block_txs() {
            let db = $setup_expr;
            $crate::l1_tests::test_get_block_txs(&db);
        }

        #[test]
        fn test_get_blockid_invalid_range() {
            let db = $setup_expr;
            $crate::l1_tests::test_get_blockid_invalid_range(&db);
        }

        #[test]
        fn test_get_blockid_range() {
            let db = $setup_expr;
            $crate::l1_tests::test_get_blockid_range(&db);
        }

        #[test]
        fn test_get_txs_fancy() {
            let db = $setup_expr;
            $crate::l1_tests::test_get_txs_fancy(&db);
        }
    };
}
