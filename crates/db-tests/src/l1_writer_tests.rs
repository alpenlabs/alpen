use strata_db::traits::L1WriterDatabase;
use strata_db::types::{BundledPayloadEntry, IntentEntry};
use strata_primitives::buf::Buf32;
use strata_test_utils::ArbitraryGenerator;

pub fn test_put_blob_new_entry<T: L1WriterDatabase>(db: &T) {
    let blob: BundledPayloadEntry = ArbitraryGenerator::new().generate();

    db.put_payload_entry(0, blob.clone()).unwrap();

    let stored_blob = db.get_payload_entry_by_idx(0).unwrap();
    assert_eq!(stored_blob, Some(blob));
}

pub fn test_put_blob_existing_entry<T: L1WriterDatabase>(db: &T) {
    let blob: BundledPayloadEntry = ArbitraryGenerator::new().generate();

    db.put_payload_entry(0, blob.clone()).unwrap();

    let result = db.put_payload_entry(0, blob);

    // Should be ok to put to existing key
    assert!(result.is_ok());
}

pub fn test_update_entry<T: L1WriterDatabase>(db: &T) {
    let entry: BundledPayloadEntry = ArbitraryGenerator::new().generate();

    // Insert
    db.put_payload_entry(0, entry.clone()).unwrap();

    let updated_entry: BundledPayloadEntry = ArbitraryGenerator::new().generate();

    // Update existing idx
    db.put_payload_entry(0, updated_entry.clone()).unwrap();
    let retrieved_entry = db.get_payload_entry_by_idx(0).unwrap().unwrap();
    assert_eq!(updated_entry, retrieved_entry);
}

pub fn test_get_last_entry_idx<T: L1WriterDatabase>(db: &T) {
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

pub fn test_put_intent_new_entry<T: L1WriterDatabase>(db: &T) {
    let intent: IntentEntry = ArbitraryGenerator::new().generate();
    let intent_id: Buf32 = [0; 32].into();

    db.put_intent_entry(intent_id, intent.clone()).unwrap();

    let stored_intent = db.get_intent_by_id(intent_id).unwrap();
    assert_eq!(stored_intent, Some(intent));
}

pub fn test_put_intent_entry<T: L1WriterDatabase>(db: &T) {
    let intent: IntentEntry = ArbitraryGenerator::new().generate();
    let intent_id: Buf32 = [0; 32].into();

    let result = db.put_intent_entry(intent_id, intent.clone());
    assert!(result.is_ok());

    let retrieved = db.get_intent_by_id(intent_id).unwrap().unwrap();
    assert_eq!(retrieved, intent);
}