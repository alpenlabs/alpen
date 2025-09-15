use std::sync::Arc;

use rockbound::{OptimisticTransactionDB, SchemaDBOperationsExt};
use strata_asm_stf::AsmStfOutput;
use strata_db::{traits::*, DbResult};

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
    fn put_asm_output(
        &self,
        block: strata_primitives::prelude::L1BlockCommitment,
        output: AsmStfOutput,
    ) -> DbResult<()> {
        self.db.put::<AsmBlockSchema>(&block, &output)?;
        Ok(())
    }

    fn get_asm_output(
        &self,
        block: strata_primitives::prelude::L1BlockCommitment,
    ) -> DbResult<Option<AsmStfOutput>> {
        Ok(self.db.get::<AsmBlockSchema>(&block)?)
    }

    fn get_latest_anchor_state(
        &self,
    ) -> DbResult<
        Option<(
            strata_primitives::prelude::L1BlockCommitment,
            strata_asm_common::AnchorState,
        )>,
    > {
        // Relying on the lexicographycal order of L1BlockCommitment.
        let mut iterator = self.db.iter::<AsmBlockSchema>()?;
        iterator.seek_to_last();

        let opt = match iterator.rev().next() {
            Some(res) => {
                let val = res.unwrap().into_tuple();
                Some((val.0, val.1.state))
            }
            None => None,
        };

        Ok(opt)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {}
