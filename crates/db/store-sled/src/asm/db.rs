use strata_db_types::{DbResult, traits::*};
use strata_primitives::l1::L1BlockCommitment;
use strata_state::asm_state::AsmState;

use super::schemas::{
    AsmLogSchema, AsmManifestHashSchema, AsmMmrMetaSchema, AsmMmrNodeSchema, AsmStateSchema,
};
use crate::define_sled_database;

define_sled_database!(
    pub struct AsmDBSled {
        asm_state_tree: AsmStateSchema,
        asm_log_tree: AsmLogSchema,
        pub mmr_node_tree: AsmMmrNodeSchema,
        pub mmr_meta_tree: AsmMmrMetaSchema,
        manifest_hash_tree: AsmManifestHashSchema,
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

    fn get_asm_states_from(
        &self,
        from_block: L1BlockCommitment,
        max_count: usize,
    ) -> DbResult<Vec<(L1BlockCommitment, AsmState)>> {
        let mut result = Vec::new();

        // Use non-transactional iteration since we only need reads
        // Iterate through all blocks and filter those >= from_block
        for item in self.asm_state_tree.iter() {
            let (block, state) = item?;

            // Skip blocks before the starting block
            if block < from_block {
                continue;
            }

            // Get corresponding logs (also non-transactional read)
            if let Some(logs) = self.asm_log_tree.get(&block)? {
                result.push((block, AsmState::new(state, logs)));

                if result.len() >= max_count {
                    break;
                }
            }
        }

        Ok(result)
    }

    fn store_manifest_hash(&self, index: u64, hash: [u8; 32]) -> DbResult<()> {
        self.manifest_hash_tree.insert(&index, &hash)?;
        Ok(())
    }

    fn get_manifest_hash(&self, index: u64) -> DbResult<Option<[u8; 32]>> {
        Ok(self.manifest_hash_tree.get(&index)?)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::asm_state_db_tests;

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(AsmDBSled, asm_state_db_tests);
}
