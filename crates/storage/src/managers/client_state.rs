//! Client state manager.
// TODO should this also include sync events?

use std::sync::Arc;

use strata_db::{traits::ClientStateDatabase, DbResult};
use strata_primitives::l1::L1BlockCommitment;
use strata_state::{client_state::ClientState, operation::ClientUpdateOutput};
use threadpool::ThreadPool;
use tokio::sync::Mutex;

use crate::{
    cache,
    ops::client_state::{ClientStateOps, Context},
};

#[expect(missing_debug_implementations)]
pub struct ClientStateManager {
    ops: ClientStateOps,

    // TODO actually use caches
    update_cache: cache::CacheTable<u64, Option<ClientUpdateOutput>>,
    state_cache: cache::CacheTable<u64, Arc<ClientState>>,

    cur_state: Mutex<CurStateTracker>,
}

impl ClientStateManager {
    pub fn new(pool: ThreadPool, db: Arc<impl ClientStateDatabase + 'static>) -> DbResult<Self> {
        let ops = Context::new(db).into_ops(pool);
        let update_cache = cache::CacheTable::new(64.try_into().unwrap());
        let state_cache = cache::CacheTable::new(64.try_into().unwrap());

        // Setup the tracker to point at the last or default pregenesis client state.
        let mut cur_state = CurStateTracker::new_empty();

        let latest_cs = ops.get_latest_client_state_blocking()?;
        if let Some((blk, cs)) = latest_cs {
            cur_state.set(blk.height(), Arc::new(cs));
        }

        Ok(Self {
            ops,
            update_cache,
            state_cache,
            cur_state: Mutex::new(cur_state),
        })
    }

    // TODO convert to managing these with Arcs
    pub async fn get_state_async(&self, block: L1BlockCommitment) -> DbResult<Option<ClientState>> {
        Ok(self
            .ops
            .get_client_update_async(block)
            .await?
            .map(|update| update.into_state()))
    }

    pub fn get_state_blocking(&self, block: L1BlockCommitment) -> DbResult<Option<ClientState>> {
        Ok(self
            .ops
            .get_client_update_blocking(block)?
            .map(|update| update.into_state()))
    }

    pub fn get_update_blocking(
        &self,
        block: &L1BlockCommitment,
    ) -> DbResult<Option<ClientUpdateOutput>> {
        self.ops.get_client_update_blocking(*block)
    }

    pub fn put_update_blocking(
        &self,
        block: &L1BlockCommitment,
        update: ClientUpdateOutput,
    ) -> DbResult<Arc<ClientState>> {
        // FIXME this is a lot of cloning, good thing the type isn't gigantic,
        // still feels bad though
        let state = Arc::new(update.state().clone());
        let height = block.height();
        self.ops
            .put_client_update_blocking(*block, update.clone())?;
        self.maybe_update_cur_state_blocking(height, &state);
        self.update_cache.insert_blocking(height, Some(update));
        self.state_cache.insert_blocking(height, state.clone());
        Ok(state)
    }

    fn maybe_update_cur_state_blocking(&self, height: u64, state: &Arc<ClientState>) -> bool {
        let mut cur = self.cur_state.blocking_lock();
        cur.maybe_update(height, state)
    }

    /// Returns either pre-genesis init [`ClientState`] or the one with the biggest height.
    pub fn fetch_most_recent_state(&self) -> DbResult<Option<(L1BlockCommitment, ClientState)>> {
        self.ops.get_latest_client_state_blocking()
    }
}

/// Internally tracks the current state so we can fetch it as needed.
#[derive(Debug)]
struct CurStateTracker {
    last_idx: Option<u64>,
    state: Option<Arc<ClientState>>,
}

impl CurStateTracker {
    fn new_empty() -> Self {
        Self {
            last_idx: None,
            state: None,
        }
    }

    fn set(&mut self, idx: u64, state: Arc<ClientState>) {
        self.last_idx = Some(idx);
        self.state = Some(state);
    }

    fn is_idx_better(&self, idx: u64) -> bool {
        self.last_idx.is_none_or(|v| idx >= v)
    }

    fn maybe_update(&mut self, idx: u64, state: &Arc<ClientState>) -> bool {
        let should = self.is_idx_better(idx);
        if should {
            self.set(idx, state.clone());
        }
        should
    }
}
