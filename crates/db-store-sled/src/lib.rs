//! Sled store for the Alpen codebase.

pub mod broadcaster;
pub mod chain_state;
pub mod checkpoint;
pub mod client_state;
mod config;
mod init;
pub mod l1;
pub mod l2;
pub mod macros;
pub mod prover;
pub mod sync_event;
#[cfg(feature = "test_utils")]
pub mod test_utils;
pub mod utils;
pub mod writer;

use std::{path::Path, sync::Arc};

// Re-exports
pub use broadcaster::db::L1BroadcastDBSled;
pub use chain_state::db::ChainstateDBSled;
pub use checkpoint::db::CheckpointDBSled;
pub use client_state::db::ClientStateDBSled;
pub use config::SledDbConfig;
pub use l1::db::L1DBSled;
pub use l2::db::L2DBSled;
use strata_db::traits::DatabaseBackend;
pub use writer::db::L1WriterDBSled;

pub use crate::init::{init_core_dbs, open_sled_database};
use crate::{init::init_sled_backend, prover::ProofDBSled, sync_event::SyncEventDBSled};

pub const SLED_NAME: &str = "strata-client";

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
    broadcast_db: Arc<L1BroadcastDBSled>,
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
        broadcast_db: Arc<L1BroadcastDBSled>,
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
            broadcast_db,
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

    fn broadcast_db(&self) -> Arc<impl strata_db::traits::L1BroadcastDatabase> {
        self.broadcast_db.clone()
    }
}
