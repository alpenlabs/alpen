use strata_db_types::traits::{BlockStatus, OLBlockDatabase};
use strata_identifiers::{Buf32, Buf64, OLBlockCommitment, OLBlockId};
use strata_ol_chain_types_new::{
    BlockFlags, OLBlock, OLBlockBody, OLBlockHeader, OLL1ManifestContainer, OLL1Update,
    OLTxSegment, SignedOLBlockHeader,
};

pub fn test_put_and_get_block_data(db: &impl OLBlockDatabase) {
    let block = get_mock_block_with_slot(0, 0);
    let block_id = block.header().compute_blkid();
    let slot = block.header().slot();
    let commitment = OLBlockCommitment::new(slot, block_id);

    db.put_block_data(commitment, block.clone())
        .expect("test: put block data");

    // Assert block was stored
    let retrieved_block = db
        .get_block_data(commitment)
        .expect("test: get block data")
        .unwrap();

    // Compare headers since OLBlock doesn't implement PartialEq
    assert_eq!(
        retrieved_block.header().compute_blkid(),
        block.header().compute_blkid()
    );
    assert_eq!(retrieved_block.header().slot(), block.header().slot());

    // Assert block status was set to `BlockStatus::Unchecked`
    let block_status = db
        .get_block_status(block_id)
        .expect("test: get block status")
        .unwrap();
    assert_eq!(block_status, BlockStatus::Unchecked);

    // Assert block height data was stored
    let block_ids = db
        .get_blocks_at_height(slot)
        .expect("test: get blocks at height");
    assert!(block_ids.contains(&block_id));
}

pub fn test_delete_block_data(db: &impl OLBlockDatabase) {
    let block = get_mock_block_with_slot(0, 0);
    let block_id = block.header().compute_blkid();
    let slot = block.header().slot();
    let commitment = OLBlockCommitment::new(slot, block_id);

    // Put block
    db.put_block_data(commitment, block.clone())
        .expect("test: put block data");

    // Verify it exists
    let retrieved = db
        .get_block_data(commitment)
        .expect("test: get block data")
        .unwrap();
    assert_eq!(
        retrieved.header().compute_blkid(),
        block.header().compute_blkid()
    );

    // Delete it
    db.del_block_data(commitment)
        .expect("test: delete block data");

    // Assert block is deleted from the db
    let deleted = db
        .get_block_data(commitment)
        .expect("test: get after delete");
    assert!(deleted.is_none());

    // Assert block status is deleted from the db
    let block_status = db
        .get_block_status(block_id)
        .expect("test: get block status after delete");
    assert!(block_status.is_none());

    // Assert block height data is deleted
    let block_ids = db
        .get_blocks_at_height(slot)
        .expect("test: get blocks at height after delete");
    assert!(!block_ids.contains(&block_id));
}

pub fn test_set_and_get_block_status(db: &impl OLBlockDatabase) {
    let block = get_mock_block_with_slot(0, 0);
    let block_id = block.header().compute_blkid();
    let slot = block.header().slot();
    let commitment = OLBlockCommitment::new(slot, block_id);

    db.put_block_data(commitment, block.clone())
        .expect("test: put block data");

    // Assert block status was set to `BlockStatus::Valid`
    db.set_block_status(block_id, BlockStatus::Valid)
        .expect("test: set block status");
    let block_status = db
        .get_block_status(block_id)
        .expect("test: get block status")
        .unwrap();
    assert_eq!(block_status, BlockStatus::Valid);

    // Assert block status was set to `BlockStatus::Invalid`
    db.set_block_status(block_id, BlockStatus::Invalid)
        .expect("test: set block status");
    let block_status = db
        .get_block_status(block_id)
        .expect("test: get block status")
        .unwrap();
    assert_eq!(block_status, BlockStatus::Invalid);

    // Assert block status was set to `BlockStatus::Unchecked`
    db.set_block_status(block_id, BlockStatus::Unchecked)
        .expect("test: set block status");
    let block_status = db
        .get_block_status(block_id)
        .expect("test: get block status")
        .unwrap();
    assert_eq!(block_status, BlockStatus::Unchecked);
}

