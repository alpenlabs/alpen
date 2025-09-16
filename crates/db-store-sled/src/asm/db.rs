use strata_db::{DbResult, traits::*};
use strata_primitives::l1::L1BlockCommitment;
use strata_state::asm_state::AsmState;

use super::schemas::AsmBlockSchema;
use crate::define_sled_database;

define_sled_database!(
    pub struct AsmDBSled {
        asm_state_tree: AsmBlockSchema,
    }
);

impl AsmDatabase for AsmDBSled {
    fn put_asm_state(&self, block: L1BlockCommitment, output: AsmState) -> DbResult<()> {
        Ok(self.asm_state_tree.insert(&block, &output)?)
    }

    fn get_asm_state(&self, block: L1BlockCommitment) -> DbResult<Option<AsmState>> {
        Ok(self.asm_state_tree.get(&block)?)
    }

    fn get_latest_asm_state(&self) -> DbResult<Option<(L1BlockCommitment, AsmState)>> {
        // Relying on the lexicographical order of L1BlockCommitment.
        let mut iter = self.asm_state_tree.iter().rev();
        let res = iter.next();
        Ok(res.transpose()?)
    }
}
