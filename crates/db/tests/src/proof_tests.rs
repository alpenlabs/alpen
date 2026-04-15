use strata_db_types::traits::CheckpointProofDatabase;
use strata_identifiers::EpochCommitment;
use zkaleido::{Proof, ProofMetadata, ProofReceipt, ProofReceiptWithMetadata, PublicValues, ZkVm};

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

// Helper functions
fn generate_proof() -> (EpochCommitment, ProofReceiptWithMetadata) {
    let epoch = EpochCommitment::null();
    let proof = Proof::default();
    let public_values = PublicValues::default();
    let receipt = ProofReceipt::new(proof, public_values);
    let metadata = ProofMetadata::new(ZkVm::Native, "0.1".to_string());
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
    };
}
