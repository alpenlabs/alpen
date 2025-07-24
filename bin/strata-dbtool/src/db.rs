use std::{path::Path, sync::Arc};

use strata_db::traits::DatabaseBackend;
use strata_db_store_rocksdb::{
    l2::db::L2Db, open_rocksdb_database, ChainstateDb, ClientStateDb, DbOpsConfig, L1Db,
    RBCheckpointDB, RocksDbBackend, SyncEventDb, ROCKSDB_NAME,
};

pub(crate) enum DbType {
    Rocksdb,
}

impl std::str::FromStr for DbType {
    type Err = Box<dyn std::error::Error>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "rocksdb" | "rocks" => Ok(DbType::Rocksdb),
            other => Err(format!("unknown db type {other}").into()),
        }
    }
}

fn init_rocksdb_components(
    rbdb: Arc<rockbound::OptimisticTransactionDB>,
    ops_config: DbOpsConfig,
) -> impl DatabaseBackend {
    let l1_db: Arc<_> = L1Db::new(rbdb.clone(), ops_config).into();
    let l2_db: Arc<_> = L2Db::new(rbdb.clone(), ops_config).into();
    let sync_ev_db: Arc<_> = SyncEventDb::new(rbdb.clone(), ops_config).into();
    let clientstate_db: Arc<_> = ClientStateDb::new(rbdb.clone(), ops_config).into();
    let chainstate_db: Arc<_> = ChainstateDb::new(rbdb.clone(), ops_config).into();
    let checkpoint_db: Arc<_> = RBCheckpointDB::new(rbdb.clone(), ops_config).into();

    RocksDbBackend::new(
        l1_db,
        l2_db,
        sync_ev_db,
        clientstate_db,
        chainstate_db,
        checkpoint_db,
        Arc::new(strata_db_store_rocksdb::writer::db::RBL1WriterDb::new(
            rbdb.clone(),
            ops_config,
        )),
        Arc::new(strata_db_store_rocksdb::prover::db::ProofDb::new(
            rbdb, ops_config,
        )),
    )
}

/// Returns a boxed trait-object that satisfies all the low-level traits.
pub(crate) fn open_database(
    path: &Path,
    db_type: DbType,
) -> Result<impl DatabaseBackend, Box<dyn std::error::Error>> {
    match db_type {
        DbType::Rocksdb => {
            let rbdb = open_rocksdb_database(path, ROCKSDB_NAME)
                .map_err(|e| format!("Failed to open rocksdb database: {e}"))?;
            Ok(init_rocksdb_components(rbdb, DbOpsConfig::new(3)))
        }
    }
}