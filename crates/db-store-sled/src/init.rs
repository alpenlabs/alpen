use std::{fs, path::Path, sync::Arc};

use anyhow::Context;
use strata_db::DbResult;
use typed_sled::SledDb;

use crate::{
    ChainstateDBSled, CheckpointDBSled, ClientStateDBSled, L1BroadcastDBSled, L1DBSled,
    L1WriterDBSled, L2DBSled, ProofDBSled, SledBackend, SledDbConfig, SyncEventDBSled,
};

// Opens sled database instance from datadir
pub fn open_sled_database(datadir: &Path, dbname: &'static str) -> anyhow::Result<Arc<SledDb>> {
    let mut database_dir = datadir.to_path_buf();
    database_dir.push("sled");
    database_dir.push(dbname);

    if !database_dir.exists() {
        fs::create_dir_all(&database_dir)?;
    }

    let sled_db = sled::open(&database_dir).context("opening sled database")?;

    let db =
        SledDb::new(sled_db).map_err(|e| anyhow::anyhow!("Failed to create sled db: {}", e))?;
    Ok(Arc::new(db))
}

pub fn init_core_dbs(sled_db: Arc<SledDb>, db_config: SledDbConfig) -> DbResult<Arc<SledBackend>> {
    init_sled_backend(sled_db, db_config)
}

/// Initialize a complete Sled backend with all database types
pub fn init_sled_backend(sled_db: Arc<SledDb>, config: SledDbConfig) -> DbResult<Arc<SledBackend>> {
    // Create shared references to avoid excessive cloning
    let db_ref = &sled_db;
    let config_ref = &config;

    let l1_db = Arc::new(L1DBSled::new(db_ref.clone(), config_ref.clone())?);
    let l2_db = Arc::new(L2DBSled::new(db_ref.clone(), config_ref.clone())?);
    let sync_event_db = Arc::new(SyncEventDBSled::new(db_ref.clone(), config_ref.clone())?);
    let client_state_db = Arc::new(ClientStateDBSled::new(db_ref.clone(), config_ref.clone())?);
    let chain_state_db = Arc::new(ChainstateDBSled::new(db_ref.clone(), config_ref.clone())?);
    let checkpoint_db = Arc::new(CheckpointDBSled::new(db_ref.clone(), config_ref.clone())?);
    let writer_db = Arc::new(L1WriterDBSled::new(db_ref.clone(), config_ref.clone())?);
    let prover_db = Arc::new(ProofDBSled::new(db_ref.clone(), config_ref.clone())?);
    let broadcast_db = Arc::new(L1BroadcastDBSled::new(sled_db, config)?);

    Ok(Arc::new(SledBackend::new(
        l1_db,
        l2_db,
        sync_event_db,
        client_state_db,
        chain_state_db,
        checkpoint_db,
        writer_db,
        prover_db,
        broadcast_db,
    )))
}
