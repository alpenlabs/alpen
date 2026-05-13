//! Storage for the Alpen codebase.

mod cache;
mod exec;
mod instrumentation;
mod managers;
pub mod ops;

use std::sync::Arc;

use anyhow::Context;
#[expect(deprecated, reason = "legacy old code is retained for compatibility")]
pub use managers::{
    asm::AsmStateManager,
    chainstate::ChainstateManager,
    checkpoint::CheckpointDbManager,
    checkpoint_proof::CheckpointProofDbManager,
    client_state::ClientStateManager,
    l1::L1BlockManager,
    l2::L2BlockManager,
    mempool::MempoolDbManager,
    mmr_index::{MmrAppendRequest, MmrIndexHandle, MmrIndexManager, MmrStateView},
    ol::OLBlockManager,
    ol_checkpoint::OLCheckpointManager,
    ol_state::OLStateManager,
    ol_state_indexing::OLStateIndexingManager,
    prover_task::ProverTaskDbManager,
    writer::L1WriterManager,
};
pub use ops::l1tx_broadcast::BroadcastDbOps;
use strata_db_store_sled::SledBackend;
pub use strata_db_types::MmrId;
use strata_db_types::{
    traits::{BlockStatus, DatabaseBackend},
    DbResult,
};
use strata_identifiers::{Epoch, EpochCommitment};

