use std::sync::Arc;

use strata_db_types::{traits::AsmDatabase, DbResult};
use strata_primitives::{buf::Buf32, l1::L1BlockCommitment};
use strata_state::asm_state::AsmState;
use threadpool::ThreadPool;

use crate::ops;

/// A manager for the persistence of [`AsmState`].
#[expect(
    missing_debug_implementations,
    reason = "Inner types don't have Debug implementation"
)]
pub struct AsmStateManager {
    ops: ops::asm::AsmDataOps,
}

impl AsmStateManager {
    /// Create new instance of [`AsmStateManager`].
    pub fn new(pool: ThreadPool, db: Arc<impl AsmDatabase + 'static>) -> Self {
        let ops = ops::asm::Context::new(db).into_ops(pool);
        Self { ops }
    }

    /// Returns [`AsmState`] that corresponds to the "highest" block.
    pub fn fetch_most_recent_state(&self) -> DbResult<Option<(L1BlockCommitment, AsmState)>> {
        self.ops.get_latest_asm_state_blocking()
    }

    /// Returns [`AsmState`] that corresponds to passed block.
    pub fn get_state(&self, block: L1BlockCommitment) -> DbResult<Option<AsmState>> {
        self.ops.get_asm_state_blocking(block)
    }

    /// Puts [`AsmState`] for the given block.
    pub fn put_state(&self, block: L1BlockCommitment, asm_state: AsmState) -> DbResult<()> {
        self.ops.put_asm_state_blocking(block, asm_state)
    }

    /// Returns [`AsmState`] entries starting from a given block up to a maximum count.
    ///
    /// Returns entries in ascending order (oldest first). If `from_block` doesn't exist,
    /// starts from the next available block after it.
    pub fn get_states_from(
        &self,
        from_block: L1BlockCommitment,
        max_count: usize,
    ) -> DbResult<Vec<(L1BlockCommitment, AsmState)>> {
        self.ops.get_asm_states_from_blocking(from_block, max_count)
    }

    /// Stores a manifest hash at the given MMR leaf index
    pub fn store_manifest_hash(&self, index: u64, hash: Buf32) -> DbResult<()> {
        self.ops.store_manifest_hash_blocking(index, hash)
    }

    /// Gets a manifest hash by MMR leaf index
    pub fn get_manifest_hash(&self, index: u64) -> DbResult<Option<Buf32>> {
        self.ops.get_manifest_hash_blocking(index)
    }
}
