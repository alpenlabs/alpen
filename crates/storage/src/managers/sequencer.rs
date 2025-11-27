use std::sync::Arc;

use strata_db_types::{traits::SequencerDatabase, DbResult};
use strata_ol_chain_types::L2BlockId;
use threadpool::ThreadPool;

use crate::ops::sequencer::{Context, SequencerPayloadOps};

/// Manager for sequencer-specific payload storage.
///
/// This manager wraps the low-level `SequencerDatabase` trait and provides
/// async and blocking interfaces for storing and retrieving execution payloads.
/// Used for EE consistency recovery when reth loses blocks on restart.
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct SequencerPayloadManager {
    ops: SequencerPayloadOps,
}

impl SequencerPayloadManager {
    pub fn new(pool: ThreadPool, db: Arc<impl SequencerDatabase + 'static>) -> Self {
        let ops = Context::new(db).into_ops(pool);
        Self { ops }
    }

    /// Stores an exec payload for a given slot and block ID. Async.
    pub async fn put_exec_payload_async(
        &self,
        slot: u64,
        block_id: L2BlockId,
        payload: Vec<u8>,
    ) -> DbResult<()> {
        self.ops
            .put_exec_payload_async(slot, block_id, payload)
            .await
    }

    /// Stores an exec payload for a given slot and block ID. Blocking.
    pub fn put_exec_payload_blocking(
        &self,
        slot: u64,
        block_id: L2BlockId,
        payload: Vec<u8>,
    ) -> DbResult<()> {
        self.ops.put_exec_payload_blocking(slot, block_id, payload)
    }

    /// Gets the exec payload for a given slot. Async.
    pub async fn get_exec_payload_async(
        &self,
        slot: u64,
    ) -> DbResult<Option<(L2BlockId, Vec<u8>)>> {
        self.ops.get_exec_payload_async(slot).await
    }

    /// Gets the exec payload for a given slot. Blocking.
    pub fn get_exec_payload_blocking(&self, slot: u64) -> DbResult<Option<(L2BlockId, Vec<u8>)>> {
        self.ops.get_exec_payload_blocking(slot)
    }

    /// Gets the highest slot that has a stored exec payload. Async.
    pub async fn get_last_exec_payload_slot_async(&self) -> DbResult<Option<u64>> {
        self.ops.get_last_exec_payload_slot_async().await
    }

    /// Gets the highest slot that has a stored exec payload. Blocking.
    pub fn get_last_exec_payload_slot_blocking(&self) -> DbResult<Option<u64>> {
        self.ops.get_last_exec_payload_slot_blocking()
    }

    /// Gets a range of exec payloads from start_slot to end_slot (inclusive). Async.
    pub async fn get_exec_payloads_in_range_async(
        &self,
        start_slot: u64,
        end_slot: u64,
    ) -> DbResult<Vec<(u64, L2BlockId, Vec<u8>)>> {
        self.ops
            .get_exec_payloads_in_range_async(start_slot, end_slot)
            .await
    }

    /// Gets a range of exec payloads from start_slot to end_slot (inclusive). Blocking.
    pub fn get_exec_payloads_in_range_blocking(
        &self,
        start_slot: u64,
        end_slot: u64,
    ) -> DbResult<Vec<(u64, L2BlockId, Vec<u8>)>> {
        self.ops
            .get_exec_payloads_in_range_blocking(start_slot, end_slot)
    }

    /// Deletes exec payloads from start_slot onwards (inclusive). Async.
    pub async fn del_exec_payloads_from_slot_async(&self, start_slot: u64) -> DbResult<Vec<u64>> {
        self.ops.del_exec_payloads_from_slot_async(start_slot).await
    }

    /// Deletes exec payloads from start_slot onwards (inclusive). Blocking.
    pub fn del_exec_payloads_from_slot_blocking(&self, start_slot: u64) -> DbResult<Vec<u64>> {
        self.ops.del_exec_payloads_from_slot_blocking(start_slot)
    }
}
