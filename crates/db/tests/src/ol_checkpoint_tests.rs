use strata_checkpoint_types::EpochSummary;
use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_db_types::{
    traits::OLCheckpointDatabase,
    types::{OLCheckpointEntry, OLCheckpointStatus},
};
use strata_identifiers::Epoch;
use strata_test_utils::ArbitraryGenerator;

pub fn test_get_nonexistent_checkpoint(db: &impl OLCheckpointDatabase) {
    let nonexistent_epoch = Epoch::from(999u32);

    let result = db
        .get_checkpoint(nonexistent_epoch)
        .expect("test: get nonexistent checkpoint");
    assert!(result.is_none());
}

pub fn test_insert_summary_single(db: &impl OLCheckpointDatabase) {
    let summary: EpochSummary = ArbitraryGenerator::new().generate();
    let commitment = summary.get_epoch_commitment();
    db.insert_epoch_summary(summary).expect("test: insert");

    let stored = db
        .get_epoch_summary(commitment)
        .expect("test: get")
        .expect("test: get missing");
    assert_eq!(stored, summary);

    let commitments = db
        .get_epoch_commitments_at(commitment.epoch() as u64)
        .expect("test: get at epoch");

    assert_eq!(commitments.as_slice(), &[commitment]);
}

pub fn test_insert_summary_overwrite(db: &impl OLCheckpointDatabase) {
    let summary: EpochSummary = ArbitraryGenerator::new().generate();
    db.insert_epoch_summary(summary).expect("test: insert");
    db.insert_epoch_summary(summary)
        .expect_err("test: passed unexpectedly");
}

pub fn test_insert_summary_multiple(db: &impl OLCheckpointDatabase) {
    let mut ag = ArbitraryGenerator::new();
    let summary1: EpochSummary = ag.generate();
    let epoch = summary1.epoch();
    let summary2 = EpochSummary::new(
        epoch,
        ag.generate(),
        ag.generate(),
        ag.generate(),
        ag.generate(),
    );

    let commitment1 = summary1.get_epoch_commitment();
    let commitment2 = summary2.get_epoch_commitment();
    db.insert_epoch_summary(summary1).expect("test: insert");
    db.insert_epoch_summary(summary2).expect("test: insert");

    let stored1 = db
        .get_epoch_summary(commitment1)
        .expect("test: get")
        .expect("test: get missing");
    assert_eq!(stored1, summary1);

    let stored2 = db
        .get_epoch_summary(commitment2)
        .expect("test: get")
        .expect("test: get missing");
    assert_eq!(stored2, summary2);

    let mut commitments = vec![commitment1, commitment2];
    commitments.sort();

    let mut stored_commitments = db
        .get_epoch_commitments_at(epoch as u64)
        .expect("test: get at epoch");
    stored_commitments.sort();

    assert_eq!(stored_commitments, commitments);
}

pub fn test_delete_nonexistent_checkpoint(db: &impl OLCheckpointDatabase) {
    let nonexistent_epoch = Epoch::from(999u32);

    let existed = db
        .del_checkpoint(nonexistent_epoch)
        .expect("test: delete nonexistent checkpoint");
    assert!(!existed);
}

pub fn test_del_epoch_summary_single(db: &impl OLCheckpointDatabase) {
    let summary: EpochSummary = ArbitraryGenerator::new().generate();
    let commitment = summary.get_epoch_commitment();

    db.insert_epoch_summary(summary).expect("test: insert");

    // Verify it exists
    let stored = db
        .get_epoch_summary(commitment)
        .expect("test: get")
        .expect("test: should exist");
    assert_eq!(stored, summary);

    // Delete it
    let deleted = db
        .del_epoch_summary(commitment)
        .expect("test: delete epoch summary");
    assert!(deleted);

    // Verify it's gone
    let stored = db
        .get_epoch_summary(commitment)
        .expect("test: get after delete");
    assert!(stored.is_none());

    // Verify commitments at epoch is empty
    let commitments = db
        .get_epoch_commitments_at(commitment.epoch() as u64)
        .expect("test: get at epoch");
    assert!(commitments.is_empty());
}

