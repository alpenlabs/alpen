//! Bounded scheduling cache for durable replayable blocks.

use std::{collections::BTreeMap, time::Duration};

use metrics::{counter, gauge};
use strata_db_types::ol_block::BlockStatus;
use strata_identifiers::{OLBlockCommitment, OLBlockId, Slot};
use strata_ol_chain_types_v1::OLBlockV1;
use tokio::time::Instant;
use tracing::warn;

use super::ExecutionDeferral;

const MAX_PENDING_BLOCKS: usize = 256;
const UNATTEMPTED_RETENTION: Duration = Duration::from_secs(1);
pub(super) const RETRY_BATCH_SIZE: usize = 32;
pub(super) const STATUS_SCAN_SIZE: usize = 64;
const MAX_BODY_READ_ATTEMPTS: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingAction {
    Execute,
    Cleanup,
}

#[derive(Debug)]
struct PendingEntry {
    action: PendingAction,
    parent: OLBlockId,
    reason: ExecutionDeferral,
    failures: u32,
    next_retry: Instant,
    last_attempt: Instant,
    first_seen: Instant,
    last_warned: Instant,
}

/// Holds only a bounded cache; status scans recover evicted entries without a restart.
#[derive(Debug, Default)]
pub(super) struct PendingBlockCache {
    entries: BTreeMap<OLBlockCommitment, PendingEntry>,
    scan_cursor: Option<OLBlockCommitment>,
    rejection_scan_cursor: Option<OLBlockId>,
    failed_body_read: Option<(OLBlockCommitment, u8)>,
}

impl PendingBlockCache {
    fn contains_id(&self, id: &OLBlockId) -> bool {
        self.entries.keys().any(|block| block.blkid() == id)
    }

    fn has_pending_child(&self, id: &OLBlockId) -> bool {
        self.entries.values().any(|entry| &entry.parent == id)
    }

    /// Requires the parent to be attached with no unfinished work before executing a child.
    pub(super) fn is_parent_ready(
        &self,
        slot: Slot,
        parent: &OLBlockId,
        is_parent_attached: &impl Fn(&OLBlockId) -> bool,
    ) -> bool {
        slot == 0 || (!self.contains_id(parent) && is_parent_attached(parent))
    }

    fn is_ready(
        &self,
        block: &OLBlockCommitment,
        entry: &PendingEntry,
        is_parent_attached: &impl Fn(&OLBlockId) -> bool,
    ) -> bool {
        entry.action == PendingAction::Cleanup
            || self.is_parent_ready(block.slot(), &entry.parent, is_parent_attached)
    }

    fn eviction_candidate(
        &self,
        is_parent_attached: &impl Fn(&OLBlockId) -> bool,
    ) -> Option<OLBlockCommitment> {
        let now = Instant::now();
        self.entries
            .iter()
            .filter(|(block, entry)| {
                (entry.failures > 0
                    || (entry.first_seen.elapsed() >= UNATTEMPTED_RETENTION
                        && !self.is_ready(block, entry, is_parent_attached)))
                    && (entry.reason == ExecutionDeferral::Dependency || entry.next_retry <= now)
                    && (entry.action == PendingAction::Cleanup
                        || !self.has_pending_child(block.blkid()))
            })
            .min_by_key(|(_, entry)| entry.last_attempt)
            .map(|(key, _)| *key)
    }

    /// Returns the last scanned status key, or `None` to scan from the beginning.
    pub(super) fn scan_cursor(&self) -> Option<OLBlockCommitment> {
        self.scan_cursor
    }

    pub(super) fn rejection_scan_cursor(&self) -> Option<OLBlockId> {
        self.rejection_scan_cursor
    }

    /// Advances through every inspected row, wrapping only at the end of the table.
    pub(super) fn record_rejection_status_page(&mut self, rows: &[(OLBlockId, bool)]) {
        self.rejection_scan_cursor = rows.last().map(|(id, _)| *id);
    }

