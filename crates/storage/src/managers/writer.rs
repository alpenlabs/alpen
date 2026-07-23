use std::sync::Arc;

use strata_db_types::l1_writer::{BundledPayloadEntry, IntentEntry, L1WriterDatabase};
use strata_db_types::DbResult;
use strata_identifiers::Buf32;
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

    pub fn get_next_payload_idx_blocking(&self) -> DbResult<u64> {
        self.ops.get_next_payload_idx_blocking()
    }

    pub fn get_payload_entry_by_idx_blocking(
        &self,
        idx: u64,
    ) -> DbResult<Option<BundledPayloadEntry>> {
        self.ops.get_payload_entry_by_idx_blocking(idx)
    }

    pub fn put_payload_entry_blocking(&self, idx: u64, entry: BundledPayloadEntry) -> DbResult<()> {
        self.ops.put_payload_entry_blocking(idx, entry)
    }

    pub fn get_next_intent_idx_blocking(&self) -> DbResult<u64> {
        self.ops.get_next_intent_idx_blocking()
    }

    pub fn get_intent_by_idx_blocking(&self, idx: u64) -> DbResult<Option<IntentEntry>> {
        self.ops.get_intent_by_idx_blocking(idx)
    }

    pub fn update_intent_entry_blocking(
        &self,
        intent_id: Buf32,
        entry: IntentEntry,
    ) -> DbResult<()> {
        self.ops.update_intent_entry_blocking(intent_id, entry)
    }
}
