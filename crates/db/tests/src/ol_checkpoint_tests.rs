use strata_checkpoint_types::EpochSummary;
use strata_checkpoint_types_ssz::{test_utils::create_test_checkpoint_payload, CheckpointPayload};
use strata_db_types::{
    traits::OLCheckpointDatabase,
    types::{L1PayloadIntentIndex, OLCheckpointL1ObservationEntry},
};
use strata_identifiers::{
    Buf32, Epoch, EpochCommitment, L1BlockCommitment, L1BlockId, OLBlockCommitment,
};
use strata_test_utils::ArbitraryGenerator;

fn checkpoint_epoch_commitment(payload: &CheckpointPayload) -> EpochCommitment {
    EpochCommitment::from_terminal(
        Epoch::from(payload.new_tip().epoch),
        *payload.new_tip().l2_commitment(),
    )
}

fn payload_for_epoch(epoch: u32) -> CheckpointPayload {
    create_test_checkpoint_payload(epoch)
}

fn l1_observation_entry(height: u32) -> OLCheckpointL1ObservationEntry {
    let blkid = L1BlockId::from(Buf32::from([height as u8; 32]));
    OLCheckpointL1ObservationEntry::new(L1BlockCommitment::new(height, blkid))
}

pub fn test_get_nonexistent_checkpoint_payload_entry(db: &impl OLCheckpointDatabase) {
    let nonexistent_epoch = Epoch::from(999u32);
    let key = EpochCommitment::from_terminal(nonexistent_epoch, OLBlockCommitment::null());

    let result = db
        .get_checkpoint_payload_entry(key)
        .expect("test: get nonexistent checkpoint payload");
    assert!(result.is_none());
}

