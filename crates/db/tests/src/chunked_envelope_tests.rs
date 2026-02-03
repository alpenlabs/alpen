use strata_db_types::{
    traits::L1ChunkedEnvelopeDatabase,
    types::{ChunkedEnvelopeEntry, ChunkedEnvelopeStatus},
};
use strata_primitives::buf::Buf32;

fn make_entry(status: ChunkedEnvelopeStatus) -> ChunkedEnvelopeEntry {
    let mut entry = ChunkedEnvelopeEntry::new_unsigned(
        vec![vec![0xAA; 100]],
        [0xDE, 0xAD, 0xBE, 0xEF],
        Buf32::zero(),
    );
    entry.status = status;
    entry
}

pub fn test_put_and_get_entry(db: &impl L1ChunkedEnvelopeDatabase) {
    let entry = make_entry(ChunkedEnvelopeStatus::Unsigned);

    db.put_chunked_envelope_entry(0, entry.clone()).unwrap();

    let stored = db.get_chunked_envelope_entry(0).unwrap();
    assert_eq!(stored, Some(entry));
}

pub fn test_put_existing_entry(db: &impl L1ChunkedEnvelopeDatabase) {
    let entry = make_entry(ChunkedEnvelopeStatus::Unsigned);
    db.put_chunked_envelope_entry(0, entry).unwrap();

    let updated = make_entry(ChunkedEnvelopeStatus::Unpublished);
    db.put_chunked_envelope_entry(0, updated.clone()).unwrap();

    let stored = db.get_chunked_envelope_entry(0).unwrap().unwrap();
    assert_eq!(stored, updated);
}

pub fn test_get_nonexistent_entry(db: &impl L1ChunkedEnvelopeDatabase) {
    let result = db.get_chunked_envelope_entry(42).unwrap();
    assert!(result.is_none());
}

pub fn test_get_next_idx_empty(db: &impl L1ChunkedEnvelopeDatabase) {
    let idx = db.get_next_chunked_envelope_idx().unwrap();
    assert_eq!(idx, 0);
}

pub fn test_get_next_idx_sequential(db: &impl L1ChunkedEnvelopeDatabase) {
    let entry = make_entry(ChunkedEnvelopeStatus::Unsigned);

    db.put_chunked_envelope_entry(0, entry.clone()).unwrap();
    assert_eq!(db.get_next_chunked_envelope_idx().unwrap(), 1);

    db.put_chunked_envelope_entry(1, entry.clone()).unwrap();
    assert_eq!(db.get_next_chunked_envelope_idx().unwrap(), 2);
}

pub fn test_del_entry_single(db: &impl L1ChunkedEnvelopeDatabase) {
    let entry = make_entry(ChunkedEnvelopeStatus::Unsigned);
    db.put_chunked_envelope_entry(5, entry).unwrap();

    assert!(db.get_chunked_envelope_entry(5).unwrap().is_some());

    let deleted = db.del_chunked_envelope_entry(5).unwrap();
    assert!(deleted, "should return true for existing entry");

    assert!(db.get_chunked_envelope_entry(5).unwrap().is_none());

    let deleted_again = db.del_chunked_envelope_entry(5).unwrap();
    assert!(!deleted_again, "should return false for non-existent entry");
}

pub fn test_del_entries_from_idx(db: &impl L1ChunkedEnvelopeDatabase) {
    let entry = make_entry(ChunkedEnvelopeStatus::Unsigned);

    // Insert at indices 1, 3, 5, 7.
    db.put_chunked_envelope_entry(1, entry.clone()).unwrap();
    db.put_chunked_envelope_entry(3, entry.clone()).unwrap();
    db.put_chunked_envelope_entry(5, entry.clone()).unwrap();
    db.put_chunked_envelope_entry(7, entry.clone()).unwrap();

    // Delete from 4 onwards — should remove 5 and 7.
    let deleted = db.del_chunked_envelope_entries_from_idx(4).unwrap();
    assert_eq!(deleted, vec![5, 7]);

    assert!(db.get_chunked_envelope_entry(1).unwrap().is_some());
    assert!(db.get_chunked_envelope_entry(3).unwrap().is_some());
    assert!(db.get_chunked_envelope_entry(5).unwrap().is_none());
    assert!(db.get_chunked_envelope_entry(7).unwrap().is_none());
}

pub fn test_del_entries_from_idx_empty(db: &impl L1ChunkedEnvelopeDatabase) {
    let deleted = db.del_chunked_envelope_entries_from_idx(0).unwrap();
    assert!(deleted.is_empty());
}

#[macro_export]
macro_rules! l1_chunked_envelope_db_tests {
    ($setup_expr:expr) => {
        #[test]
        fn test_put_and_get_entry() {
            let db = $setup_expr;
            $crate::chunked_envelope_tests::test_put_and_get_entry(&db);
        }

        #[test]
        fn test_put_existing_entry() {
            let db = $setup_expr;
            $crate::chunked_envelope_tests::test_put_existing_entry(&db);
        }

        #[test]
        fn test_get_nonexistent_entry() {
            let db = $setup_expr;
            $crate::chunked_envelope_tests::test_get_nonexistent_entry(&db);
        }

        #[test]
        fn test_get_next_idx_empty() {
            let db = $setup_expr;
            $crate::chunked_envelope_tests::test_get_next_idx_empty(&db);
        }

        #[test]
        fn test_get_next_idx_sequential() {
            let db = $setup_expr;
            $crate::chunked_envelope_tests::test_get_next_idx_sequential(&db);
        }

        #[test]
        fn test_del_entry_single() {
            let db = $setup_expr;
            $crate::chunked_envelope_tests::test_del_entry_single(&db);
        }

        #[test]
        fn test_del_entries_from_idx() {
            let db = $setup_expr;
            $crate::chunked_envelope_tests::test_del_entries_from_idx(&db);
        }

        #[test]
        fn test_del_entries_from_idx_empty() {
            let db = $setup_expr;
            $crate::chunked_envelope_tests::test_del_entries_from_idx_empty(&db);
        }
    };
}
