use std::sync::Arc;

use rockbound::{rocksdb, OptimisticTransactionDB};
use tempfile::TempDir;

use crate::{
    broadcaster::db::L1BroadcastDb, l2::db::L2Db, AsmDb, ChainstateDb, ClientStateDb, DbOpsConfig,
    L1Db, ProofDb, RBCheckpointDB, RBL1WriterDb, RocksDbBackend,
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

pub fn get_rocksdb_backend() -> Arc<RocksDbBackend> {
    let (rbdb, db_ops) = get_rocksdb_tmp_instance().unwrap();
    let asm_db = Arc::new(AsmDb::new(rbdb.clone(), db_ops));
    let l1_db = Arc::new(L1Db::new(rbdb.clone(), db_ops));
    let l2_db = Arc::new(L2Db::new(rbdb.clone(), db_ops));
    let cs_db = Arc::new(ClientStateDb::new(rbdb.clone(), db_ops));
    let chst_db = Arc::new(ChainstateDb::new(rbdb.clone(), db_ops));
    let chpt_db = Arc::new(RBCheckpointDB::new(rbdb.clone(), db_ops));
    let writer_db = Arc::new(RBL1WriterDb::new(rbdb.clone(), db_ops));
    let prover_db = Arc::new(ProofDb::new(rbdb.clone(), db_ops));
    let broadcast_db = Arc::new(L1BroadcastDb::new(rbdb, db_ops));
    Arc::new(RocksDbBackend::new(
        asm_db,
        l1_db,
        l2_db,
        cs_db,
        chst_db,
        chpt_db,
        writer_db,
        prover_db,
        broadcast_db,
    ))
}
