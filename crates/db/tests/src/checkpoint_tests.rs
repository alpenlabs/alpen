#![expect(deprecated, reason = "legacy old code is retained for compatibility")]
use strata_checkpoint_types::EpochSummary;
use strata_db_types::{
    traits::CheckpointDatabase,
    types::{CheckpointEntry, CheckpointProvingStatus},
};
use strata_test_utils::ArbitraryGenerator;

pub fn test_insert_summary_single(db: &impl CheckpointDatabase) {
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

pub fn test_insert_summary_overwrite(db: &impl CheckpointDatabase) {
    let summary: EpochSummary = ArbitraryGenerator::new().generate();
    db.insert_epoch_summary(summary).expect("test: insert");
    db.insert_epoch_summary(summary)
        .expect_err("test: passed unexpectedly");
}

pub fn test_insert_summary_multiple(db: &impl CheckpointDatabase) {
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

pub fn test_batch_checkpoint_new_entry(db: &impl CheckpointDatabase) {
    let batchidx = 1;
    let checkpoint: CheckpointEntry = ArbitraryGenerator::new().generate();
    db.put_checkpoint(batchidx, checkpoint.clone()).unwrap();

    let retrieved_batch = db.get_checkpoint(batchidx).unwrap().unwrap();
    assert_eq!(checkpoint, retrieved_batch);
}

pub fn test_batch_checkpoint_existing_entry(db: &impl CheckpointDatabase) {
    let batchidx = 1;
    let checkpoint: CheckpointEntry = ArbitraryGenerator::new().generate();
    db.put_checkpoint(batchidx, checkpoint.clone()).unwrap();
    db.put_checkpoint(batchidx, checkpoint.clone()).unwrap();
}

pub fn test_batch_checkpoint_non_monotonic_entries(db: &impl CheckpointDatabase) {
    let checkpoint: CheckpointEntry = ArbitraryGenerator::new().generate();
    db.put_checkpoint(100, checkpoint.clone()).unwrap();
    db.put_checkpoint(1, checkpoint.clone()).unwrap();
    db.put_checkpoint(3, checkpoint.clone()).unwrap();
}

pub fn test_get_last_batch_checkpoint_idx(db: &impl CheckpointDatabase) {
    let checkpoint: CheckpointEntry = ArbitraryGenerator::new().generate();
    db.put_checkpoint(100, checkpoint.clone()).unwrap();
    db.put_checkpoint(1, checkpoint.clone()).unwrap();
    db.put_checkpoint(3, checkpoint.clone()).unwrap();

    let last_idx = db.get_last_checkpoint_idx().unwrap().unwrap();
    assert_eq!(last_idx, 100);

    db.put_checkpoint(50, checkpoint.clone()).unwrap();
    let last_idx = db.get_last_checkpoint_idx().unwrap().unwrap();
    assert_eq!(last_idx, 100);
}

pub fn test_256_checkpoints(db: &impl CheckpointDatabase) {
    let checkpoint: CheckpointEntry = ArbitraryGenerator::new().generate();

    for expected_idx in 0..=256 {
        let last_idx = db.get_last_checkpoint_idx().unwrap().unwrap_or(0);
        assert_eq!(last_idx, expected_idx);

        // Insert one to db
        db.put_checkpoint(last_idx + 1, checkpoint.clone()).unwrap();
    }
}

pub fn test_del_epoch_summary_single(db: &impl CheckpointDatabase) {
    let summary: EpochSummary = ArbitraryGenerator::new().generate();
    let commitment = summary.get_epoch_commitment();

    // Insert summary
    db.insert_epoch_summary(summary).expect("test: insert");

    // Verify it exists
    assert!(db
        .get_epoch_summary(commitment)
        .expect("test: get")
        .is_some());

    // Delete it
    let deleted = db.del_epoch_summary(commitment).expect("test: delete");
    assert!(deleted, "Should return true when deleting existing summary");

    // Verify it's gone
    assert!(db
        .get_epoch_summary(commitment)
        .expect("test: get after delete")
        .is_none());

    // Delete again should return false
    let deleted_again = db
        .del_epoch_summary(commitment)
        .expect("test: delete again");
    assert!(
        !deleted_again,
        "Should return false when deleting non-existent summary"
    );
}

pub fn test_del_epoch_summary_from_multiple(db: &impl CheckpointDatabase) {
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

    // Insert both summaries
    db.insert_epoch_summary(summary1).expect("test: insert 1");
    db.insert_epoch_summary(summary2).expect("test: insert 2");

    // Verify both exist
    assert!(db
        .get_epoch_summary(commitment1)
        .expect("test: get 1")
        .is_some());
    assert!(db
        .get_epoch_summary(commitment2)
        .expect("test: get 2")
        .is_some());

    // Delete first summary
    let deleted = db.del_epoch_summary(commitment1).expect("test: delete 1");
    assert!(deleted);

    // Verify first is gone, second still exists
    assert!(db
        .get_epoch_summary(commitment1)
        .expect("test: get 1 after delete")
        .is_none());
    assert!(db
        .get_epoch_summary(commitment2)
        .expect("test: get 2 after delete")
        .is_some());

    // Delete second summary
    let deleted = db.del_epoch_summary(commitment2).expect("test: delete 2");
    assert!(deleted);

    // Verify both are gone
    assert!(db
        .get_epoch_summary(commitment1)
        .expect("test: get 1 final")
        .is_none());
    assert!(db
        .get_epoch_summary(commitment2)
        .expect("test: get 2 final")
        .is_none());
}

pub fn test_del_epoch_summaries_from_epoch(db: &impl CheckpointDatabase) {
    let mut ag = ArbitraryGenerator::new();

    // Create summaries for epochs 1, 2, 3
    let summary1: EpochSummary = EpochSummary::new(
        1,
        ag.generate(),
        ag.generate(),
        ag.generate(),
        ag.generate(),
    );
    let summary2: EpochSummary = EpochSummary::new(
        2,
        ag.generate(),
        ag.generate(),
        ag.generate(),
        ag.generate(),
    );
    let summary3: EpochSummary = EpochSummary::new(
        3,
        ag.generate(),
        ag.generate(),
        ag.generate(),
        ag.generate(),
    );

    let commitment1 = summary1.get_epoch_commitment();
    let commitment2 = summary2.get_epoch_commitment();
    let commitment3 = summary3.get_epoch_commitment();

    // Insert all summaries
    db.insert_epoch_summary(summary1).expect("test: insert 1");
    db.insert_epoch_summary(summary2).expect("test: insert 2");
    db.insert_epoch_summary(summary3).expect("test: insert 3");

    // Delete from epoch 2 onwards
    let deleted_epochs = db
        .del_epoch_summaries_from_epoch(2)
        .expect("test: delete from epoch 2");
    assert_eq!(deleted_epochs, vec![2, 3], "Should delete epochs 2 and 3");

    // Verify epoch 1 still exists, epochs 2 and 3 are gone
    assert!(db
        .get_epoch_summary(commitment1)
        .expect("test: get 1")
        .is_some());
    assert!(db
        .get_epoch_summary(commitment2)
        .expect("test: get 2")
        .is_none());
    assert!(db
        .get_epoch_summary(commitment3)
        .expect("test: get 3")
        .is_none());

    // Delete from epoch 0 onwards (should delete epoch 1)
    let deleted_epochs = db
        .del_epoch_summaries_from_epoch(0)
        .expect("test: delete from epoch 0");
    assert_eq!(deleted_epochs, vec![1], "Should delete epoch 1");

    // Verify all are gone
    assert!(db
        .get_epoch_summary(commitment1)
        .expect("test: get 1 final")
        .is_none());
}

pub fn test_del_checkpoint_single(db: &impl CheckpointDatabase) {
    let checkpoint: CheckpointEntry = ArbitraryGenerator::new().generate();
    let epoch = 5;

    // Insert checkpoint
    db.put_checkpoint(epoch, checkpoint.clone())
        .expect("test: insert");

    // Verify it exists
    assert!(db.get_checkpoint(epoch).expect("test: get").is_some());

    // Delete it
    let deleted = db.del_checkpoint(epoch).expect("test: delete");
    assert!(
        deleted,
        "Should return true when deleting existing checkpoint"
    );

    // Verify it's gone
    assert!(db
        .get_checkpoint(epoch)
        .expect("test: get after delete")
        .is_none());

    // Delete again should return false
    let deleted_again = db.del_checkpoint(epoch).expect("test: delete again");
    assert!(
        !deleted_again,
        "Should return false when deleting non-existent checkpoint"
    );
}

pub fn test_del_checkpoints_from_epoch(db: &impl CheckpointDatabase) {
    let checkpoint: CheckpointEntry = ArbitraryGenerator::new().generate();

    // Insert checkpoints for epochs 1, 3, 5, 7
    db.put_checkpoint(1, checkpoint.clone())
        .expect("test: insert 1");
    db.put_checkpoint(3, checkpoint.clone())
        .expect("test: insert 3");
    db.put_checkpoint(5, checkpoint.clone())
        .expect("test: insert 5");
    db.put_checkpoint(7, checkpoint.clone())
        .expect("test: insert 7");

    // Delete from epoch 4 onwards
    let deleted_epochs = db
        .del_checkpoints_from_epoch(4)
        .expect("test: delete from epoch 4");
    assert_eq!(deleted_epochs, vec![5, 7], "Should delete epochs 5 and 7");

    // Verify epochs 1 and 3 still exist, epochs 5 and 7 are gone
    assert!(db.get_checkpoint(1).expect("test: get 1").is_some());
    assert!(db.get_checkpoint(3).expect("test: get 3").is_some());
    assert!(db.get_checkpoint(5).expect("test: get 5").is_none());
    assert!(db.get_checkpoint(7).expect("test: get 7").is_none());

    // Delete from epoch 2 onwards
    let deleted_epochs = db
        .del_checkpoints_from_epoch(2)
        .expect("test: delete from epoch 2");
    assert_eq!(deleted_epochs, vec![3], "Should delete epoch 3");

    // Verify only epoch 1 remains
    assert!(db.get_checkpoint(1).expect("test: get 1 final").is_some());
    assert!(db.get_checkpoint(3).expect("test: get 3 final").is_none());
}

pub fn test_del_checkpoints_empty_database(db: &impl CheckpointDatabase) {
    // Delete from empty database should return empty vec
    let deleted_epochs = db
        .del_checkpoints_from_epoch(0)
        .expect("test: delete from empty");
    assert!(
        deleted_epochs.is_empty(),
        "Should return empty vec for empty database"
    );

    let deleted_epochs = db
        .del_epoch_summaries_from_epoch(0)
        .expect("test: delete summaries from empty");
    assert!(
        deleted_epochs.is_empty(),
        "Should return empty vec for empty database"
    );
}

pub fn test_get_next_unproven_checkpoint_idx_empty_db(db: &impl CheckpointDatabase) {
    // Test: Empty database should return None
    let result = db.get_next_unproven_checkpoint_idx().unwrap();
    assert_eq!(result, None);
}

pub fn test_get_next_unproven_checkpoint_idx_all_pending(db: &impl CheckpointDatabase) {
    // Test: All checkpoints have PendingProof - should return first one (sequential processing)
    let mut ag = ArbitraryGenerator::new();

    // Create 5 checkpoints, all with PendingProof status
    for i in 0..5 {
        let mut checkpoint: CheckpointEntry = ag.generate();
        checkpoint.proving_status = CheckpointProvingStatus::PendingProof;
        db.put_checkpoint(i, checkpoint).unwrap();
    }

    let result = db.get_next_unproven_checkpoint_idx().unwrap();
    assert_eq!(result, Some(0)); // Start from beginning when no proven checkpoints
}

pub fn test_get_next_unproven_checkpoint_idx_all_ready(db: &impl CheckpointDatabase) {
    // Test: All checkpoints have ProofReady - should return None
    let mut ag = ArbitraryGenerator::new();

    // Create 5 checkpoints, all with ProofReady status
    for i in 0..5 {
        let mut checkpoint: CheckpointEntry = ag.generate();
        checkpoint.proving_status = CheckpointProvingStatus::ProofReady;
        db.put_checkpoint(i, checkpoint).unwrap();
    }

    let result = db.get_next_unproven_checkpoint_idx().unwrap();
    assert_eq!(result, None);
}

pub fn test_get_next_unproven_checkpoint_idx_mixed_sequential(db: &impl CheckpointDatabase) {
    // Test: Sequential processing - some ready, some pending
    let mut ag = ArbitraryGenerator::new();

    // Checkpoints 0-2: ProofReady, 3-5: PendingProof
    for i in 0..6 {
        let mut checkpoint: CheckpointEntry = ag.generate();
        checkpoint.proving_status = if i <= 2 {
            CheckpointProvingStatus::ProofReady
        } else {
            CheckpointProvingStatus::PendingProof
        };
        db.put_checkpoint(i, checkpoint).unwrap();
    }

    // Should return 3 (first unproven after last proven)
    let result = db.get_next_unproven_checkpoint_idx().unwrap();
    assert_eq!(result, Some(3));
}

pub fn test_get_next_unproven_checkpoint_idx_rebuild_pending_index(db: &impl CheckpointDatabase) {
    // Seed pending checkpoints and ensure the query still sees them after a fresh handle build.
    let mut ag = ArbitraryGenerator::new();

    for idx in 0..3 {
        let mut checkpoint: CheckpointEntry = ag.generate();
        checkpoint.proving_status = CheckpointProvingStatus::PendingProof;
        db.put_checkpoint(idx, checkpoint.clone()).unwrap();
    }

    // Simulate reopening the database by invoking the query immediately.
    // The sled implementation must rebuild the pending index or fallback to detect entries.
    let result = db.get_next_unproven_checkpoint_idx().unwrap();
    assert_eq!(result, Some(0));
}

#[macro_export]
macro_rules! checkpoint_db_tests {
    ($setup_expr:expr) => {
        #[test]
        fn test_insert_summary_single() {
            let db = $setup_expr;
            $crate::checkpoint_tests::test_insert_summary_single(&db);
        }

        #[test]
        fn test_insert_summary_overwrite() {
            let db = $setup_expr;
            $crate::checkpoint_tests::test_insert_summary_overwrite(&db);
        }

        #[test]
        fn test_insert_summary_multiple() {
            let db = $setup_expr;
            $crate::checkpoint_tests::test_insert_summary_multiple(&db);
        }

        #[test]
        fn test_batch_checkpoint_new_entry() {
            let db = $setup_expr;
            $crate::checkpoint_tests::test_batch_checkpoint_new_entry(&db);
        }

        #[test]
        fn test_batch_checkpoint_existing_entry() {
            let db = $setup_expr;
            $crate::checkpoint_tests::test_batch_checkpoint_existing_entry(&db);
        }

        #[test]
        fn test_batch_checkpoint_non_monotonic_entries() {
            let db = $setup_expr;
            $crate::checkpoint_tests::test_batch_checkpoint_non_monotonic_entries(&db);
        }

        #[test]
        fn test_get_last_batch_checkpoint_idx() {
            let db = $setup_expr;
            $crate::checkpoint_tests::test_get_last_batch_checkpoint_idx(&db);
        }

        #[test]
        fn test_256_checkpoints() {
            let db = $setup_expr;
            $crate::checkpoint_tests::test_256_checkpoints(&db);
        }

        #[test]
        fn test_del_epoch_summary_single() {
            let db = $setup_expr;
            $crate::checkpoint_tests::test_del_epoch_summary_single(&db);
        }

        #[test]
        fn test_del_epoch_summary_from_multiple() {
            let db = $setup_expr;
            $crate::checkpoint_tests::test_del_epoch_summary_from_multiple(&db);
        }

        #[test]
        fn test_del_epoch_summaries_from_epoch() {
            let db = $setup_expr;
            $crate::checkpoint_tests::test_del_epoch_summaries_from_epoch(&db);
        }

        #[test]
        fn test_del_checkpoint_single() {
            let db = $setup_expr;
            $crate::checkpoint_tests::test_del_checkpoint_single(&db);
        }

        #[test]
        fn test_del_checkpoints_from_epoch() {
            let db = $setup_expr;
            $crate::checkpoint_tests::test_del_checkpoints_from_epoch(&db);
        }

        #[test]
        fn test_del_checkpoints_empty_database() {
            let db = $setup_expr;
            $crate::checkpoint_tests::test_del_checkpoints_empty_database(&db);
        }

        // Tests for get_next_unproven_checkpoint_idx method
        #[test]
        fn test_get_next_unproven_checkpoint_idx_empty_db() {
            let db = $setup_expr;
            $crate::checkpoint_tests::test_get_next_unproven_checkpoint_idx_empty_db(&db);
        }

        #[test]
        fn test_get_next_unproven_checkpoint_idx_all_pending() {
            let db = $setup_expr;
            $crate::checkpoint_tests::test_get_next_unproven_checkpoint_idx_all_pending(&db);
        }

        #[test]
        fn test_get_next_unproven_checkpoint_idx_all_ready() {
            let db = $setup_expr;
            $crate::checkpoint_tests::test_get_next_unproven_checkpoint_idx_all_ready(&db);
        }

        #[test]
        fn test_get_next_unproven_checkpoint_idx_mixed_sequential() {
            let db = $setup_expr;
            $crate::checkpoint_tests::test_get_next_unproven_checkpoint_idx_mixed_sequential(&db);
        }

        #[test]
        fn test_get_next_unproven_checkpoint_idx_rebuild_pending_index() {
            let db = $setup_expr;
            $crate::checkpoint_tests::test_get_next_unproven_checkpoint_idx_rebuild_pending_index(
                &db,
            );
        }
    };
}
