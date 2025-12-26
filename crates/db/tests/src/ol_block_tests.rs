use strata_db_types::traits::{BlockStatus, OLBlockDatabase};
use strata_identifiers::{Buf32, OLBlockId};
use strata_ol_chain_types_new::OLBlock;

pub fn test_get_nonexistent_block(db: &impl OLBlockDatabase) {
    let nonexistent_id = OLBlockId::from(Buf32::from([0xffu8; 32]));

    let result = db
        .get_block_data(nonexistent_id)
        .expect("test: get nonexistent block");
    assert!(result.is_none());
}

pub fn test_delete_nonexistent_block(db: &impl OLBlockDatabase) {
    let nonexistent_id = OLBlockId::from(Buf32::from([0xffu8; 32]));

    let existed = db
        .del_block_data(nonexistent_id)
        .expect("test: delete nonexistent block");
    assert!(!existed);
}

pub fn test_set_status_nonexistent_block(db: &impl OLBlockDatabase) {
    let nonexistent_id = OLBlockId::from(Buf32::from([0xffu8; 32]));

    let result = db.set_block_status(nonexistent_id, BlockStatus::Valid);
    assert!(result.is_err());
}

pub fn test_get_status_nonexistent_block(db: &impl OLBlockDatabase) {
    let nonexistent_id = OLBlockId::from(Buf32::from([0xffu8; 32]));

    let status = db
        .get_block_status(nonexistent_id)
        .expect("test: get status of nonexistent block");
    assert!(status.is_none());
}

pub fn test_get_blocks_at_empty_height(db: &impl OLBlockDatabase) {
    let empty_slot = 999u64;

    let block_ids = db
        .get_blocks_at_height(empty_slot)
        .expect("test: get blocks at empty height");
    assert!(block_ids.is_empty());
}

// Proptest-based tests for random block data
pub fn proptest_put_and_get_random_block(db: &impl OLBlockDatabase, block: OLBlock) {
    let block_id = block.header().compute_blkid();

    db.put_block_data(block.clone())
        .expect("test: put random block");

    let retrieved = db
        .get_block_data(block_id)
        .expect("test: get random block")
        .unwrap();

    assert_eq!(
        retrieved.header().compute_blkid(),
        block.header().compute_blkid()
    );
}

pub fn proptest_put_twice_idempotent(db: &impl OLBlockDatabase, block: OLBlock) {
    let block_id = block.header().compute_blkid();
    let slot = block.header().slot();

    db.put_block_data(block.clone())
        .expect("test: put block first time");
    db.put_block_data(block.clone())
        .expect("test: put block second time");

    let blocks = db
        .get_blocks_at_height(slot)
        .expect("test: get blocks at height");
    assert_eq!(blocks.len(), 1);
    assert!(blocks.contains(&block_id));
}

pub fn proptest_delete_random_block(db: &impl OLBlockDatabase, block: OLBlock) {
    let block_id = block.header().compute_blkid();

    db.put_block_data(block.clone())
        .expect("test: put random block");

    let existed = db
        .del_block_data(block_id)
        .expect("test: delete random block");
    assert!(existed);

    let deleted = db.get_block_data(block_id).expect("test: get after delete");
    assert!(deleted.is_none());
}

pub fn proptest_status_transitions(db: &impl OLBlockDatabase, block: OLBlock) {
    let block_id = block.header().compute_blkid();

    db.put_block_data(block.clone())
        .expect("test: put random block");

    // Initially Unchecked
    let status = db
        .get_block_status(block_id)
        .expect("test: get initial status")
        .unwrap();
    assert_eq!(status, BlockStatus::Unchecked);

    // Set to Valid
    db.set_block_status(block_id, BlockStatus::Valid)
        .expect("test: set to valid");
    let status = db
        .get_block_status(block_id)
        .expect("test: get valid status")
        .unwrap();
    assert_eq!(status, BlockStatus::Valid);

    // Set to Invalid
    db.set_block_status(block_id, BlockStatus::Invalid)
        .expect("test: set to invalid");
    let status = db
        .get_block_status(block_id)
        .expect("test: get invalid status")
        .unwrap();
    assert_eq!(status, BlockStatus::Invalid);
}