    /// Advances past handled status rows, wrapping on an empty page.
    ///
    /// Rows come from storage in slot then block-ID order. Failed body reads stay ahead
    /// of the cursor for bounded retries, then are revisited on the next scan cycle.
    pub(super) fn record_status_page(&mut self, rows: &[(OLBlockCommitment, BlockStatus)]) {
        self.scan_cursor = rows.last().map(|(id, _)| *id);
        if self
            .failed_body_read
            .is_some_and(|(failed, _)| self.scan_cursor.is_none_or(|cursor| cursor >= failed))
        {
            self.failed_body_read = None;
        }
    }

    /// Returns whether discovery should move past a repeatedly unreadable body.
    pub(super) fn record_body_read_failure(&mut self, block: OLBlockCommitment) -> bool {
        let attempts = match self.failed_body_read {
            Some((previous, attempts)) if previous == block => attempts.saturating_add(1),
            _ => 1,
        };
        self.failed_body_read = Some((block, attempts));
        attempts >= MAX_BODY_READ_ATTEMPTS
    }

    /// Prunes finalized execution entries and makes bounded room for durable discovery when full.
    ///
    /// Retains missing-parent entries so parent arrival can trigger a retry without
    /// waiting for the durable scan to wrap. Capacity eviction still applies to them.
    /// Storage- and indexing-deferred entries retain their retry deadlines even when the cache is
    /// full. Ready entries keep their places until attempted; only blocked entries age out.
    /// Keeping the cursor when no room is available prevents skipping undiscovered work.
    pub(super) fn prepare_refill(
        &mut self,
        finalized_slot: Slot,
        limit: usize,
        is_parent_attached: impl Fn(&OLBlockId) -> bool,
    ) -> usize {
        self.entries.retain(|block, entry| {
            block.slot() > finalized_slot || entry.action == PendingAction::Cleanup
        });
        // Rotate only when full, preserving ready work until it gets an attempt.
        if self.entries.len() == MAX_PENDING_BLOCKS {
            for _ in 0..limit {
                let Some(block) = self.eviction_candidate(&is_parent_attached) else {
                    break;
                };
                self.entries.remove(&block);
                counter!("strata_fcm_pending_evicted_total").increment(1);
            }
        }
        self.record_size();
        (MAX_PENDING_BLOCKS - self.entries.len()).min(limit)
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Removes a resolved block from the cache and records the new queue size.
    pub(super) fn remove(&mut self, block: OLBlockCommitment) {
        if self.entries.remove(&block).is_some() {
            counter!("strata_fcm_pending_resolved_total").increment(1);
        }
        self.record_size();
    }

    /// Caches a newly discovered block for immediate retry without resetting existing backoff.
    ///
    /// At capacity, evicts the least recently attempted eligible leaf. Unattempted
    /// ready entries stay until attempted; blocked entries can be evicted after one second.
    /// Storage- and indexing-deferred entries remain protected until their retry deadline.
    /// Parents of cached children retain their places. Durable scans recover evictions.
    pub(super) fn cache_block(
        &mut self,
        block: &OLBlockV1,
        is_parent_attached: impl Fn(&OLBlockId) -> bool,
    ) {
        let key = block.header().compute_block_commitment();
        if self.entries.contains_key(&key) {
            return;
        }
        if self.entries.len() == MAX_PENDING_BLOCKS {
            // Rotate the oldest attempted entry back to durable storage so a queue of
            // mismatches cannot prevent later discovered blocks from ever being tried.
            // Keep entries that anchor pending descendants so eviction cannot make a
            // child appear ready before its parent attaches to the chain tracker.
            let Some(oldest) = self.eviction_candidate(&is_parent_attached) else {
                // Give recently discovered work a turn before replacing it.
                return;
            };
            self.entries.remove(&oldest);
            counter!("strata_fcm_pending_evicted_total").increment(1);
        }
        let now = Instant::now();
        self.entries.insert(
            key,
            PendingEntry {
                action: PendingAction::Execute,
                parent: *block.header().parent_blkid(),
                reason: ExecutionDeferral::Dependency,
                failures: 0,
                next_retry: now,
                last_attempt: now,
                first_seen: now,
                last_warned: now,
            },
        );
        self.record_size();
    }

    /// Schedules rejection cleanup independently of execution and parent readiness.
    pub(super) fn cache_cleanup(
        &mut self,
        block: &OLBlockV1,
        is_parent_attached: impl Fn(&OLBlockId) -> bool,
    ) {
        self.cache_block(block, is_parent_attached);
        if let Some(entry) = self
            .entries
            .get_mut(&block.header().compute_block_commitment())
        {
            entry.action = PendingAction::Cleanup;
        }
    }

    pub(super) fn needs_cleanup(&self, block: OLBlockCommitment) -> bool {
        self.entries
            .get(&block)
            .is_some_and(|entry| entry.action == PendingAction::Cleanup)
    }

    /// Caches a deferred block when space permits and schedules its next attempt.
    pub(super) fn defer(
        &mut self,
        block: &OLBlockV1,
        reason: ExecutionDeferral,
        is_parent_attached: impl Fn(&OLBlockId) -> bool,
    ) {
        self.cache_block(block, is_parent_attached);
        let key = block.header().compute_block_commitment();
        self.delay(key, reason);
        counter!("strata_fcm_blocks_deferred_total").increment(1);
    }

    /// Applies storage backoff to a cached block after a failed retry.
    pub(super) fn record_storage_failure(&mut self, block: OLBlockCommitment) {
        self.delay(block, ExecutionDeferral::Storage);
    }

    /// Delays dependency retries by one second and storage or indexing retries by exponential
    /// backoff capped at 32 seconds. Long-lived entries emit a warning at most every five
    /// minutes.
    fn delay(&mut self, key: OLBlockCommitment, reason: ExecutionDeferral) {
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.reason = reason;
            entry.failures = entry.failures.saturating_add(1);
            let seconds = match reason {
                ExecutionDeferral::Dependency => 1,
                ExecutionDeferral::Storage | ExecutionDeferral::Indexing => {
                    1u64 << entry.failures.saturating_sub(1).min(5)
                }
            };
            entry.last_attempt = Instant::now();
            entry.next_retry = entry.last_attempt + Duration::from_secs(seconds);
            if entry.last_warned.elapsed() >= Duration::from_secs(300) {
                let slot = key.slot();
                let id = key.blkid();
                warn!(slot, %id, ?reason, pending_seconds = entry.first_seen.elapsed().as_secs(), "block still waiting for execution dependencies");
                entry.last_warned = entry.last_attempt;
            }
        }
    }

