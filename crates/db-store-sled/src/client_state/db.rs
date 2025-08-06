use std::sync::Arc;

use strata_db::{DbResult, errors::*, traits::*};
use strata_state::operation::*;
use typed_sled::{SledDb, SledTree};

use super::schemas::ClientUpdateOutputSchema;
use crate::utils::first;

#[derive(Debug)]
pub struct ClientStateDBSled {
    client_update_tree: SledTree<ClientUpdateOutputSchema>,
}

impl ClientStateDBSled {
    pub fn new(db: Arc<SledDb>) -> DbResult<Self> {
        Ok(Self {
            client_update_tree: db.get_tree()?,
        })
    }
}

impl ClientStateDatabase for ClientStateDBSled {
    fn put_client_update(&self, idx: u64, output: ClientUpdateOutput) -> DbResult<()> {
        let expected_idx = match self.client_update_tree.last()?.map(first) {
            Some(last_idx) => last_idx + 1,
            // We don't have a separate way to insert the init client state, so
            // we special case this here.
            None => 0,
        };

        if idx != expected_idx {
            return Err(DbError::OooInsert("consensus_store", idx));
        }

        self.client_update_tree.insert(&idx, &output)?;
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

    fn setup_db() -> ClientStateDBSled {
        let db = sled::Config::new().temporary(true).open().unwrap();
        let sled_db = SledDb::new(Arc::new(db)).unwrap();
        ClientStateDBSled::new(sled_db.into()).unwrap()
    }

    client_state_db_tests!(setup_db());
}

