use strata_db_types::{DbResult, traits::OLStateDatabase};
use strata_identifiers::{EpochCommitment, OLBlockCommitment};
use strata_ol_state_types::{OLAccountState, OLState, WriteBatch};

use super::schemas::{OLPresealStateSchema, OLStateSchema, OLWriteBatchSchema};
use crate::define_sled_database;

define_sled_database!(
    pub struct OLStateDBSled {
        state_tree: OLStateSchema,
        preseal_state_tree: OLPresealStateSchema,
        write_batch_tree: OLWriteBatchSchema,
    }
);

impl OLStateDatabase for OLStateDBSled {
    fn put_toplevel_ol_state(&self, commitment: OLBlockCommitment, state: OLState) -> DbResult<()> {
        self.config
            .with_retry((&self.state_tree,), |(state_tree,)| {
                state_tree.insert(&commitment, &state)?;
                Ok(())
            })?;
        Ok(())
    }

    fn get_toplevel_ol_state(&self, commitment: OLBlockCommitment) -> DbResult<Option<OLState>> {
        Ok(self.state_tree.get(&commitment)?)
    }

    fn get_latest_toplevel_ol_state(&self) -> DbResult<Option<(OLBlockCommitment, OLState)>> {
        // Relying on the lexicographical order of OLBlockCommitment (slot + block ID).
        // The last entry should be the one with the highest slot.
        Ok(self.state_tree.last()?)
    }

    fn del_toplevel_ol_state(&self, commitment: OLBlockCommitment) -> DbResult<()> {
        self.state_tree.remove(&commitment)?;
        Ok(())
    }

    fn put_preseal_ol_state(&self, commitment: EpochCommitment, state: OLState) -> DbResult<()> {
        self.config
            .with_retry((&self.preseal_state_tree,), |(state_tree,)| {
                state_tree.insert(&commitment, &state)?;
                Ok(())
            })?;
        Ok(())
    }

    fn get_preseal_ol_state(&self, commitment: EpochCommitment) -> DbResult<Option<OLState>> {
        Ok(self.preseal_state_tree.get(&commitment)?)
    }

    fn del_preseal_ol_state(&self, commitment: EpochCommitment) -> DbResult<()> {
        self.preseal_state_tree.remove(&commitment)?;
        Ok(())
    }

    fn put_ol_write_batch(
        &self,
        commitment: OLBlockCommitment,
        wb: WriteBatch<OLAccountState>,
    ) -> DbResult<()> {
        self.config
            .with_retry((&self.write_batch_tree,), |(wb_tree,)| {
                wb_tree.insert(&commitment, &wb)?;
                Ok(())
            })?;
        Ok(())
    }

    fn get_ol_write_batch(
        &self,
        commitment: OLBlockCommitment,
    ) -> DbResult<Option<WriteBatch<OLAccountState>>> {
        Ok(self.write_batch_tree.get(&commitment)?)
    }

    fn del_ol_write_batch(&self, commitment: OLBlockCommitment) -> DbResult<()> {
        self.write_batch_tree.remove(&commitment)?;
        Ok(())
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::ol_state_db_tests;

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(OLStateDBSled, ol_state_db_tests);
}