pub fn test_insert_summary_single(db: &impl OLCheckpointDatabase) {
    let summary: EpochSummary = ArbitraryGenerator::new().generate();
    let commitment = summary.get_epoch_commitment();
    let epoch = Epoch::from(summary.epoch());

    db.insert_epoch_summary(summary).expect("test: insert");

    let stored = db
        .get_epoch_summary(commitment)
        .expect("test: get")
        .expect("test: get missing");
    assert_eq!(stored, summary);

    let commitments = db
        .get_epoch_commitments_at(epoch)
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
    let epoch_u32 = summary1.epoch();
    let epoch = Epoch::from(epoch_u32);
    let summary2 = EpochSummary::new(
        epoch_u32,
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

pub fn test_del_epoch_summary_single(db: &impl OLCheckpointDatabase) {
    let summary: EpochSummary = ArbitraryGenerator::new().generate();
    let commitment = summary.get_epoch_commitment();
    let epoch = Epoch::from(summary.epoch());

    db.insert_epoch_summary(summary).expect("test: insert");

    let deleted = db
        .del_epoch_summary(commitment)
        .expect("test: delete epoch summary");
    assert!(deleted);

    let stored = db
        .get_epoch_summary(commitment)
        .expect("test: get after delete");
    assert!(stored.is_none());

    let commitments = db
        .get_epoch_commitments_at(epoch)
        .expect("test: get at epoch");
    assert!(commitments.is_empty());
}

pub fn test_del_epoch_summary_nonexistent(db: &impl OLCheckpointDatabase) {
    let summary: EpochSummary = ArbitraryGenerator::new().generate();
    let commitment = summary.get_epoch_commitment();

    let deleted = db
        .del_epoch_summary(commitment)
        .expect("test: delete nonexistent");
    assert!(!deleted);
}

pub fn test_del_epoch_summary_multiple(db: &impl OLCheckpointDatabase) {
    let mut ag = ArbitraryGenerator::new();
    let summary1: EpochSummary = ag.generate();
    let epoch_u32 = summary1.epoch();
    let epoch = Epoch::from(epoch_u32);
    let summary2 = EpochSummary::new(
        epoch_u32,
        ag.generate(),
        ag.generate(),
        ag.generate(),
        ag.generate(),
    );

    let commitment1 = summary1.get_epoch_commitment();
    let commitment2 = summary2.get_epoch_commitment();

    db.insert_epoch_summary(summary1).expect("test: insert 1");
    db.insert_epoch_summary(summary2).expect("test: insert 2");

    let deleted = db
        .del_epoch_summary(commitment1)
        .expect("test: delete first");
    assert!(deleted);

    assert!(db.get_epoch_summary(commitment1).expect("get 1").is_none());

    let stored2 = db
        .get_epoch_summary(commitment2)
        .expect("get 2")
        .expect("should still exist");
    assert_eq!(stored2, summary2);

    let commitments = db
        .get_epoch_commitments_at(epoch)
        .expect("test: get at epoch");
    assert_eq!(commitments, vec![commitment2]);
}

pub fn test_get_last_checkpoint_payload_epoch_empty(db: &impl OLCheckpointDatabase) {
    let last = db
        .get_last_checkpoint_payload_epoch()
        .expect("test: get last payload epoch empty");
    assert!(last.is_none());
}

pub fn test_get_next_unsigned_checkpoint_epoch_empty(db: &impl OLCheckpointDatabase) {
    let next = db
        .get_next_unsigned_checkpoint_epoch()
        .expect("test: get next unsigned empty");
    assert!(next.is_none());
}

pub fn test_del_checkpoint_payload_entries_from_epoch_empty(db: &impl OLCheckpointDatabase) {
    let deleted = db
        .del_checkpoint_payload_entries_from_epoch(Epoch::from(0u32))
        .expect("test: delete from epoch empty");
    assert!(deleted.is_empty());
}

pub fn test_unsigned_epoch_index_mixed_operations(db: &impl OLCheckpointDatabase) {
    let payload0 = payload_for_epoch(0);
    let payload1 = payload_for_epoch(1);

    let epoch0 = Epoch::from(0u32);
    let epoch1 = Epoch::from(1u32);
    let key0 = checkpoint_epoch_commitment(&payload0);
    let key1 = checkpoint_epoch_commitment(&payload1);

    db.put_checkpoint_payload_entry(key0, payload0)
        .expect("put payload epoch0");
    db.put_checkpoint_payload_entry(key1, payload1)
        .expect("put payload epoch1");

    assert_eq!(
        db.get_next_unsigned_checkpoint_epoch()
            .expect("get next unsigned after payload puts"),
        Some(epoch0)
    );

    db.put_checkpoint_signing_entry(key0, 11)
        .expect("sign epoch0");
    assert_eq!(
        db.get_next_unsigned_checkpoint_epoch()
            .expect("get next unsigned after signing epoch0"),
        Some(epoch1)
    );

    db.del_checkpoint_payload_entry(key1)
        .expect("delete payload epoch1");
    assert_eq!(
        db.get_next_unsigned_checkpoint_epoch()
            .expect("get next unsigned after deleting epoch1 payload"),
        None
    );

    let payload1_reinsert = payload_for_epoch(1);
    db.put_checkpoint_payload_entry(key1, payload1_reinsert)
        .expect("reinsert payload epoch1");
    assert_eq!(
        db.get_next_unsigned_checkpoint_epoch()
            .expect("get next unsigned after reinsert epoch1 payload"),
        Some(epoch1)
    );

    db.put_checkpoint_signing_entry(key1, 22)
        .expect("sign epoch1");
    assert_eq!(
        db.get_next_unsigned_checkpoint_epoch()
            .expect("get next unsigned after signing epoch1"),
        None
    );

    db.del_checkpoint_signing_entry(key1)
        .expect("delete signing epoch1");
    assert_eq!(
        db.get_next_unsigned_checkpoint_epoch()
            .expect("get next unsigned after deleting epoch1 signing"),
        Some(epoch1)
    );
}

pub fn test_put_checkpoint_signing_entry_requires_payload(db: &impl OLCheckpointDatabase) {
    let payload = payload_for_epoch(7);
    let key = checkpoint_epoch_commitment(&payload);

    assert!(db.put_checkpoint_signing_entry(key, 99).is_err());
}

pub fn test_put_checkpoint_payload_entry_rejects_mismatched_commitment(
    db: &impl OLCheckpointDatabase,
) {
    let payload = payload_for_epoch(9);
    let mismatched_commitment =
        EpochCommitment::from_terminal(Epoch::from(10u32), *payload.new_tip().l2_commitment());
    assert!(db
        .put_checkpoint_payload_entry(mismatched_commitment, payload)
        .is_err());
}

pub fn test_del_checkpoint_payload_entry_deletes_signing_entry(db: &impl OLCheckpointDatabase) {
    let payload = payload_for_epoch(8);
    let key = checkpoint_epoch_commitment(&payload);

    db.put_checkpoint_payload_entry(key, payload)
        .expect("put payload");
    db.put_checkpoint_signing_entry(key, 42)
        .expect("put signing");

    db.del_checkpoint_payload_entry(key)
        .expect("delete payload should succeed");

    assert!(db
        .get_checkpoint_signing_entry(key)
        .expect("get signing after payload delete")
        .is_none());
}

pub fn test_del_checkpoint_signing_entries_from_epoch(db: &impl OLCheckpointDatabase) {
    for epoch in 0u32..4 {
        let payload = payload_for_epoch(epoch);
        let key = checkpoint_epoch_commitment(&payload);
        db.put_checkpoint_payload_entry(key, payload)
            .expect("put payload");
        db.put_checkpoint_signing_entry(key, epoch as u64)
            .expect("put signing");
    }

    let deleted = db
        .del_checkpoint_signing_entries_from_epoch(Epoch::from(2u32))
        .expect("delete signing entries from epoch");
    let expected: Vec<EpochCommitment> = (2u32..4)
        .map(|e| checkpoint_epoch_commitment(&payload_for_epoch(e)))
        .collect();
    assert_eq!(deleted, expected);

    for epoch in 0u32..2 {
        let payload = payload_for_epoch(epoch);
        let key = checkpoint_epoch_commitment(&payload);
        assert!(db
            .get_checkpoint_signing_entry(key)
            .expect("get retained signing")
            .is_some());
    }

    for epoch in 2u32..4 {
        let payload = payload_for_epoch(epoch);
        let key = checkpoint_epoch_commitment(&payload);
        assert!(db
            .get_checkpoint_signing_entry(key)
            .expect("get deleted signing")
            .is_none());
    }
}

pub fn test_put_checkpoint_l1_observation_entry_allows_missing_payload(
    db: &impl OLCheckpointDatabase,
) {
    let payload = payload_for_epoch(13);
    let key = checkpoint_epoch_commitment(&payload);

    let expected_observation = l1_observation_entry(100);
    db.put_checkpoint_l1_observation_entry(key, expected_observation)
        .expect("put l1 observation without payload");

    let observed = db
        .get_checkpoint_l1_observation_entry(key)
        .expect("get l1 observation")
        .expect("l1 observation should exist");
    assert_eq!(observed, expected_observation);
}

pub fn test_checkpoint_l1_observation_entry_roundtrip(db: &impl OLCheckpointDatabase) {
    let payload = payload_for_epoch(14);
    let key = checkpoint_epoch_commitment(&payload);

    db.put_checkpoint_payload_entry(key, payload)
        .expect("put payload");
    let expected_observation = l1_observation_entry(123);
    db.put_checkpoint_l1_observation_entry(key, expected_observation)
        .expect("put l1 observation");

    let observed = db
        .get_checkpoint_l1_observation_entry(key)
        .expect("get l1 observation")
        .expect("l1 observation should exist");
    assert_eq!(observed, expected_observation);

    let deleted = db
        .del_checkpoint_l1_observation_entry(key)
        .expect("delete l1 observation");
    assert!(deleted);

    assert!(db
        .get_checkpoint_l1_observation_entry(key)
        .expect("get l1 observation after delete")
        .is_none());
}

pub fn test_del_checkpoint_payload_entry_deletes_l1_observation_entry(
    db: &impl OLCheckpointDatabase,
) {
    let payload = payload_for_epoch(15);
    let key = checkpoint_epoch_commitment(&payload);

    db.put_checkpoint_payload_entry(key, payload)
        .expect("put payload");
    db.put_checkpoint_l1_observation_entry(key, l1_observation_entry(200))
        .expect("put l1 observation");

    db.del_checkpoint_payload_entry(key)
        .expect("delete payload should succeed");

    assert!(db
        .get_checkpoint_l1_observation_entry(key)
        .expect("get l1 observation after payload delete")
        .is_none());
}

pub fn test_del_checkpoint_l1_observation_entries_from_epoch(db: &impl OLCheckpointDatabase) {
    for epoch in 0u32..4 {
        let payload = payload_for_epoch(epoch);
        let key = checkpoint_epoch_commitment(&payload);
        db.put_checkpoint_payload_entry(key, payload)
            .expect("put payload");
        db.put_checkpoint_l1_observation_entry(key, l1_observation_entry(100 + epoch))
            .expect("put l1 observation");
    }

    let deleted = db
        .del_checkpoint_l1_observation_entries_from_epoch(Epoch::from(2u32))
        .expect("delete l1 observation entries from epoch");
    let expected: Vec<EpochCommitment> = (2u32..4)
        .map(|e| checkpoint_epoch_commitment(&payload_for_epoch(e)))
        .collect();
    assert_eq!(deleted, expected);

    for epoch in 0u32..2 {
        let payload = payload_for_epoch(epoch);
        let key = checkpoint_epoch_commitment(&payload);
        assert!(db
            .get_checkpoint_l1_observation_entry(key)
            .expect("get retained l1 observation")
            .is_some());
    }

    for epoch in 2u32..4 {
        let payload = payload_for_epoch(epoch);
        let key = checkpoint_epoch_commitment(&payload);
        assert!(db
            .get_checkpoint_l1_observation_entry(key)
            .expect("get deleted l1 observation")
            .is_none());
    }
}

pub fn test_get_next_unsigned_checkpoint_epoch_all_signed_returns_none(
    db: &impl OLCheckpointDatabase,
) {
    for epoch in 0u32..3 {
        let payload = payload_for_epoch(epoch);
        let key = checkpoint_epoch_commitment(&payload);
        db.put_checkpoint_payload_entry(key, payload)
            .expect("put payload");
        db.put_checkpoint_signing_entry(key, epoch as u64)
            .expect("put signing");
    }

    assert_eq!(
        db.get_next_unsigned_checkpoint_epoch()
            .expect("get next unsigned when all signed"),
        None
    );
}

pub fn test_get_next_unsigned_checkpoint_epoch_non_contiguous(db: &impl OLCheckpointDatabase) {
    for epoch in [1u32, 3u32, 7u32] {
        let payload = payload_for_epoch(epoch);
        let key = checkpoint_epoch_commitment(&payload);
        db.put_checkpoint_payload_entry(key, payload)
            .expect("put payload");
    }

    assert_eq!(
        db.get_next_unsigned_checkpoint_epoch()
            .expect("next unsigned initial"),
        Some(Epoch::from(1u32))
    );
}

pub fn proptest_put_and_get_checkpoint_payload_entry(
    db: &impl OLCheckpointDatabase,
    checkpoint: CheckpointPayload,
) {
    let key = checkpoint_epoch_commitment(&checkpoint);

    db.put_checkpoint_payload_entry(key, checkpoint.clone())
        .expect("test: put checkpoint payload");

    let retrieved = db
        .get_checkpoint_payload_entry(key)
        .expect("test: get checkpoint payload")
        .expect("checkpoint payload should exist");

    assert_eq!(retrieved, checkpoint);
}

pub fn proptest_put_twice_idempotent(
    db: &impl OLCheckpointDatabase,
    checkpoint: CheckpointPayload,
) {
    let key = checkpoint_epoch_commitment(&checkpoint);

    db.put_checkpoint_payload_entry(key, checkpoint.clone())
        .expect("test: put first time");
    db.put_checkpoint_payload_entry(key, checkpoint.clone())
        .expect("test: put second time");

    let retrieved = db
        .get_checkpoint_payload_entry(key)
        .expect("test: get checkpoint payload")
        .expect("checkpoint payload should exist");

    assert_eq!(retrieved, checkpoint);
}

pub fn proptest_delete_checkpoint_payload_entry(
    db: &impl OLCheckpointDatabase,
    checkpoint: CheckpointPayload,
) {
    let key = checkpoint_epoch_commitment(&checkpoint);

    db.put_checkpoint_payload_entry(key, checkpoint)
        .expect("test: put payload");

    let existed = db
        .del_checkpoint_payload_entry(key)
        .expect("test: delete payload");
    assert!(existed);

    let deleted = db
        .get_checkpoint_payload_entry(key)
        .expect("test: get after delete");
    assert!(deleted.is_none());
}

pub fn proptest_get_last_checkpoint_payload_epoch(db: &impl OLCheckpointDatabase, count: u32) {
    let mut last_key: Option<EpochCommitment> = None;
    for e in 0..count {
        let payload = payload_for_epoch(e);
        let key = checkpoint_epoch_commitment(&payload);
        db.put_checkpoint_payload_entry(key, payload)
            .expect("test: put payload");
        last_key = Some(key);
    }

    let last = db
        .get_last_checkpoint_payload_epoch()
        .expect("test: get last payload epoch")
        .expect("should have payloads");

    assert_eq!(Some(last), last_key);
}

pub fn proptest_get_next_unsigned_checkpoint_epoch(
    db: &impl OLCheckpointDatabase,
    intent_index: L1PayloadIntentIndex,
    count: u32,
) {
    for e in 0..count {
        let payload = payload_for_epoch(e);
        let key = checkpoint_epoch_commitment(&payload);
        db.put_checkpoint_payload_entry(key, payload)
            .expect("test: put payload");
    }

    let next = db
        .get_next_unsigned_checkpoint_epoch()
        .expect("test: get next unsigned")
        .expect("should have unsigned");
    assert_eq!(next, Epoch::from(0u32));

    let first_payload = payload_for_epoch(0);
    let first_key = checkpoint_epoch_commitment(&first_payload);
    db.put_checkpoint_signing_entry(first_key, intent_index)
        .expect("test: put signing");

    let next = db
        .get_next_unsigned_checkpoint_epoch()
        .expect("test: get next unsigned after sign")
        .expect("should still have unsigned");
    assert_eq!(next, Epoch::from(1u32));
}

pub fn proptest_del_checkpoint_payload_entries_from_epoch(
    db: &impl OLCheckpointDatabase,
    count: u32,
    cutoff: u32,
) {
    for e in 0..count {
        let payload = payload_for_epoch(e);
        let key = checkpoint_epoch_commitment(&payload);
        db.put_checkpoint_payload_entry(key, payload)
            .expect("test: put payload");
    }

    let deleted = db
        .del_checkpoint_payload_entries_from_epoch(Epoch::from(cutoff))
        .expect("test: delete from epoch");

    let expected_deleted = count.saturating_sub(cutoff);
    assert_eq!(deleted.len(), expected_deleted as usize);

    for e in cutoff..count {
        let key = checkpoint_epoch_commitment(&payload_for_epoch(e));
        assert!(deleted.contains(&key));
    }

    for e in 0..cutoff {
        let key = checkpoint_epoch_commitment(&payload_for_epoch(e));
        assert!(db
            .get_checkpoint_payload_entry(key)
            .expect("get remaining")
            .is_some());
    }

    for e in cutoff..count {
        let key = checkpoint_epoch_commitment(&payload_for_epoch(e));
        assert!(db
            .get_checkpoint_payload_entry(key)
            .expect("get deleted")
            .is_none());
    }
}

pub fn proptest_signing_entry_roundtrip(
    db: &impl OLCheckpointDatabase,
    checkpoint: CheckpointPayload,
    intent_index: L1PayloadIntentIndex,
) {
    let key = checkpoint_epoch_commitment(&checkpoint);

    db.put_checkpoint_payload_entry(key, checkpoint)
        .expect("test: put payload");
    db.put_checkpoint_signing_entry(key, intent_index)
        .expect("test: put signing");

    let stored = db
        .get_checkpoint_signing_entry(key)
        .expect("test: get signing")
        .expect("signing entry should exist");
    assert_eq!(stored, intent_index);

    let deleted = db
        .del_checkpoint_signing_entry(key)
        .expect("test: del signing");
    assert!(deleted);

    let stored = db
        .get_checkpoint_signing_entry(key)
        .expect("test: get signing");
    assert!(stored.is_none());
}

pub fn proptest_l1_observation_entry_roundtrip(
    db: &impl OLCheckpointDatabase,
    checkpoint: CheckpointPayload,
    observed_height: u32,
) {
    let key = checkpoint_epoch_commitment(&checkpoint);
    let observation = l1_observation_entry(observed_height);

    db.put_checkpoint_payload_entry(key, checkpoint)
        .expect("test: put payload");
    db.put_checkpoint_l1_observation_entry(key, observation)
        .expect("test: put l1 observation");

    let stored = db
        .get_checkpoint_l1_observation_entry(key)
        .expect("test: get l1 observation")
        .expect("l1 observation entry should exist");
    assert_eq!(stored, observation);
}

#[macro_export]
macro_rules! ol_checkpoint_db_tests {
    ($setup_expr:expr) => {
        use strata_checkpoint_types_ssz::test_utils as checkpoint_test_utils;

        #[test]
        fn test_get_nonexistent_checkpoint_payload_entry() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_get_nonexistent_checkpoint_payload_entry(&db);
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
        fn test_get_last_checkpoint_payload_epoch_empty() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_get_last_checkpoint_payload_epoch_empty(&db);
        }

        #[test]
        fn test_get_next_unsigned_checkpoint_epoch_empty() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_get_next_unsigned_checkpoint_epoch_empty(&db);
        }

        #[test]
        fn test_del_checkpoint_payload_entries_from_epoch_empty() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_del_checkpoint_payload_entries_from_epoch_empty(&db);
        }

        #[test]
        fn test_unsigned_epoch_index_mixed_operations() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_unsigned_epoch_index_mixed_operations(&db);
        }

        #[test]
        fn test_put_checkpoint_signing_entry_requires_payload() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_put_checkpoint_signing_entry_requires_payload(&db);
        }

        #[test]
        fn test_put_checkpoint_payload_entry_rejects_mismatched_commitment() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_put_checkpoint_payload_entry_rejects_mismatched_commitment(&db);
        }

        #[test]
        fn test_del_checkpoint_payload_entry_deletes_signing_entry() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_del_checkpoint_payload_entry_deletes_signing_entry(&db);
        }

        #[test]
        fn test_del_checkpoint_signing_entries_from_epoch() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_del_checkpoint_signing_entries_from_epoch(&db);
        }

        #[test]
        fn test_put_checkpoint_l1_observation_entry_allows_missing_payload() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_put_checkpoint_l1_observation_entry_allows_missing_payload(&db);
        }

        #[test]
        fn test_checkpoint_l1_observation_entry_roundtrip() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_checkpoint_l1_observation_entry_roundtrip(&db);
        }

        #[test]
        fn test_del_checkpoint_payload_entry_deletes_l1_observation_entry() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_del_checkpoint_payload_entry_deletes_l1_observation_entry(&db);
        }

        #[test]
        fn test_del_checkpoint_l1_observation_entries_from_epoch() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_del_checkpoint_l1_observation_entries_from_epoch(&db);
        }

        #[test]
        fn test_get_next_unsigned_checkpoint_epoch_all_signed_returns_none() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_get_next_unsigned_checkpoint_epoch_all_signed_returns_none(&db);
        }

        #[test]
        fn test_get_next_unsigned_checkpoint_epoch_non_contiguous() {
            let db = $setup_expr;
            $crate::ol_checkpoint_tests::test_get_next_unsigned_checkpoint_epoch_non_contiguous(&db);
        }

        proptest::proptest! {
            #[test]
            fn proptest_put_and_get_checkpoint_payload_entry(
                checkpoint in checkpoint_test_utils::checkpoint_payload_strategy()
            ) {
                let db = $setup_expr;
                $crate::ol_checkpoint_tests::proptest_put_and_get_checkpoint_payload_entry(&db, checkpoint);
            }

            #[test]
            fn proptest_put_twice_idempotent(
                checkpoint in checkpoint_test_utils::checkpoint_payload_strategy()
            ) {
                let db = $setup_expr;
                $crate::ol_checkpoint_tests::proptest_put_twice_idempotent(&db, checkpoint);
            }

            #[test]
            fn proptest_delete_checkpoint_payload_entry(
                checkpoint in checkpoint_test_utils::checkpoint_payload_strategy()
            ) {
                let db = $setup_expr;
                $crate::ol_checkpoint_tests::proptest_delete_checkpoint_payload_entry(&db, checkpoint);
            }

            #[test]
            fn proptest_get_last_checkpoint_payload_epoch(
                count in 1u32..10u32
            ) {
                let db = $setup_expr;
                $crate::ol_checkpoint_tests::proptest_get_last_checkpoint_payload_epoch(&db, count);
            }

            #[test]
            fn proptest_get_next_unsigned_checkpoint_epoch(
                intent_index in proptest::prelude::any::<u64>(),
                count in 2u32..10u32
            ) {
                let db = $setup_expr;
                $crate::ol_checkpoint_tests::proptest_get_next_unsigned_checkpoint_epoch(&db, intent_index, count);
            }

            #[test]
            fn proptest_del_checkpoint_payload_entries_from_epoch(
                count in 1u32..10u32,
                cutoff_ratio in 0.0f64..1.0f64
            ) {
                let cutoff = ((count as f64) * cutoff_ratio) as u32;
                let db = $setup_expr;
                $crate::ol_checkpoint_tests::proptest_del_checkpoint_payload_entries_from_epoch(&db, count, cutoff);
            }

            #[test]
            fn proptest_signing_entry_roundtrip(
                checkpoint in checkpoint_test_utils::checkpoint_payload_strategy(),
                intent_index in proptest::prelude::any::<u64>()
            ) {
                let db = $setup_expr;
                $crate::ol_checkpoint_tests::proptest_signing_entry_roundtrip(&db, checkpoint, intent_index);
            }

            #[test]
            fn proptest_l1_observation_entry_roundtrip(
                checkpoint in checkpoint_test_utils::checkpoint_payload_strategy(),
                observed_height in proptest::prelude::any::<u32>()
            ) {
                let db = $setup_expr;
                $crate::ol_checkpoint_tests::proptest_l1_observation_entry_roundtrip(&db, checkpoint, observed_height);
            }
        }
    };
}
