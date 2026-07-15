use strata_db_types::ol_block::{BlockAvailability, BlockStatus, OLBlockDatabase};
use strata_db_types::DbError;
use strata_identifiers::{Buf32, EpochCommitment, OLBlockCommitment, OLBlockId};
use strata_ol_chain_types::OLBlock;

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

pub fn test_get_empty_history_base(db: &impl OLBlockDatabase) {
    let history_base = db.get_history_base().expect("test: get empty history base");
    assert!(history_base.is_none());
}

fn canonical_id(byte: u8) -> OLBlockId {
    OLBlockId::from(Buf32::from([byte; 32]))
}

pub fn test_get_canonical_block_empty(db: &impl OLBlockDatabase) {
    let result = db
        .get_canonical_block(7)
        .expect("test: get canonical at empty slot");
    assert!(result.is_none());
    let err = db
        .get_tip_slot()
        .expect_err("test: empty canonical index has no tip slot");
    assert!(matches!(err, DbError::NotBootstrapped));
}

pub fn test_replace_canonical_suffix_from_extend(db: &impl OLBlockDatabase) {
    // Seed slots 0..=4, then extend by appending slot 5 with an empty truncation.
    let seed: Vec<OLBlockId> = (0..=4).map(|s| canonical_id(s as u8)).collect();
    db.replace_canonical_suffix_from(0, seed.clone())
        .expect("test: seed canonical");

    db.replace_canonical_suffix_from(5, vec![canonical_id(50)])
        .expect("test: extend canonical");

    for (slot, id) in seed.iter().enumerate() {
        assert_eq!(db.get_canonical_block(slot as u64).unwrap(), Some(*id));
    }
    assert_eq!(db.get_canonical_block(5).unwrap(), Some(canonical_id(50)));
}

pub fn test_replace_canonical_suffix_from_reorg_shorter(db: &impl OLBlockDatabase) {
    // Seed slots 0..=9, then reorg at pivot 5 onto a shorter branch (slots 6,7).
    let seed: Vec<OLBlockId> = (0..=9).map(|s| canonical_id(s as u8)).collect();
    db.replace_canonical_suffix_from(0, seed)
        .expect("test: seed canonical");

    let branch = vec![canonical_id(0x60), canonical_id(0x70)];
    db.replace_canonical_suffix_from(6, branch)
        .expect("test: reorg canonical");

    // Slots 0..=5 untouched.
    for s in 0..=5u64 {
        assert_eq!(
            db.get_canonical_block(s).unwrap(),
            Some(canonical_id(s as u8))
        );
    }
    // Slots 6,7 rewritten to the new branch.
    assert_eq!(db.get_canonical_block(6).unwrap(), Some(canonical_id(0x60)));
    assert_eq!(db.get_canonical_block(7).unwrap(), Some(canonical_id(0x70)));
    // Slots 8,9 from the abandoned branch are gone.
    assert_eq!(db.get_canonical_block(8).unwrap(), None);
    assert_eq!(db.get_canonical_block(9).unwrap(), None);
}

pub fn test_replace_canonical_suffix_from_revert_empty_branch(db: &impl OLBlockDatabase) {
    // Seed slots 0..=4, then revert to slot 2 with an empty branch.
    let seed: Vec<OLBlockId> = (0..=4).map(|s| canonical_id(s as u8)).collect();
    db.replace_canonical_suffix_from(0, seed)
        .expect("test: seed canonical");

    db.replace_canonical_suffix_from(3, Vec::new())
        .expect("test: revert canonical");

    for s in 0..=2u64 {
        assert_eq!(
            db.get_canonical_block(s).unwrap(),
            Some(canonical_id(s as u8))
        );
    }
    assert_eq!(db.get_canonical_block(3).unwrap(), None);
    assert_eq!(db.get_canonical_block(4).unwrap(), None);
    assert_eq!(db.get_tip_slot().unwrap(), 2);
}

pub fn test_replace_canonical_suffix_from_max(db: &impl OLBlockDatabase) {
    db.replace_canonical_suffix_from(u64::MAX, vec![canonical_id(0xaa)])
        .expect("test: seed canonical at max slot");

    db.replace_canonical_suffix_from(u64::MAX, vec![canonical_id(0xbb)])
        .expect("test: replace suffix from max slot");

    assert_eq!(
        db.get_canonical_block(u64::MAX).unwrap(),
        Some(canonical_id(0xbb))
    );
}