pub fn test_del_epoch_summary_nonexistent(db: &impl OLCheckpointDatabase) {
    let summary: EpochSummary = ArbitraryGenerator::new().generate();
    let commitment = summary.get_epoch_commitment();

    // Try to delete a nonexistent summary
    let deleted = db
        .del_epoch_summary(commitment)
        .expect("test: delete nonexistent");
    assert!(!deleted);
}

pub fn test_del_epoch_summary_multiple(db: &impl OLCheckpointDatabase) {
    let mut ag = ArbitraryGenerator::new();
    let summary1: EpochSummary = ag.generate();
    let epoch = summary1.epoch();
    let summary2 = EpochSummary::new(
        epoch,
        ag.generate(),
        ag.generate(),
        ag.generate(),
        ag.generate(),
    );

    let commitment1 = summary1.get_epoch_commitment();
    let commitment2 = summary2.get_epoch_commitment();

    db.insert_epoch_summary(summary1).expect("test: insert 1");
    db.insert_epoch_summary(summary2).expect("test: insert 2");

    // Delete only one
    let deleted = db
        .del_epoch_summary(commitment1)
        .expect("test: delete first");
    assert!(deleted);

    // First should be gone
    assert!(db.get_epoch_summary(commitment1).expect("get 1").is_none());

    // Second should still exist
    let stored2 = db
        .get_epoch_summary(commitment2)
        .expect("get 2")
        .expect("should still exist");
    assert_eq!(stored2, summary2);

    // Commitments at epoch should only have second
    let commitments = db
        .get_epoch_commitments_at(epoch as u64)
        .expect("test: get at epoch");
    assert_eq!(commitments, vec![commitment2]);
}

pub fn test_get_last_checkpoint_epoch_empty(db: &impl OLCheckpointDatabase) {
    let last = db
        .get_last_checkpoint_epoch()
        .expect("test: get last epoch empty");
    assert!(last.is_none());
}

pub fn test_get_next_unsigned_checkpoint_epoch_empty(db: &impl OLCheckpointDatabase) {
    let next = db
        .get_next_unsigned_checkpoint_epoch()
        .expect("test: get next unsigned empty");
    assert!(next.is_none());
}

pub fn test_del_checkpoints_from_epoch_empty(db: &impl OLCheckpointDatabase) {
    let deleted = db
        .del_checkpoints_from_epoch(Epoch::from(0u32))
        .expect("test: delete from epoch empty");
    assert!(deleted.is_empty());
}

// Proptest-based tests
pub fn proptest_put_and_get_checkpoint(
    db: &impl OLCheckpointDatabase,
    epoch: Epoch,
    checkpoint: CheckpointPayload,
) {
    let entry = OLCheckpointEntry::new_unsigned(checkpoint.clone());

    db.put_checkpoint(epoch, entry.clone())
        .expect("test: put checkpoint");

    let retrieved = db
        .get_checkpoint(epoch)
        .expect("test: get checkpoint")
        .expect("checkpoint should exist");

    assert_eq!(retrieved.status, OLCheckpointStatus::Unsigned);
    assert_eq!(
        retrieved.checkpoint.new_tip().l1_height(),
        checkpoint.new_tip().l1_height()
    );
}

pub fn proptest_put_twice_idempotent(
    db: &impl OLCheckpointDatabase,
    epoch: Epoch,
    checkpoint: CheckpointPayload,
) {
    let entry = OLCheckpointEntry::new_unsigned(checkpoint.clone());

    db.put_checkpoint(epoch, entry.clone())
        .expect("test: put first time");
    db.put_checkpoint(epoch, entry.clone())
        .expect("test: put second time");

    let retrieved = db
        .get_checkpoint(epoch)
        .expect("test: get checkpoint")
        .expect("checkpoint should exist");

    assert_eq!(
        retrieved.checkpoint.new_tip().l1_height(),
        checkpoint.new_tip().l1_height()
    );
}

