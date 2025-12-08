use std::sync::Arc;

use strata_db_types::{traits::AsmDatabase, DbResult};
use strata_primitives::l1::L1BlockCommitment;
use strata_state::asm_state::AsmState;
use threadpool::ThreadPool;

use crate::{mmr_db::SledMmrDb, ops};

/// A manager for the persistence of [`AsmState`].
#[expect(
    missing_debug_implementations,
    reason = "Inner types don't have Debug implementation"
)]
pub struct AsmStateManager {
    ops: ops::asm::AsmDataOps,
    /// Raw sled database for creating MMR database instances
    raw_db: Option<Arc<sled::Db>>,
}

impl AsmStateManager {
    /// Create new instance of [`AsmStateManager`].
    pub fn new(pool: ThreadPool, db: Arc<impl AsmDatabase + 'static>) -> Self {
        let ops = ops::asm::Context::new(db).into_ops(pool);
        Self { ops, raw_db: None }
    }

    /// Create new instance with raw sled database for MMR support
    pub fn new_with_raw_db(
        pool: ThreadPool,
        db: Arc<impl AsmDatabase + 'static>,
        raw_db: Arc<sled::Db>,
    ) -> Self {
        let ops = ops::asm::Context::new(db).into_ops(pool);
        Self {
            ops,
            raw_db: Some(raw_db),
        }
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

    /// Creates a new MMR database instance for proof generation
    ///
    /// Opens the raw sled trees used for MMR storage. Requires that the manager
    /// was created with `new_with_raw_db()`.
    pub fn create_mmr_database(&self) -> DbResult<SledMmrDb> {
        let raw_db = self.raw_db.as_ref().ok_or_else(|| {
            strata_db_types::DbError::Other("Raw database not available".to_string())
        })?;

        // Open the MMR trees by name
        let mmr_node_tree = raw_db
            .open_tree(b"AsmMmrNodeSchema")
            .map_err(|e| strata_db_types::DbError::Other(e.to_string()))?;
        let mmr_meta_tree = raw_db
            .open_tree(b"AsmMmrMetaSchema")
            .map_err(|e| strata_db_types::DbError::Other(e.to_string()))?;

        SledMmrDb::new(mmr_node_tree, mmr_meta_tree)
            .map_err(|e| strata_db_types::DbError::Other(e.to_string()))
    }

    /// Stores a manifest hash at the given MMR leaf index
    pub fn store_manifest_hash(&self, index: u64, hash: [u8; 32]) -> DbResult<()> {
        self.ops.store_manifest_hash_blocking(index, hash)
    }

    /// Gets a manifest hash by MMR leaf index
    pub fn get_manifest_hash(&self, index: u64) -> DbResult<Option<[u8; 32]>> {
        self.ops.get_manifest_hash_blocking(index)
    }
}
