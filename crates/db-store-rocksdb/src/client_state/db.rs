use std::sync::Arc;

use rockbound::{OptimisticTransactionDB, SchemaDBOperationsExt};
use strata_csm_types::{ClientState, ClientUpdateOutput};
use strata_db::{traits::*, DbResult};
use strata_primitives::l1::L1BlockCommitment;

use super::schemas::ClientUpdateOutputSchema;
use crate::DbOpsConfig;

#[derive(Debug)]
pub struct ClientStateDb {
    db: Arc<OptimisticTransactionDB>,
    _ops: DbOpsConfig,
}

impl ClientStateDb {
    /// Wraps an existing database handle.
    ///
    /// Assumes it was opened with column families as defined in `STORE_COLUMN_FAMILIES`.
    // FIXME Make it better/generic.
    pub fn new(db: Arc<OptimisticTransactionDB>, ops: DbOpsConfig) -> Self {
        Self { db, _ops: ops }
    }
}

impl ClientStateDatabase for ClientStateDb {
    fn put_client_update(
        &self,
        block: L1BlockCommitment,
        output: ClientUpdateOutput,
    ) -> DbResult<()> {
        self.db.put::<ClientUpdateOutputSchema>(&block, &output)?;
        Ok(())
    }

    fn get_client_update(&self, block: L1BlockCommitment) -> DbResult<Option<ClientUpdateOutput>> {
        Ok(self.db.get::<ClientUpdateOutputSchema>(&block)?)
    }

    fn get_latest_client_state(&self) -> DbResult<Option<(L1BlockCommitment, ClientState)>> {
        // Relying on the lexicographycal order of L1BlockCommitment.
        let mut iterator = self.db.iter::<ClientUpdateOutputSchema>()?;
        iterator.seek_to_last();

        let opt = match iterator.rev().next() {
            Some(res) => {
                let val = res.unwrap().into_tuple();
                Some((val.0, val.1.into_state()))
            }
            None => None,
        };

        Ok(opt)
    }
}

#[cfg(test)]
mod tests {
    use strata_db_tests::client_state_db_tests;

    use super::*;
    use crate::test_utils::get_rocksdb_tmp_instance;

    fn setup_db() -> ClientStateDb {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        ClientStateDb::new(db, db_ops)
    }

    client_state_db_tests!(setup_db());
}