/// A consolidation of database managers.
// TODO move this to its own module
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct NodeStorage {
    /// Database backend for raw database access (needed for sequencer tasks)
    db: Arc<SledBackend>,
    /// Thread pool for blocking database operations
    pool: threadpool::ThreadPool,

    asm_state_manager: Arc<AsmStateManager>,
    l1_block_manager: Arc<L1BlockManager>,
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    l2_block_manager: Arc<L2BlockManager>,

    chainstate_manager: Arc<ChainstateManager>,

    client_state_manager: Arc<ClientStateManager>,

    // TODO maybe move this into a different one?
    // update: probably not, would require moving data around
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    checkpoint_manager: Arc<CheckpointDbManager>,

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
            pool: self.pool.clone(),
            asm_state_manager: self.asm_state_manager.clone(),
            l1_block_manager: self.l1_block_manager.clone(),
            l2_block_manager: self.l2_block_manager.clone(),
            chainstate_manager: self.chainstate_manager.clone(),
            client_state_manager: self.client_state_manager.clone(),
            checkpoint_manager: self.checkpoint_manager.clone(),
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

    /// Returns the thread pool for blocking database operations.
    pub fn pool(&self) -> &threadpool::ThreadPool {
        &self.pool
    }

    pub fn asm(&self) -> &Arc<AsmStateManager> {
        &self.asm_state_manager
    }

    pub fn l1(&self) -> &Arc<L1BlockManager> {
        &self.l1_block_manager
    }

    #[deprecated(note = "use `ol_block()` for OL/EE-decoupled block storage")]
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    pub fn l2(&self) -> &Arc<L2BlockManager> {
        &self.l2_block_manager
    }

    pub fn chainstate(&self) -> &Arc<ChainstateManager> {
        &self.chainstate_manager
    }

    pub fn client_state(&self) -> &Arc<ClientStateManager> {
        &self.client_state_manager
    }

    #[deprecated(note = "use `ol_checkpoint()` for OL/EE-decoupled checkpoint storage")]
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    pub fn checkpoint(&self) -> &Arc<CheckpointDbManager> {
        &self.checkpoint_manager
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

    /// Finds the valid epoch commitment for an epoch.
    ///
    /// Epoch commitment storage can contain fork candidates. This lookup
    /// resolves each candidate's terminal block status and returns the first
    /// valid one.
    pub async fn find_valid_epoch_commitment_at_async(
        &self,
        epoch: Epoch,
    ) -> DbResult<Option<EpochCommitment>> {
        let commitments = self
            .ol_checkpoint()
            .get_epoch_commitments_at_async(epoch)
            .await?;

        for commitment in commitments {
            let block_id = *commitment.last_blkid();
            if matches!(
                self.ol_block().get_block_status_async(block_id).await?,
                Some(BlockStatus::Valid)
            ) {
                return Ok(Some(commitment));
            }
        }

        Ok(None)
    }

    /// Finds the valid epoch commitment for an epoch.
    ///
    /// Epoch commitment storage can contain fork candidates. This lookup
    /// resolves each candidate's terminal block status and returns the first
    /// valid one.
    pub fn find_valid_epoch_commitment_at_blocking(
        &self,
        epoch: Epoch,
    ) -> DbResult<Option<EpochCommitment>> {
        let commitments = self
            .ol_checkpoint()
            .get_epoch_commitments_at_blocking(epoch)?;

        for commitment in commitments {
            let block_id = *commitment.last_blkid();
            if matches!(
                self.ol_block().get_block_status_blocking(block_id)?,
                Some(BlockStatus::Valid)
            ) {
                return Ok(Some(commitment));
            }
        }

        Ok(None)
    }
}

/// Given a raw database, creates storage managers and returns a [`NodeStorage`]
/// instance around the underlying raw database.
pub fn create_node_storage(
    db: Arc<SledBackend>,
    pool: threadpool::ThreadPool,
) -> anyhow::Result<NodeStorage> {
    // Extract database references
    let asm_db = db.asm_db();
    let l1_db = db.l1_db();
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    let l2_db = db.l2_db();
    let chainstate_db = db.chain_state_db();
    let client_state_db = db.client_state_db();
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    let checkpoint_db = db.checkpoint_db();
    let ol_block_db = db.ol_block_db();
    let mempool_db = db.mempool_db();
    let ol_state_db = db.ol_state_db();
    let ol_state_indexing_db = db.ol_state_indexing_db();
    let ol_checkpoint_db = db.ol_checkpoint_db();
    let mmr_index_db = db.mmr_index_db();
    let proof_db = db.checkpoint_proof_db();
    let prover_task_db = db.prover_task_db();

    let asm_manager = Arc::new(AsmStateManager::new(pool.clone(), asm_db));
    let l1_block_manager = Arc::new(L1BlockManager::new(pool.clone(), l1_db));
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    let l2_block_manager = Arc::new(L2BlockManager::new(pool.clone(), l2_db));
    let chainstate_manager = Arc::new(ChainstateManager::new(pool.clone(), chainstate_db));

    let client_state_manager = Arc::new(
        ClientStateManager::new(pool.clone(), client_state_db).context("open client state")?,
    );

    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    let checkpoint_manager = Arc::new(CheckpointDbManager::new(pool.clone(), checkpoint_db));

    let ol_block_manager = Arc::new(OLBlockManager::new(pool.clone(), ol_block_db));
    let mmr_index_manager = Arc::new(MmrIndexManager::new(pool.clone(), mmr_index_db));
    let mempool_db_manager = Arc::new(MempoolDbManager::new(pool.clone(), mempool_db));
    let ol_state_manager = Arc::new(OLStateManager::new(pool.clone(), ol_state_db.clone()));
    let ol_state_indexing_manager = Arc::new(OLStateIndexingManager::new(
        pool.clone(),
        ol_state_indexing_db,
    ));
    let ol_checkpoint_manager = Arc::new(OLCheckpointManager::new(pool.clone(), ol_checkpoint_db));
    let proof_manager = Arc::new(CheckpointProofDbManager::new(pool.clone(), proof_db));
    let prover_task_manager = Arc::new(ProverTaskDbManager::new(pool.clone(), prover_task_db));
    let l1_writer_manager = Arc::new(L1WriterManager::new(pool.clone(), db.writer_db()));

    Ok(NodeStorage {
        db,
        pool,
        asm_state_manager: asm_manager,
        l1_block_manager,
        l2_block_manager,
        chainstate_manager,
        client_state_manager,
        checkpoint_manager,
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

#[cfg(test)]
mod tests {
    use strata_checkpoint_types::EpochSummary;
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::traits::BlockStatus;
    use strata_identifiers::{
        Buf32, Buf64, L1BlockCommitment, L1BlockId, OLBlockCommitment, OLBlockId,
    };
    use strata_ol_chain_types_new::{
        BlockFlags, OLBlock, OLBlockBody, OLBlockHeader, OLTxSegment, SignedOLBlockHeader,
    };
    use threadpool::ThreadPool;

    use super::*;

    fn setup_storage() -> NodeStorage {
        let db = get_test_sled_backend();
        create_node_storage(db, ThreadPool::new(1)).expect("create node storage")
    }

    fn make_block(slot: u64, epoch: Epoch, seed: u8) -> OLBlock {
        let tx_segment = OLTxSegment::new(Vec::new()).expect("empty tx segment is valid");
        let body = OLBlockBody::new_common(tx_segment);
        let mut flags = BlockFlags::zero();
        flags.set_is_terminal(true);
        let header = OLBlockHeader::new(
            1_000_000 + slot,
            flags,
            slot,
            epoch,
            OLBlockId::from(Buf32::from([seed; 32])),
            body.compute_hash_commitment(),
            Buf32::from([seed.saturating_add(1); 32]),
            Buf32::from([seed.saturating_add(2); 32]),
        );

        OLBlock::new(SignedOLBlockHeader::new(header, Buf64::zero()), body)
    }

    fn insert_summary(storage: &NodeStorage, block: &OLBlock) -> EpochCommitment {
        let commitment = block.header().compute_block_commitment();
        let prev_terminal = OLBlockCommitment::new(0, OLBlockId::from(Buf32::zero()));
        let new_l1 = L1BlockCommitment::new(1, L1BlockId::from(Buf32::from([3; 32])));
        let summary = EpochSummary::new(
            block.header().epoch(),
            commitment,
            prev_terminal,
            new_l1,
            Buf32::from([4; 32]),
        );
        let epoch_commitment = summary.get_epoch_commitment();

        storage
            .ol_checkpoint()
            .insert_epoch_summary_blocking(summary)
            .expect("insert summary");

        epoch_commitment
    }

    #[test]
    fn find_valid_epoch_commitment_returns_valid_candidate() {
        let storage = setup_storage();
        let epoch = 1;
        let invalid_block = make_block(10, epoch, 1);
        let valid_block = make_block(11, epoch, 2);

        storage
            .ol_block()
            .put_block_data_blocking(invalid_block.clone())
            .expect("put invalid block");
        storage
            .ol_block()
            .put_block_data_blocking(valid_block.clone())
            .expect("put valid block");
        let _invalid_commitment = insert_summary(&storage, &invalid_block);
        let valid_commitment = insert_summary(&storage, &valid_block);

        storage
            .ol_block()
            .set_block_status_blocking(invalid_block.header().compute_blkid(), BlockStatus::Invalid)
            .expect("mark invalid");
        storage
            .ol_block()
            .set_block_status_blocking(valid_block.header().compute_blkid(), BlockStatus::Valid)
            .expect("mark valid");

        let found = storage
            .find_valid_epoch_commitment_at_blocking(epoch)
            .expect("find commitment")
            .expect("valid commitment");
        assert_eq!(found, valid_commitment);
    }

    #[test]
    fn find_valid_epoch_commitment_returns_none_without_valid_candidate() {
        let storage = setup_storage();
        let epoch = 1;
        let block = make_block(10, epoch, 1);

        storage
            .ol_block()
            .put_block_data_blocking(block.clone())
            .expect("put block");
        insert_summary(&storage, &block);
        storage
            .ol_block()
            .set_block_status_blocking(block.header().compute_blkid(), BlockStatus::Invalid)
            .expect("mark invalid");

        let found = storage
            .find_valid_epoch_commitment_at_blocking(epoch)
            .expect("find commitment");
        assert_eq!(found, None);
    }
}