pub fn proptest_get_blocks_at_height(
    db: &impl OLBlockDatabase,
    mut block1: OLBlock,
    mut block2: OLBlock,
) {
    let slot = 10u64;

    // Override both blocks to same slot
    block1.signed_header.header.slot = slot;
    block2.signed_header.header.slot = slot;

    let block_id1 = block1.header().compute_blkid();
    let block_id2 = block2.header().compute_blkid();

    // Put two blocks at the same slot
    db.put_block_data(block1).expect("test: put block 1");
    db.put_block_data(block2).expect("test: put block 2");

    // Get blocks at height
    let block_ids = db
        .get_blocks_at_height(slot)
        .expect("test: get blocks at height");
    assert_eq!(block_ids.len(), 2);
    assert!(block_ids.contains(&block_id1));
    assert!(block_ids.contains(&block_id2));
}

pub fn proptest_get_tip_slot(db: &impl OLBlockDatabase, mut block1: OLBlock, mut block2: OLBlock) {
    // Override to different slots
    block1.signed_header.header.slot = 5u64;
    block2.signed_header.header.slot = 10u64;

    let block_id2 = block2.header().compute_blkid();

    // Put blocks
    db.put_block_data(block1).expect("test: put block 1");
    db.put_block_data(block2).expect("test: put block 2");

    // Set block2 as valid (higher slot)
    db.set_block_status(block_id2, BlockStatus::Valid)
        .expect("test: set block 2 status");

    // Get tip slot - should be 10 (highest valid slot)
    let tip_slot = db.get_tip_slot().expect("test: get tip slot");
    assert_eq!(tip_slot, 10u64);
}

#[macro_export]
macro_rules! ol_block_db_tests {
    ($setup_expr:expr) => {
        #[test]
        fn test_get_nonexistent_block() {
            let db = $setup_expr;
            $crate::ol_block_tests::test_get_nonexistent_block(&db);
        }

        #[test]
        fn test_delete_nonexistent_block() {
            let db = $setup_expr;
            $crate::ol_block_tests::test_delete_nonexistent_block(&db);
        }

        #[test]
        fn test_set_status_nonexistent_block() {
            let db = $setup_expr;
            $crate::ol_block_tests::test_set_status_nonexistent_block(&db);
        }

        #[test]
        fn test_get_status_nonexistent_block() {
            let db = $setup_expr;
            $crate::ol_block_tests::test_get_status_nonexistent_block(&db);
        }

        #[test]
        fn test_get_blocks_at_empty_height() {
            let db = $setup_expr;
            $crate::ol_block_tests::test_get_blocks_at_empty_height(&db);
        }

        proptest::proptest! {
            #[test]
            fn proptest_put_and_get_random_block(block in ol_test_utils::ol_block_strategy()) {
                let db = $setup_expr;
                $crate::ol_block_tests::proptest_put_and_get_random_block(&db, block);
            }

            #[test]
            fn proptest_put_twice_idempotent(block in ol_test_utils::ol_block_strategy()) {
                let db = $setup_expr;
                $crate::ol_block_tests::proptest_put_twice_idempotent(&db, block);
            }

            #[test]
            fn proptest_delete_random_block(block in ol_test_utils::ol_block_strategy()) {
                let db = $setup_expr;
                $crate::ol_block_tests::proptest_delete_random_block(&db, block);
            }

            #[test]
            fn proptest_status_transitions(block in ol_test_utils::ol_block_strategy()) {
                let db = $setup_expr;
                $crate::ol_block_tests::proptest_status_transitions(&db, block);
            }

            #[test]
            fn proptest_get_blocks_at_height(block1 in ol_test_utils::ol_block_strategy(), block2 in ol_test_utils::ol_block_strategy()) {
                let db = $setup_expr;
                $crate::ol_block_tests::proptest_get_blocks_at_height(&db, block1, block2);
            }

            #[test]
            fn proptest_get_tip_slot(block1 in ol_test_utils::ol_block_strategy(), block2 in ol_test_utils::ol_block_strategy()) {
                let db = $setup_expr;
                $crate::ol_block_tests::proptest_get_tip_slot(&db, block1, block2);
            }
        }
    };
}
