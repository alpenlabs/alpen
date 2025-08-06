use std::{fs, path::Path, sync::Arc};

use anyhow::Context;
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

    let typed_sled = SledDb::new(sled_db)
        .map_err(|e| anyhow::anyhow!("Failed to create typed sled db: {}", e))?;
    Ok(Arc::new(typed_sled))
}

pub fn init_core_dbs(sled_db: Arc<SledDb>, db_config: SledDbConfig) -> Arc<SledBackend> {
    init_sled_backend(sled_db, db_config)
}

/// Initialize a complete Sled backend with all database types
pub(crate) fn init_sled_backend(sled_db: Arc<SledDb>, config: SledDbConfig) -> Arc<SledBackend> {
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
    let prover_db = Arc::new(
        ProofDBSled::new(sled_db.clone(), config.clone()).expect("Failed to create ProofDBSled"),
    );

    let broadcast_db = Arc::new(
        L1BroadcastDBSled::new(sled_db, config).expect("Failed to create L1BroadcastDBSled"),
    );
    Arc::new(SledBackend::new(
        l1_db,
        l2_db,
        sync_event_db,
        client_state_db,
        chain_state_db,
        checkpoint_db,
        writer_db,
        prover_db,
        broadcast_db,
    ))
}
