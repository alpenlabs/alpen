use std::sync::Arc;

use rockbound::{rocksdb, OptimisticTransactionDB};
use strata_db::database::CommonDatabase;
use tempfile::TempDir;

use crate::{
    l2::db::L2Db, ChainstateDb, ClientStateDb, DbOpsConfig, L1Db, RBCheckpointDB, SyncEventDb,
};

pub fn get_rocksdb_tmp_instance() -> anyhow::Result<(Arc<OptimisticTransactionDB>, DbOpsConfig)> {
    let cfs = crate::STORE_COLUMN_FAMILIES;
    get_rocksdb_tmp_instance_core(cfs)
}

pub fn get_rocksdb_tmp_instance_for_prover(
) -> anyhow::Result<(Arc<OptimisticTransactionDB>, DbOpsConfig)> {
    let cfs = crate::PROVER_COLUMN_FAMILIES;
    get_rocksdb_tmp_instance_core(cfs)
}

fn get_rocksdb_tmp_instance_core(
    cfs: &[&str],
) -> anyhow::Result<(Arc<OptimisticTransactionDB>, DbOpsConfig)> {
    let dbname = crate::ROCKSDB_NAME;
    let mut opts = rocksdb::Options::default();

    opts.create_missing_column_families(true);
    opts.create_if_missing(true);

    let temp_dir = TempDir::new().expect("failed to create temp dir");

    let rbdb = rockbound::OptimisticTransactionDB::open(
        temp_dir.keep(),
        dbname,
        cfs.iter().map(|s| s.to_string()),
        &opts,
    )?;

    let db_ops = DbOpsConfig { retry_count: 5 };

    Ok((Arc::new(rbdb), db_ops))
}

pub fn get_common_db(
) -> Arc<CommonDatabase<L1Db, L2Db, SyncEventDb, ClientStateDb, ChainstateDb, RBCheckpointDB>> {
    let (rbdb, db_ops) = get_rocksdb_tmp_instance().unwrap();
    let l1_db = Arc::new(L1Db::new(rbdb.clone(), db_ops));
    let l2_db = Arc::new(L2Db::new(rbdb.clone(), db_ops));
    let sync_ev_db = Arc::new(SyncEventDb::new(rbdb.clone(), db_ops));
    let cs_db = Arc::new(ClientStateDb::new(rbdb.clone(), db_ops));
    let chst_db = Arc::new(ChainstateDb::new(rbdb.clone(), db_ops));
    let chpt_db = Arc::new(RBCheckpointDB::new(rbdb.clone(), db_ops));
    Arc::new(CommonDatabase::new(
        l1_db, l2_db, sync_ev_db, cs_db, chst_db, chpt_db,
    ))
}
