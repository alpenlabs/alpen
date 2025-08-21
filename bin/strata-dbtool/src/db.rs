use std::{path::Path, sync::Arc};

use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{DatabaseBackend, L1BroadcastDatabase};
use strata_db_store_rocksdb::{
    l2::db::L2Db, open_rocksdb_database, prover::db::ProofDb, writer::db::RBL1WriterDb,
    ChainstateDb, ClientStateDb, DbOpsConfig, L1BroadcastDb, L1Db, RBCheckpointDB, RocksDbBackend,
    SyncEventDb, ROCKSDB_NAME,
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

/// Common database backend that includes both the core database backend and L1 broadcast database
pub(crate) struct CommonDbBackend<T: DatabaseBackend, B: L1BroadcastDatabase> {
    pub(crate) core: T,
    pub(crate) broadcast: Arc<B>,
}

impl<T: DatabaseBackend, B: L1BroadcastDatabase> CommonDbBackend<T, B> {
    /// Returns a reference to the L1 broadcast database
    pub(crate) fn broadcast_db(&self) -> Arc<B> {
        self.broadcast.clone()
    }
}

fn init_rocksdb_components(
    rbdb: Arc<rockbound::OptimisticTransactionDB>,
    ops_config: DbOpsConfig,
) -> CommonDbBackend<RocksDbBackend, L1BroadcastDb> {
    let l1_db: Arc<_> = L1Db::new(rbdb.clone(), ops_config).into();
    let l2_db: Arc<_> = L2Db::new(rbdb.clone(), ops_config).into();
    let sync_ev_db: Arc<_> = SyncEventDb::new(rbdb.clone(), ops_config).into();
    let clientstate_db: Arc<_> = ClientStateDb::new(rbdb.clone(), ops_config).into();
    let chainstate_db: Arc<_> = ChainstateDb::new(rbdb.clone(), ops_config).into();
    let checkpoint_db: Arc<_> = RBCheckpointDB::new(rbdb.clone(), ops_config).into();
    let l1_writer_db: Arc<_> = RBL1WriterDb::new(rbdb.clone(), ops_config).into();
    let proof_db: Arc<_> = ProofDb::new(rbdb.clone(), ops_config).into();

    let core = RocksDbBackend::new(
        l1_db,
        l2_db,
        sync_ev_db,
        clientstate_db,
        chainstate_db,
        checkpoint_db,
        l1_writer_db,
        proof_db,
    );

    let broadcast: Arc<_> = L1BroadcastDb::new(rbdb, ops_config).into();

    CommonDbBackend { core, broadcast }
}

/// Returns a common database that includes implementations of both DatabaseBackend and
/// L1BroadcastDatabase
pub(crate) fn open_database(
    path: &Path,
    db_type: DbType,
) -> Result<CommonDbBackend<impl DatabaseBackend, impl L1BroadcastDatabase>, DisplayedError> {
    match db_type {
        DbType::Rocksdb => {
            let rbdb = open_rocksdb_database(path, ROCKSDB_NAME)
                .internal_error("Failed to open rocksdb database")?;
            Ok(init_rocksdb_components(rbdb, DbOpsConfig::new(3)))
        }
    }
}
