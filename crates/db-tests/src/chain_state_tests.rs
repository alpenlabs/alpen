use strata_db::traits::ChainstateDatabase;
use strata_db::errors::DbError;
use strata_state::{
    chain_state::Chainstate,
    id::L2BlockId,
    state_op::{WriteBatch, WriteBatchEntry},
};
use strata_test_utils::ArbitraryGenerator;

pub fn test_write_genesis_state<T: ChainstateDatabase>(db: &T) {
    let mut generator = ArbitraryGenerator::new();
    let genesis_state: Chainstate = generator.generate();
    let genesis_blockid: L2BlockId = generator.generate();

    let res = db.get_earliest_write_idx();
    assert!(res.is_err_and(|x| matches!(x, DbError::NotBootstrapped)));

    let res = db.get_last_write_idx();
    assert!(res.is_err_and(|x| matches!(x, DbError::NotBootstrapped)));

    let res = db.write_genesis_state(genesis_state.clone(), genesis_blockid);
    assert!(res.is_ok());

    let res = db.get_earliest_write_idx();
    assert!(res.is_ok_and(|x| matches!(x, 0)));

    let res = db.get_last_write_idx();
    assert!(res.is_ok_and(|x| matches!(x, 0)));

    let res = db.write_genesis_state(genesis_state, genesis_blockid);
    assert!(res.is_err_and(|x| matches!(x, DbError::OverwriteStateUpdate(0))));
}

pub fn test_write_state_update<T: ChainstateDatabase>(db: &T) {
    let mut generator = ArbitraryGenerator::new();
    let genesis_state: Chainstate = generator.generate();
    let genesis_blockid: L2BlockId = generator.generate();
    let batch = WriteBatch::new_replace(genesis_state.clone());

    let res = db.put_write_batch(1, WriteBatchEntry::new(batch.clone(), genesis_blockid));
    assert!(res.is_err_and(|x| matches!(x, DbError::NotBootstrapped)));

    db.write_genesis_state(genesis_state, genesis_blockid)
        .unwrap();

    let res = db.put_write_batch(1, WriteBatchEntry::new(batch.clone(), generator.generate()));
    assert!(res.is_ok());

    let res = db.put_write_batch(2, WriteBatchEntry::new(batch.clone(), generator.generate()));
    assert!(res.is_ok());

    let res = db.put_write_batch(2, WriteBatchEntry::new(batch.clone(), generator.generate()));
    assert!(res.is_err_and(|x| matches!(x, DbError::OverwriteStateUpdate(2))));

    let res = db.put_write_batch(4, WriteBatchEntry::new(batch.clone(), generator.generate()));
    assert!(res.is_err_and(|x| matches!(x, DbError::OooInsert("Chainstate", 4))));
}

pub fn test_get_earliest_and_last_state_idx<T: ChainstateDatabase>(db: &T) {
    let mut generator = ArbitraryGenerator::new();
    let genesis_state: Chainstate = generator.generate();
    let genesis_blockid: L2BlockId = generator.generate();

    let batch = WriteBatch::new_replace(genesis_state.clone());

    db.write_genesis_state(genesis_state, genesis_blockid)
        .unwrap();
    for i in 1..=5 {
        assert_eq!(db.get_earliest_write_idx().unwrap(), 0);
        db.put_write_batch(i, WriteBatchEntry::new(batch.clone(), generator.generate()))
            .unwrap();
        assert_eq!(db.get_last_write_idx().unwrap(), i);
    }
}

pub fn test_purge<T: ChainstateDatabase>(db: &T) {
    let mut generator = ArbitraryGenerator::new();
    let genesis_state: Chainstate = ArbitraryGenerator::new().generate();
    let batch = WriteBatch::new_replace(genesis_state.clone());

    db.write_genesis_state(genesis_state, generator.generate())
        .unwrap();
    for i in 1..=5 {
        assert_eq!(db.get_earliest_write_idx().unwrap(), 0);
        db.put_write_batch(i, WriteBatchEntry::new(batch.clone(), generator.generate()))
            .unwrap();
        assert_eq!(db.get_last_write_idx().unwrap(), i);
    }

    db.purge_entries_before(3).unwrap();
    // Ensure that calling the purge again does not fail
    db.purge_entries_before(3).unwrap();

    assert_eq!(db.get_earliest_write_idx().unwrap(), 3);
    assert_eq!(db.get_last_write_idx().unwrap(), 5);

    for i in 0..3 {
        assert!(db.get_write_batch(i).unwrap().is_none());
    }

    for i in 3..=5 {
        assert!(db.get_write_batch(i).unwrap().is_some());
    }

    let res = db.purge_entries_before(2);
    assert!(res.is_err_and(|x| matches!(x, DbError::MissingL2State(2))));

    let res = db.purge_entries_before(1);
    assert!(res.is_err_and(|x| matches!(x, DbError::MissingL2State(1))));
}

pub fn test_rollback<T: ChainstateDatabase>(db: &T) {
    let mut generator = ArbitraryGenerator::new();
    let genesis_state: Chainstate = generator.generate();
    let batch = WriteBatch::new_replace(genesis_state.clone());

    db.write_genesis_state(genesis_state, generator.generate())
        .unwrap();
    for i in 1..=5 {
        db.put_write_batch(i, WriteBatchEntry::new(batch.clone(), generator.generate()))
            .unwrap();
    }

    db.rollback_writes_to(3).unwrap();
    // Ensures that calling the rollback again does not fail
    db.rollback_writes_to(3).unwrap();

    for i in 4..=5 {
        assert!(db.get_write_batch(i).unwrap().is_none());
    }

    // For genesis there is no BatchWrites
    for i in 1..=3 {
        assert!(db.get_write_batch(i).unwrap().is_some());
    }

    assert_eq!(db.get_earliest_write_idx().unwrap(), 0);
    assert_eq!(db.get_last_write_idx().unwrap(), 3);

    let res = db.rollback_writes_to(5);
    assert!(res.is_err_and(|x| matches!(x, DbError::RevertAboveCurrent(5, 3))));

    let res = db.rollback_writes_to(4);
    assert!(res.is_err_and(|x| matches!(x, DbError::RevertAboveCurrent(4, 3))));

    let res = db.rollback_writes_to(3);
    assert!(res.is_ok());

    db.rollback_writes_to(2).unwrap();
    assert_eq!(db.get_earliest_write_idx().unwrap(), 0);
    assert_eq!(db.get_last_write_idx().unwrap(), 2);
}

pub fn test_purge_and_rollback<T: ChainstateDatabase>(db: &T) {
    let mut generator = ArbitraryGenerator::new();
    let genesis_state: Chainstate = generator.generate();
    let batch = WriteBatch::new_replace(genesis_state.clone());

    db.write_genesis_state(genesis_state, generator.generate())
        .unwrap();
    for i in 1..=5 {
        db.put_write_batch(i, WriteBatchEntry::new(batch.clone(), generator.generate()))
            .unwrap();
    }

    db.purge_entries_before(3).unwrap();

    let res = db.rollback_writes_to(3);
    assert!(res.is_ok());

    let res = db.rollback_writes_to(2);
    assert!(res.is_err_and(|x| matches!(x, DbError::MissingL2State(2))));
}