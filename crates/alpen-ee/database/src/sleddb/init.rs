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
use tokio::runtime::Handle;
use typed_sled::SledDb;

use crate::{
    sleddb::{EeNodeDBSled, EeProverDbSled},
    storage::EeNodeStorage,
};

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
    /// Prover-side persistence: shared task store + chunk receipts + acct proofs.
    pub(crate) prover_db: Arc<EeProverDbSled>,
}

impl EeDatabases {
    /// Creates [`EeNodeStorage`] from the EE node database, dispatching blocking
    /// work via the given runtime handle.
    pub fn node_storage(&self, handle: Handle) -> EeNodeStorage {
        EeNodeStorage::new(handle, self.ee_node_db.clone())
    }

    /// Returns a clone of the witness database.
    pub fn witness_db(&self) -> Arc<SledWitnessDB> {
        self.witness_db.clone()
    }

    /// Creates [`BroadcastDbOps`] from the broadcast database, dispatching
    /// blocking work via the given runtime handle.
    pub fn broadcast_ops(&self, handle: Handle) -> BroadcastDbOps {
        BroadcastDbOps::new(handle, self.broadcast_db.clone())
    }

    /// Creates [`ChunkedEnvelopeOps`] from the chunked envelope database,
    /// dispatching blocking work via the given runtime handle.
    pub fn chunked_envelope_ops(&self, handle: Handle) -> ChunkedEnvelopeOps {
        ChunkedEnvelopeOps::new(handle, self.chunked_envelope_db.clone())
    }

    /// Returns a clone of the DA context database.
    pub fn da_context_db(&self) -> Arc<EeDaContextDb<SledWitnessDB>> {
        self.da_context_db.clone()
    }

    /// Returns a clone of the prover database (shared task store +
    /// chunk receipts + acct proofs).
    pub fn prover_db(&self) -> Arc<EeProverDbSled> {
        self.prover_db.clone()
    }
}

/// Opens a single sled instance at `<datadir>/sled` and creates all database types from it.
///
/// This is the raw offline-tooling boundary and does not run startup maintenance. All typed-sled
/// trees coexist in one sled directory, with unique names to avoid collisions.
pub(crate) fn open_database(datadir: &Path, db_retry_count: u16) -> Result<EeDatabases> {
    open_database_inner(datadir, db_retry_count)
}

/// Opens the databases and completes every required startup-maintenance job before returning.
pub(crate) fn open_database_for_node(datadir: &Path, db_retry_count: u16) -> Result<EeDatabases> {
    let databases = open_database_inner(datadir, db_retry_count)?;
    super::maintenance::run_startup_jobs(&databases.ee_node_db)
        .map_err(|e| eyre!("failed to run EE database startup maintenance: {e}"))?;
    Ok(databases)
}

fn open_database_inner(datadir: &Path, db_retry_count: u16) -> Result<EeDatabases> {
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
        L1ChunkedEnvelopeDBSled::new(typed_sled.clone(), config.clone())
            .map_err(|e| eyre!("failed to create chunked envelope db: {e}"))?,
    );

    let prover_db = Arc::new(
        EeProverDbSled::new(typed_sled.clone(), config)
            .map_err(|e| eyre!("failed to create EE prover db: {e}"))?,
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
        prover_db,
    })
}

#[cfg(test)]
mod tests {
    use alpen_ee_common::{Chunk, ChunkStatus};
    use strata_acct_types::Hash;

    use super::*;
    use crate::database::EeNodeDb;

    fn hash_from_u8(value: u8) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[31] = value;
        Hash::from(bytes)
    }

    fn chunk(idx: u64, prev: u8, last: u8) -> Chunk {
        Chunk::new(
            idx,
            hash_from_u8(prev),
            hash_from_u8(last),
            idx,
            0,
            Vec::new(),
        )
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tooling_open_skips_jobs_and_node_open_rebuilds_before_returning() {
        let datadir = tempfile::tempdir().unwrap();
        let databases = open_database(datadir.path(), 2).unwrap();
        let node_db = &databases.ee_node_db;

        let sealed = chunk(0, 1, 2);
        let pending = chunk(1, 2, 3);
        node_db.save_next_chunk(sealed).unwrap();
        node_db.save_next_chunk(pending.clone()).unwrap();
        node_db
            .update_chunk_status(pending.id(), ChunkStatus::ProofPending("task".to_string()))
            .unwrap();

        node_db.sealed_chunk_work_by_idx_tree.remove(&0).unwrap();
        node_db
            .proof_pending_chunk_work_by_idx_tree
            .remove(&1)
            .unwrap();
        drop(databases);

        let tooling_databases = open_database(datadir.path(), 2).unwrap();
        assert!(tooling_databases
            .ee_node_db
            .get_sealed_chunks(0, 10)
            .unwrap()
            .is_empty());
        assert!(tooling_databases
            .ee_node_db
            .get_proof_pending_chunks(0, 10)
            .unwrap()
            .is_empty());
        drop(tooling_databases);

        let node_databases = crate::open_for_node(datadir.path().to_path_buf(), 2)
            .await
            .unwrap();
        assert_eq!(
            node_databases
                .ee_node_db
                .get_sealed_chunks(0, 10)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            node_databases
                .ee_node_db
                .get_proof_pending_chunks(0, 10)
                .unwrap()
                .len(),
            1
        );
    }
}
