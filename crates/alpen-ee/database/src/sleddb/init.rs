use std::{fs, path::Path, sync::Arc};

use alpen_reth_db::sled::{EeDaContextDb, WitnessDB as SledWitnessDB};
use eyre::{eyre, Context, Result};
use strata_db_store_sled::{
    broadcaster::db::L1BroadcastDBSled, chunked_envelope::L1ChunkedEnvelopeDBSled, SledDbConfig,
};
/// Re-export ops types for callers.
pub use strata_storage::ops::{
    chunked_envelope::ChunkedEnvelopeOps, l1tx_broadcast::BroadcastDbOps,
};
use strata_storage::ops::{
    chunked_envelope::Context as ChunkedEnvelopeContext,
    l1tx_broadcast::Context as BroadcastContext,
};
use threadpool::ThreadPool;
use typed_sled::SledDb;

use crate::{sleddb::EeNodeDBSled, storage::EeNodeStorage};

/// Container for all EE database instances.
///
/// Opens a single sled instance and creates all typed database trees from it.
/// Callers wrap individual DBs in ops/managers/threadpools as needed.
#[derive(Debug)]
pub struct EeDatabases {
    /// EE node database for chain state.
    pub(crate) ee_node_db: Arc<EeNodeDBSled>,
    /// Witness database for state diffs and block witnesses.
    pub(crate) witness_db: Arc<SledWitnessDB>,
    /// L1 broadcast transaction database.
    pub(crate) broadcast_db: Arc<L1BroadcastDBSled>,
    /// Chunked envelope database.
    pub(crate) chunked_envelope_db: Arc<L1ChunkedEnvelopeDBSled>,
    /// DA filter for cross-batch deduplication (bytecodes, extensible for addresses etc.).
    pub(crate) da_context_db: Arc<EeDaContextDb<SledWitnessDB>>,
}

impl EeDatabases {
    /// Creates [`EeNodeStorage`] from the EE node database with the given
    /// threadpool.
    pub fn node_storage(&self, pool: ThreadPool) -> EeNodeStorage {
        EeNodeStorage::new(pool, self.ee_node_db.clone())
    }

    /// Returns a clone of the witness database.
    pub fn witness_db(&self) -> Arc<SledWitnessDB> {
        self.witness_db.clone()
    }

    /// Creates [`BroadcastDbOps`] from the broadcast database with the given
    /// threadpool.
    pub fn broadcast_ops(&self, pool: ThreadPool) -> BroadcastDbOps {
        BroadcastContext::new(self.broadcast_db.clone()).into_ops(pool)
    }

    /// Creates [`ChunkedEnvelopeOps`] from the chunked envelope database with
    /// the given threadpool.
    pub fn chunked_envelope_ops(&self, pool: ThreadPool) -> ChunkedEnvelopeOps {
        ChunkedEnvelopeContext::new(self.chunked_envelope_db.clone()).into_ops(pool)
    }

    /// Returns a clone of the DA context database.
    pub fn da_context_db(&self) -> Arc<EeDaContextDb<SledWitnessDB>> {
        self.da_context_db.clone()
    }
}

/// Opens a single sled instance at `<datadir>/sled` and creates all database
/// types from it.
///
/// All typed-sled trees coexist in one sled directory — each DB type uses
/// uniquely named trees so there are no collisions.
pub(crate) fn init_database(datadir: &Path, db_retry_count: u16) -> Result<EeDatabases> {
    let database_dir = datadir.join("sled");

    fs::create_dir_all(&database_dir)
        .wrap_err_with(|| format!("creating database directory at {database_dir:?}"))?;

    let sled_db = sled::open(&database_dir).wrap_err("opening sled database")?;

    let typed_sled =
        Arc::new(SledDb::new(sled_db).map_err(|e| eyre!("failed to create typed sled db: {e}"))?);

    let retry_delay_ms = 200u64;
    let config = SledDbConfig::new_with_constant_backoff(db_retry_count, retry_delay_ms);

    let ee_node_db = Arc::new(
        EeNodeDBSled::new(typed_sled.clone(), config.clone())
            .map_err(|e| eyre!("failed to create EE node db: {e}"))?,
    );

    let witness_db = Arc::new(
        SledWitnessDB::new(typed_sled.clone())
            .map_err(|e| eyre!("failed to create witness db: {e}"))?,
    );

    let broadcast_db = Arc::new(
        L1BroadcastDBSled::new(typed_sled.clone(), config.clone())
            .map_err(|e| eyre!("failed to create broadcast db: {e}"))?,
    );

    let chunked_envelope_db = Arc::new(
        L1ChunkedEnvelopeDBSled::new(typed_sled.clone(), config)
            .map_err(|e| eyre!("failed to create chunked envelope db: {e}"))?,
    );

    let da_context_db = Arc::new(
        EeDaContextDb::new(typed_sled, witness_db.clone())
            .map_err(|e| eyre!("failed to create DA context db: {e}"))?,
    );

    Ok(EeDatabases {
        ee_node_db,
        witness_db,
        broadcast_db,
        chunked_envelope_db,
        da_context_db,
    })
}