pub fn test_replace_canonical_suffix_from_overflow(db: &impl OLBlockDatabase) {
    let err = db
        .replace_canonical_suffix_from(u64::MAX, vec![canonical_id(0xaa), canonical_id(0xbb)])
        .expect_err("test: overflowing canonical suffix must fail");
    assert!(matches!(
        err,
        DbError::OLCanonicalSuffixOverflow {
            start_slot: u64::MAX,
            block_count: 2,
        }
    ));
}

pub fn test_replace_canonical_suffix_from_idempotent(db: &impl OLBlockDatabase) {
    let seed: Vec<OLBlockId> = (0..=4).map(|s| canonical_id(s as u8)).collect();
    db.replace_canonical_suffix_from(0, seed.clone())
        .expect("test: seed canonical");
    // Re-applying the same suffix is a no-op.
    db.replace_canonical_suffix_from(0, seed.clone())
        .expect("test: re-apply canonical");

    for (slot, id) in seed.iter().enumerate() {
        assert_eq!(db.get_canonical_block(slot as u64).unwrap(), Some(*id));
    }
    assert_eq!(db.get_tip_slot().unwrap(), 4);
}

pub fn test_promote_to_history_anchor_atomic_surface_and_idempotency(db: &impl OLBlockDatabase) {
    let seed: Vec<OLBlockId> = (0..=14).map(|slot| canonical_id(slot as u8)).collect();
    db.replace_canonical_suffix_from(0, seed.clone())
        .expect("test: seed canonical suffix");

    let anchor = EpochCommitment::new(3, 10, canonical_id(0xa0));
    db.promote_to_history_anchor(anchor)
        .expect("test: promote history anchor");

    assert_eq!(
        db.get_history_base().expect("test: get history base"),
        Some(anchor)
    );
    for (slot, id) in seed.iter().enumerate().take(10) {
        assert_eq!(
            db.get_canonical_block(slot as u64)
                .expect("test: get preserved canonical block"),
            Some(*id)
        );
    }
    assert_eq!(
        db.get_canonical_block(anchor.last_slot())
            .expect("test: get canonical anchor"),
        Some(*anchor.last_blkid())
    );
    for slot in 11..=14 {
        assert_eq!(
            db.get_canonical_block(slot)
                .expect("test: get removed canonical suffix"),
            None
        );
    }
    assert_eq!(db.get_tip_slot().expect("test: get promoted tip"), 10);

    db.promote_to_history_anchor(anchor)
        .expect("test: re-promote same history anchor");
    assert_eq!(
        db.get_history_base()
            .expect("test: get idempotent history base"),
        Some(anchor)
    );
    assert_eq!(
        db.get_canonical_block(anchor.last_slot())
            .expect("test: get idempotent canonical anchor"),
        Some(*anchor.last_blkid())
    );
}

pub fn test_promote_to_history_anchor_refuses_different_marker(db: &impl OLBlockDatabase) {
    let first = EpochCommitment::new(3, 10, canonical_id(0xa0));
    let attempted = EpochCommitment::new(4, 11, canonical_id(0xb0));
    db.promote_to_history_anchor(first)
        .expect("test: promote initial history anchor");

    let err = db
        .promote_to_history_anchor(attempted)
        .expect_err("test: reject different history anchor");
    assert!(matches!(
        err,
        DbError::OLHistoryBaseConflict {
            attempted: actual_attempted,
            current,
        } if actual_attempted == attempted && current == first
    ));
    assert_eq!(
        db.get_history_base().expect("test: get retained marker"),
        Some(first)
    );
    assert_eq!(
        db.get_canonical_block(first.last_slot())
            .expect("test: get retained canonical anchor"),
        Some(*first.last_blkid())
    );
    assert_eq!(
        db.get_canonical_block(attempted.last_slot())
            .expect("test: get rejected canonical anchor"),
        None
    );
}

pub fn test_delete_canonical_block_clears_canonical_index(
    db: &impl OLBlockDatabase,
    mut block: OLBlock,
) {
    block.signed_header.header.slot = 11;
    let block_id = block.header().compute_blkid();

    db.put_block_data(block).expect("test: put block");
    db.replace_canonical_suffix_from(11, vec![block_id])
        .expect("test: seed canonical");

    assert_eq!(db.get_tip_slot().expect("test: get tip slot"), 11);

    db.del_block_data(block_id).expect("test: delete block");

    assert_eq!(
        db.get_canonical_block(11)
            .expect("test: get deleted canonical slot"),
        None
    );
    let err = db
        .get_tip_slot()
        .expect_err("test: deleted canonical tip leaves no tip slot");
    assert!(matches!(err, DbError::NotBootstrapped));
}