    /// Selects a retry batch and marks each selected block as attempted now.
    ///
    /// Progress bypasses dependency delays, but preserves storage and indexing backoff. Non-genesis
    /// blocks wait until their parent is attached and absent from this cache. Selection
    /// favors the least recently attempted blocks to prevent starvation, then returns
    /// the batch in slot order. Excludes blocks already attempted during this pass.
    /// Entries remain cached until resolved or evicted.
    pub(super) fn select_retry_batch(
        &mut self,
        progress: bool,
        limit: usize,
        attempted: &[OLBlockCommitment],
        is_parent_attached: impl Fn(&OLBlockId) -> bool,
    ) -> Vec<OLBlockCommitment> {
        let now = Instant::now();
        let mut candidates: Vec<_> = self
            .entries
            .iter()
            .filter(|(block, _)| !attempted.contains(block))
            .filter(|(_, entry)| {
                entry.next_retry <= now
                    || (progress && entry.reason == ExecutionDeferral::Dependency)
            })
            .filter(|(block, entry)| self.is_ready(block, entry, &is_parent_attached))
            .map(|(key, entry)| (*key, entry.last_attempt))
            .collect();
        // Fairness across permanently deferred lower slots; execute each selected batch
        // in slot order so a ready parent precedes its child.
        candidates.sort_by_key(|(_, attempted)| *attempted);
        candidates.truncate(limit);
        let mut keys: Vec<_> = candidates.into_iter().map(|(key, _)| key).collect();
        keys.sort();
        for key in &keys {
            if let Some(entry) = self.entries.get_mut(key) {
                entry.last_attempt = now;
            }
        }
        keys
    }

    fn record_size(&self) {
        gauge!("strata_fcm_pending_blocks").set(self.len() as f64);
    }
}

#[cfg(test)]
mod tests {
    use strata_identifiers::Buf32;

    use super::*;

