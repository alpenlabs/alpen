//! ASM state database interface.

use strata_asm_common::AuxData;
use strata_primitives::prelude::*;
use strata_state::asm_state::AsmState;

use crate::DbResult;

/// Database interface to control our view of ASM state.
#[cfg_attr(
    feature = "proxies",
    strata_db_macros::gen_proxy(error = crate::DbError, tracing_component = "storage:asm")
)]
pub trait AsmDatabase: Send + Sync + 'static {
    /// Writes a new ASM state for a given l1 block.
    fn put_asm_state(&self, block: L1BlockCommitment, state: AsmState) -> DbResult<()>;

    /// Gets the ASM state for the given l1 block.
    fn get_asm_state(&self, block: L1BlockCommitment) -> DbResult<Option<AsmState>>;

    /// Gets latest ASM state (the entry that corresponds to the highest l1 block).
    fn get_latest_asm_state(&self) -> DbResult<Option<(L1BlockCommitment, AsmState)>>;

    /// Gets ASM states starting from a given L1BlockCommitment up to a maximum count.
    ///
    /// Returns entries in ascending order (oldest first). If `from_block` doesn't exist,
    /// starts from the next available block after it.
    fn get_asm_states_from(
        &self,
        from_block: L1BlockCommitment,
        max_count: usize,
    ) -> DbResult<Vec<(L1BlockCommitment, AsmState)>>;

    /// Writes auxiliary data for a given L1 block.
    fn put_aux_data(&self, block: L1BlockCommitment, data: AuxData) -> DbResult<()>;

    /// Gets auxiliary data for the given L1 block.
    fn get_aux_data(&self, block: L1BlockCommitment) -> DbResult<Option<AuxData>>;
}