pub fn test_delete_canonical_block_truncates_canonical_suffix(
    db: &impl OLBlockDatabase,
    mut block1: OLBlock,
    mut block2: OLBlock,
    mut block3: OLBlock,
) {
    block1.signed_header.header.slot = 10;
    block2.signed_header.header.slot = 11;
    block3.signed_header.header.slot = 12;
    let block1_id = block1.header().compute_blkid();
    let block2_id = block2.header().compute_blkid();
    let block3_id = block3.header().compute_blkid();

    db.put_block_data(block1).expect("test: put block 1");
    db.put_block_data(block2).expect("test: put block 2");
    db.put_block_data(block3).expect("test: put block 3");
    db.replace_canonical_suffix_from(10, vec![block1_id, block2_id, block3_id])
        .expect("test: seed canonical suffix");

    db.del_block_data(block2_id)
        .expect("test: delete middle canonical block");

    assert_eq!(
        db.get_canonical_block(10)
            .expect("test: get canonical slot 10"),
        Some(block1_id)
    );
    assert_eq!(
        db.get_canonical_block(11)
            .expect("test: get canonical slot 11"),
        None
    );
    assert_eq!(
        db.get_canonical_block(12)
            .expect("test: get canonical slot 12"),
        None
    );
    assert_eq!(db.get_tip_slot().expect("test: get truncated tip slot"), 10);
}

