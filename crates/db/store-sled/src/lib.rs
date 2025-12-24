//! Sled store for the Alpen codebase.

pub mod asm;
pub mod broadcaster;
pub mod chain_state;
pub mod checkpoint;
pub mod client_state;
mod config;
mod init;
pub mod l1;
pub mod l2;
pub mod macros;
pub mod ol;
pub mod ol_state;
pub mod prover;
pub mod snark_msg_mmr;
#[cfg(feature = "test_utils")]
pub mod test_utils;
pub mod utils;
pub mod writer;

use std::{path::Path, sync::Arc};

// Re-exports
pub use asm::AsmDBSled;
use broadcaster::db::L1BroadcastDBSled;
use chain_state::db::ChainstateDBSled;
use checkpoint::db::CheckpointDBSled;
use client_state::db::ClientStateDBSled;
pub use config::SledDbConfig;
use l1::db::L1DBSled;
use l2::db::L2DBSled;
use ol::db::OLBlockDBSled;
use ol_state::db::OLStateDBSled;
use snark_msg_mmr::SnarkMsgMmrDb;
use strata_db_types::{
    DbResult,
    traits::{DatabaseBackend, OLBlockDatabase, OLStateDatabase},
};
use typed_sled::SledDb;
use writer::db::L1WriterDBSled;

pub use crate::{
    init::{init_core_dbs, open_sled_database},
    prover::ProofDBSled,
};

pub const SLED_NAME: &str = "strata-client";

/// Opens a complete Sled backend from datadir with all database types
pub fn open_sled_backend(
    datadir: &Path,
    dbname: &'static str,
    ops_config: SledDbConfig,
) -> anyhow::Result<Arc<SledBackend>> {
    let sled_db = open_sled_database(datadir, dbname)?;
    SledBackend::new(sled_db, ops_config)
        .map_err(|e| anyhow::anyhow!("Failed to initialize sled backend: {}", e))
        .map(Arc::new)
}

/// Complete Sled backend with all database types
#[derive(Debug)]
pub struct SledBackend {
    asm_db: Arc<AsmDBSled>,
    l1_db: Arc<L1DBSled>,
    l2_db: Arc<L2DBSled>,
    client_state_db: Arc<ClientStateDBSled>,
    chain_state_db: Arc<ChainstateDBSled>,
    ol_block_db: Arc<OLBlockDBSled>,
    ol_state_db: Arc<OLStateDBSled>,
    checkpoint_db: Arc<CheckpointDBSled>,
    writer_db: Arc<L1WriterDBSled>,
    prover_db: Arc<ProofDBSled>,
    broadcast_db: Arc<L1BroadcastDBSled>,
    snark_msg_mmr_db: Arc<SnarkMsgMmrDb>,
}

impl SledBackend {
    pub fn new(sled_db: Arc<SledDb>, config: SledDbConfig) -> DbResult<Self> {
        let db_ref = &sled_db;
        let config_ref = &config;

        let asm_db = Arc::new(AsmDBSled::new(db_ref.clone(), config_ref.clone())?);
        let l1_db = Arc::new(L1DBSled::new(db_ref.clone(), config_ref.clone())?);
        let l2_db = Arc::new(L2DBSled::new(db_ref.clone(), config_ref.clone())?);
        let client_state_db = Arc::new(ClientStateDBSled::new(db_ref.clone(), config_ref.clone())?);
        let chain_state_db = Arc::new(ChainstateDBSled::new(db_ref.clone(), config_ref.clone())?);
        let ol_block_db = Arc::new(OLBlockDBSled::new(db_ref.clone(), config_ref.clone())?);
        let ol_state_db = Arc::new(OLStateDBSled::new(db_ref.clone(), config_ref.clone())?);
        let checkpoint_db = Arc::new(CheckpointDBSled::new(db_ref.clone(), config_ref.clone())?);
        let writer_db = Arc::new(L1WriterDBSled::new(db_ref.clone(), config_ref.clone())?);
        let prover_db = Arc::new(ProofDBSled::new(db_ref.clone(), config_ref.clone())?);
        let snark_msg_mmr_db = Arc::new(SnarkMsgMmrDb::new(db_ref.clone(), config_ref.clone())?);
        let broadcast_db = Arc::new(L1BroadcastDBSled::new(sled_db, config)?);
        Ok(Self {
            asm_db,
            l1_db,
            l2_db,
            client_state_db,
            chain_state_db,
            ol_block_db,
            ol_state_db,
            checkpoint_db,
            writer_db,
            prover_db,
            broadcast_db,
            snark_msg_mmr_db,
        })
    }
}

impl DatabaseBackend for SledBackend {
    fn asm_db(&self) -> Arc<impl strata_db_types::traits::AsmDatabase> {
        self.asm_db.clone()
    }

    fn l1_db(&self) -> Arc<impl strata_db_types::traits::L1Database> {
        self.l1_db.clone()
    }

    fn l2_db(&self) -> Arc<impl strata_db_types::traits::L2BlockDatabase> {
        self.l2_db.clone()
    }

    fn client_state_db(&self) -> Arc<impl strata_db_types::traits::ClientStateDatabase> {
        self.client_state_db.clone()
    }

    fn chain_state_db(&self) -> Arc<impl strata_db_types::chainstate::ChainstateDatabase> {
        self.chain_state_db.clone()
    }

    fn ol_block_db(&self) -> Arc<impl OLBlockDatabase> {
        self.ol_block_db.clone()
    }

    fn ol_state_db(&self) -> Arc<impl OLStateDatabase> {
        self.ol_state_db.clone()
    }

    fn checkpoint_db(&self) -> Arc<impl strata_db_types::traits::CheckpointDatabase> {
        self.checkpoint_db.clone()
    }

    fn writer_db(&self) -> Arc<impl strata_db_types::traits::L1WriterDatabase> {
        self.writer_db.clone()
    }

    fn prover_db(&self) -> Arc<impl strata_db_types::traits::ProofDatabase> {
        self.prover_db.clone()
    }

    fn broadcast_db(&self) -> Arc<impl strata_db_types::traits::L1BroadcastDatabase> {
        self.broadcast_db.clone()
    }
}

impl SledBackend {
    /// Get the ASM MMR database
    pub fn asm_mmr_db(&self) -> Arc<AsmDBSled> {
        self.asm_db.clone()
    }

    /// Get the Snark Message MMR database
    pub fn snark_msg_mmr_db(&self) -> Arc<SnarkMsgMmrDb> {
        self.snark_msg_mmr_db.clone()
    }

    /// Get the MMR database (deprecated - use asm_mmr_db)
    #[deprecated(note = "Use asm_mmr_db() instead")]
    pub fn mmr_db(&self) -> Arc<AsmDBSled> {
        self.asm_db.clone()
    }
}
