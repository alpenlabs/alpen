use strata_db::{DbResult, traits::*};
use strata_primitives::l1::L1BlockCommitment;
use strata_state::asm_state::AsmState;

use super::schemas::{AsmLogSchema, AsmStateSchema};
use crate::define_sled_database;

define_sled_database!(
    pub struct AsmDBSled {
        asm_state_tree: AsmStateSchema,
        asm_log_tree: AsmLogSchema,
    }
);

impl AsmDatabase for AsmDBSled {
    fn put_asm_state(&self, block: L1BlockCommitment, state: AsmState) -> DbResult<()> {
        self.config.with_retry(
            (&self.asm_state_tree, &self.asm_log_tree),
            |(state_tree, log_tree)| {
                state_tree.insert(&block, state.state())?;
                log_tree.insert(&block, state.logs())?;

                Ok(())
            },
        )?;

        Ok(())
    }

    fn get_asm_state(&self, block: L1BlockCommitment) -> DbResult<Option<AsmState>> {
        self.config.with_retry(
            (&self.asm_state_tree, &self.asm_log_tree),
            |(state_tree, log_tree)| {
                let state = state_tree.get(&block)?;
                let logs = log_tree.get(&block)?;

                Ok(state.and_then(|s| logs.map(|l| AsmState::new(s, l))))
            },
        )
    }

    fn get_latest_asm_state(&self) -> DbResult<Option<(L1BlockCommitment, AsmState)>> {
        // Relying on the lexicographical order of L1BlockCommitment.
        let state = self.asm_state_tree.last()?;
        let logs = self.asm_log_tree.last()?;

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
