//! Storage for the Alpen codebase.

mod cache;
mod instrumentation;
mod managers;

/// Database ops wrappers, re-exported from the `gen_proxy`-generated proxies in
/// [`strata_db_types`] under their legacy `*Ops` names.
pub mod ops {
    pub mod asm {
        pub use strata_db_types::asm::AsmDatabaseProxy as AsmDataOps;
    }
    pub mod checkpoint_proof {
        pub use strata_db_types::checkpoint_proof::CheckpointProofDatabaseProxy as CheckpointProofDbOps;
    }
    pub mod chunked_envelope {
        pub use strata_db_types::chunked_envelope::L1ChunkedEnvelopeDatabaseProxy as ChunkedEnvelopeOps;
    }
    pub mod client_state {
        pub use strata_db_types::client_state::ClientStateDatabaseProxy as ClientStateOps;
    }
    pub mod l1 {
        pub use strata_db_types::l1::L1DatabaseProxy as L1DataOps;
    }
    pub mod l1tx_broadcast {
        pub use strata_db_types::l1_broadcast::L1BroadcastDatabaseProxy as BroadcastDbOps;
    }
    pub mod mempool {
        pub use strata_db_types::mempool::MempoolDatabaseProxy as MempoolDataOps;
    }
    pub mod mmr_index {
        pub use strata_db_types::mmr_index::MmrIndexDatabaseProxy as MmrIndexOps;
    }
    pub mod ol {
        pub use strata_db_types::ol_block::OLBlockDatabaseProxy as OLBlockOps;
    }
    pub mod ol_checkpoint {
        pub use strata_db_types::ol_checkpoint::OLCheckpointDatabaseProxy as OLCheckpointOps;
    }
    pub mod ol_state {
        pub use strata_db_types::ol_state::OLStateDatabaseProxy as OLStateOps;
    }
    pub mod ol_state_indexing {
        pub use strata_db_types::ol_state_index::OLStateIndexingDatabaseProxy as OLStateIndexingOps;
    }
    pub mod prover_task {
        pub use strata_db_types::prover_task::ProverTaskDatabaseProxy as ProverTaskDbOps;
    }
    pub mod writer {
        pub use strata_db_types::l1_writer::L1WriterDatabaseProxy as EnvelopeDataOps;
    }
}

use std::sync::Arc;

use anyhow::Context;
pub use managers::asm::AsmStateManager;
pub use managers::checkpoint_proof::CheckpointProofDbManager;
pub use managers::client_state::ClientStateManager;
pub use managers::l1::L1BlockManager;
pub use managers::mempool::MempoolDbManager;
pub use managers::mmr_index::{MmrAppendRequest, MmrIndexHandle, MmrIndexManager, MmrStateView};
pub use managers::ol::OLBlockManager;
pub use managers::ol_checkpoint::OLCheckpointManager;
pub use managers::ol_state::OLStateManager;
pub use managers::ol_state_indexing::OLStateIndexingManager;
pub use managers::prover_task::ProverTaskDbManager;
pub use managers::writer::L1WriterManager;
pub use ops::l1tx_broadcast::BroadcastDbOps;
use strata_db_store_sled::SledBackend;
use strata_db_types::backend::DatabaseBackend;
pub use strata_db_types::MmrId;
use strata_db_types::DbResult;
use strata_primitives::L1BlockCommitment;
use strata_state::asm_state::AsmState;
use tokio::runtime::Handle;
use tracing::warn;

/// A consolidation of database managers.
// TODO(STR-3679): move this to its own module
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct NodeStorage {
    /// Database backend for raw database access (needed for sequencer tasks)
    db: Arc<SledBackend>,
    /// Tokio runtime handle used to offload blocking database operations.
    handle: Handle,

    asm_state_manager: Arc<AsmStateManager>,
    l1_block_manager: Arc<L1BlockManager>,

    client_state_manager: Arc<ClientStateManager>,

    ol_block_manager: Arc<OLBlockManager>,
    mmr_index_manager: Arc<MmrIndexManager>,
    mempool_db_manager: Arc<MempoolDbManager>,
    ol_state_manager: Arc<OLStateManager>,
    ol_state_indexing_manager: Arc<OLStateIndexingManager>,
    ol_checkpoint_manager: Arc<OLCheckpointManager>,
    proof_manager: Arc<CheckpointProofDbManager>,
    prover_task_manager: Arc<ProverTaskDbManager>,
    l1_writer_manager: Arc<L1WriterManager>,
}

impl Clone for NodeStorage {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            handle: self.handle.clone(),
            asm_state_manager: self.asm_state_manager.clone(),
            l1_block_manager: self.l1_block_manager.clone(),
            client_state_manager: self.client_state_manager.clone(),
            ol_block_manager: self.ol_block_manager.clone(),
            mmr_index_manager: self.mmr_index_manager.clone(),
            mempool_db_manager: self.mempool_db_manager.clone(),
            ol_state_manager: self.ol_state_manager.clone(),
            ol_state_indexing_manager: self.ol_state_indexing_manager.clone(),
            ol_checkpoint_manager: self.ol_checkpoint_manager.clone(),
            proof_manager: self.proof_manager.clone(),
            prover_task_manager: self.prover_task_manager.clone(),
            l1_writer_manager: self.l1_writer_manager.clone(),
        }
    }
}

