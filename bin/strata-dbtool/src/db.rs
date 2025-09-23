use std::{path::Path, sync::Arc};

use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{Database, L1BroadcastDatabase, L1WriterDatabase};
use strata_rocksdb::{
    broadcaster::db::L1BroadcastDb, l2::db::L2Db, open_rocksdb_database, writer::db::RBL1WriterDb,
    ChainstateDb, ClientStateDb, DbOpsConfig, L1Db, RBCheckpointDB, SyncEventDb, ROCKSDB_NAME,
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

/// Common database backend that includes core database, L1 broadcast database, and L1 writer
/// database
pub(crate) struct CommonDbBackend<T: Database, B: L1BroadcastDatabase, W: L1WriterDatabase> {
    pub(crate) core: T,
    pub(crate) broadcast: Arc<B>,
    pub(crate) writer: Arc<W>,
}

impl<T: Database, B: L1BroadcastDatabase, W: L1WriterDatabase> CommonDbBackend<T, B, W> {
    /// Returns a reference to the L1 broadcast database
    pub(crate) fn broadcast_db(&self) -> Arc<B> {
        self.broadcast.clone()
    }

    /// Returns a reference to the L1 writer database
    pub(crate) fn writer_db(&self) -> Arc<W> {
        self.writer.clone()
    }
}

fn init_rocksdb_components(
    rbdb: Arc<rockbound::OptimisticTransactionDB>,
    ops_config: DbOpsConfig,
) -> CommonDbBackend<impl Database, L1BroadcastDb, RBL1WriterDb> {
    let l1_db: Arc<_> = L1Db::new(rbdb.clone(), ops_config).into();
    let l2_db: Arc<_> = L2Db::new(rbdb.clone(), ops_config).into();
    let sync_ev_db: Arc<_> = SyncEventDb::new(rbdb.clone(), ops_config).into();
    let clientstate_db: Arc<_> = ClientStateDb::new(rbdb.clone(), ops_config).into();
    let chainstate_db: Arc<_> = ChainstateDb::new(rbdb.clone(), ops_config).into();
    let checkpoint_db: Arc<_> = RBCheckpointDB::new(rbdb.clone(), ops_config).into();

    use strata_db::database::CommonDatabase;
    let core = CommonDatabase::new(
        l1_db,
        l2_db,
        sync_ev_db,
        clientstate_db,
        chainstate_db,
        checkpoint_db,
    );

    let broadcast: Arc<_> = L1BroadcastDb::new(rbdb.clone(), ops_config).into();
    let writer: Arc<_> = RBL1WriterDb::new(rbdb, ops_config).into();

    CommonDbBackend {
        core,
        broadcast,
        writer,
    }
}

/// Returns a common database that includes implementations of Database, L1BroadcastDatabase, and
/// L1WriterDatabase
pub(crate) fn open_database(
    path: &Path,
    db_type: DbType,
) -> Result<
    CommonDbBackend<impl Database, impl L1BroadcastDatabase, impl L1WriterDatabase>,
    DisplayedError,
> {
    match db_type {
        DbType::Rocksdb => {
            let rbdb = open_rocksdb_database(path, ROCKSDB_NAME)
                .internal_error("Failed to open rocksdb database")?;
            Ok(init_rocksdb_components(rbdb, DbOpsConfig::new(3)))
        }
    }
}
