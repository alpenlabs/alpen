use strata_db::{
    traits::L1WriterDatabase,
    types::{BundledPayloadEntry, IntentEntry},
};
use strata_primitives::buf::Buf32;
use strata_test_utils::ArbitraryGenerator;

// ===== Payload Entry Tests =====

pub fn test_put_blob_new_entry(db: &impl L1WriterDatabase) {
    let blob: BundledPayloadEntry = ArbitraryGenerator::new().generate();

    db.put_payload_entry(0, blob.clone()).unwrap();

    let stored_blob = db.get_payload_entry_by_idx(0).unwrap();
    assert_eq!(stored_blob, Some(blob));
}

pub fn test_put_blob_existing_entry(db: &impl L1WriterDatabase) {
    let blob: BundledPayloadEntry = ArbitraryGenerator::new().generate();

    db.put_payload_entry(0, blob.clone()).unwrap();

    let result = db.put_payload_entry(0, blob);

    // Should be ok to put to existing key
    assert!(result.is_ok());
}

pub fn test_update_entry(db: &impl L1WriterDatabase) {
    let entry: BundledPayloadEntry = ArbitraryGenerator::new().generate();

    // Insert
    db.put_payload_entry(0, entry.clone()).unwrap();

    let updated_entry: BundledPayloadEntry = ArbitraryGenerator::new().generate();

    // Update existing idx
    db.put_payload_entry(0, updated_entry.clone()).unwrap();
    let retrieved_entry = db.get_payload_entry_by_idx(0).unwrap().unwrap();
    assert_eq!(updated_entry, retrieved_entry);
}

pub fn test_get_last_entry_idx(db: &impl L1WriterDatabase) {
    let blob: BundledPayloadEntry = ArbitraryGenerator::new().generate();

    let next_blob_idx = db.get_next_payload_idx().unwrap();
    assert_eq!(
        next_blob_idx, 0,
        "There is no last blobidx in the beginning"
    );

    db.put_payload_entry(next_blob_idx, blob.clone()).unwrap();
    // Now the next idx is 1

    let blob: BundledPayloadEntry = ArbitraryGenerator::new().generate();

    db.put_payload_entry(1, blob.clone()).unwrap();
    let next_blob_idx = db.get_next_payload_idx().unwrap();
    // Now the last idx is 2

    assert_eq!(next_blob_idx, 2);
}

pub fn test_del_payload_entry_single(db: &impl L1WriterDatabase) {
    let payload: BundledPayloadEntry = ArbitraryGenerator::new().generate();
    let idx = 5;

    // Insert payload
    db.put_payload_entry(idx, payload.clone())
        .expect("test: insert");

    // Verify it exists
    assert!(db
        .get_payload_entry_by_idx(idx)
        .expect("test: get")
        .is_some());

    // Delete it
    let deleted = db.del_payload_entry(idx).expect("test: delete");
    assert!(deleted, "Should return true when deleting existing payload");

    // Verify it's gone
    assert!(db
        .get_payload_entry_by_idx(idx)
        .expect("test: get after delete")
        .is_none());

    // Delete again should return false
    let deleted_again = db.del_payload_entry(idx).expect("test: delete again");
    assert!(
        !deleted_again,
        "Should return false when deleting non-existent payload"
    );
}

