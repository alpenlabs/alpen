use strata_db::{errors::DbError, traits::ClientStateDatabase};
use strata_state::operation::ClientUpdateOutput;
use strata_test_utils::ArbitraryGenerator;

pub fn test_write_consensus_output(db: &impl ClientStateDatabase) {
    let output: ClientUpdateOutput = ArbitraryGenerator::new().generate();

    let res = db.put_client_update(2, output.clone());
    assert!(matches!(res, Err(DbError::OooInsert("consensus_store", 2))));

    db.put_client_update(0, output.clone())
        .expect("test: insert");

    let res = db.put_client_update(0, output.clone());
    assert!(matches!(res, Err(DbError::OooInsert("consensus_store", 0))));

    let res = db.put_client_update(2, output.clone());
    assert!(matches!(res, Err(DbError::OooInsert("consensus_store", 2))));
}

pub fn test_get_last_write_idx(db: &impl ClientStateDatabase) {
    let idx = db.get_last_state_idx();
    assert!(matches!(idx, Err(DbError::NotBootstrapped)));

    let output: ClientUpdateOutput = ArbitraryGenerator::new().generate();
    db.put_client_update(0, output.clone())
        .expect("test: insert");
    db.put_client_update(1, output.clone())
        .expect("test: insert");

    let idx = db.get_last_state_idx().expect("test: get last");
    assert_eq!(idx, 1);
}

pub fn test_get_consensus_update(db: &impl ClientStateDatabase) {
    let output: ClientUpdateOutput = ArbitraryGenerator::new().generate();

    db.put_client_update(0, output.clone())
        .expect("test: insert");

    db.put_client_update(1, output.clone())
        .expect("test: insert");

    let update = db.get_client_update(1).expect("test: get").unwrap();
    assert_eq!(update, output);
}

#[macro_export]
macro_rules! client_state_db_tests {
    ($setup_expr:expr) => {
        #[test]
        fn test_write_consensus_output() {
            let db = $setup_expr;
            $crate::client_state_tests::test_write_consensus_output(&db);
        }

        #[test]
        fn test_get_last_write_idx() {
            let db = $setup_expr;
            $crate::client_state_tests::test_get_last_write_idx(&db);
        }

        #[test]
        fn test_get_consensus_update() {
            let db = $setup_expr;
            $crate::client_state_tests::test_get_consensus_update(&db);
        }
    };
}
