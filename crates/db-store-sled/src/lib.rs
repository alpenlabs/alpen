pub mod broadcaster;
pub mod chain_state;
pub mod checkpoint;
pub mod client_state;
pub mod l1;
pub mod l2;
pub mod macros;
pub mod prover;
pub mod sync_event;
pub mod utils;
pub mod writer;

use std::{fs, path::Path, sync::Arc};

use anyhow::Context;
// Re-exports
pub use broadcaster::db::{BroadcastDb, L1BroadcastDBSled};
pub use chain_state::db::ChainstateDBSled;
pub use checkpoint::db::CheckpointDBSled;
pub use client_state::db::ClientStateDBSled;
pub use l1::db::L1DBSled;
pub use l2::db::L2DBSled;
pub use prover::db::ProofDBSled;
use sled::transaction::ConflictableTransactionResult;
use strata_db::{DbError, DbResult, traits::DatabaseBackend};
pub use sync_event::db::SyncEventDBSled;
use typed_sled::{
    SledDb,
    transaction::{Backoff, ConstantBackoff, SledTransactional},
};
pub use writer::db::L1WriterDBSled;

pub const SLED_NAME: &str = "strata-client";

/// database operations configuration
#[derive(Debug, Clone)]
pub struct SledDbConfig {
    pub retry_count: u16,
    pub backoff: Arc<dyn Backoff>,
}

impl SledDbConfig {
    pub fn new(retry_count: u16, backoff: Arc<dyn Backoff>) -> Self {
        Self {
            retry_count,
            backoff,
        }
    }

    pub fn new_with_constant_backoff(retry_count: u16, delay: u64) -> Self {
        let const_backoff = ConstantBackoff::new(delay);
        Self {
            retry_count,
            backoff: Arc::new(const_backoff),
        }
    }

    /// Execute a transaction with retry logic using this config's settings
    pub fn with_retry<Trees, F, R>(&self, trees: Trees, f: F) -> DbResult<R>
    where
        Trees: SledTransactional,
        F: Fn(Trees::View) -> ConflictableTransactionResult<R, typed_sled::error::Error>,
    {
        trees
            .transaction_with_retry(self.backoff.as_ref(), self.retry_count.into(), f)
            .map_err(|e| DbError::Other(format!("{:?}", e)))
    }
}

// Opens sled database instance from datadir
pub fn open_sled_database(datadir: &Path, dbname: &'static str) -> anyhow::Result<Arc<SledDb>> {
    let mut database_dir = datadir.to_path_buf();
    database_dir.push("sled");
    database_dir.push(dbname);

    if !database_dir.exists() {
        fs::create_dir_all(&database_dir)?;
    }

    let sled_db = sled::open(&database_dir).context("opening sled database")?;

    let typed_sled = SledDb::new(sled_db)
        .map_err(|e| anyhow::anyhow!("Failed to create typed sled db: {}", e))?;
    Ok(Arc::new(typed_sled))
}

/// Opens a complete Sled backend from datadir with all database types
pub fn open_sled_backend(
    datadir: &Path,
    dbname: &'static str,
    ops_config: SledDbConfig,
) -> anyhow::Result<Arc<SledBackend>> {
    let sled_db = open_sled_database(datadir, dbname)?;
    Ok(init_sled_backend(sled_db, ops_config))
}

/// Complete Sled backend with all database types
#[derive(Debug)]
pub struct SledBackend {
    l1_db: Arc<L1DBSled>,
    l2_db: Arc<L2DBSled>,
    sync_event_db: Arc<SyncEventDBSled>,
    client_state_db: Arc<ClientStateDBSled>,
    chain_state_db: Arc<ChainstateDBSled>,
    checkpoint_db: Arc<CheckpointDBSled>,
    writer_db: Arc<L1WriterDBSled>,
    prover_db: Arc<ProofDBSled>,
}

impl SledBackend {
    #[allow(clippy::too_many_arguments)] // hard to avoid here
    pub fn new(
        l1_db: Arc<L1DBSled>,
        l2_db: Arc<L2DBSled>,
        sync_event_db: Arc<SyncEventDBSled>,
        client_state_db: Arc<ClientStateDBSled>,
        chain_state_db: Arc<ChainstateDBSled>,
        checkpoint_db: Arc<CheckpointDBSled>,
        writer_db: Arc<L1WriterDBSled>,
        prover_db: Arc<ProofDBSled>,
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

impl DatabaseBackend for SledBackend {
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

    fn chain_state_db(&self) -> Arc<impl strata_db::chainstate::ChainstateDatabase> {
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

pub fn init_core_dbs(sled_db: Arc<SledDb>, db_config: SledDbConfig) -> Arc<SledBackend> {
    init_sled_backend(sled_db, db_config)
}

pub fn init_broadcaster_database(sled_db: Arc<SledDb>, config: SledDbConfig) -> Arc<BroadcastDb> {
    let l1_broadcast_db =
        L1BroadcastDBSled::new(sled_db, config).expect("Failed to create L1BroadcastDBSled");
    BroadcastDb::new(l1_broadcast_db.into()).into()
}

pub fn init_writer_database(sled_db: Arc<SledDb>, config: SledDbConfig) -> Arc<L1WriterDBSled> {
    L1WriterDBSled::new(sled_db, config)
        .expect("Failed to create L1WriterDBSled")
        .into()
}

pub fn init_prover_database(sled_db: Arc<SledDb>, config: SledDbConfig) -> Arc<ProofDBSled> {
    ProofDBSled::new(sled_db, config)
        .expect("Failed to create ProofDBSled")
        .into()
}

/// Initialize a complete Sled backend with all database types
pub fn init_sled_backend(sled_db: Arc<SledDb>, config: SledDbConfig) -> Arc<SledBackend> {
    let l1_db = Arc::new(
        L1DBSled::new(sled_db.clone(), config.clone()).expect("Failed to create L1DBSled"),
    );
    let l2_db = Arc::new(
        L2DBSled::new(sled_db.clone(), config.clone()).expect("Failed to create L2DBSled"),
    );
    let sync_event_db = Arc::new(
        SyncEventDBSled::new(sled_db.clone(), config.clone())
            .expect("Failed to create SyncEventDBSled"),
    );
    let client_state_db = Arc::new(
        ClientStateDBSled::new(sled_db.clone(), config.clone())
            .expect("Failed to create ClientStateDBSled"),
    );
    let chain_state_db = Arc::new(
        ChainstateDBSled::new(sled_db.clone(), config.clone())
            .expect("Failed to create ChainstateDBSled"),
    );
    let checkpoint_db = Arc::new(
        CheckpointDBSled::new(sled_db.clone(), config.clone())
            .expect("Failed to create CheckpointDBSled"),
    );
    let writer_db = Arc::new(
        L1WriterDBSled::new(sled_db.clone(), config.clone())
            .expect("Failed to create L1WriterDBSled"),
    );
    let prover_db =
        Arc::new(ProofDBSled::new(sled_db, config.clone()).expect("Failed to create ProofDBSled"));

    Arc::new(SledBackend::new(
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
