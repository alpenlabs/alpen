use strata_db::traits::{L2BlockDatabase, BlockStatus};
use strata_state::{block::L2BlockBundle, prelude::*};
use strata_test_utils::ArbitraryGenerator;

pub fn test_set_and_get_block_data(db: &impl L2BlockDatabase) {
    let bundle = get_mock_data();
    let block_hash = bundle.block().header().get_blockid();
    let block_height = bundle.block().header().slot();

    db.put_block_data(bundle.clone())
        .expect("failed to put block data");

    // assert block was stored
    let received_block = db
        .get_block_data(block_hash)
        .expect("failed to retrieve block data")
        .unwrap();
    assert_eq!(received_block, bundle);

    // assert block status was set to `BlockStatus::Unchecked``
    let block_status = db
        .get_block_status(block_hash)
        .expect("failed to retrieve block data")
        .unwrap();
    assert_eq!(block_status, BlockStatus::Unchecked);

    // assert block height data was stored
    let block_ids = db
        .get_blocks_at_height(block_height)
        .expect("failed to retrieve block data");
    assert!(block_ids.contains(&block_hash))
}

pub fn test_del_and_get_block_data(db: &impl L2BlockDatabase) {
    let bundle = get_mock_data();
    let block_hash = bundle.block().header().get_blockid();
    let block_height = bundle.block().header().slot();

    // deleting non existing block should return false
    let res = db
        .del_block_data(block_hash)
        .expect("failed to remove the block");
    assert!(!res);

    // deleting existing block should return true
    db.put_block_data(bundle.clone())
        .expect("failed to put block data");
    let res = db
        .del_block_data(block_hash)
        .expect("failed to remove the block");
    assert!(res);

    // assert block is deleted from the db
    let received_block = db
        .get_block_data(block_hash)
        .expect("failed to retrieve block data");
    assert!(received_block.is_none());

    // assert block status is deleted from the db
    let block_status = db
        .get_block_status(block_hash)
        .expect("failed to retrieve block status");
    assert!(block_status.is_none());

    // assert block height data is deleted
    let block_ids = db
        .get_blocks_at_height(block_height)
        .expect("failed to retrieve block data");
    assert!(!block_ids.contains(&block_hash))
}

pub fn test_set_and_get_block_status(db: &impl L2BlockDatabase) {
    let bundle = get_mock_data();
    let block_hash = bundle.block().header().get_blockid();

    db.put_block_data(bundle.clone())
        .expect("failed to put block data");

    // assert block status was set to `BlockStatus::Valid``
    db.set_block_status(block_hash, BlockStatus::Valid)
        .expect("failed to update block status");
    let block_status = db
        .get_block_status(block_hash)
        .expect("failed to retrieve block status")
        .unwrap();
    assert_eq!(block_status, BlockStatus::Valid);

    // assert block status was set to `BlockStatus::Invalid``
    db.set_block_status(block_hash, BlockStatus::Invalid)
        .expect("failed to update block status");
    let block_status = db
        .get_block_status(block_hash)
        .expect("failed to retrieve block status")
        .unwrap();
    assert_eq!(block_status, BlockStatus::Invalid);

    // assert block status was set to `BlockStatus::Unchecked``
    db.set_block_status(block_hash, BlockStatus::Unchecked)
        .expect("failed to update block status");
    let block_status = db
        .get_block_status(block_hash)
        .expect("failed to retrieve block status")
        .unwrap();
    assert_eq!(block_status, BlockStatus::Unchecked);
}

// Helper function to generate mock data
fn get_mock_data() -> L2BlockBundle {
    let mut arb = ArbitraryGenerator::new_with_size(1 << 14);
    let l2_block: L2BlockBundle = arb.generate();
    l2_block
}

#[macro_export]
macro_rules! l2_db_tests {
    ($setup_expr:expr) => {
        #[test]
        fn set_and_get_block_data() {
            let db = $setup_expr;
            $crate::l2_tests::test_set_and_get_block_data(&db);
        }

        #[test]
        fn del_and_get_block_data() {
            let db = $setup_expr;
            $crate::l2_tests::test_del_and_get_block_data(&db);
        }

        #[test]
        fn set_and_get_block_status() {
            let db = $setup_expr;
            $crate::l2_tests::test_set_and_get_block_status(&db);
        }
    };
}