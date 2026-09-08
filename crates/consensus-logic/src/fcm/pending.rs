//! Bounded scheduling cache for durable unchecked blocks.

use std::{collections::BTreeMap, time::Duration};

use metrics::{counter, gauge};
use strata_identifiers::{OLBlockId, Slot};
use strata_ol_chain_types_v1::OLBlockV1;
use tokio::time::Instant;
use tracing::warn;

use super::ExecutionDeferral;

const MAX_PENDING_BLOCKS: usize = 256;
pub(super) const RETRY_BATCH_SIZE: usize = 32;
pub(super) const STATUS_SCAN_SIZE: usize = 64;

type PendingKey = (Slot, OLBlockId);

#[derive(Debug)]
struct PendingEntry {
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
pub(super) struct PendingBlocks {
    entries: BTreeMap<PendingKey, PendingEntry>,
    pub(super) scan_cursor: Option<OLBlockId>,
}

impl PendingBlocks {
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn remove(&mut self, slot: Slot, id: OLBlockId) {
        if self.entries.remove(&(slot, id)).is_some() {
            counter!("strata_fcm_pending_resolved_total").increment(1);
        }
        self.record_size();
    }

    pub(super) fn discover(&mut self, block: &OLBlockV1) {
        let key = (block.header().slot(), block.header().compute_blkid());
        if self.entries.contains_key(&key) {
            return;
        }
        if self.entries.len() == MAX_PENDING_BLOCKS {
            // Rotate the oldest attempted entry back to durable storage so a queue of
            // mismatches cannot prevent later discovered blocks from ever being tried.
            let Some(oldest) = self
                .entries
                .iter()
                .filter(|(_, entry)| entry.failures > 0)
                .min_by_key(|(_, entry)| entry.last_attempt)
                .map(|(key, _)| *key)
            else {
                // Preserve work that has never had a turn. The durable scan will
                // revisit this incoming entry after the current batch is attempted.
                return;
            };
            self.entries.remove(&oldest);
            counter!("strata_fcm_pending_evicted_total").increment(1);
        }
        let now = Instant::now();
        self.entries.insert(
            key,
            PendingEntry {
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

    pub(super) fn defer(&mut self, block: &OLBlockV1, reason: ExecutionDeferral) {
        self.discover(block);
        let key = (block.header().slot(), block.header().compute_blkid());
        self.delay(key, reason);
        counter!("strata_fcm_blocks_deferred_total").increment(1);
    }

    pub(super) fn storage_failure(&mut self, slot: Slot, id: OLBlockId) {
        self.delay((slot, id), ExecutionDeferral::Storage);
    }

    fn delay(&mut self, key: PendingKey, reason: ExecutionDeferral) {
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.reason = reason;
            entry.failures = entry.failures.saturating_add(1);
            let seconds = match reason {
                ExecutionDeferral::Dependency => 1,
                ExecutionDeferral::Storage => 1u64 << entry.failures.saturating_sub(1).min(5),
            };
            entry.last_attempt = Instant::now();
            entry.next_retry = entry.last_attempt + Duration::from_secs(seconds);
            if entry.last_warned.elapsed() >= Duration::from_secs(300) {
                let (slot, id) = key;
                warn!(slot, %id, ?reason, pending_seconds = entry.first_seen.elapsed().as_secs(), "block still waiting for execution dependencies");
                entry.last_warned = entry.last_attempt;
            }
        }
    }

    pub(super) fn due(&mut self, progress: bool, limit: usize) -> Vec<PendingKey> {
        let now = Instant::now();
        let mut candidates: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| {
                entry.next_retry <= now
                    || (progress && entry.reason == ExecutionDeferral::Dependency)
            })
            .filter(|((slot, _), entry)| {
                slot.checked_sub(1).is_none_or(|parent_slot| {
                    !self.entries.contains_key(&(parent_slot, entry.parent))
                })
            })
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
        let mut pending = PendingBlocks::default();
        pending
            .entries
            .insert((1, parent), entry(OLBlockId::null()));
        pending.entries.insert((2, child), entry(parent));
        assert_eq!(pending.due(true, 32), vec![(1, parent)]);
        pending.remove(1, parent);
        assert_eq!(pending.due(true, 32), vec![(2, child)]);
    }

    #[test]
    fn storage_backoff_is_capped_and_progress_does_not_bypass_it() {
        let id = OLBlockId::from(Buf32::from([1; 32]));
        let mut pending = PendingBlocks::default();
        pending.entries.insert((1, id), entry(OLBlockId::null()));
        for _ in 0..40 {
            pending.storage_failure(1, id);
        }
        let entry = pending.entries.get(&(1, id)).unwrap();
        assert_eq!(
            entry.next_retry.duration_since(entry.last_attempt),
            Duration::from_secs(32)
        );
        assert!(pending.due(true, 32).is_empty());
        pending.entries.get_mut(&(1, id)).unwrap().next_retry = Instant::now();
        assert_eq!(pending.due(false, 32), vec![(1, id)]);
    }
}
