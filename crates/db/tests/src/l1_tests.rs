use strata_asm_common::AsmManifest;
use strata_db_types::traits::L1Database;
use strata_test_utils::ArbitraryGenerator;

pub fn test_insert_into_empty_db(db: &impl L1Database) {
    let mut arb = ArbitraryGenerator::new_with_size(1 << 12);
    let idx = 1;

    // TODO maybe tweak this to make it a bit more realistic?
    let mf = AsmManifest::new(idx, arb.generate(), arb.generate(), vec![]);

    // Insert block data
    let res = db.put_block_data(mf.clone(), idx);
    assert!(res.is_ok(), "put should work but got: {}", res.unwrap_err());
    let res = db.set_canonical_chain_entry(idx, *mf.blkid());
    assert!(res.is_ok(), "put should work but got: {}", res.unwrap_err());

    // insert another block with arbitrary id
    let idx = 200_011;
    let mf = AsmManifest::new(idx, arb.generate(), arb.generate(), vec![]);

    // Insert block data
    let res = db.put_block_data(mf.clone(), idx);
    assert!(res.is_ok(), "put should work but got: {}", res.unwrap_err());
    let res = db.set_canonical_chain_entry(idx, *mf.blkid());
    assert!(res.is_ok(), "put should work but got: {}", res.unwrap_err());
}

pub fn test_insert_into_canonical_chain(db: &impl L1Database) {
    let heights = vec![1, 2, 5000, 1000, 1002, 999];
    let mut blockids = Vec::new();
    for height in &heights {
        let mut arb = ArbitraryGenerator::new();
        let mf = AsmManifest::new(*height, arb.generate(), arb.generate(), vec![]);
        let blockid = *mf.blkid();
        db.put_block_data(mf, *height).unwrap();
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
    let start_height = 1;
    let end_height = 10;
    for h in start_height..=end_height {
        insert_block_data(h, db);
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
    let mf = insert_block_data(idx, db);

    // fetch non existent block
    let non_idx = 200;
    let observed_blockid = db
        .get_canonical_blockid_at_height(non_idx)
        .expect("Could not fetch from db");
    assert_eq!(observed_blockid, None);

    // fetch and check, existent block
    let observed_mf = db
        .get_block_manifest(*mf.blkid())
        .expect("Could not fetch from db");
    assert_eq!(observed_mf, Some(mf));
}

pub fn test_get_chain_tip(db: &impl L1Database) {
    assert_eq!(
        db.get_canonical_chain_tip().unwrap(),
        None,
        "chain tip of empty db should be unset"
    );

    // Insert some block data
    insert_block_data(1, db);
    assert!(matches!(
        db.get_canonical_chain_tip().unwrap(),
        Some((1, _))
    ));
    insert_block_data(2, db);
    assert!(matches!(
        db.get_canonical_chain_tip().unwrap(),
        Some((2, _))
    ));
}

pub fn test_get_blockid_invalid_range(db: &impl L1Database) {
    let _ = insert_block_data(1, db);
    let _ = insert_block_data(2, db);
    let _ = insert_block_data(3, db);

    let range = db.get_canonical_blockid_range(3, 1).unwrap();
    assert_eq!(range.len(), 0);
}

pub fn test_get_blockid_range(db: &impl L1Database) {
    let mf1 = insert_block_data(1, db);
    let mf2 = insert_block_data(2, db);
    let mf3 = insert_block_data(3, db);

    let range = db.get_canonical_blockid_range(1, 4).unwrap();
    assert_eq!(range.len(), 3);
    assert_eq!(range, vec![*mf1.blkid(), *mf2.blkid(), *mf3.blkid()]);
}

// Helper function to insert block data
fn insert_block_data(height: u64, db: &impl L1Database) -> AsmManifest {
    let mut arb = ArbitraryGenerator::new_with_size(1 << 12);

    let mf = AsmManifest::new(height, arb.generate(), arb.generate(), vec![]);

    // Insert block data
    let res = db.put_block_data(mf.clone(), height);
    assert!(res.is_ok(), "put should work but got: {}", res.unwrap_err());
    let res = db.set_canonical_chain_entry(height, *mf.blkid());
    assert!(res.is_ok(), "put should work but got: {}", res.unwrap_err());

    mf
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
        fn test_get_chain_tip() {
            let db = $setup_expr;
            $crate::l1_tests::test_get_chain_tip(&db);
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
    };
}
