use std::sync::Arc;

use strata_asm_common::AuxData;
use strata_db_types::{asm::AsmDatabase, DbResult};
use strata_primitives::L1BlockCommitment;
use strata_state::asm_state::AsmState;
use tokio::runtime::Handle;

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
    pub fn new(handle: Handle, db: Arc<impl AsmDatabase + 'static>) -> Self {
        let ops = ops::asm::AsmDataOps::new(handle, db);
        Self { ops }
    }

    /// Returns [`AsmState`] that corresponds to the "highest" block.
    pub fn fetch_most_recent_state_blocking(
        &self,
    ) -> DbResult<Option<(L1BlockCommitment, AsmState)>> {
        self.ops.get_latest_asm_state_blocking()
    }

    /// Returns [`AsmState`] that corresponds to the "highest" block.
    pub async fn fetch_most_recent_state_async(
        &self,
    ) -> DbResult<Option<(L1BlockCommitment, AsmState)>> {
        self.ops.get_latest_asm_state_async().await
    }

    /// Returns [`AsmState`] that corresponds to passed block.
    pub fn get_state_blocking(&self, block: L1BlockCommitment) -> DbResult<Option<AsmState>> {
        self.ops.get_asm_state_blocking(block)
    }

    /// Puts [`AsmState`] for the given block.
    pub fn put_state_blocking(
        &self,
        block: L1BlockCommitment,
        asm_state: AsmState,
    ) -> DbResult<()> {
        self.ops.put_asm_state_blocking(block, asm_state)
    }

    /// Returns [`AsmState`] entries starting from a given block up to a maximum count.
    ///
    /// Returns entries in ascending order (oldest first). If `from_block` doesn't exist,
    /// starts from the next available block after it.
    pub fn get_states_from_blocking(
        &self,
        from_block: L1BlockCommitment,
        max_count: usize,
    ) -> DbResult<Vec<(L1BlockCommitment, AsmState)>> {
        self.ops.get_asm_states_from_blocking(from_block, max_count)
    }

    /// Puts [`AuxData`] for the given block.
    pub fn put_aux_data_blocking(&self, block: L1BlockCommitment, data: AuxData) -> DbResult<()> {
        self.ops.put_aux_data_blocking(block, data)
    }

    /// Returns [`AuxData`] that corresponds to passed block.
    pub fn get_aux_data_blocking(&self, block: L1BlockCommitment) -> DbResult<Option<AuxData>> {
        self.ops.get_aux_data_blocking(block)
    }
}
