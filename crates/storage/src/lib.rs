//! Storage for the Alpen codebase.

mod cache;
mod exec;
mod managers;
pub mod ops;

use std::sync::Arc;

use anyhow::Context;
use managers::mmr::MmrManager;
pub use managers::{
    asm::AsmStateManager, chainstate::ChainstateManager, checkpoint::CheckpointDbManager,
    client_state::ClientStateManager, l1::L1BlockManager, l2::L2BlockManager,
};
pub use ops::l1tx_broadcast::BroadcastDbOps;
use strata_db_types::traits::DatabaseBackend;

/// A consolidation of database managers.
// TODO move this to its own module
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct NodeStorage {
    asm_state_manager: Arc<AsmStateManager>,
    l1_block_manager: Arc<L1BlockManager>,
    l2_block_manager: Arc<L2BlockManager>,

    chainstate_manager: Arc<ChainstateManager>,

    client_state_manager: Arc<ClientStateManager>,

    // TODO maybe move this into a different one?
    // update: probably not, would require moving data around
    checkpoint_manager: Arc<CheckpointDbManager>,

    mmr_manager: Option<Arc<MmrManager>>,

    /// Direct access to MMR database for WorkerContext
    mmr_db: Option<Arc<strata_db_store_sled::asm::SledMmrDb>>,
}

impl Clone for NodeStorage {
    fn clone(&self) -> Self {
        Self {
            asm_state_manager: self.asm_state_manager.clone(),
            l1_block_manager: self.l1_block_manager.clone(),
            l2_block_manager: self.l2_block_manager.clone(),
            chainstate_manager: self.chainstate_manager.clone(),
            client_state_manager: self.client_state_manager.clone(),
            checkpoint_manager: self.checkpoint_manager.clone(),
            mmr_manager: self.mmr_manager.clone(),
            mmr_db: self.mmr_db.clone(),
        }
    }
}

impl NodeStorage {
    pub fn asm(&self) -> &Arc<AsmStateManager> {
        &self.asm_state_manager
    }

    pub fn l1(&self) -> &Arc<L1BlockManager> {
        &self.l1_block_manager
    }

    pub fn l2(&self) -> &Arc<L2BlockManager> {
        &self.l2_block_manager
    }

    pub fn chainstate(&self) -> &Arc<ChainstateManager> {
        &self.chainstate_manager
    }

    pub fn client_state(&self) -> &Arc<ClientStateManager> {
        &self.client_state_manager
    }

    pub fn checkpoint(&self) -> &Arc<CheckpointDbManager> {
        &self.checkpoint_manager
    }

    pub fn mmr(&self) -> Option<&Arc<MmrManager>> {
        self.mmr_manager.as_ref()
    }

    /// Get direct access to the MMR database (for WorkerContext)
    pub fn mmr_db(&self) -> Option<&Arc<strata_db_store_sled::asm::SledMmrDb>> {
        self.mmr_db.as_ref()
    }
}

/// Given a raw database, creates storage managers and returns a [`NodeStorage`]
/// instance around the underlying raw database.
///
/// Note: This does not initialize MMR manager. Use `create_node_storage_with_sled()` for full
/// functionality.
pub fn create_node_storage<D>(
    db: Arc<D>,
    pool: threadpool::ThreadPool,
) -> anyhow::Result<NodeStorage>
where
    D: DatabaseBackend + 'static,
{
    // Extract database references first to ensure they live long enough
    let asm_db = db.asm_db();
    let l1_db = db.l1_db();
    let l2_db = db.l2_db();
    let chainstate_db = db.chain_state_db();
    let client_state_db = db.client_state_db();
    let checkpoint_db = db.checkpoint_db();

    let asm_manager = Arc::new(AsmStateManager::new(pool.clone(), asm_db));
    let l1_block_manager = Arc::new(L1BlockManager::new(pool.clone(), l1_db));
    let l2_block_manager = Arc::new(L2BlockManager::new(pool.clone(), l2_db));
    let chainstate_manager = Arc::new(ChainstateManager::new(pool.clone(), chainstate_db));

    let client_state_manager = Arc::new(
        ClientStateManager::new(pool.clone(), client_state_db).context("open client state")?,
    );

    let checkpoint_manager = Arc::new(CheckpointDbManager::new(pool.clone(), checkpoint_db));

    Ok(NodeStorage {
        asm_state_manager: asm_manager,
        l1_block_manager,
        l2_block_manager,
        chainstate_manager,
        client_state_manager,
        checkpoint_manager,
        mmr_manager: None,
        mmr_db: None,
    })
}

/// Creates node storage from Sled backend with MMR database support
///
/// This function creates all storage managers including the MMR manager for proof generation.
pub fn create_node_storage_with_sled(
    db: Arc<impl DatabaseBackend + 'static>,
    asm_db_sled: Arc<strata_db_store_sled::AsmDBSled>,
    pool: threadpool::ThreadPool,
) -> anyhow::Result<NodeStorage> {
    // Extract database references first to ensure they live long enough
    let asm_db = db.asm_db();
    let l1_db = db.l1_db();
    let l2_db = db.l2_db();
    let chainstate_db = db.chain_state_db();
    let client_state_db = db.client_state_db();
    let checkpoint_db = db.checkpoint_db();

    let asm_manager = Arc::new(AsmStateManager::new(pool.clone(), asm_db));
    let l1_block_manager = Arc::new(L1BlockManager::new(pool.clone(), l1_db));
    let l2_block_manager = Arc::new(L2BlockManager::new(pool.clone(), l2_db));
    let chainstate_manager = Arc::new(ChainstateManager::new(pool.clone(), chainstate_db));

    let client_state_manager = Arc::new(
        ClientStateManager::new(pool.clone(), client_state_db).context("open client state")?,
    );

    let checkpoint_manager = Arc::new(CheckpointDbManager::new(pool.clone(), checkpoint_db));

    // Create MMR manager from Sled MMR database
    let mmr_db = Arc::new(
        strata_db_store_sled::asm::SledMmrDb::new(
            asm_db_sled.mmr_node_tree.clone(),
            asm_db_sled.mmr_meta_tree.clone(),
        )
        .context("create MMR database")?,
    );
    let mmr_manager_sled = Arc::new(MmrManager::new(pool.clone(), mmr_db.clone()));

    Ok(NodeStorage {
        asm_state_manager: asm_manager,
        l1_block_manager,
        l2_block_manager,
        chainstate_manager,
        client_state_manager,
        checkpoint_manager,
        mmr_manager: Some(mmr_manager_sled),
        mmr_db: Some(mmr_db),
    })
}
