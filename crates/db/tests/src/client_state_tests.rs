use strata_csm_types::ClientUpdateOutput;
use strata_db_types::traits::ClientStateDatabase;
use strata_primitives::l1::{L1BlockCommitment, L1BlockId};
use strata_test_utils::ArbitraryGenerator;

pub fn test_get_consensus_update(db: &impl ClientStateDatabase) {
    let output: ClientUpdateOutput = ArbitraryGenerator::new().generate();

    db.put_client_update(L1BlockCommitment::default(), output.clone())
        .expect("test: insert");

    let another_block = L1BlockCommitment::from_height_u64(1, L1BlockId::default())
        .expect("height should be valid");
    db.put_client_update(another_block, output.clone())
        .expect("test: insert");

    let update = db
        .get_client_update(another_block)
        .expect("test: get")
        .unwrap();
    assert_eq!(update, output);
}

// TODO(QQ): add more tests.
#[macro_export]
macro_rules! client_state_db_tests {
    ($setup_expr:expr) => {
        #[test]
        fn test_get_consensus_update() {
            let db = $setup_expr;
            $crate::client_state_tests::test_get_consensus_update(&db);
        }
    };
}