pub fn proptest_delete_checkpoint(
    db: &impl OLCheckpointDatabase,
    epoch: Epoch,
    checkpoint: CheckpointPayload,
) {
    let entry = OLCheckpointEntry::new_unsigned(checkpoint);

    db.put_checkpoint(epoch, entry).expect("test: put");

    let existed = db.del_checkpoint(epoch).expect("test: delete");
    assert!(existed);

    let deleted = db.get_checkpoint(epoch).expect("test: get after delete");
    assert!(deleted.is_none());
}

pub fn proptest_get_last_checkpoint_epoch(
    db: &impl OLCheckpointDatabase,
    checkpoint: CheckpointPayload,
    count: u32,
) {
    // Use continuous epochs starting from 0
    for e in 0..count {
        let entry = OLCheckpointEntry::new_unsigned(checkpoint.clone());
        db.put_checkpoint(Epoch::from(e), entry).expect("test: put");
    }

    let last = db
        .get_last_checkpoint_epoch()
        .expect("test: get last")
        .expect("should have checkpoints");

    assert_eq!(last, Epoch::from(count - 1));
}

pub fn proptest_get_next_unsigned_checkpoint_epoch(
    db: &impl OLCheckpointDatabase,
    checkpoint: CheckpointPayload,
    intent_index: u64,
    count: u32,
) {
    // Add unsigned checkpoints at continuous epochs starting from 0
    for e in 0..count {
        let entry = OLCheckpointEntry::new_unsigned(checkpoint.clone());
        db.put_checkpoint(Epoch::from(e), entry).expect("test: put");
    }

    // Next unsigned should be 0 (lowest)
    let next = db
        .get_next_unsigned_checkpoint_epoch()
        .expect("test: get next unsigned")
        .expect("should have unsigned");
    assert_eq!(next, Epoch::from(0u32));

    // Mark epoch 0 as signed
    let signed_entry =
        OLCheckpointEntry::new(checkpoint.clone(), OLCheckpointStatus::Signed(intent_index));
    db.put_checkpoint(Epoch::from(0u32), signed_entry)
        .expect("test: put signed");

    // Next unsigned should now be 1
    let next = db
        .get_next_unsigned_checkpoint_epoch()
        .expect("test: get next unsigned after sign")
        .expect("should still have unsigned");
    assert_eq!(next, Epoch::from(1u32));
}

pub fn proptest_del_checkpoints_from_epoch(
    db: &impl OLCheckpointDatabase,
    checkpoint: CheckpointPayload,
    count: u32,
    cutoff: u32,
) {
    // Add checkpoints at continuous epochs starting from 0
    for e in 0..count {
        let entry = OLCheckpointEntry::new_unsigned(checkpoint.clone());
        db.put_checkpoint(Epoch::from(e), entry).expect("test: put");
    }

    // Delete from cutoff onwards
    let deleted = db
        .del_checkpoints_from_epoch(Epoch::from(cutoff))
        .expect("test: delete from epoch");

    // Verify correct number deleted
    let expected_deleted = count.saturating_sub(cutoff);
    assert_eq!(deleted.len(), expected_deleted as usize);

    // Verify deleted epochs are correct
    for e in cutoff..count {
        assert!(deleted.contains(&Epoch::from(e)));
    }

    // Epochs before cutoff should still exist
    for e in 0..cutoff {
        assert!(db
            .get_checkpoint(Epoch::from(e))
            .expect("get remaining")
            .is_some());
    }

    // Epochs from cutoff onwards should be gone
    for e in cutoff..count {
        assert!(db
            .get_checkpoint(Epoch::from(e))
            .expect("get deleted")
            .is_none());
    }
}

