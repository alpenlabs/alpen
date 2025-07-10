//! Storage for the Alpen codebase.

mod cache;
mod exec;
mod managers;
pub mod ops;

use std::sync::Arc;

use anyhow::Context;
pub use managers::{
    chainstate::ChainstateManager, checkpoint::CheckpointDbManager,
    client_state::ClientStateManager, l1::L1BlockManager, l2::L2BlockManager,
    sync_event::SyncEventManager,
};
pub use ops::l1tx_broadcast::BroadcastDbOps;
use strata_db::traits::DatabaseBackend;

/// A consolidation of database managers.
// TODO move this to its own module
#[derive(Clone)]
#[expect(missing_debug_implementations)]
pub struct NodeStorage {
    l1_block_manager: Arc<L1BlockManager>,
    l2_block_manager: Arc<L2BlockManager>,
    chainstate_manager: Arc<ChainstateManager>,

    sync_event_manager: Arc<SyncEventManager>,
    client_state_manager: Arc<ClientStateManager>,

    // TODO maybe move this into a different one?
    // update: probably not, would require moving data around
    checkpoint_manager: Arc<CheckpointDbManager>,
}

impl NodeStorage {
    pub fn l1(&self) -> &Arc<L1BlockManager> {
        &self.l1_block_manager
    }

    pub fn l2(&self) -> &Arc<L2BlockManager> {
        &self.l2_block_manager
    }

    pub fn chainstate(&self) -> &Arc<ChainstateManager> {
        &self.chainstate_manager
    }

    pub fn sync_event(&self) -> &Arc<SyncEventManager> {
        &self.sync_event_manager
    }

    pub fn client_state(&self) -> &Arc<ClientStateManager> {
        &self.client_state_manager
    }

    pub fn checkpoint(&self) -> &Arc<CheckpointDbManager> {
        &self.checkpoint_manager
    }
}

/// Given a raw database, creates storage managers and returns a [`NodeStorage`]
/// instance around the underlying raw database.
pub fn create_node_storage<D>(
    db: Arc<D>,
    pool: threadpool::ThreadPool,
) -> anyhow::Result<NodeStorage>
where
    D: DatabaseBackend + 'static,
{
    // Extract database references first to ensure they live long enough
    let l1_db = db.l1_db();
    let l2_db = db.l2_db();
    let chainstate_db = db.chain_state_db();
    let sync_event_db = db.sync_event_db();
    let client_state_db = db.client_state_db();
    let checkpoint_db = db.checkpoint_db();

    let l1_block_manager = Arc::new(L1BlockManager::new(pool.clone(), l1_db));
    let l2_block_manager = Arc::new(L2BlockManager::new(pool.clone(), l2_db));
    let chainstate_manager = Arc::new(ChainstateManager::new(pool.clone(), chainstate_db));

    let sync_event_manager = Arc::new(SyncEventManager::new(
        pool.clone(),
        sync_event_db,
    ));
    let client_state_manager =
        Arc::new(ClientStateManager::new(pool.clone(), client_state_db).context("open client state")?);

    // (see above)
    let checkpoint_manager = Arc::new(CheckpointDbManager::new(
        pool.clone(),
        checkpoint_db,
    ));

    Ok(NodeStorage {
        l1_block_manager,
        l2_block_manager,
        chainstate_manager,

        sync_event_manager,
        client_state_manager,

        // (see above)
        checkpoint_manager,
    })
}