pub fn test_get_blocks_at_height(db: &impl OLBlockDatabase) {
    let slot = 10u64;
    let block1 = get_mock_block_with_slot(slot, 1);
    let block_id1 = block1.header().compute_blkid();
    let commitment1 = OLBlockCommitment::new(slot, block_id1);

    let block2 = get_mock_block_with_slot(slot, 2);
    let block_id2 = block2.header().compute_blkid();
    let commitment2 = OLBlockCommitment::new(slot, block_id2);

    // Put two blocks at the same slot
    db.put_block_data(commitment1, block1)
        .expect("test: put block 1");
    db.put_block_data(commitment2, block2)
        .expect("test: put block 2");

    // Get blocks at height
    let block_ids = db
        .get_blocks_at_height(slot)
        .expect("test: get blocks at height");
    assert_eq!(block_ids.len(), 2);
    assert!(block_ids.contains(&block_id1));
    assert!(block_ids.contains(&block_id2));
}

pub fn test_get_tip_block(db: &impl OLBlockDatabase) {
    // Create blocks at different slots
    let block1 = get_mock_block_with_slot(5u64, 1);
    let block_id1 = block1.header().compute_blkid();
    let commitment1 = OLBlockCommitment::new(5u64, block_id1);

    let block2 = get_mock_block_with_slot(10u64, 2);
    let block_id2 = block2.header().compute_blkid();
    let commitment2 = OLBlockCommitment::new(10u64, block_id2);

    // Put blocks
    db.put_block_data(commitment1, block1)
        .expect("test: put block 1");
    db.put_block_data(commitment2, block2)
        .expect("test: put block 2");

    // Set block2 as valid (higher slot)
    db.set_block_status(block_id2, BlockStatus::Valid)
        .expect("test: set block 2 status");

    // Get tip block - should be block2 (highest valid slot)
    let tip = db.get_tip_block().expect("test: get tip block");
    assert_eq!(tip, block_id2);
}

// Helper function to create a minimal test block with a specific slot
// `id_byte` is used to create a unique body_root to ensure unique block IDs
fn get_mock_block_with_slot(slot: u64, id_byte: u8) -> OLBlock {
    let mut bytes = [0u8; 32];
    bytes[0..8].copy_from_slice(&slot.to_le_bytes());
    // Use id_byte to ensure unique block IDs for different calls
    bytes[8] = id_byte;
    let body_root = Buf32::from(bytes);

    let header = OLBlockHeader::new(
        0,                              // timestamp
        BlockFlags::from(0),            // flags
        slot,                           // slot
        0,                              // epoch
        OLBlockId::from(Buf32::zero()), // parent_blkid
        body_root,                      // body_root (unique per id_byte)
        Buf32::zero(),                  // state_root
        Buf32::zero(),                  // logs_root
    );
    let signed_header = SignedOLBlockHeader::new(header, Buf64::zero());
    let body = OLBlockBody {
        tx_segment: Some(OLTxSegment { txs: vec![].into() }).into(),
        l1_update: Some(OLL1Update {
            preseal_state_root: Buf32::zero(),
            manifest_cont: OLL1ManifestContainer::new(vec![])
                .expect("empty manifest should succeed"),
        })
        .into(),
    };
    OLBlock::new(signed_header, body)
}

#[macro_export]
macro_rules! ol_block_db_tests {
    ($setup_expr:expr) => {
        #[test]
        fn test_put_and_get_block_data() {
            let db = $setup_expr;
            $crate::ol_block_tests::test_put_and_get_block_data(&db);
        }

        #[test]
        fn test_delete_block_data() {
            let db = $setup_expr;
            $crate::ol_block_tests::test_delete_block_data(&db);
        }

        #[test]
        fn test_set_and_get_block_status() {
            let db = $setup_expr;
            $crate::ol_block_tests::test_set_and_get_block_status(&db);
        }

        #[test]
        fn test_get_blocks_at_height() {
            let db = $setup_expr;
            $crate::ol_block_tests::test_get_blocks_at_height(&db);
        }

        #[test]
        fn test_get_tip_block() {
            let db = $setup_expr;
            $crate::ol_block_tests::test_get_tip_block(&db);
        }
    };
}