pub fn test_del_payload_entries_from_idx(db: &impl L1WriterDatabase) {
    let payload: BundledPayloadEntry = ArbitraryGenerator::new().generate();

    // Insert payloads at indices 1, 3, 5, 7
    db.put_payload_entry(1, payload.clone())
        .expect("test: insert 1");
    db.put_payload_entry(3, payload.clone())
        .expect("test: insert 3");
    db.put_payload_entry(5, payload.clone())
        .expect("test: insert 5");
    db.put_payload_entry(7, payload.clone())
        .expect("test: insert 7");

    // Delete from index 4 onwards
    let deleted_indices = db
        .del_payload_entries_from_idx(4)
        .expect("test: delete from idx 4");
    assert_eq!(deleted_indices, vec![5, 7], "Should delete indices 5 and 7");

    // Verify indices 1 and 3 still exist, indices 5 and 7 are gone
    assert!(db
        .get_payload_entry_by_idx(1)
        .expect("test: get 1")
        .is_some());
    assert!(db
        .get_payload_entry_by_idx(3)
        .expect("test: get 3")
        .is_some());
    assert!(db
        .get_payload_entry_by_idx(5)
        .expect("test: get 5")
        .is_none());
    assert!(db
        .get_payload_entry_by_idx(7)
        .expect("test: get 7")
        .is_none());

    // Delete from index 2 onwards
    let deleted_indices = db
        .del_payload_entries_from_idx(2)
        .expect("test: delete from idx 2");
    assert_eq!(deleted_indices, vec![3], "Should delete index 3");

    // Verify only index 1 remains
    assert!(db
        .get_payload_entry_by_idx(1)
        .expect("test: get 1 final")
        .is_some());
    assert!(db
        .get_payload_entry_by_idx(3)
        .expect("test: get 3 final")
        .is_none());
}

pub fn test_del_payload_entries_empty_database(db: &impl L1WriterDatabase) {
    // Delete from empty database should return empty vec
    let deleted_indices = db
        .del_payload_entries_from_idx(0)
        .expect("test: delete from empty");
    assert!(
        deleted_indices.is_empty(),
        "Should return empty vec for empty database"
    );
}

// ===== Intent Entry Tests =====

pub fn test_put_intent_new_entry(db: &impl L1WriterDatabase) {
    let intent: IntentEntry = ArbitraryGenerator::new().generate();
    let intent_id: Buf32 = [0; 32].into();

    db.put_intent_entry(intent_id, intent.clone()).unwrap();

    let stored_intent = db.get_intent_by_id(intent_id).unwrap();
    assert_eq!(stored_intent, Some(intent));
}

pub fn test_put_intent_entry(db: &impl L1WriterDatabase) {
    let intent: IntentEntry = ArbitraryGenerator::new().generate();
    let intent_id: Buf32 = [0; 32].into();

    let result = db.put_intent_entry(intent_id, intent.clone());
    assert!(result.is_ok());

    let retrieved = db.get_intent_by_id(intent_id).unwrap().unwrap();
    assert_eq!(retrieved, intent);
}

pub fn test_del_intent_entry_single(db: &impl L1WriterDatabase) {
    let intent: IntentEntry = ArbitraryGenerator::new().generate();
    let intent_id: Buf32 = [1; 32].into();

    // Insert intent
    db.put_intent_entry(intent_id, intent.clone())
        .expect("test: insert");

    // Verify it exists
    assert!(db.get_intent_by_id(intent_id).expect("test: get").is_some());

    // Delete it
    let deleted = db.del_intent_entry(intent_id).expect("test: delete");
    assert!(deleted, "Should return true when deleting existing intent");

    // Verify it's gone
    assert!(db
        .get_intent_by_id(intent_id)
        .expect("test: get after delete")
        .is_none());

    // Delete again should return false
    let deleted_again = db.del_intent_entry(intent_id).expect("test: delete again");
    assert!(
        !deleted_again,
        "Should return false when deleting non-existent intent"
    );
}