pub fn proptest_status_transition(
    db: &impl OLCheckpointDatabase,
    epoch: Epoch,
    checkpoint: CheckpointPayload,
    intent_index: u64,
) {
    // Put as unsigned
    let entry = OLCheckpointEntry::new_unsigned(checkpoint.clone());
    db.put_checkpoint(epoch, entry).expect("test: put unsigned");

    let retrieved = db.get_checkpoint(epoch).expect("get").unwrap();
    assert_eq!(retrieved.status, OLCheckpointStatus::Unsigned);

    // Update to signed
    let signed_entry = OLCheckpointEntry::new(checkpoint, OLCheckpointStatus::Signed(intent_index));
    db.put_checkpoint(epoch, signed_entry)
        .expect("test: put signed");

    let retrieved = db.get_checkpoint(epoch).expect("get").unwrap();
    assert_eq!(retrieved.status, OLCheckpointStatus::Signed(intent_index));
}

pub fn proptest_interleaved_statuses_and_delete(
    db: &impl OLCheckpointDatabase,
    checkpoint: CheckpointPayload,
    count: u32,
    signed_prefix: u32,
    cutoff: u32,
    intent_index: u64,
) {
    // Insert continuous epochs as unsigned.
    for e in 0..count {
        let entry = OLCheckpointEntry::new_unsigned(checkpoint.clone());
        db.put_checkpoint(Epoch::from(e), entry)
            .expect("test: put unsigned");
    }

    // Mark a prefix as signed (unsigned -> signed).
    for e in 0..signed_prefix {
        let entry =
            OLCheckpointEntry::new(checkpoint.clone(), OLCheckpointStatus::Signed(intent_index));
        db.put_checkpoint(Epoch::from(e), entry)
            .expect("test: put signed");
    }

    // Delete from cutoff onwards.
    let deleted = db
        .del_checkpoints_from_epoch(Epoch::from(cutoff))
        .expect("test: delete from epoch");
    let expected_deleted = count.saturating_sub(cutoff);
    assert_eq!(deleted.len(), expected_deleted as usize);

    // Remaining epochs are < cutoff.
    for e in 0..cutoff {
        assert!(db
            .get_checkpoint(Epoch::from(e))
            .expect("get remaining")
            .is_some());
    }
    for e in cutoff..count {
        assert!(db
            .get_checkpoint(Epoch::from(e))
            .expect("get deleted")
            .is_none());
    }

    // Next unsigned should be the first unsigned remaining, if any.
    let expected_next = if signed_prefix < cutoff {
        Some(Epoch::from(signed_prefix))
    } else {
        None
    };
    let next = db
        .get_next_unsigned_checkpoint_epoch()
        .expect("test: get next unsigned");
    assert_eq!(next, expected_next);
}

