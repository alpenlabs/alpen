use std::sync::Arc;

use strata_asm_common::AnchorState;
use strata_db::{
    traits::{AsmDatabase, L1Database},
    DbError, DbResult,
};
use strata_primitives::l1::{L1BlockCommitment, L1BlockId, L1BlockManifest, L1Tx, L1TxRef};
use threadpool::ThreadPool;
use tracing::error;

use crate::ops;

/// Caching manager of L1 block data
#[expect(missing_debug_implementations)]
pub struct AsmManager {
    ops: ops::asm::AsmDataOps,
}

impl AsmManager {
    /// Create new instance of [`AsmManager`].
    pub fn new(pool: ThreadPool, db: Arc<impl AsmDatabase + 'static>) -> Self {
        let ops = ops::asm::Context::new(db).into_ops(pool);
        Self { ops }
    }

    /// Returns [`AnchorState`] that corresponds to the "heighest" block.
    pub fn fetch_most_recent_state(&self) -> DbResult<Option<(L1BlockCommitment, AnchorState)>> {
        self.ops.get_latest_anchor_state_blocking()
    }
}
