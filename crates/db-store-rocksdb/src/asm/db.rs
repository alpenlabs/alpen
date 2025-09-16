use std::sync::Arc;

use rockbound::{OptimisticTransactionDB, SchemaDBOperationsExt};
use strata_db::{traits::*, DbResult};
use strata_state::asm_state::AsmState;

use crate::{asm::schemas::AsmBlockSchema, DbOpsConfig};

#[derive(Debug)]
pub struct AsmDb {
    db: Arc<OptimisticTransactionDB>,
    _ops: DbOpsConfig,
}

impl AsmDb {
    // NOTE: db is expected to open all the column families defined in STORE_COLUMN_FAMILIES.
    // FIXME: Make it better/generic.
    pub fn new(db: Arc<OptimisticTransactionDB>, ops: DbOpsConfig) -> Self {
        Self { db, _ops: ops }
    }
}

impl AsmDatabase for AsmDb {
    fn put_asm_state(
        &self,
        block: strata_primitives::prelude::L1BlockCommitment,
        state: AsmState,
    ) -> DbResult<()> {
        self.db.put::<AsmBlockSchema>(&block, &state)?;
        Ok(())
    }

    fn get_asm_state(
        &self,
        block: strata_primitives::prelude::L1BlockCommitment,
    ) -> DbResult<Option<AsmState>> {
        Ok(self.db.get::<AsmBlockSchema>(&block)?)
    }

    fn get_latest_asm_state(
        &self,
    ) -> DbResult<Option<(strata_primitives::prelude::L1BlockCommitment, AsmState)>> {
        // Relying on the lexicographycal order of L1BlockCommitment.
        let mut iterator = self.db.iter::<AsmBlockSchema>()?;
        iterator.seek_to_last();
        let opt = iterator.rev().next().map(|res| res.unwrap().into_tuple());

        Ok(opt)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {}