pub fn test_del_intent_entries_from_idx(db: &impl L1WriterDatabase) {
    let intent: IntentEntry = ArbitraryGenerator::new().generate();

    // Create different intent IDs
    let intent_id1: Buf32 = [1; 32].into();
    let intent_id2: Buf32 = [2; 32].into();
    let intent_id3: Buf32 = [3; 32].into();
    let intent_id4: Buf32 = [4; 32].into();

    // Insert intents - they will get consecutive indices
    db.put_intent_entry(intent_id1, intent.clone())
        .expect("test: insert 1");
    db.put_intent_entry(intent_id2, intent.clone())
        .expect("test: insert 2");
    db.put_intent_entry(intent_id3, intent.clone())
        .expect("test: insert 3");
    db.put_intent_entry(intent_id4, intent.clone())
        .expect("test: insert 4");

    // Verify all exist
    assert!(db.get_intent_by_idx(0).expect("test: get idx 0").is_some());
    assert!(db.get_intent_by_idx(1).expect("test: get idx 1").is_some());
    assert!(db.get_intent_by_idx(2).expect("test: get idx 2").is_some());
    assert!(db.get_intent_by_idx(3).expect("test: get idx 3").is_some());

    // Delete from index 2 onwards
    let deleted_indices = db
        .del_intent_entries_from_idx(2)
        .expect("test: delete from idx 2");
    assert_eq!(deleted_indices, vec![2, 3], "Should delete indices 2 and 3");

    // Verify indices 0 and 1 still exist, indices 2 and 3 are gone
    assert!(db
        .get_intent_by_idx(0)
        .expect("test: get idx 0 after")
        .is_some());
    assert!(db
        .get_intent_by_idx(1)
        .expect("test: get idx 1 after")
        .is_some());
    assert!(db
        .get_intent_by_idx(2)
        .expect("test: get idx 2 after")
        .is_none());
    assert!(db
        .get_intent_by_idx(3)
        .expect("test: get idx 3 after")
        .is_none());

    // Also verify the intent entries themselves are gone
    assert!(db
        .get_intent_by_id(intent_id3)
        .expect("test: get id 3")
        .is_none());
    assert!(db
        .get_intent_by_id(intent_id4)
        .expect("test: get id 4")
        .is_none());
}

pub fn test_del_intent_entries_empty_database(db: &impl L1WriterDatabase) {
    // Delete from empty database should return empty vec
    let deleted_indices = db
        .del_intent_entries_from_idx(0)
        .expect("test: delete from empty");
    assert!(
        deleted_indices.is_empty(),
        "Should return empty vec for empty database"
    );
}

#[macro_export]
macro_rules! l1_writer_db_tests {
    ($setup_expr:expr) => {
        #[test]
        fn test_put_blob_new_entry() {
            let db = $setup_expr;
            $crate::l1_writer_tests::test_put_blob_new_entry(&db);
        }

        #[test]
        fn test_put_blob_existing_entry() {
            let db = $setup_expr;
            $crate::l1_writer_tests::test_put_blob_existing_entry(&db);
        }

        #[test]
        fn test_update_entry() {
            let db = $setup_expr;
            $crate::l1_writer_tests::test_update_entry(&db);
        }

        #[test]
        fn test_get_last_entry_idx() {
            let db = $setup_expr;
            $crate::l1_writer_tests::test_get_last_entry_idx(&db);
        }

        #[test]
        fn test_del_payload_entry_single() {
            let db = $setup_expr;
            $crate::l1_writer_tests::test_del_payload_entry_single(&db);
        }

        #[test]
        fn test_del_payload_entries_from_idx() {
            let db = $setup_expr;
            $crate::l1_writer_tests::test_del_payload_entries_from_idx(&db);
        }

        #[test]
        fn test_del_payload_entries_empty_database() {
            let db = $setup_expr;
            $crate::l1_writer_tests::test_del_payload_entries_empty_database(&db);
        }

        #[test]
        fn test_put_intent_new_entry() {
            let db = $setup_expr;
            $crate::l1_writer_tests::test_put_intent_new_entry(&db);
        }

        #[test]
        fn test_put_intent_entry() {
            let db = $setup_expr;
            $crate::l1_writer_tests::test_put_intent_entry(&db);
        }

        #[test]
        fn test_del_intent_entry_single() {
            let db = $setup_expr;
            $crate::l1_writer_tests::test_del_intent_entry_single(&db);
        }

        #[test]
        fn test_del_intent_entries_from_idx() {
            let db = $setup_expr;
            $crate::l1_writer_tests::test_del_intent_entries_from_idx(&db);
        }

        #[test]
        fn test_del_intent_entries_empty_database() {
            let db = $setup_expr;
            $crate::l1_writer_tests::test_del_intent_entries_empty_database(&db);
        }
    };
}
