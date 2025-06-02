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
use strata_db::traits::Database;

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
    D: Database + Sync + Send + 'static,
{
    let l1_block_manager = Arc::new(L1BlockManager::new(pool.clone(), db.l1_db().clone()));
    let l2_block_manager = Arc::new(L2BlockManager::new(pool.clone(), db.clone()));
    let chainstate_manager = Arc::new(ChainstateManager::new(pool.clone(), db.clone()));

    let sync_event_manager = Arc::new(SyncEventManager::new(
        pool.clone(),
        db.sync_event_db().clone(),
    ));
    let client_state_manager =
        Arc::new(ClientStateManager::new(pool.clone(), db.clone()).context("open client state")?);

    // (see above)
    let checkpoint_manager = Arc::new(CheckpointDbManager::new(
        pool.clone(),
        db.checkpoint_db().clone(),
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
