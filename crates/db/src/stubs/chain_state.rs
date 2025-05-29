use std::collections::*;

use parking_lot::Mutex;
use strata_state::{chain_state::Chainstate, id::L2BlockId, state_op::WriteBatchEntry};
use tracing::*;

use crate::{errors::DbError, traits::*, DbResult};

#[derive(Debug)]
struct InnerState {
    write_batches: BTreeMap<u64, WriteBatchEntry>,
    toplevels: BTreeMap<u64, Chainstate>,
}

impl InnerState {
    pub(crate) const fn new() -> Self {
        Self {
            write_batches: BTreeMap::new(),
            toplevels: BTreeMap::new(),
        }
    }

    fn find_last_write_batch(&self) -> u64 {
        self.toplevels
            .last_key_value()
            .map(|(k, _)| *k)
            .expect("chainstatedb: genesis not written")
    }
}

#[derive(Debug)]
pub struct StubChainstateDb {
    state: Mutex<InnerState>,
}

impl Default for StubChainstateDb {
    fn default() -> Self {
        Self::new()
    }
}

impl StubChainstateDb {
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(InnerState::new()),
        }
    }
}

impl ChainstateDatabase for StubChainstateDb {
    fn write_genesis_state(&self, toplevel: Chainstate, _blockid: L2BlockId) -> DbResult<()> {
        let mut st = self.state.lock();
        st.toplevels.insert(0, toplevel.clone());
        Ok(())
    }

    fn put_write_batch(&self, idx: u64, batch: WriteBatchEntry) -> DbResult<()> {
        let mut st = self.state.lock();

        let last_idx = st.find_last_write_batch();
        if idx <= last_idx {
            return Err(DbError::OooInsert("chainstate", idx));
        }

        let _toplevel = st
            .toplevels
            .get(&last_idx)
            .cloned()
            .expect("chainstatedb: nonsense");

        // Compute new state and insert things.
        st.write_batches.insert(idx, batch.clone());
        let (wb, _blockid) = batch.to_parts();
        st.toplevels.insert(idx, wb.into_toplevel());

        Ok(())
    }

    fn purge_entries_before(&self, before_idx: u64) -> DbResult<()> {
        let mut st = self.state.lock();

        if !st.toplevels.contains_key(&before_idx) {
            return Err(DbError::UnknownIdx(before_idx));
        }

        let last_idx = st.find_last_write_batch();
        if before_idx >= last_idx {
            return Err(DbError::PurgeTooRecent);
        }

        // Remove from the two tables.  This does have to touch every state in
        // the table but it's fine because this will never be used in production.
        let states_removed = st.toplevels.extract_if(0..before_idx, |_, _| true).count();
        let writes_removed = st
            .write_batches
            .extract_if(0..before_idx, |_, _| true)
            .count();

        // In case it screws up we should remember it.
        trace!(%states_removed, %writes_removed, %before_idx, "purge_historical_state_before");

        Ok(())
    }

    fn rollback_writes_to(&self, new_tip_idx: u64) -> DbResult<()> {
        let mut st = self.state.lock();

        if !st.toplevels.contains_key(&new_tip_idx) {
            return Err(DbError::UnknownIdx(new_tip_idx));
        }

        let last_idx = st.find_last_write_batch();
        if new_tip_idx > last_idx {
            return Err(DbError::RevertAboveCurrent(new_tip_idx, last_idx));
        }

        // We take a more sensitive approach to this since we don't want to have to
        let to_remove = st
            .toplevels
            .iter()
            .rev()
            .take_while(|(idx, _)| **idx > new_tip_idx)
            .map(|(idx, _)| *idx)
            .collect::<Vec<_>>();

        for rem in to_remove {
            assert!(st.toplevels.remove(&rem).is_some());
            assert!(st.write_batches.remove(&rem).is_some());
        }

        // Check that we're where we expect to be.
        let (k, _) = st.toplevels.last_key_value().unwrap();
        assert_eq!(*k, new_tip_idx);

        Ok(())
    }

    fn get_last_write_idx(&self) -> DbResult<u64> {
        let st = self.state.lock();
        Ok(st.find_last_write_batch())
    }

    fn get_earliest_write_idx(&self) -> DbResult<u64> {
        let st = self.state.lock();
        let idx = st
            .toplevels
            .first_key_value()
            .map(|(k, _)| *k)
            .expect("chainstatedb: genesis not written");
        Ok(idx)
    }

    fn get_write_batch(&self, idx: u64) -> DbResult<Option<WriteBatchEntry>> {
        let st = self.state.lock();
        Ok(st.write_batches.get(&idx).cloned())
    }
}
