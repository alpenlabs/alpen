use strata_db::traits::ClientStateDatabase;
use strata_primitives::l1::{L1BlockCommitment, L1BlockId};
use strata_state::operation::ClientUpdateOutput;
use strata_test_utils::ArbitraryGenerator;

pub fn test_get_consensus_update(db: &impl ClientStateDatabase) {
    let output: ClientUpdateOutput = ArbitraryGenerator::new().generate();

    db.put_client_update(L1BlockCommitment::default(), output.clone())
        .expect("test: insert");

    let another_block = L1BlockCommitment::new(1, L1BlockId::default());
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
