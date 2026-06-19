use std::sync::Arc;

use strata_db_types::l1_writer::{BundledPayloadEntry, L1WriterDatabase};
use strata_db_types::DbResult;
use tokio::runtime::Handle;

use crate::ops;

/// Database manager for L1 writer / envelope payload persistence.
#[expect(
    missing_debug_implementations,
    reason = "Inner types don't have Debug implementation"
)]
pub struct L1WriterManager {
    ops: ops::writer::EnvelopeDataOps,
}

impl L1WriterManager {
    /// Creates a new [`L1WriterManager`].
    pub fn new(handle: Handle, db: Arc<impl L1WriterDatabase + 'static>) -> Self {
        let ops = ops::writer::EnvelopeDataOps::new(handle, db);
        Self { ops }
    }

    pub async fn get_next_payload_idx_async(&self) -> DbResult<u64> {
        self.ops.get_next_payload_idx_async().await
    }

    pub async fn get_payload_entry_by_idx_async(
        &self,
        idx: u64,
    ) -> DbResult<Option<BundledPayloadEntry>> {
        self.ops.get_payload_entry_by_idx_async(idx).await
    }

    pub async fn put_payload_entry_async(
        &self,
        idx: u64,
        entry: BundledPayloadEntry,
    ) -> DbResult<()> {
        self.ops.put_payload_entry_async(idx, entry).await
    }
}
