use std::sync::Arc;

use typed_sled::SledDb;

use crate::{
    ChainstateDBSled, CheckpointDBSled, ClientStateDBSled, L1BroadcastDBSled, L1DBSled,
    L1WriterDBSled, L2DBSled, SledBackend, SledDbConfig, prover::ProofDBSled,
    sync_event::SyncEventDBSled,
};

pub fn get_test_sled_db() -> SledDb {
    let db = sled::Config::new().temporary(true).open().unwrap();
    SledDb::new(db).unwrap()
}

pub fn get_test_sled_config() -> SledDbConfig {
    SledDbConfig::new_with_constant_backoff(3, 200)
}

pub fn get_test_sled_backend() -> Arc<SledBackend> {
    let sdb = Arc::new(get_test_sled_db());
    let cnf = get_test_sled_config();
    let l1_db = Arc::new(L1DBSled::new(sdb.clone(), cnf.clone()).unwrap());
    let l2_db = Arc::new(L2DBSled::new(sdb.clone(), cnf.clone()).unwrap());
    let sync_ev_db = Arc::new(SyncEventDBSled::new(sdb.clone(), cnf.clone()).unwrap());
    let cs_db = Arc::new(ClientStateDBSled::new(sdb.clone(), cnf.clone()).unwrap());
    let chst_db = Arc::new(ChainstateDBSled::new(sdb.clone(), cnf.clone()).unwrap());
    let chpt_db = Arc::new(CheckpointDBSled::new(sdb.clone(), cnf.clone()).unwrap());
    let writer_db = Arc::new(L1WriterDBSled::new(sdb.clone(), cnf.clone()).unwrap());
    let prover_db = Arc::new(ProofDBSled::new(sdb.clone(), cnf.clone()).unwrap());
    let bcast_db = Arc::new(L1BroadcastDBSled::new(sdb, cnf).unwrap());
    Arc::new(SledBackend::new(
        l1_db, l2_db, sync_ev_db, cs_db, chst_db, chpt_db, writer_db, prover_db, bcast_db,
    ))
}
