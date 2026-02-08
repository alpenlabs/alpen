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

pub fn test_client_state_ordering_over_256(db: &impl ClientStateDatabase) {
    let output: ClientUpdateOutput = ArbitraryGenerator::new().generate();
    let max_height = 300u64;

    for height in 0..=max_height {
        let block = L1BlockCommitment::from_height_u64(height, L1BlockId::default())
            .expect("height should be valid");
        db.put_client_update(block, output.clone())
            .expect("test: insert");
    }

    let (latest_block, _) = db
        .get_latest_client_state()
        .expect("test: get latest")
        .expect("latest should exist");
    assert_eq!(latest_block.height_u64(), max_height);

    let start_height = 100u64;
    let start_block = L1BlockCommitment::from_height_u64(start_height, L1BlockId::default())
        .expect("height should be valid");
    let updates = db
        .get_client_updates_from(start_block, 50)
        .expect("test: range");
    assert_eq!(updates.len(), 50);
    assert_eq!(updates.first().unwrap().0.height_u64(), start_height);

    let mut last_height = start_height;
    for (block, _) in updates {
        let height = block.height_u64();
        assert!(height >= last_height);
        last_height = height;
    }
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

        #[test]
        fn test_client_state_ordering_over_256() {
            let db = $setup_expr;
            $crate::client_state_tests::test_client_state_ordering_over_256(&db);
        }
    };
}
