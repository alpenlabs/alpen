use strata_db_types::{
    traits::{BlockStatus, OLBlockDatabase},
    DbError,
};
use strata_identifiers::{Buf32, OLBlockCommitment, OLBlockId};
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

pub fn test_get_empty_block_high_watermark(db: &impl OLBlockDatabase) {
    let high_watermark = db
        .get_block_high_watermark()
        .expect("test: get block high-watermark");
    assert!(high_watermark.is_none());
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

pub fn proptest_put_block_data_does_not_advance_high_watermark(
    db: &impl OLBlockDatabase,
    block: OLBlock,
) {
    db.put_block_data(block)
        .expect("test: put block without high-watermark");

    let high_watermark = db
        .get_block_high_watermark()
        .expect("test: get block high-watermark");
    assert!(high_watermark.is_none());
}

pub fn proptest_put_block_data_with_high_watermark(
    db: &impl OLBlockDatabase,
    mut block1: OLBlock,
    mut block2: OLBlock,
) {
    let slot = 10u64;
    block1.signed_header.header.slot = slot;
    block1.signed_header.header.timestamp = 1;
    block2.signed_header.header.slot = slot;
    block2.signed_header.header.timestamp = 2;

    let block1_id = block1.header().compute_blkid();
    let block2_id = block2.header().compute_blkid();
    let block1_commitment = OLBlockCommitment::new(slot, block1_id);

    let applied = db
        .put_block_data_with_high_watermark(block1.clone())
        .expect("test: put block with high-watermark");
    assert_eq!(applied, block1_commitment);
    assert_eq!(
        db.get_block_high_watermark()
            .expect("test: get block high-watermark"),
        Some(block1_commitment)
    );
    assert_eq!(
        db.get_block_status(block1_id)
            .expect("test: get block status"),
        Some(BlockStatus::Unchecked)
    );

    let err = db
        .put_block_data_with_high_watermark(block2.clone())
        .expect_err("test: same-slot block should not advance high-watermark");
    match err {
        DbError::BlockHighWatermarkConflict { attempted, current } => {
            assert_eq!(attempted, OLBlockCommitment::new(slot, block2_id));
            assert_eq!(current, block1_commitment);
        }
        other => panic!("unexpected error: {other:?}"),
    }

    assert!(db
        .get_block_data(block2_id)
        .expect("test: get rejected block")
        .is_none());
    assert_eq!(
        db.get_blocks_at_height(slot)
            .expect("test: get blocks at rejected slot"),
        vec![block1_id]
    );

    let next_slot = slot + 1;
    block2.signed_header.header.slot = next_slot;
    let block2_next_id = block2.header().compute_blkid();
    let block2_next_commitment = OLBlockCommitment::new(next_slot, block2_next_id);

    let applied = db
        .put_block_data_with_high_watermark(block2)
        .expect("test: put next-slot block with high-watermark");
    assert_eq!(applied, block2_next_commitment);
    assert_eq!(
        db.get_block_high_watermark()
            .expect("test: get advanced block high-watermark"),
        Some(block2_next_commitment)
    );
}

pub fn proptest_clear_block_high_watermark(
    db: &impl OLBlockDatabase,
    mut block1: OLBlock,
    mut block2: OLBlock,
) {
    let slot = 10u64;
    block1.signed_header.header.slot = slot;
    block1.signed_header.header.timestamp = 1;
    block2.signed_header.header.slot = slot;
    block2.signed_header.header.timestamp = 2;

    let block1_id = block1.header().compute_blkid();
    let block2_id = block2.header().compute_blkid();
    let block1_commitment = OLBlockCommitment::new(slot, block1_id);
    let block2_commitment = OLBlockCommitment::new(slot, block2_id);

    let cleared = db
        .clear_block_high_watermark(block1_commitment)
        .expect("test: clear empty block high-watermark");
    assert!(!cleared);

    db.put_block_data_with_high_watermark(block1.clone())
        .expect("test: put block with high-watermark");

    let cleared = db
        .clear_block_high_watermark(block2_commitment)
        .expect("test: clear mismatched block high-watermark");
    assert!(!cleared);
    assert_eq!(
        db.get_block_high_watermark()
            .expect("test: get block high-watermark"),
        Some(block1_commitment)
    );

    let cleared = db
        .clear_block_high_watermark(block1_commitment)
        .expect("test: clear current block high-watermark");
    assert!(cleared);
    assert_eq!(
        db.get_block_high_watermark()
            .expect("test: get cleared block high-watermark"),
        None
    );
    assert_eq!(
        db.get_block_data(block1_id)
            .expect("test: get block after high-watermark clear")
            .map(|block| block.header().compute_blkid()),
        Some(block1_id)
    );
    assert_eq!(
        db.get_block_status(block1_id)
            .expect("test: get block status after high-watermark clear"),
        Some(BlockStatus::Unchecked)
    );
    assert_eq!(
        db.get_blocks_at_height(slot)
            .expect("test: get blocks at slot after high-watermark clear"),
        vec![block1_id]
    );

    db.put_block_data_with_high_watermark(block2)
        .expect("test: put same-slot replacement after high-watermark clear");
    assert_eq!(
        db.get_block_high_watermark()
            .expect("test: get replacement block high-watermark"),
        Some(block2_commitment)
    );
    assert_eq!(
        db.get_blocks_at_height(slot)
            .expect("test: get blocks at replacement slot")
            .len(),
        2
    );
}

pub fn proptest_high_watermark_monotonic_under_mixed_puts(
    db: &impl OLBlockDatabase,
    ops: Vec<(u8, OLBlock)>,
) {
    let mut expected_high_watermark: Option<OLBlockCommitment> = None;

    for (op_selector, block) in ops {
        let use_high_watermark = op_selector % 2 == 0;
        let slot = block.header().slot();
        let block_id = block.header().compute_blkid();
        let commitment = OLBlockCommitment::new(slot, block_id);

        if use_high_watermark {
            match expected_high_watermark {
                Some(expected_current) if slot <= expected_current.slot() => {
                    let err = db
                        .put_block_data_with_high_watermark(block)
                        .expect_err("test: non-advancing guarded put should fail");
                    match err {
                        DbError::BlockHighWatermarkConflict { attempted, current } => {
                            assert_eq!(attempted, commitment);
                            assert_eq!(current, expected_current);
                        }
                        other => panic!("unexpected error: {other:?}"),
                    }
                }
                _ => {
                    let applied = db
                        .put_block_data_with_high_watermark(block)
                        .expect("test: advancing guarded put should succeed");
                    assert_eq!(applied, commitment);
                    expected_high_watermark = Some(commitment);
                }
            }
        } else {
            db.put_block_data(block)
                .expect("test: unguarded put should succeed");
        }

        assert_eq!(
            db.get_block_high_watermark()
                .expect("test: get block high-watermark"),
            expected_high_watermark,
            "high-watermark should equal the max successful guarded slot"
        );
    }
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

        #[test]
        fn test_get_empty_block_high_watermark() {
            let db = $setup_expr;
            $crate::ol_block_tests::test_get_empty_block_high_watermark(&db);
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
            fn proptest_put_block_data_does_not_advance_high_watermark(block in ol_test_utils::ol_block_strategy()) {
                let db = $setup_expr;
                $crate::ol_block_tests::proptest_put_block_data_does_not_advance_high_watermark(&db, block);
            }

            #[test]
            fn proptest_put_block_data_with_high_watermark(block1 in ol_test_utils::ol_block_strategy(), block2 in ol_test_utils::ol_block_strategy()) {
                let db = $setup_expr;
                $crate::ol_block_tests::proptest_put_block_data_with_high_watermark(&db, block1, block2);
            }

            #[test]
            fn proptest_clear_block_high_watermark(block1 in ol_test_utils::ol_block_strategy(), block2 in ol_test_utils::ol_block_strategy()) {
                let db = $setup_expr;
                $crate::ol_block_tests::proptest_clear_block_high_watermark(&db, block1, block2);
            }

            #[test]
            fn proptest_high_watermark_monotonic_under_mixed_puts(
                ops in proptest::collection::vec((0u8..=1, ol_test_utils::ol_block_strategy()), 0..32)
            ) {
                let db = $setup_expr;
                $crate::ol_block_tests::proptest_high_watermark_monotonic_under_mixed_puts(&db, ops);
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
