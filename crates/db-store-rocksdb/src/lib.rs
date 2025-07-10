//! RocksDB store for the Alpen codebase.

pub mod broadcaster;
pub mod chain_state;
pub mod checkpoint;
pub mod client_state;
pub mod l1;
pub mod l2;
pub mod prover;
pub mod sync_event;
pub mod writer;

pub mod macros;
mod sequence;
pub mod utils;

use anyhow::Context;
use strata_db::traits::DatabaseBackend;

#[cfg(feature = "test_utils")]
pub mod test_utils;

use std::{fs, path::Path, sync::Arc};

pub const PROVER_COLUMN_FAMILIES: &[ColumnFamilyName] = &[
    SequenceSchema::COLUMN_FAMILY_NAME,
    prover::schemas::ProofSchema::COLUMN_FAMILY_NAME,
    prover::schemas::ProofDepsSchema::COLUMN_FAMILY_NAME,
];

// Re-exports
pub use broadcaster::db::L1BroadcastDb;
use broadcaster::{
    db::BroadcastDb,
    schemas::{BcastL1TxIdSchema, BcastL1TxSchema},
};
pub use chain_state::db::ChainstateDb;
pub use checkpoint::db::RBCheckpointDB;
use checkpoint::schemas::*;
pub use client_state::db::ClientStateDb;
pub use l1::db::L1Db;
use l2::{
    db::L2Db,
    schemas::{L2BlockHeightSchema, L2BlockSchema, L2BlockStatusSchema},
};
pub use prover::db::ProofDb;
use rockbound::{schema::ColumnFamilyName, Schema, TransactionRetry};
pub use sync_event::db::SyncEventDb;
pub use writer::db::RBL1WriterDb;
use writer::schemas::{IntentIdxSchema, IntentSchema, PayloadSchema};

use crate::{
    chain_state::schemas::WriteBatchSchema,
    client_state::schemas::ClientUpdateOutputSchema,
    l1::schemas::{L1BlockSchema, L1BlocksByHeightSchema, L1CanonicalBlockSchema, TxnSchema},
    sequence::SequenceSchema,
    sync_event::schemas::SyncEventSchema,
};

pub const ROCKSDB_NAME: &str = "strata-client";

#[rustfmt::skip]
pub const STORE_COLUMN_FAMILIES: &[ColumnFamilyName] = &[
    // Core
    SequenceSchema::COLUMN_FAMILY_NAME,
    ClientUpdateOutputSchema::COLUMN_FAMILY_NAME,
    L1BlockSchema::COLUMN_FAMILY_NAME,
    TxnSchema::COLUMN_FAMILY_NAME,
    L1BlocksByHeightSchema::COLUMN_FAMILY_NAME,
    L1CanonicalBlockSchema::COLUMN_FAMILY_NAME,
    SyncEventSchema::COLUMN_FAMILY_NAME,
    L2BlockSchema::COLUMN_FAMILY_NAME,
    L2BlockStatusSchema::COLUMN_FAMILY_NAME,
    L2BlockHeightSchema::COLUMN_FAMILY_NAME,
    WriteBatchSchema::COLUMN_FAMILY_NAME,

    // Payload/intent schemas
    PayloadSchema::COLUMN_FAMILY_NAME,
    IntentSchema::COLUMN_FAMILY_NAME,
    IntentIdxSchema::COLUMN_FAMILY_NAME,

    // Bcast schemas
    BcastL1TxIdSchema::COLUMN_FAMILY_NAME,
    BcastL1TxSchema::COLUMN_FAMILY_NAME,

    // Checkpoint schemas
    CheckpointSchema::COLUMN_FAMILY_NAME,
    EpochSummarySchema::COLUMN_FAMILY_NAME,
];

/// database operations configuration
#[derive(Clone, Copy, Debug)]
pub struct DbOpsConfig {
    pub retry_count: u16,
}

impl DbOpsConfig {
    pub fn new(retry_count: u16) -> Self {
        Self { retry_count }
    }

    pub fn txn_retry_count(&self) -> TransactionRetry {
        TransactionRetry::Count(self.retry_count)
    }
}

// Opens rocksdb database instance from datadir
pub fn open_rocksdb_database(
    datadir: &Path,
    dbname: &'static str,
) -> anyhow::Result<Arc<rockbound::OptimisticTransactionDB>> {
    let mut database_dir = datadir.to_path_buf();
    database_dir.push("rocksdb");

    if !database_dir.exists() {
        fs::create_dir_all(&database_dir)?;
    }

    let mut opts = rockbound::rocksdb::Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);

    let rbdb = rockbound::OptimisticTransactionDB::open(
        &database_dir,
        dbname,
        STORE_COLUMN_FAMILIES.iter().map(|s| s.to_string()),
        &opts,
    )
    .context("opening database")?;

    Ok(Arc::new(rbdb))
}

