use strata_db::{traits::CheckpointDatabase, types::CheckpointEntry};
use strata_state::batch::EpochSummary;
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
        .get_epoch_commitments_at(commitment.epoch())
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
        .get_epoch_commitments_at(epoch)
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
    };
}
