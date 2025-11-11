use strata_db_types::{
    DbError, DbResult,
    chainstate::{ChainstateDatabase, StateInstanceId, WriteBatchId},
};
use strata_ol_chainstate_types::{Chainstate, WriteBatch};
use strata_primitives::buf::Buf32;

use crate::{
    chain_state::schemas::{StateInstanceEntry, StateInstanceSchema, WriteBatchSchema},
    define_sled_database,
    utils::find_next_available_id,
};

define_sled_database!(
    pub struct ChainstateDBSled {
        state_tree: StateInstanceSchema,
        write_batch_tree: WriteBatchSchema,
    }
);

impl ChainstateDBSled {
    fn next_state_id_or_zero(&self) -> DbResult<u64> {
        let next_id = match self.state_tree.last()? {
            Some((id, _)) => id + 1,
            None => 0,
        };
        Ok(next_id)
    }
}

impl ChainstateDatabase for ChainstateDBSled {
    fn create_new_inst(&self, toplevel: Chainstate) -> DbResult<StateInstanceId> {
        let entry = StateInstanceEntry::new(toplevel);
        let next_id = self.next_state_id_or_zero()?;
        let result = self.config.with_retry((&self.state_tree,), |view| {
            let st_tree = view.0;
            let nxt = find_next_available_id(&st_tree, next_id)?;
            st_tree.insert(&nxt, &entry)?;
            Ok(nxt)
        })?;
        Ok(result)
    }

    fn clone_inst(&self, id: StateInstanceId) -> DbResult<StateInstanceId> {
        let entry = self
            .state_tree
            .get(&id)?
            .ok_or(DbError::MissingStateInstance)?;
        let next_id = self.next_state_id_or_zero()?;
        if next_id == 0 {
            return Err(DbError::MissingStateInstance);
        }
        let next = self.config.with_retry((&self.state_tree,), |(st_tree,)| {
            let mut nxt = next_id;
            while st_tree.get(&nxt)?.is_some() {
                nxt += 1;
            }
            st_tree.insert(&nxt, &entry)?;
            Ok(nxt)
        })?;
        Ok(next)
    }

    fn del_inst(&self, id: StateInstanceId) -> DbResult<()> {
        self.state_tree.remove(&id)?;
        Ok(())
    }

    fn get_insts(&self) -> DbResult<Vec<StateInstanceId>> {
        let mut ids = Vec::new();
        for item in self.state_tree.iter() {
            let (id, _) = item?;
            ids.push(id);
        }
        Ok(ids)
    }

    fn get_inst_root(&self, id: StateInstanceId) -> DbResult<Buf32> {
        self.get_inst_toplevel_state(id)
            .map(|chs| chs.compute_state_root())
    }

    fn get_inst_toplevel_state(&self, id: StateInstanceId) -> DbResult<Chainstate> {
        let entry = self
            .state_tree
            .get(&id)?
            .ok_or(DbError::MissingStateInstance)?;
        Ok(entry.into_toplevel_state())
    }

    fn put_write_batch(&self, id: WriteBatchId, wb: WriteBatch) -> DbResult<()> {
        self.write_batch_tree.insert(&id, &wb)?;
        Ok(())
    }

    fn get_write_batch(&self, id: WriteBatchId) -> DbResult<Option<WriteBatch>> {
        Ok(self.write_batch_tree.get(&id)?)
    }

    fn del_write_batch(&self, id: WriteBatchId) -> DbResult<()> {
        self.write_batch_tree.remove(&id)?;
        Ok(())
    }

    fn merge_write_batches(
        &self,
        state_id: StateInstanceId,
        wb_ids: Vec<WriteBatchId>,
    ) -> DbResult<()> {
        // Since we have a really simple state merge concept now, we can just
        // fudge the details on this one.

        let last_entry = self
            .state_tree
            .get(&state_id)?
            .ok_or(DbError::MissingStateInstance)?;

        // Just iterate over all the write batch IDs to make sure they
        // exist.
        //
        // Keep the last one so we don't have to read it twice.
        let mut last_wb = None;
        for wb_id in &wb_ids {
            let wb = self
                .write_batch_tree
                .get(wb_id)?
                .ok_or(DbError::MissingWriteBatch(*wb_id))?;
            last_wb = Some(wb);
        }

        // Applying the last write batch is really simple.
        if let Some(last_wb) = last_wb {
            let entry = StateInstanceEntry::new(last_wb.into_toplevel());
            self.state_tree
                .compare_and_swap(state_id, Some(last_entry), Some(entry))?;
        }

        Ok(())
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::chain_state_db_tests;

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(ChainstateDBSled, chain_state_db_tests);
}