/// Opens a complete RocksDB backend from datadir with all database types
pub fn open_rocksdb_backend(
    datadir: &Path,
    dbname: &'static str,
    ops_config: DbOpsConfig,
) -> anyhow::Result<Arc<RocksDbBackend>> {
    let rbdb = open_rocksdb_database(datadir, dbname)?;
    Ok(init_rocksdb_backend(rbdb, ops_config))
}

/// Complete RocksDB backend with all database types
#[derive(Debug)]
pub struct RocksDbBackend {
    l1_db: Arc<L1Db>,
    l2_db: Arc<L2Db>,
    sync_event_db: Arc<SyncEventDb>,
    client_state_db: Arc<ClientStateDb>,
    chain_state_db: Arc<ChainstateDb>,
    checkpoint_db: Arc<RBCheckpointDB>,
    writer_db: Arc<RBL1WriterDb>,
    prover_db: Arc<prover::db::ProofDb>,
}

impl RocksDbBackend {
    #[allow(clippy::too_many_arguments)] // hard to avoid here
    pub fn new(
        l1_db: Arc<L1Db>,
        l2_db: Arc<L2Db>,
        sync_event_db: Arc<SyncEventDb>,
        client_state_db: Arc<ClientStateDb>,
        chain_state_db: Arc<ChainstateDb>,
        checkpoint_db: Arc<RBCheckpointDB>,
        writer_db: Arc<RBL1WriterDb>,
        prover_db: Arc<prover::db::ProofDb>,
    ) -> Self {
        Self {
            l1_db,
            l2_db,
            sync_event_db,
            client_state_db,
            chain_state_db,
            checkpoint_db,
            writer_db,
            prover_db,
        }
    }
}

impl DatabaseBackend for RocksDbBackend {
    fn l1_db(&self) -> Arc<impl strata_db::traits::L1Database> {
        self.l1_db.clone()
    }

    fn l2_db(&self) -> Arc<impl strata_db::traits::L2BlockDatabase> {
        self.l2_db.clone()
    }

    fn sync_event_db(&self) -> Arc<impl strata_db::traits::SyncEventDatabase> {
        self.sync_event_db.clone()
    }

    fn client_state_db(&self) -> Arc<impl strata_db::traits::ClientStateDatabase> {
        self.client_state_db.clone()
    }

    fn chain_state_db(&self) -> Arc<impl strata_db::traits::ChainstateDatabase> {
        self.chain_state_db.clone()
    }

    fn checkpoint_db(&self) -> Arc<impl strata_db::traits::CheckpointDatabase> {
        self.checkpoint_db.clone()
    }

    fn writer_db(&self) -> Arc<impl strata_db::traits::L1WriterDatabase> {
        self.writer_db.clone()
    }

    fn prover_db(&self) -> Arc<impl strata_db::traits::ProofDatabase> {
        self.prover_db.clone()
    }
}

pub fn init_core_dbs(
    rbdb: Arc<rockbound::OptimisticTransactionDB>,
    ops_config: DbOpsConfig,
) -> Arc<RocksDbBackend> {
    init_rocksdb_backend(rbdb, ops_config)
}

pub fn init_broadcaster_database(
    rbdb: Arc<rockbound::OptimisticTransactionDB>,
    ops_config: DbOpsConfig,
) -> Arc<BroadcastDb> {
    let l1_broadcast_db = L1BroadcastDb::new(rbdb.clone(), ops_config);
    BroadcastDb::new(l1_broadcast_db.into()).into()
}

pub fn init_writer_database(
    rbdb: Arc<rockbound::OptimisticTransactionDB>,
    ops_config: DbOpsConfig,
) -> Arc<RBL1WriterDb> {
    RBL1WriterDb::new(rbdb, ops_config).into()
}

pub fn init_prover_database(
    rbdb: Arc<rockbound::OptimisticTransactionDB>,
    ops_config: DbOpsConfig,
) -> Arc<ProofDb> {
    ProofDb::new(rbdb, ops_config).into()
}

/// Initialize a complete RocksDB backend with all database types
pub fn init_rocksdb_backend(
    rbdb: Arc<rockbound::OptimisticTransactionDB>,
    ops_config: DbOpsConfig,
) -> Arc<RocksDbBackend> {
    let l1_db = Arc::new(L1Db::new(rbdb.clone(), ops_config));
    let l2_db = Arc::new(L2Db::new(rbdb.clone(), ops_config));
    let sync_event_db = Arc::new(SyncEventDb::new(rbdb.clone(), ops_config));
    let client_state_db = Arc::new(ClientStateDb::new(rbdb.clone(), ops_config));
    let chain_state_db = Arc::new(ChainstateDb::new(rbdb.clone(), ops_config));
    let checkpoint_db = Arc::new(RBCheckpointDB::new(rbdb.clone(), ops_config));
    let writer_db = Arc::new(RBL1WriterDb::new(rbdb.clone(), ops_config));
    let prover_db = Arc::new(ProofDb::new(rbdb, ops_config));

    Arc::new(RocksDbBackend::new(
        l1_db,
        l2_db,
        sync_event_db,
        client_state_db,
        chain_state_db,
        checkpoint_db,
        writer_db,
        prover_db,
    ))
}
