//! ASM state database interface.

// TODO(trey): replace AsmExecOutput with versionable serde-ish wrappers

use serde::{Deserialize, Serialize};
use strata_asm_common::{AnchorState, AsmLogEntry, AuxData};
#[cfg(feature = "proxies")]
use strata_db_macros::gen_proxy;
use strata_primitives::prelude::*;

#[cfg(feature = "proxies")]
use crate::DbError;
use crate::DbResult;

/// Full output of an ASM state transition, as persisted by the node.
///
/// Bundles the post-state anchor state with the logs the transition emitted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AsmExecOutput {
    state: AnchorState,
    logs: Vec<AsmLogEntry>,
}

impl AsmExecOutput {
    pub fn new(state: AnchorState, logs: Vec<AsmLogEntry>) -> Self {
        Self { state, logs }
    }

    pub fn logs(&self) -> &Vec<AsmLogEntry> {
        &self.logs
    }

    pub fn state(&self) -> &AnchorState {
        &self.state
    }
}

/// Database interface to control our view of ASM state.
#[cfg_attr(
    feature = "proxies",
    gen_proxy(error = DbError, tracing_component = "storage:asm")
)]
pub trait AsmDatabase: Send + Sync + 'static {
    /// Writes a new ASM state for a given l1 block.
    fn put_asm_state(&self, block: L1BlockCommitment, state: AsmExecOutput) -> DbResult<()>;

    /// Writes just the anchor state for a given l1 block, leaving that block's
    /// logs untouched.
    ///
    /// The ASM worker produces the two halves at different points: the logs
    /// arrive with the block's manifest, the anchor state afterwards as the
    /// block's commit point. Callers must write the logs first, since
    /// [`Self::get_asm_state`] only yields a block once both halves are present.
    fn put_anchor_state(&self, block: L1BlockCommitment, state: AnchorState) -> DbResult<()>;

    /// Writes just the ASM logs for a given l1 block, leaving that block's
    /// anchor state untouched.
    ///
    /// See [`Self::put_anchor_state`] for the ordering this pairs with.
    fn put_asm_logs(&self, block: L1BlockCommitment, logs: Vec<AsmLogEntry>) -> DbResult<()>;

    /// Gets the ASM state for the given l1 block.
    fn get_asm_state(&self, block: L1BlockCommitment) -> DbResult<Option<AsmExecOutput>>;

    /// Gets latest ASM state (the entry that corresponds to the highest l1 block).
    fn get_latest_asm_state(&self) -> DbResult<Option<(L1BlockCommitment, AsmExecOutput)>>;

    /// Gets just the anchor state for the given l1 block, without requiring that
    /// block's logs to be present.
    ///
    /// The genesis anchor state has no logs — it is not produced by processing a
    /// block, so no manifest ever carries logs for it — so readers that only need
    /// the anchor state must use this rather than [`Self::get_asm_state`].
    fn get_anchor_state(&self, block: L1BlockCommitment) -> DbResult<Option<AnchorState>>;

    /// Gets the anchor state at the highest l1 block, without requiring that
    /// block's logs to be present.
    ///
    /// See [`Self::get_anchor_state`] for why this does not go through
    /// [`Self::get_latest_asm_state`].
    fn get_latest_anchor_state(&self) -> DbResult<Option<(L1BlockCommitment, AnchorState)>>;

    /// Gets ASM states starting from a given L1BlockCommitment up to a maximum count.
    ///
    /// Returns entries in ascending order (oldest first). If `from_block` doesn't exist,
    /// starts from the next available block after it.
    fn get_asm_states_from(
        &self,
        from_block: L1BlockCommitment,
        max_count: usize,
    ) -> DbResult<Vec<(L1BlockCommitment, AsmExecOutput)>>;

    /// Writes auxiliary data for a given L1 block.
    fn put_aux_data(&self, block: L1BlockCommitment, data: AuxData) -> DbResult<()>;

    /// Gets auxiliary data for the given L1 block.
    fn get_aux_data(&self, block: L1BlockCommitment) -> DbResult<Option<AuxData>>;
}