impl NodeStorage {
    /// Returns the raw database backend for direct access to databases without managers.
    pub fn db(&self) -> &Arc<SledBackend> {
        &self.db
    }

    /// Returns the tokio runtime handle used for blocking database operations.
    pub fn handle(&self) -> &Handle {
        &self.handle
    }

    pub fn asm(&self) -> &Arc<AsmStateManager> {
        &self.asm_state_manager
    }

    pub fn l1(&self) -> &Arc<L1BlockManager> {
        &self.l1_block_manager
    }

    pub fn client_state(&self) -> &Arc<ClientStateManager> {
        &self.client_state_manager
    }

    pub fn mmr_index(&self) -> &Arc<MmrIndexManager> {
        &self.mmr_index_manager
    }

    pub fn ol_block(&self) -> &Arc<OLBlockManager> {
        &self.ol_block_manager
    }

    pub fn mempool(&self) -> &Arc<MempoolDbManager> {
        &self.mempool_db_manager
    }

    pub fn ol_state(&self) -> &Arc<OLStateManager> {
        &self.ol_state_manager
    }

    pub fn ol_state_indexing(&self) -> &Arc<OLStateIndexingManager> {
        &self.ol_state_indexing_manager
    }

    pub fn ol_checkpoint(&self) -> &Arc<OLCheckpointManager> {
        &self.ol_checkpoint_manager
    }

    pub fn checkpoint_proof(&self) -> &Arc<CheckpointProofDbManager> {
        &self.proof_manager
    }

    pub fn prover_tasks(&self) -> &Arc<ProverTaskDbManager> {
        &self.prover_task_manager
    }

    pub fn l1_writer(&self) -> &Arc<L1WriterManager> {
        &self.l1_writer_manager
    }

    /// Returns the latest persisted ASM state on the canonical L1 chain, or
    /// [`None`] if there is none. May lag the L1 canonical tip when ASM is behind.
    ///
    /// [`AsmStateManager::fetch_most_recent_state_blocking`] can return an orphan
    /// from an abandoned reorg branch (those rows are never pruned); this resolves
    /// down the canonical chain instead. See STR-3832.
    pub fn fetch_canonical_asm_state_blocking(
        &self,
    ) -> DbResult<Option<(L1BlockCommitment, AsmState)>> {
        let Some((recent_block, recent_state)) =
            self.asm_state_manager.fetch_most_recent_state_blocking()?
        else {
            return Ok(None);
        };

        if self
            .l1_block_manager
            .get_canonical_blockid_at_height(recent_block.height())?
            == Some(*recent_block.blkid())
        {
            return Ok(Some((recent_block, recent_state)));
        }

        // Most-recent is an orphan. Resolve down from the canonical tip to the
        // highest block with a persisted ASM state.
        let Some((tip_height, _)) = self.l1_block_manager.get_canonical_chain_tip()? else {
            return Ok(None);
        };
        for height in (0..=tip_height).rev() {
            let Some(blockid) = self
                .l1_block_manager
                .get_canonical_blockid_at_height(height)?
            else {
                continue;
            };
            let block = L1BlockCommitment::new(height, blockid);
            if let Some(state) = self.asm_state_manager.get_state_blocking(block)? {
                return Ok(Some((block, state)));
            }
        }

        warn!(%recent_block, tip_height, "ASM has states but none on the canonical chain");
        Ok(None)
    }
}

/// Given a raw database, creates storage managers and returns a [`NodeStorage`]
/// instance around the underlying raw database.
pub fn create_node_storage(db: Arc<SledBackend>, handle: Handle) -> anyhow::Result<NodeStorage> {
    // Extract database references
    let asm_db = db.asm_db();
    let l1_db = db.l1_db();
    let client_state_db = db.client_state_db();
    let ol_block_db = db.ol_block_db();
    let mempool_db = db.mempool_db();
    let ol_state_db = db.ol_state_db();
    let ol_state_indexing_db = db.ol_state_indexing_db();
    let ol_checkpoint_db = db.ol_checkpoint_db();
    let mmr_index_db = db.mmr_index_db();
    let proof_db = db.checkpoint_proof_db();
    let prover_task_db = db.prover_task_db();

    let asm_manager = Arc::new(AsmStateManager::new(handle.clone(), asm_db));
    let l1_block_manager = Arc::new(L1BlockManager::new(handle.clone(), l1_db));

    let client_state_manager = Arc::new(
        ClientStateManager::new(handle.clone(), client_state_db).context("open client state")?,
    );

    let ol_block_manager = Arc::new(OLBlockManager::new(handle.clone(), ol_block_db));
    let mmr_index_manager = Arc::new(MmrIndexManager::new(handle.clone(), mmr_index_db));
    let mempool_db_manager = Arc::new(MempoolDbManager::new(handle.clone(), mempool_db));
    let ol_state_manager = Arc::new(OLStateManager::new(handle.clone(), ol_state_db.clone()));
    let ol_state_indexing_manager = Arc::new(OLStateIndexingManager::new(
        handle.clone(),
        ol_state_indexing_db,
    ));
    let ol_checkpoint_manager =
        Arc::new(OLCheckpointManager::new(handle.clone(), ol_checkpoint_db));
    let proof_manager = Arc::new(CheckpointProofDbManager::new(handle.clone(), proof_db));
    let prover_task_manager = Arc::new(ProverTaskDbManager::new(handle.clone(), prover_task_db));
    let l1_writer_manager = Arc::new(L1WriterManager::new(handle.clone(), db.writer_db()));

    Ok(NodeStorage {
        db,
        handle,
        asm_state_manager: asm_manager,
        l1_block_manager,
        client_state_manager,
        ol_block_manager,
        mmr_index_manager,
        mempool_db_manager,
        ol_state_manager,
        ol_state_indexing_manager,
        ol_checkpoint_manager,
        proof_manager,
        prover_task_manager,
        l1_writer_manager,
    })
}

