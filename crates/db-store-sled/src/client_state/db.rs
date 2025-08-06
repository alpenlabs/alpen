use std::sync::Arc;

use strata_db::{DbResult, errors::*, traits::*};
use strata_state::operation::*;
use typed_sled::{SledDb, SledTree};

use super::schemas::ClientUpdateOutputSchema;
use crate::{SledDbConfig, utils::first};

#[derive(Debug)]
pub struct ClientStateDBSled {
    client_update_tree: SledTree<ClientUpdateOutputSchema>,
    _config: SledDbConfig,
}

impl ClientStateDBSled {
    pub fn new(db: Arc<SledDb>, config: SledDbConfig) -> DbResult<Self> {
        Ok(Self {
            client_update_tree: db.get_tree()?,
            _config: config,
        })
    }

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

    fn setup_db() -> ClientStateDBSled {
        let db = sled::Config::new().temporary(true).open().unwrap();
        let sled_db = SledDb::new(db).unwrap();
        let config = SledDbConfig::new_with_constant_backoff(3, 200);
        ClientStateDBSled::new(sled_db.into(), config).unwrap()
    }

    client_state_db_tests!(setup_db());
}