    fn entry(parent: OLBlockId) -> PendingEntry {
        let now = Instant::now();
        PendingEntry {
            action: PendingAction::Execute,
            parent,
            reason: ExecutionDeferral::Dependency,
            failures: 0,
            next_retry: now,
            last_attempt: now,
            first_seen: now,
            last_warned: now,
        }
    }

    #[test]
    fn missing_parent_blocks_child_until_parent_resolves() {
        let parent = OLBlockId::from(Buf32::from([1; 32]));
        let child = OLBlockId::from(Buf32::from([2; 32]));
        let mut pending = PendingBlockCache::default();
        pending
            .entries
            .insert(OLBlockCommitment::new(1, parent), entry(OLBlockId::null()));
        pending
            .entries
            .insert(OLBlockCommitment::new(2, child), entry(parent));
        assert_eq!(
            pending.select_retry_batch(true, 32, &[], |_| true),
            vec![OLBlockCommitment::new(1, parent)]
        );
        pending.remove(OLBlockCommitment::new(1, parent));
        assert_eq!(
            pending.select_retry_batch(true, 32, &[], |_| true),
            vec![OLBlockCommitment::new(2, child)]
        );
    }

    #[test]
    fn finality_preserves_cleanup_with_storage_backoff() {
        let parent = OLBlockId::null();
        let rejected = OLBlockCommitment::new(1, OLBlockId::from(Buf32::from([1; 32])));
        let finalized = OLBlockCommitment::new(2, OLBlockId::from(Buf32::from([2; 32])));
        let mut pending = PendingBlockCache::default();
        let mut cleanup = entry(parent);
        cleanup.action = PendingAction::Cleanup;
        pending.entries.insert(rejected, cleanup);
        pending.entries.insert(finalized, entry(parent));
        pending.delay(rejected, ExecutionDeferral::Storage);

        assert_eq!(
            pending.prepare_refill(2, STATUS_SCAN_SIZE, |_| false),
            STATUS_SCAN_SIZE
        );
        assert!(pending.needs_cleanup(rejected));
        assert_eq!(pending.len(), 1);
        assert!(pending
            .select_retry_batch(true, RETRY_BATCH_SIZE, &[], |_| true)
            .is_empty());
    }

    #[test]
    fn missing_parent_outside_queue_blocks_child_until_parent_attaches() {
        let parent = OLBlockId::from(Buf32::from([1; 32]));
        let child = OLBlockId::from(Buf32::from([2; 32]));
        let mut pending = PendingBlockCache::default();
        pending
            .entries
            .insert(OLBlockCommitment::new(2, child), entry(parent));

        assert!(pending
            .select_retry_batch(true, 32, &[], |_| false)
            .is_empty());
        assert_eq!(
            pending.select_retry_batch(true, 32, &[], |id| *id == parent),
            vec![OLBlockCommitment::new(2, child)]
        );
    }

    #[test]
    fn eviction_preserves_entries_with_pending_children() {
        let parent = OLBlockId::from(Buf32::from([1; 32]));
        let child = OLBlockId::from(Buf32::from([2; 32]));
        let leaf = OLBlockId::from(Buf32::from([3; 32]));
        let mut pending = PendingBlockCache::default();
        let mut parent_entry = entry(OLBlockId::null());
        parent_entry.failures = 1;
        parent_entry.last_attempt -= Duration::from_secs(2);
        let mut leaf_entry = entry(OLBlockId::null());
        leaf_entry.failures = 1;
        leaf_entry.last_attempt -= Duration::from_secs(1);
        pending
            .entries
            .insert(OLBlockCommitment::new(1, parent), parent_entry);
        pending
            .entries
            .insert(OLBlockCommitment::new(2, child), entry(parent));
        pending
            .entries
            .insert(OLBlockCommitment::new(3, leaf), leaf_entry);

        assert_eq!(
            pending.eviction_candidate(&|_| true),
            Some(OLBlockCommitment::new(3, leaf))
        );
    }

