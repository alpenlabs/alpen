//! Sled store for the Alpen codebase.

pub mod asm;
pub mod broadcaster;
pub mod chunked_envelope;
pub mod client_state;
mod config;
mod init;
mod instrumentation;
pub mod l1;
pub mod macros;
pub mod mempool;
pub mod mmr_index;
pub mod ol;
pub mod ol_checkpoint;
pub mod ol_state;
pub mod ol_state_index;
pub mod prover;
#[cfg(feature = "test_utils")]
pub mod test_utils;
pub mod utils;
pub mod writer;

use std::{path::Path, sync::Arc};

// Re-exports
pub use asm::AsmDBSled;
use broadcaster::db::L1BroadcastDBSled;
use chunked_envelope::db::L1ChunkedEnvelopeDBSled;
use client_state::db::ClientStateDBSled;
pub use config::SledDbConfig;
use l1::db::L1DBSled;
use mempool::db::MempoolDBSled;
pub use mmr_index::MmrIndexDb;
use ol::db::OLBlockDBSled;
use ol_checkpoint::db::OLCheckpointDBSled;
use ol_state::db::OLStateDBSled;
use ol_state_index::db::OLStateIndexingDBSled;
use rkyv as _;
use strata_db_types::{
    asm::AsmDatabase, backend::DatabaseBackend, checkpoint_proof::CheckpointProofDatabase,
    chunked_envelope::L1ChunkedEnvelopeDatabase, client_state::ClientStateDatabase, l1::L1Database,
    l1_broadcast::L1BroadcastDatabase, l1_writer::L1WriterDatabase, mempool::MempoolDatabase,
    ol_block::OLBlockDatabase, ol_checkpoint::OLCheckpointDatabase, ol_state::OLStateDatabase,
    ol_state_index::OLStateIndexingDatabase, prover_task::ProverTaskDatabase, DbResult,
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
    client_state_db: Arc<ClientStateDBSled>,
    ol_block_db: Arc<OLBlockDBSled>,
    ol_state_db: Arc<OLStateDBSled>,
    ol_checkpoint_db: Arc<OLCheckpointDBSled>,
    writer_db: Arc<L1WriterDBSled>,
    prover_db: Arc<ProofDBSled>,
    broadcast_db: Arc<L1BroadcastDBSled>,
    chunked_envelope_db: Arc<L1ChunkedEnvelopeDBSled>,
    mmr_index_db: Arc<MmrIndexDb>,
    mempool_db: Arc<MempoolDBSled>,
    ol_state_indexing_db: Arc<OLStateIndexingDBSled>,
}

impl SledBackend {
    pub fn new(sled_db: Arc<SledDb>, config: SledDbConfig) -> DbResult<Self> {
        let db_ref = &sled_db;
        let config_ref = &config;

        let asm_db = Arc::new(AsmDBSled::new(db_ref.clone(), config_ref.clone())?);
        let l1_db = Arc::new(L1DBSled::new(db_ref.clone(), config_ref.clone())?);
        let client_state_db = Arc::new(ClientStateDBSled::new(db_ref.clone(), config_ref.clone())?);
        let ol_block_db = Arc::new(OLBlockDBSled::new(db_ref.clone(), config_ref.clone())?);
        let ol_state_db = Arc::new(OLStateDBSled::new(db_ref.clone(), config_ref.clone())?);
        let ol_checkpoint_db =
            Arc::new(OLCheckpointDBSled::new(db_ref.clone(), config_ref.clone())?);
        let writer_db = Arc::new(L1WriterDBSled::new(db_ref.clone(), config_ref.clone())?);
        let prover_db = Arc::new(ProofDBSled::new(db_ref.clone(), config_ref.clone())?);
        let mmr_index_db = Arc::new(MmrIndexDb::new(db_ref.clone(), config_ref.clone())?);
        let broadcast_db = Arc::new(L1BroadcastDBSled::new(db_ref.clone(), config_ref.clone())?);
        let chunked_envelope_db = Arc::new(L1ChunkedEnvelopeDBSled::new(
            db_ref.clone(),
            config_ref.clone(),
        )?);
        let ol_state_indexing_db = Arc::new(OLStateIndexingDBSled::new(
            db_ref.clone(),
            config_ref.clone(),
        )?);
        let mempool_db = Arc::new(MempoolDBSled::new(sled_db, config)?);
        Ok(Self {
            asm_db,
            l1_db,
            client_state_db,
            ol_block_db,
            ol_state_db,
            ol_checkpoint_db,
            writer_db,
            prover_db,
            broadcast_db,
            chunked_envelope_db,
            mmr_index_db,
            mempool_db,
            ol_state_indexing_db,
        })
    }
}

impl DatabaseBackend for SledBackend {
    fn asm_db(&self) -> Arc<impl AsmDatabase> {
        self.asm_db.clone()
    }

    fn l1_db(&self) -> Arc<impl L1Database> {
        self.l1_db.clone()
    }

    fn client_state_db(&self) -> Arc<impl ClientStateDatabase> {
        self.client_state_db.clone()
    }

    fn ol_block_db(&self) -> Arc<impl OLBlockDatabase> {
        self.ol_block_db.clone()
    }

    fn ol_state_db(&self) -> Arc<impl OLStateDatabase> {
        self.ol_state_db.clone()
    }

    fn ol_checkpoint_db(&self) -> Arc<impl OLCheckpointDatabase> {
        self.ol_checkpoint_db.clone()
    }

    fn writer_db(&self) -> Arc<impl L1WriterDatabase> {
        self.writer_db.clone()
    }

    fn checkpoint_proof_db(&self) -> Arc<impl CheckpointProofDatabase> {
        self.prover_db.clone()
    }

    fn prover_task_db(&self) -> Arc<impl ProverTaskDatabase> {
        self.prover_db.clone()
    }

    fn broadcast_db(&self) -> Arc<impl L1BroadcastDatabase> {
        self.broadcast_db.clone()
    }

    fn chunked_envelope_db(&self) -> Arc<impl L1ChunkedEnvelopeDatabase> {
        self.chunked_envelope_db.clone()
    }

    fn mempool_db(&self) -> Arc<impl MempoolDatabase> {
        self.mempool_db.clone()
    }

    fn ol_state_indexing_db(&self) -> Arc<impl OLStateIndexingDatabase> {
        self.ol_state_indexing_db.clone()
    }
}

impl SledBackend {
    /// Get the MMR index database
    pub fn mmr_index_db(&self) -> Arc<MmrIndexDb> {
        self.mmr_index_db.clone()
    }

    /// Get the prover database with its concrete Sled-backed type.
    pub fn prover_db(&self) -> Arc<ProofDBSled> {
        self.prover_db.clone()
    }
}