#[macro_export]
macro_rules! ol_checkpoint_db_tests {
    ($setup_expr:expr) => {
        use strata_checkpoint_types_ssz::test_utils as checkpoint_test_utils;
        use strata_identifiers::test_utils::epoch_strategy;

        #[test]
        fn test_get_nonexistent_checkpoint() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_get_nonexistent_checkpoint(&db);
        }

        #[test]
        fn test_insert_summary_single() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_insert_summary_single(&db);
        }

        #[test]
        fn test_insert_summary_overwrite() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_insert_summary_overwrite(&db);
        }

        #[test]
        fn test_insert_summary_multiple() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_insert_summary_multiple(&db);
        }

        #[test]
        fn test_delete_nonexistent_checkpoint() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_delete_nonexistent_checkpoint(&db);
        }

        #[test]
        fn test_del_epoch_summary_single() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_del_epoch_summary_single(&db);
        }

        #[test]
        fn test_del_epoch_summary_nonexistent() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_del_epoch_summary_nonexistent(&db);
        }

        #[test]
        fn test_del_epoch_summary_multiple() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_del_epoch_summary_multiple(&db);
        }

        #[test]
        fn test_get_last_checkpoint_epoch_empty() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_get_last_checkpoint_epoch_empty(&db);
        }

        #[test]
        fn test_get_next_unsigned_checkpoint_epoch_empty() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_get_next_unsigned_checkpoint_epoch_empty(&db);
        }

        #[test]
        fn test_del_checkpoints_from_epoch_empty() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_del_checkpoints_from_epoch_empty(&db);
        }

        proptest::proptest! {
            #[test]
            fn proptest_put_and_get_checkpoint(
                epoch in epoch_strategy(),
                checkpoint in checkpoint_test_utils::checkpoint_payload_strategy()
            ) {
                let db = $setup_expr;
                $crate::ol_checkpoint_tests::proptest_put_and_get_checkpoint(&db, epoch, checkpoint);
            }

            #[test]
            fn proptest_put_twice_idempotent(
                epoch in epoch_strategy(),
                checkpoint in checkpoint_test_utils::checkpoint_payload_strategy()
            ) {
                let db = $setup_expr;
                $crate::ol_checkpoint_tests::proptest_put_twice_idempotent(&db, epoch, checkpoint);
            }

            #[test]
            fn proptest_delete_checkpoint(
                epoch in epoch_strategy(),
                checkpoint in checkpoint_test_utils::checkpoint_payload_strategy()
            ) {
                let db = $setup_expr;
                $crate::ol_checkpoint_tests::proptest_delete_checkpoint(&db, epoch, checkpoint);
            }

            #[test]
            fn proptest_get_last_checkpoint_epoch(
                checkpoint in checkpoint_test_utils::checkpoint_payload_strategy(),
                count in 1u32..10u32
            ) {
                let db = $setup_expr;
                $crate::ol_checkpoint_tests::proptest_get_last_checkpoint_epoch(&db, checkpoint, count);
            }

            #[test]
            fn proptest_get_next_unsigned_checkpoint_epoch(
                checkpoint in checkpoint_test_utils::checkpoint_payload_strategy(),
                intent_index in proptest::prelude::any::<u64>(),
                count in 2u32..10u32
            ) {
                let db = $setup_expr;
                $crate::ol_checkpoint_tests::proptest_get_next_unsigned_checkpoint_epoch(&db, checkpoint, intent_index, count);
            }

            #[test]
            fn proptest_del_checkpoints_from_epoch(
                checkpoint in checkpoint_test_utils::checkpoint_payload_strategy(),
                count in 1u32..10u32,
                cutoff_ratio in 0.0f64..1.0f64
            ) {
                let cutoff = ((count as f64) * cutoff_ratio) as u32;
                let db = $setup_expr;
                $crate::ol_checkpoint_tests::proptest_del_checkpoints_from_epoch(&db, checkpoint, count, cutoff);
            }

            #[test]
            fn proptest_status_transition(
                epoch in epoch_strategy(),
                checkpoint in checkpoint_test_utils::checkpoint_payload_strategy(),
                intent_index in proptest::prelude::any::<u64>()
            ) {
                let db = $setup_expr;
                $crate::ol_checkpoint_tests::proptest_status_transition(&db, epoch, checkpoint, intent_index);
            }

            #[test]
            fn proptest_interleaved_statuses_and_delete(
                checkpoint in checkpoint_test_utils::checkpoint_payload_strategy(),
                count in 3u32..10u32,
                signed_prefix in 0u32..10u32,
                cutoff in 0u32..10u32,
                intent_index in proptest::prelude::any::<u64>()
            ) {
                let count = count.max(1);
                let signed_prefix = signed_prefix.min(count);
                let cutoff = cutoff.min(count);
                let db = $setup_expr;
                $crate::ol_checkpoint_tests::proptest_interleaved_statuses_and_delete(
                    &db,
                    checkpoint,
                    count,
                    signed_prefix,
                    cutoff,
                    intent_index,
                );
            }
        }
    };
}
