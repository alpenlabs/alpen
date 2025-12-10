use strata_db_types::traits::ProofDatabase;
use strata_primitives::{
    buf::Buf32,
    evm_exec::EvmEeBlockCommitment,
    l2::L2BlockCommitment,
    proof::{ProofContext, ProofKey, ProofZkVm},
};
use zkaleido::{Proof, ProofMetadata, ProofReceipt, ProofReceiptWithMetadata, PublicValues, ZkVm};

pub fn test_insert_new_proof(db: &impl ProofDatabase) {
    let (proof_key, proof) = generate_proof();

    let result = db.put_proof(proof_key, proof.clone());
    assert!(
        result.is_ok(),
        "ProofReceiptWithMetadata should be inserted successfully"
    );

    let stored_proof = db.get_proof(&proof_key).unwrap();
    assert_eq!(stored_proof, Some(proof));
}

pub fn test_insert_duplicate_proof(db: &impl ProofDatabase) {
    let (proof_key, proof) = generate_proof();

    db.put_proof(proof_key, proof.clone()).unwrap();

    let result = db.put_proof(proof_key, proof);
    assert!(result.is_err(), "Duplicate proof insertion should fail");
}

pub fn test_get_nonexistent_proof(db: &impl ProofDatabase) {
    let (proof_key, proof) = generate_proof();
    db.put_proof(proof_key, proof.clone()).unwrap();

    let res = db.del_proof(proof_key);
    assert!(matches!(res, Ok(true)));

    let res = db.del_proof(proof_key);
    assert!(matches!(res, Ok(false)));

    let stored_proof = db.get_proof(&proof_key).unwrap();
    assert_eq!(stored_proof, None, "Nonexistent proof should return None");
}

pub fn test_insert_new_deps(db: &impl ProofDatabase) {
    let (proof_context, deps) = generate_proof_context_with_deps();

    let result = db.put_proof_deps(proof_context, deps.clone());
    assert!(
        result.is_ok(),
        "ProofReceiptWithMetadata should be inserted successfully"
    );

    let stored_deps = db.get_proof_deps(proof_context).unwrap();
    assert_eq!(stored_deps, Some(deps));
}

pub fn test_insert_duplicate_proof_deps(db: &impl ProofDatabase) {
    let (proof_context, deps) = generate_proof_context_with_deps();

    db.put_proof_deps(proof_context, deps.clone()).unwrap();

    let result = db.put_proof_deps(proof_context, deps);
    assert!(
        result.is_err(),
        "Duplicate proof deps insertion should fail"
    );
}

pub fn test_get_nonexistent_proof_deps(db: &impl ProofDatabase) {
    let (proof_context, deps) = generate_proof_context_with_deps();
    db.put_proof_deps(proof_context, deps.clone()).unwrap();

    let res = db.del_proof_deps(proof_context);
    assert!(matches!(res, Ok(true)));

    let res = db.del_proof_deps(proof_context);
    assert!(matches!(res, Ok(false)));

    let stored_proof = db.get_proof_deps(proof_context).unwrap();
    assert_eq!(
        stored_proof, None,
        "Nonexistent proof deps should return None"
    );
}

// Helper functions
fn generate_proof() -> (ProofKey, ProofReceiptWithMetadata) {
    let proof_context = ProofContext::OLStf(L2BlockCommitment::null(), L2BlockCommitment::null());
    let host = ProofZkVm::Native;
    let proof_key = ProofKey::new(proof_context, host);
    let proof = Proof::default();
    let public_values = PublicValues::default();
    let receipt = ProofReceipt::new(proof, public_values);
    let metadata = ProofMetadata::new(ZkVm::Native, "0.1".to_string());
    let proof_receipt = ProofReceiptWithMetadata::new(receipt, metadata);
    (proof_key, proof_receipt)
}

fn generate_proof_context_with_deps() -> (ProofContext, Vec<ProofContext>) {
    // Constants for test block IDs
    const BLOCK_1_ID: [u8; 32] = [1u8; 32];
    const BLOCK_2_ID: [u8; 32] = [2u8; 32];

    // Create block IDs
    let evm_block_1 = Buf32::from(BLOCK_1_ID);
    let evm_block_2 = Buf32::from(BLOCK_2_ID);

    // Create L2 block commitments
    let evm_commitment_1 = EvmEeBlockCommitment::new(1, evm_block_1);
    let evm_commitment_2 = EvmEeBlockCommitment::new(2, evm_block_2);

    // Create main proof context
    let main_context = ProofContext::OLStf(L2BlockCommitment::null(), L2BlockCommitment::null());

    // Create dependency proof contexts
    let deps = vec![ProofContext::EvmEeStf(evm_commitment_1, evm_commitment_2)];

    (main_context, deps)
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
        fn test_insert_new_deps() {
            let db = $setup_expr;
            $crate::proof_tests::test_insert_new_deps(&db);
        }

        #[test]
        fn test_insert_duplicate_proof_deps() {
            let db = $setup_expr;
            $crate::proof_tests::test_insert_duplicate_proof_deps(&db);
        }

        #[test]
        fn test_get_nonexistent_proof_deps() {
            let db = $setup_expr;
            $crate::proof_tests::test_get_nonexistent_proof_deps(&db);
        }
    };
}