pub fn test_delete_noncanonical_block_preserves_canonical_index(
    db: &impl OLBlockDatabase,
    mut canonical: OLBlock,
    mut noncanonical: OLBlock,
) {
    canonical.signed_header.header.slot = 7;
    noncanonical.signed_header.header.slot = 7;
    let canonical_id = canonical.header().compute_blkid();
    let noncanonical_id = noncanonical.header().compute_blkid();
    if canonical_id == noncanonical_id {
        return;
    }

    db.put_block_data(canonical).expect("test: put canonical");
    db.put_block_data(noncanonical)
        .expect("test: put noncanonical");
    db.replace_canonical_suffix_from(7, vec![canonical_id])
        .expect("test: seed canonical");

    db.del_block_data(noncanonical_id)
        .expect("test: delete noncanonical block");

    assert_eq!(
        db.get_canonical_block(7).expect("test: get canonical slot"),
        Some(canonical_id)
    );
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

pub fn proptest_terminal_header_roundtrip_and_mismatch(db: &impl OLBlockDatabase, block: OLBlock) {
    let header = block.header().clone();
    let block_id = header.compute_blkid();

    db.put_terminal_header(block_id, header.clone())
        .expect("test: put terminal header");
    assert_eq!(
        db.get_terminal_header(block_id)
            .expect("test: get terminal header"),
        Some(header.clone())
    );

    let first_mismatch = canonical_id(0xfe);
    let mismatched_id = if first_mismatch == block_id {
        canonical_id(0xfd)
    } else {
        first_mismatch
    };
    let err = db
        .put_terminal_header(mismatched_id, header)
        .expect_err("test: reject mismatched terminal header");
    assert!(matches!(
        err,
        DbError::OLTerminalHeaderIdMismatch { key, computed }
            if key == mismatched_id && computed == block_id
    ));
    assert!(db
        .get_terminal_header(mismatched_id)
        .expect("test: get rejected terminal header")
        .is_none());
}

pub fn proptest_block_availability_with_history_base(db: &impl OLBlockDatabase, block: OLBlock) {
    let mut above = block.clone();
    above.signed_header.header.slot = 11;
    above.signed_header.header.timestamp = 11;
    let above_commitment = above.header().compute_block_commitment();

    let mut below = block.clone();
    below.signed_header.header.slot = 5;
    below.signed_header.header.timestamp = 5;
    let below_commitment = below.header().compute_block_commitment();

    let mut genesis = block.clone();
    genesis.signed_header.header.slot = 0;
    genesis.signed_header.header.timestamp = 0;
    let genesis_commitment = genesis.header().compute_block_commitment();

    db.put_block_data(above)
        .expect("test: put block above base");
    db.put_block_data(below)
        .expect("test: put block below base");
    db.put_block_data(genesis).expect("test: put genesis block");

    let mut anchor_header = block.clone();
    anchor_header.signed_header.header.slot = 10;
    anchor_header.signed_header.header.timestamp = 10;
    let anchor = EpochCommitment::new(1, 10, anchor_header.header().compute_blkid());
    db.promote_to_history_anchor(anchor)
        .expect("test: promote history base");

    assert!(matches!(
        db.get_block_at(above_commitment)
            .expect("test: get available block above base"),
        BlockAvailability::Available(block) if block.header().slot() == 11
    ));
    assert!(matches!(
        db.get_block_at(below_commitment)
            .expect("test: get available block below base"),
        BlockAvailability::Available(block) if block.header().slot() == 5
    ));
    assert!(matches!(
        db.get_block_at(genesis_commitment)
            .expect("test: get available genesis block"),
        BlockAvailability::Available(block) if block.header().is_genesis_slot()
    ));

    let mut absent_below = block.clone();
    absent_below.signed_header.header.slot = 9;
    absent_below.signed_header.header.timestamp = 9;
    assert!(matches!(
        db.get_block_at(absent_below.header().compute_block_commitment())
            .expect("test: classify absent block below base"),
        BlockAvailability::Pruned
    ));

    let mut absent_above = block;
    absent_above.signed_header.header.slot = 12;
    absent_above.signed_header.header.timestamp = 12;
    assert!(matches!(
        db.get_block_at(absent_above.header().compute_block_commitment())
            .expect("test: classify absent block above base"),
        BlockAvailability::Missing
    ));
}

pub fn proptest_block_availability_without_history_base(db: &impl OLBlockDatabase, block: OLBlock) {
    let mut genesis = block.clone();
    genesis.signed_header.header.slot = 0;
    genesis.signed_header.header.timestamp = 0;
    let genesis_commitment = genesis.header().compute_block_commitment();
    db.put_block_data(genesis)
        .expect("test: put genesis without history base");

    assert!(matches!(
        db.get_block_at(genesis_commitment)
            .expect("test: get genesis without history base"),
        BlockAvailability::Available(block) if block.header().is_genesis_slot()
    ));

    let mut absent_genesis = block.clone();
    absent_genesis.signed_header.header.slot = 0;
    absent_genesis.signed_header.header.timestamp = 1;
    assert!(matches!(
        db.get_block_at(absent_genesis.header().compute_block_commitment())
            .expect("test: classify absent genesis without history base"),
        BlockAvailability::Missing
    ));

    let mut absent_later = block;
    absent_later.signed_header.header.slot = 100;
    absent_later.signed_header.header.timestamp = 100;
    assert!(matches!(
        db.get_block_at(absent_later.header().compute_block_commitment())
            .expect("test: classify absent later block without history base"),
        BlockAvailability::Missing
    ));
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

pub fn proptest_rollback_block_high_watermark(
    db: &impl OLBlockDatabase,
    mut block1: OLBlock,
    mut block2: OLBlock,
) {
    block1.signed_header.header.slot = 10;
    block2.signed_header.header.slot = 11;

    let block1_commitment = db
        .put_block_data_with_high_watermark(block1)
        .expect("test: put target block with high-watermark");
    let block2_commitment = db
        .put_block_data_with_high_watermark(block2)
        .expect("test: put later block with high-watermark");

    let rolled_back = db
        .rollback_block_high_watermark(block1_commitment)
        .expect("test: roll back block high-watermark");
    assert!(rolled_back);
    assert_eq!(
        db.get_block_high_watermark()
            .expect("test: get rolled-back block high-watermark"),
        Some(block1_commitment)
    );

    let unchanged = db
        .rollback_block_high_watermark(block2_commitment)
        .expect("test: non-rollback target above current high-watermark");
    assert!(!unchanged);
    assert_eq!(
        db.get_block_high_watermark()
            .expect("test: get unchanged block high-watermark"),
        Some(block1_commitment)
    );
}

pub fn proptest_rollback_block_high_watermark_missing_target(
    db: &impl OLBlockDatabase,
    mut block: OLBlock,
) {
    block.signed_header.header.slot = 10;
    db.put_block_data_with_high_watermark(block)
        .expect("test: put block with high-watermark");

    let missing_target = OLBlockCommitment::new(9, OLBlockId::from(Buf32::from([0xeeu8; 32])));
    let err = db
        .rollback_block_high_watermark(missing_target)
        .expect_err("test: missing rollback target should fail");
    assert!(matches!(err, DbError::NonExistentEntry));
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

    let block_id1 = block1.header().compute_blkid();
    let block_id2 = block2.header().compute_blkid();

    // Put blocks
    db.put_block_data(block1).expect("test: put block 1");
    db.put_block_data(block2).expect("test: put block 2");

    // Set block2 as valid, but keep the canonical index at block1.
    db.set_block_status(block_id1, BlockStatus::Valid)
        .expect("test: set block 1 status");
    db.set_block_status(block_id2, BlockStatus::Valid)
        .expect("test: set block 2 status");
    db.replace_canonical_suffix_from(5, vec![block_id1])
        .expect("test: seed canonical index");

    // Get tip slot - should be 5 (highest canonical slot), not the higher valid fork.
    let tip_slot = db.get_tip_slot().expect("test: get tip slot");
    assert_eq!(tip_slot, 5u64);
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

        #[test]
        fn test_get_empty_history_base() {
            let db = $setup_expr;
            $crate::ol_block_tests::test_get_empty_history_base(&db);
        }

        #[test]
        fn test_get_canonical_block_empty() {
            let db = $setup_expr;
            $crate::ol_block_tests::test_get_canonical_block_empty(&db);
        }

        #[test]
        fn test_replace_canonical_suffix_from_extend() {
            let db = $setup_expr;
            $crate::ol_block_tests::test_replace_canonical_suffix_from_extend(&db);
        }

        #[test]
        fn test_replace_canonical_suffix_from_reorg_shorter() {
            let db = $setup_expr;
            $crate::ol_block_tests::test_replace_canonical_suffix_from_reorg_shorter(&db);
        }

        #[test]
        fn test_replace_canonical_suffix_from_revert_empty_branch() {
            let db = $setup_expr;
            $crate::ol_block_tests::test_replace_canonical_suffix_from_revert_empty_branch(&db);
        }

        #[test]
        fn test_replace_canonical_suffix_from_max() {
            let db = $setup_expr;
            $crate::ol_block_tests::test_replace_canonical_suffix_from_max(&db);
        }

        #[test]
        fn test_replace_canonical_suffix_from_overflow() {
            let db = $setup_expr;
            $crate::ol_block_tests::test_replace_canonical_suffix_from_overflow(&db);
        }

        #[test]
        fn test_replace_canonical_suffix_from_idempotent() {
            let db = $setup_expr;
            $crate::ol_block_tests::test_replace_canonical_suffix_from_idempotent(&db);
        }

        #[test]
        fn test_promote_to_history_anchor_atomic_surface_and_idempotency() {
            let db = $setup_expr;
            $crate::ol_block_tests::test_promote_to_history_anchor_atomic_surface_and_idempotency(&db);
        }

        #[test]
        fn test_promote_to_history_anchor_refuses_different_marker() {
            let db = $setup_expr;
            $crate::ol_block_tests::test_promote_to_history_anchor_refuses_different_marker(&db);
        }

        proptest::proptest! {
            #[test]
            fn test_delete_canonical_block_clears_canonical_index(block in ol_test_utils::ol_block_strategy()) {
                let db = $setup_expr;
                $crate::ol_block_tests::test_delete_canonical_block_clears_canonical_index(&db, block);
            }

            #[test]
            fn test_delete_canonical_block_truncates_canonical_suffix(
                block1 in ol_test_utils::ol_block_strategy(),
                block2 in ol_test_utils::ol_block_strategy(),
                block3 in ol_test_utils::ol_block_strategy(),
            ) {
                let db = $setup_expr;
                $crate::ol_block_tests::test_delete_canonical_block_truncates_canonical_suffix(
                    &db, block1, block2, block3,
                );
            }

            #[test]
            fn test_delete_noncanonical_block_preserves_canonical_index(
                canonical in ol_test_utils::ol_block_strategy(),
                noncanonical in ol_test_utils::ol_block_strategy(),
            ) {
                let db = $setup_expr;
                $crate::ol_block_tests::test_delete_noncanonical_block_preserves_canonical_index(
                    &db, canonical, noncanonical,
                );
            }

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
            fn proptest_terminal_header_roundtrip_and_mismatch(block in ol_test_utils::ol_block_strategy()) {
                let db = $setup_expr;
                $crate::ol_block_tests::proptest_terminal_header_roundtrip_and_mismatch(&db, block);
            }

            #[test]
            fn proptest_block_availability_with_history_base(block in ol_test_utils::ol_block_strategy()) {
                let db = $setup_expr;
                $crate::ol_block_tests::proptest_block_availability_with_history_base(&db, block);
            }

            #[test]
            fn proptest_block_availability_without_history_base(block in ol_test_utils::ol_block_strategy()) {
                let db = $setup_expr;
                $crate::ol_block_tests::proptest_block_availability_without_history_base(&db, block);
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
            fn proptest_rollback_block_high_watermark(
                block1 in ol_test_utils::ol_block_strategy(),
                block2 in ol_test_utils::ol_block_strategy()
            ) {
                let db = $setup_expr;
                $crate::ol_block_tests::proptest_rollback_block_high_watermark(&db, block1, block2);
            }

            #[test]
            fn proptest_rollback_block_high_watermark_missing_target(block in ol_test_utils::ol_block_strategy()) {
                let db = $setup_expr;
                $crate::ol_block_tests::proptest_rollback_block_high_watermark_missing_target(&db, block);
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
