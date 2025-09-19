use std::sync::Arc;

use rockbound::{OptimisticTransactionDB, SchemaDBOperationsExt, TransactionRetry};
use strata_db::{traits::*, DbError, DbResult};
use strata_state::asm_state::AsmState;

use super::schemas::{AsmLogSchema, AsmStateSchema};
use crate::DbOpsConfig;

#[derive(Debug)]
pub struct AsmDb {
    db: Arc<OptimisticTransactionDB>,
    ops: DbOpsConfig,
}

impl AsmDb {
    // NOTE: db is expected to open all the column families defined in STORE_COLUMN_FAMILIES.
    // FIXME: Make it better/generic.
    pub fn new(db: Arc<OptimisticTransactionDB>, ops: DbOpsConfig) -> Self {
        Self { db, ops }
    }
}

impl AsmDatabase for AsmDb {
    fn put_asm_state(
        &self,
        block: strata_primitives::prelude::L1BlockCommitment,
        state: AsmState,
    ) -> DbResult<()> {
        self.db
            .with_optimistic_txn(
                TransactionRetry::Count(self.ops.retry_count),
                |txn| -> Result<(), anyhow::Error> {
                    txn.put::<AsmStateSchema>(&block, state.state())?;
                    txn.put::<AsmLogSchema>(&block, state.logs())?;

                    Ok(())
                },
            )
            .map_err(|e| DbError::TransactionError(e.to_string()))
    }

    fn get_asm_state(
        &self,
        block: strata_primitives::prelude::L1BlockCommitment,
    ) -> DbResult<Option<AsmState>> {
        self.db
            .with_optimistic_txn(
                TransactionRetry::Count(self.ops.retry_count),
                |txn| -> Result<Option<AsmState>, anyhow::Error> {
                    let state = txn.get::<AsmStateSchema>(&block)?;
                    let logs = txn.get::<AsmLogSchema>(&block)?;

                    Ok(state.and_then(|s| logs.map(|l| AsmState::new(s, l))))
                },
            )
            .map_err(|e| DbError::TransactionError(e.to_string()))
    }

    fn get_latest_asm_state(
        &self,
    ) -> DbResult<Option<(strata_primitives::prelude::L1BlockCommitment, AsmState)>> {
        // Relying on the lexicographycal order of L1BlockCommitment.
        let mut state_iter = self.db.iter::<AsmStateSchema>()?;
        state_iter.seek_to_last();
        let state = state_iter.rev().next().map(|res| res.unwrap().into_tuple());

        let mut logs_iter = self.db.iter::<AsmLogSchema>()?;
        logs_iter.seek_to_last();
        let logs = logs_iter.rev().next().map(|res| res.unwrap().into_tuple());

        // Assert that the block for the state and for the logs is the same.
        // It should be because we are putting it within transaction.
        Ok(state.and_then(|s| {
            logs.map(|l| {
                assert_eq!(s.0, l.0);
                (s.0, AsmState::new(s.1, l.1))
            })
        }))
    }
}

#[cfg(test)]
mod tests {
    use strata_db_tests::asm_state_db_tests;

    use super::*;
    use crate::test_utils::get_rocksdb_tmp_instance;

    fn setup_db() -> AsmDb {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        AsmDb::new(db, db_ops)
    }

    asm_state_db_tests!(setup_db());
}