    #[test]
    fn aged_ready_entries_keep_their_first_attempt() {
        let parent = OLBlockId::from(Buf32::from([1; 32]));
        let block = OLBlockCommitment::new(1, OLBlockId::from(Buf32::from([2; 32])));
        let mut pending = PendingBlockCache::default();
        let mut waiting = entry(parent);
        waiting.first_seen -= Duration::from_secs(60);
        pending.entries.insert(block, waiting);

        assert_eq!(pending.eviction_candidate(&|_| true), None);
        assert_eq!(pending.eviction_candidate(&|_| false), Some(block));
        assert_eq!(
            pending.select_retry_batch(true, 1, &[], |_| true),
            vec![block]
        );
        pending.delay(block, ExecutionDeferral::Dependency);
        assert_eq!(pending.eviction_candidate(&|_| true), Some(block));
    }

    #[test]
    fn full_blocked_chain_releases_aged_unattempted_leaves() {
        let anchor = OLBlockId::null();
        let mut parent = anchor;
        let mut pending = PendingBlockCache::default();
        for slot in 1..=MAX_PENDING_BLOCKS {
            let mut bytes = [0; 32];
            bytes[..8].copy_from_slice(&(slot as u64).to_le_bytes());
            let id = OLBlockId::from(Buf32::from(bytes));
            let mut blocked = entry(parent);
            blocked.first_seen -= UNATTEMPTED_RETENTION;
            blocked.last_attempt = blocked.first_seen;
            pending
                .entries
                .insert(OLBlockCommitment::new(slot as u64, id), blocked);
            parent = id;
        }

        assert_eq!(
            pending.prepare_refill(0, STATUS_SCAN_SIZE, |id| *id == anchor),
            STATUS_SCAN_SIZE
        );
        assert_eq!(pending.len(), MAX_PENDING_BLOCKS - STATUS_SCAN_SIZE);
        let batch = pending.select_retry_batch(true, RETRY_BATCH_SIZE, &[], |id| *id == anchor);
        assert_eq!(batch.len(), 1);
        assert_eq!(
            batch[0].slot(),
            1,
            "the root must survive eviction of blocked descendants"
        );
    }

    #[test]
    fn refill_prunes_finalized_entries_and_retains_missing_parent_descendants() {
        let parent = OLBlockId::from(Buf32::from([1; 32]));
        let child = OLBlockId::from(Buf32::from([2; 32]));
        let descendant = OLBlockId::from(Buf32::from([3; 32]));
        let mut pending = PendingBlockCache::default();
        pending
            .entries
            .insert(OLBlockCommitment::new(1, parent), entry(OLBlockId::null()));
        pending
            .entries
            .insert(OLBlockCommitment::new(2, child), entry(parent));
        pending
            .entries
            .insert(OLBlockCommitment::new(3, descendant), entry(child));

        assert_eq!(
            pending.prepare_refill(1, STATUS_SCAN_SIZE, |_| false),
            STATUS_SCAN_SIZE
        );
        assert_eq!(
            pending.entries.keys().copied().collect::<Vec<_>>(),
            vec![
                OLBlockCommitment::new(2, child),
                OLBlockCommitment::new(3, descendant),
            ]
        );
    }

    #[test]
    fn storage_and_indexing_backoff_are_capped_and_progress_cannot_bypass_them() {
        for reason in [ExecutionDeferral::Storage, ExecutionDeferral::Indexing] {
            let id = OLBlockId::from(Buf32::from([1; 32]));
            let mut pending = PendingBlockCache::default();
            pending
                .entries
                .insert(OLBlockCommitment::new(1, id), entry(OLBlockId::null()));
            for _ in 0..40 {
                pending.delay(OLBlockCommitment::new(1, id), reason);
            }
            let entry = pending.entries.get(&OLBlockCommitment::new(1, id)).unwrap();
            assert_eq!(
                entry.next_retry.duration_since(entry.last_attempt),
                Duration::from_secs(32)
            );
            assert!(pending
                .select_retry_batch(true, 32, &[], |_| true)
                .is_empty());
            pending
                .entries
                .get_mut(&OLBlockCommitment::new(1, id))
                .unwrap()
                .next_retry = Instant::now();
            assert_eq!(
                pending.select_retry_batch(false, 32, &[], |_| true),
                vec![OLBlockCommitment::new(1, id)]
            );
        }
    }
}
