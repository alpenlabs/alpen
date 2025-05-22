//! Client state manager.
// TODO should this also include sync events?

use std::sync::Arc;

use strata_db::{traits::Database, DbError, DbResult};
use strata_state::{
    client_state::{ClientState, L1ClientState},
    l1::L1BlockId,
    operation::ClientUpdateOutput,
};
use threadpool::ThreadPool;
use tokio::sync::Mutex;
use tracing::*;

use crate::{cache, ops};

#[expect(missing_debug_implementations)]
pub struct ClientStateManager {
    ops: ops::client_state::ClientStateOps,

    // TODO actually use caches
    update_cache: cache::CacheTable<u64, Option<ClientUpdateOutput>>,
    state_cache: cache::CacheTable<u64, Arc<ClientState>>,
    l1_cs_cache: cache::CacheTable<L1BlockId, Option<L1ClientState>>,

    cur_state: Mutex<CurStateTracker>,
}

impl ClientStateManager {
    pub fn new<D: Database + Sync + Send + 'static>(
        pool: ThreadPool,
        db: Arc<D>,
    ) -> DbResult<Self> {
        let ops = ops::client_state::Context::new(db.client_state_db().clone()).into_ops(pool);
        let update_cache = cache::CacheTable::new(64.try_into().unwrap());
        let state_cache = cache::CacheTable::new(64.try_into().unwrap());
        let l1_cs_cache = cache::CacheTable::new(64.try_into().unwrap());

        // Figure out the current state so we can access it.
        let mut cur_state = CurStateTracker::new_empty();
        match ops.get_last_state_idx_blocking() {
            Ok(last_idx) => {
                let last_state = ops
                    .get_client_update_blocking(last_idx)?
                    .ok_or(DbError::UnknownIdx(last_idx))?
                    .into_state();
                cur_state.set(last_idx, Arc::new(last_state));
            }
            Err(DbError::NotBootstrapped) => {
                warn!("haven't bootstrapped yet, unable to prepopulate the cur state cache");
            }
            Err(e) => return Err(e),
        }

        Ok(Self {
            ops,
            update_cache,
            state_cache,
            l1_cs_cache,
            cur_state: Mutex::new(cur_state),
        })
    }

    pub fn get_last_state_idx_blocking(&self) -> DbResult<u64> {
        self.ops.get_last_state_idx_blocking()
    }

    // TODO convert to managing these with Arcs
    pub async fn get_state_async(&self, idx: u64) -> DbResult<Option<ClientState>> {
        self.ops
            .get_client_update_async(idx)
            .await
            .map(|res| res.map(|update| update.into_state()))
    }

    pub fn get_state_blocking(&self, idx: u64) -> DbResult<Option<ClientState>> {
        self.ops
            .get_client_update_blocking(idx)
            .map(|res| res.map(|update| update.into_state()))
    }

    pub async fn get_update_async(&self, idx: u64) -> DbResult<Option<ClientUpdateOutput>> {
        self.ops.get_client_update_async(idx).await
    }

    pub fn put_update_blocking(
        &self,
        idx: u64,
        update: ClientUpdateOutput,
    ) -> DbResult<Arc<ClientState>> {
        // FIXME this is a lot of cloning, good thing the type isn't gigantic,
        // still feels bad though
        let state = Arc::new(update.state().clone());
        self.ops.put_client_update_blocking(idx, update.clone())?;
        self.maybe_update_cur_state_blocking(idx, &state);
        self.update_cache.insert(idx, Some(update));
        self.state_cache.insert(idx, state.clone());
        Ok(state)
    }

    // TODO rollback and whatnot

    // Internal functions.

    fn maybe_update_cur_state_blocking(&self, idx: u64, state: &Arc<ClientState>) -> bool {
        let mut cur = self.cur_state.blocking_lock();
        cur.maybe_update(idx, state)
    }

    // Convenience functions.

    /// Gets the highest known state and its idx.
    pub async fn get_most_recent_state(&self) -> Option<(u64, Arc<ClientState>)> {
        let cur = self.cur_state.lock().await;
        cur.get_clone().map(|state| (cur.get_idx(), state))
    }

    /// Gets the highest known state and its idx.
    pub fn get_most_recent_state_blocking(&self) -> Option<(u64, Arc<ClientState>)> {
        let cur = self.cur_state.blocking_lock();
        cur.get_clone().map(|state| (cur.get_idx(), state))
    }

    pub async fn get_l1_state(&self, id: &L1BlockId) -> DbResult<Option<L1ClientState>> {
        self.l1_cs_cache
            .get_or_fetch(id, || self.ops.get_client_state_chan(*id))
            .await
    }

    pub fn get_l1_state_blocking(&self, id: &L1BlockId) -> DbResult<Option<L1ClientState>> {
        self.l1_cs_cache
            .get_or_fetch_blocking(id, || self.ops.get_client_state_blocking(*id))
    }

    pub async fn put_l1_state(&self, id: L1BlockId, state: L1ClientState) -> DbResult<()> {
        self.ops.put_client_state_async(id, state).await?;
        self.l1_cs_cache.purge(&id);
        Ok(())
    }

    pub fn put_l1_state_blocking(&self, id: L1BlockId, state: L1ClientState) -> DbResult<()> {
        self.ops.put_client_state_blocking(id, state)?;
        self.l1_cs_cache.purge(&id);
        Ok(())
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

    fn get_idx(&self) -> u64 {
        self.last_idx.unwrap_or_default()
    }

    fn get_clone(&self) -> Option<Arc<ClientState>> {
        self.state.clone()
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
