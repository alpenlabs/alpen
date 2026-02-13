//! Storage for the Alpen codebase.

mod cache;
mod exec;
mod instrumentation;
mod managers;
pub mod ops;

use std::sync::Arc;

use anyhow::Context;
pub use managers::{
    account_genesis::AccountGenesisManager,
    asm::AsmStateManager,
    chainstate::ChainstateManager,
    checkpoint::CheckpointDbManager,
    client_state::ClientStateManager,
    global_mmr::{GlobalMmrManager, MmrHandle},
    l1::L1BlockManager,
    l2::L2BlockManager,
    mempool::MempoolDbManager,
    ol::OLBlockManager,
    ol_checkpoint::OLCheckpointManager,
    ol_state::OLStateManager,
};
pub use ops::l1tx_broadcast::BroadcastDbOps;
use strata_db_store_sled::SledBackend;
use strata_db_types::traits::DatabaseBackend;
// Re-export MmrId from identifiers for convenience
pub use strata_identifiers::MmrId;

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

    account_genesis_manager: Arc<AccountGenesisManager>,
    asm_state_manager: Arc<AsmStateManager>,
    l1_block_manager: Arc<L1BlockManager>,
    l2_block_manager: Arc<L2BlockManager>,

    chainstate_manager: Arc<ChainstateManager>,

    client_state_manager: Arc<ClientStateManager>,

    // TODO maybe move this into a different one?
    // update: probably not, would require moving data around
    checkpoint_manager: Arc<CheckpointDbManager>,

    ol_block_manager: Arc<OLBlockManager>,
    global_mmr_manager: Arc<GlobalMmrManager>,
    mempool_db_manager: Arc<MempoolDbManager>,
    ol_state_manager: Arc<OLStateManager>,
    ol_checkpoint_manager: Arc<OLCheckpointManager>,
}

impl Clone for NodeStorage {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            pool: self.pool.clone(),
            account_genesis_manager: self.account_genesis_manager.clone(),
            asm_state_manager: self.asm_state_manager.clone(),
            l1_block_manager: self.l1_block_manager.clone(),
            l2_block_manager: self.l2_block_manager.clone(),
            chainstate_manager: self.chainstate_manager.clone(),
            client_state_manager: self.client_state_manager.clone(),
            checkpoint_manager: self.checkpoint_manager.clone(),
            ol_block_manager: self.ol_block_manager.clone(),
            global_mmr_manager: self.global_mmr_manager.clone(),
            mempool_db_manager: self.mempool_db_manager.clone(),
            ol_state_manager: self.ol_state_manager.clone(),
            ol_checkpoint_manager: self.ol_checkpoint_manager.clone(),
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

    pub fn account_genesis(&self) -> &Arc<AccountGenesisManager> {
        &self.account_genesis_manager
    }

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

    pub fn global_mmr(&self) -> &Arc<GlobalMmrManager> {
        &self.global_mmr_manager
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

    pub fn ol_checkpoint(&self) -> &Arc<OLCheckpointManager> {
        &self.ol_checkpoint_manager
    }
}

/// Given a raw database, creates storage managers and returns a [`NodeStorage`]
/// instance around the underlying raw database.
pub fn create_node_storage(
    db: Arc<SledBackend>,
    pool: threadpool::ThreadPool,
) -> anyhow::Result<NodeStorage> {
    // Extract database references
    let account_genesis_db = db.account_genesis_db();
    let asm_db = db.asm_db();
    let l1_db = db.l1_db();
    let l2_db = db.l2_db();
    let chainstate_db = db.chain_state_db();
    let client_state_db = db.client_state_db();
    let checkpoint_db = db.checkpoint_db();
    let ol_block_db = db.ol_block_db();
    let mempool_db = db.mempool_db();
    let ol_state_db = db.ol_state_db();
    let ol_checkpoint_db = db.ol_checkpoint_db();
    let global_mmr_db = db.global_mmr_db();

    let account_genesis_manager =
        Arc::new(AccountGenesisManager::new(pool.clone(), account_genesis_db));
    let asm_manager = Arc::new(AsmStateManager::new(pool.clone(), asm_db));
    let l1_block_manager = Arc::new(L1BlockManager::new(pool.clone(), l1_db));
    let l2_block_manager = Arc::new(L2BlockManager::new(pool.clone(), l2_db));
    let chainstate_manager = Arc::new(ChainstateManager::new(pool.clone(), chainstate_db));

    let client_state_manager = Arc::new(
        ClientStateManager::new(pool.clone(), client_state_db).context("open client state")?,
    );

    let checkpoint_manager = Arc::new(CheckpointDbManager::new(pool.clone(), checkpoint_db));

    let ol_block_manager = Arc::new(OLBlockManager::new(pool.clone(), ol_block_db));
    let global_mmr_manager = Arc::new(GlobalMmrManager::new(pool.clone(), global_mmr_db));
    let mempool_db_manager = Arc::new(MempoolDbManager::new(pool.clone(), mempool_db));
    let ol_state_manager = Arc::new(OLStateManager::new(pool.clone(), ol_state_db.clone()));
    let ol_checkpoint_manager = Arc::new(OLCheckpointManager::new(pool.clone(), ol_checkpoint_db));

    Ok(NodeStorage {
        db,
        pool,
        account_genesis_manager,
        asm_state_manager: asm_manager,
        l1_block_manager,
        l2_block_manager,
        chainstate_manager,
        client_state_manager,
        checkpoint_manager,
        ol_block_manager,
        global_mmr_manager,
        mempool_db_manager,
        ol_state_manager,
        ol_checkpoint_manager,
    })
}
