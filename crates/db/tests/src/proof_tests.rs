use strata_db_types::traits::{CheckpointProofDatabase, ProverTaskDatabase};
use strata_identifiers::EpochCommitment;
use strata_paas::{TaskRecordData, TaskStatus};
use zkaleido::{
    ProgramId, Proof, ProofMetadata, ProofReceipt, ProofReceiptWithMetadata, ProofType,
    PublicValues, ZkVm,
};

pub fn test_insert_new_proof(db: &impl CheckpointProofDatabase) {
    let (epoch, proof) = generate_proof();

    let result = db.put_proof(epoch, proof.clone());
    assert!(
        result.is_ok(),
        "ProofReceiptWithMetadata should be inserted successfully"
    );

    let stored_proof = db.get_proof(epoch).unwrap();
    assert_eq!(stored_proof, Some(proof));
}

pub fn test_insert_duplicate_proof(db: &impl CheckpointProofDatabase) {
    let (epoch, proof) = generate_proof();

    db.put_proof(epoch, proof.clone()).unwrap();

    let result = db.put_proof(epoch, proof);
    assert!(result.is_err(), "Duplicate proof insertion should fail");
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
fn generate_proof() -> (EpochCommitment, ProofReceiptWithMetadata) {
    let epoch = EpochCommitment::null();
    let proof = Proof::default();
    let public_values = PublicValues::default();
    let receipt = ProofReceipt::new(proof, public_values);
    let metadata = ProofMetadata::new(
        ZkVm::Native,
        ProgramId([0u8; 32]),
        "0.1".to_string(),
        ProofType::Groth16,
    );
    let proof_receipt = ProofReceiptWithMetadata::new(receipt, metadata);
    (epoch, proof_receipt)
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
        fn test_insert_duplicate_proof() {
            let db = $setup_expr;
            $crate::proof_tests::test_insert_duplicate_proof(&db);
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