/// Returns a tokio runtime [`Handle`] backed by a process-lifetime runtime, for
/// constructing storage managers in tests.
///
/// Database proxies dispatch blocking work via this handle, so tests that build
/// a [`NodeStorage`] (or individual managers) need one. The backing runtime is
/// created once and lives for the process, so the handle stays valid for both
/// the `*_blocking` and `*_async` paths regardless of the calling test's own
/// runtime.
#[cfg(any(test, feature = "test-utils"))]
pub fn test_runtime_handle() -> Handle {
    use std::sync::OnceLock;

    use tokio::runtime::Runtime;
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| Runtime::new().expect("test: build runtime"))
        .handle()
        .clone()
}

#[cfg(test)]
mod tests {
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_tests::asm_tests::make_test_asm_state;
    use strata_identifiers::{Buf32, L1BlockCommitment, L1BlockId};

    use super::*;

    fn setup() -> NodeStorage {
        let db = get_test_sled_backend();
        create_node_storage(db, test_runtime_handle()).expect("test: create node storage")
    }

    fn blkid(n: u8) -> L1BlockId {
        L1BlockId::from(Buf32::from([n; 32]))
    }

    fn extend_canonical(storage: &NodeStorage, up_to: u32) {
        for height in 1..=up_to {
            storage
                .l1()
                .extend_canonical_chain(&blkid(height as u8), height)
                .expect("test: extend canonical chain");
        }
    }

    // Orphan above the canonical tip: resolving to it would under-delete.
    #[test]
    fn canonical_resolution_prefers_canonical_over_higher_orphan() {
        let storage = setup();
        extend_canonical(&storage, 10);

        let canonical = L1BlockCommitment::new(10, blkid(10));
        // Height 12 is not on the canonical chain (canonical only reaches 10).
        let orphan = L1BlockCommitment::new(12, blkid(99));
        storage
            .asm()
            .put_state_blocking(canonical, make_test_asm_state())
            .unwrap();
        storage
            .asm()
            .put_state_blocking(orphan, make_test_asm_state())
            .unwrap();

        // Precondition: most-recent (highest-keyed) is the orphan.
        let (recent, _) = storage
            .asm()
            .fetch_most_recent_state_blocking()
            .unwrap()
            .unwrap();
        assert_eq!(recent, orphan, "setup: most-recent should be the orphan");

        let (resolved, _) = storage
            .fetch_canonical_asm_state_blocking()
            .unwrap()
            .unwrap();
        assert_eq!(resolved, canonical);
        assert_ne!(resolved, orphan);
    }

    // Same-height orphan sibling sorting above canonical: would over-delete.
    #[test]
    fn canonical_resolution_prefers_canonical_sibling_at_same_height() {
        let storage = setup();
        extend_canonical(&storage, 10);

        let canonical = L1BlockCommitment::new(10, blkid(10));
        let orphan = L1BlockCommitment::new(10, blkid(200));
        storage
            .asm()
            .put_state_blocking(canonical, make_test_asm_state())
            .unwrap();
        storage
            .asm()
            .put_state_blocking(orphan, make_test_asm_state())
            .unwrap();

        let (recent, _) = storage
            .asm()
            .fetch_most_recent_state_blocking()
            .unwrap()
            .unwrap();
        assert_eq!(
            recent, orphan,
            "setup: most-recent should be the higher-id orphan sibling"
        );

        let (resolved, _) = storage
            .fetch_canonical_asm_state_blocking()
            .unwrap()
            .unwrap();
        assert_eq!(resolved, canonical);
    }

    // No canonical chain tracked: resolve to nothing so the caller skips.
    #[test]
    fn canonical_resolution_none_without_canonical_state() {
        let storage = setup();
        let orphan = L1BlockCommitment::new(5, blkid(99));
        storage
            .asm()
            .put_state_blocking(orphan, make_test_asm_state())
            .unwrap();

        assert!(storage
            .fetch_canonical_asm_state_blocking()
            .unwrap()
            .is_none());
    }
}
