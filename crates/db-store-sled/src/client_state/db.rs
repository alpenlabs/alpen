use strata_db::{DbResult, errors::*, traits::*};
use strata_state::operation::*;

use super::schemas::ClientUpdateOutputSchema;
use crate::{define_sled_database, utils::first};

define_sled_database!(
    pub struct ClientStateDBSled {
        client_update_tree: ClientUpdateOutputSchema,
    }
);

impl ClientStateDBSled {
    fn get_next_idx(&self) -> DbResult<u64> {
        match self.client_update_tree.last()? {
            Some((idx, _)) => Ok(idx + 1),
            None => Ok(0),
        }
    }
}

impl ClientStateDatabase for ClientStateDBSled {
    fn put_client_update(&self, idx: u64, output: ClientUpdateOutput) -> DbResult<()> {
        let next = self.get_next_idx()?;
        if idx != next {
            return Err(DbError::OooInsert("consensus_store", idx));
        }
        self.client_update_tree
            .compare_and_swap(idx, None, Some(output))?;
        Ok(())
    }

    fn get_client_update(&self, idx: u64) -> DbResult<Option<ClientUpdateOutput>> {
        Ok(self.client_update_tree.get(&idx)?)
    }

    fn get_last_state_idx(&self) -> DbResult<u64> {
        match self.client_update_tree.last()?.map(first) {
            Some(idx) => Ok(idx),
            None => Err(DbError::NotBootstrapped),
        }
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::client_state_db_tests;

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(ClientStateDBSled, client_state_db_tests);
}
