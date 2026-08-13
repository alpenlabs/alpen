use strata_db_types::checkpoint_proof::{CheckpointProofDatabase, ProofReceiptEntry};
use strata_db_types::prover_task::ProverTaskDatabase;
use strata_identifiers::EpochCommitment;
use strata_paas::{TaskRecordData, TaskStatus};

pub fn test_insert_new_proof(db: &impl CheckpointProofDatabase) {
    let (epoch, proof) = generate_proof();

    let result = db.put_proof(epoch, proof.clone());
    assert!(
        result.is_ok(),
        "proof receipt should be inserted successfully"
    );

    let stored_proof = db.get_proof(epoch).unwrap();
    assert_eq!(stored_proof, Some(proof));
}

pub fn test_put_proof_overwrites(db: &impl CheckpointProofDatabase) {
    let (epoch, first) = generate_proof();
    db.put_proof(epoch, first).unwrap();

    // Second put with a distinct receipt for the same epoch upserts.
    // Re-proves attest to the same statement, so overwriting is safe and
    // keeps the receipt hook idempotent.
    let second = distinct_proof();
    db.put_proof(epoch, second.clone()).unwrap();

    let stored = db.get_proof(epoch).unwrap();
    assert_eq!(stored, Some(second), "second put should replace the first");
}

pub fn test_get_nonexistent_proof(db: &impl CheckpointProofDatabase) {
    let (epoch, proof) = generate_proof();
    db.put_proof(epoch, proof.clone()).unwrap();

    let res = db.del_proof(epoch);
    assert!(matches!(res, Ok(true)));

    let res = db.del_proof(epoch);
    assert!(matches!(res, Ok(false)));

    let stored_proof = db.get_proof(epoch).unwrap();
    assert_eq!(stored_proof, None, "Nonexistent proof should return None");
}

pub fn test_delete_task_roundtrip(db: &impl ProverTaskDatabase) {
    let key = b"task-key-1".to_vec();
    let record = TaskRecordData::new(TaskStatus::Pending);

    // Deleting a missing key reports false.
    assert!(matches!(db.delete_task(key.clone()), Ok(false)));

    db.insert_task(key.clone(), record).unwrap();
    assert!(db.get_task(key.clone()).unwrap().is_some());

    // First delete reports true; second reports false.
    assert!(matches!(db.delete_task(key.clone()), Ok(true)));
    assert!(matches!(db.delete_task(key.clone()), Ok(false)));
    assert!(db.get_task(key).unwrap().is_none());
}

// Helper functions
//
// Receipts are opaque to the database, so these fixtures are plain byte payloads -- the
// storage layer must round-trip them without interpreting their contents.
fn generate_proof() -> (EpochCommitment, ProofReceiptEntry) {
    (
        EpochCommitment::null(),
        ProofReceiptEntry::new(vec![0xA5; 24]),
    )
}

/// Distinct payload so equality comparisons can prove the upsert actually replaced the row
/// rather than being a no-op.
fn distinct_proof() -> ProofReceiptEntry {
    ProofReceiptEntry::new(vec![0x5A; 32])
}

#[macro_export]
macro_rules! proof_db_tests {
    ($setup_expr:expr) => {
        #[test]
        fn test_insert_new_proof() {
            let db = $setup_expr;
            $crate::proof_tests::test_insert_new_proof(&db);
        }

        #[test]
        fn test_put_proof_overwrites() {
            let db = $setup_expr;
            $crate::proof_tests::test_put_proof_overwrites(&db);
        }

        #[test]
        fn test_get_nonexistent_proof() {
            let db = $setup_expr;
            $crate::proof_tests::test_get_nonexistent_proof(&db);
        }

        #[test]
        fn test_delete_task_roundtrip() {
            let db = $setup_expr;
            $crate::proof_tests::test_delete_task_roundtrip(&db);
        }
    };
}
