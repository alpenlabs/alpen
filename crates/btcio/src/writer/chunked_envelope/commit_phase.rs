//! Mutual exclusion between chunked-commit fee bumping and reveal enqueueing.
//!
//! Replacing a chunked commit changes its txid, which invalidates every reveal that spends one of
//! its outputs. The two operations that must never interleave are therefore:
//!
//! - the fee bumper replacing a commit, and
//! - the writer handing that envelope's reveals to the broadcaster.
//!
//! Both run as tasks in the same process, so a per-envelope in-process latch is enough to serialise
//! them. Checking a persisted flag would not be: the check and the writes that follow it are not
//! one transaction, and a reveal can be enqueued in the gap.
//!
//! The latch is advisory across restarts, which is deliberate. On restart nothing is mid-operation,
//! and the durable fail-closed guard (the envelope row's reveal txids checked against the broadcast
//! DB, plus the tx-node tree) is what decides whether a commit is still replaceable.

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

/// Tracks which envelopes currently have an operation in flight that the other side must not race.
///
/// Cloning shares the underlying state; the writer and the fee bumper are handed clones of the same
/// latch.
#[derive(Debug, Clone, Default)]
pub struct CommitPhaseLatch {
    inner: Arc<Mutex<HashSet<u64>>>,
}

/// Releases its envelope's claim when dropped.
#[derive(Debug)]
pub struct CommitPhaseClaim {
    envelope_idx: u64,
    latch: CommitPhaseLatch,
}

impl Drop for CommitPhaseClaim {
    fn drop(&mut self) {
        self.latch.release(self.envelope_idx);
    }
}

impl CommitPhaseClaim {
    /// Returns the envelope this claim covers.
    pub fn envelope_idx(&self) -> u64 {
        self.envelope_idx
    }
}

impl CommitPhaseLatch {
    /// Creates an empty latch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Claims `envelope_idx`, or returns `None` when the other side already holds it.
    ///
    /// Callers must treat `None` as "try again next tick", never as permission to proceed.
    pub fn try_claim(&self, envelope_idx: u64) -> Option<CommitPhaseClaim> {
        let claimed = self
            .inner
            .lock()
            .expect("btcio: commit phase latch poisoned")
            .insert(envelope_idx);

        claimed.then(|| CommitPhaseClaim {
            envelope_idx,
            latch: self.clone(),
        })
    }

    fn release(&self, envelope_idx: u64) {
        self.inner
            .lock()
            .expect("btcio: commit phase latch poisoned")
            .remove(&envelope_idx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claim_excludes_the_other_side() {
        let latch = CommitPhaseLatch::new();

        let claim = latch.try_claim(7).expect("first claim succeeds");
        assert!(latch.try_claim(7).is_none(), "second claim must be refused");

        drop(claim);
        assert!(
            latch.try_claim(7).is_some(),
            "claim must be reusable once released"
        );
    }

    #[test]
    fn claims_are_per_envelope() {
        let latch = CommitPhaseLatch::new();

        let _first = latch.try_claim(1).expect("envelope 1 claim succeeds");
        assert!(
            latch.try_claim(2).is_some(),
            "a different envelope must not be blocked"
        );
    }

    #[test]
    fn clones_share_state() {
        let latch = CommitPhaseLatch::new();
        let other = latch.clone();

        let _claim = latch.try_claim(3).expect("claim succeeds");
        assert!(
            other.try_claim(3).is_none(),
            "a clone must observe the same claim"
        );
    }
}
