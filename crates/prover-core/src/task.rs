//! Task lifecycle types: status, result, stored record, and the
//! seconds-since-epoch time helpers that go with them.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

// ============================================================================
// TaskStatus / TaskResult
// ============================================================================

/// The three retry budgets a working (non-terminal) task accrues.
///
/// Bundled into one value and embedded in *every* non-terminal status
/// (`Proving`, `Blocked`, `TransientFailure`) so no counter is silently reset
/// when a task moves between them. A `Blocked` dependency wait interspersed with
/// transient infra errors used to reset the retry/resubmit budget on each block
/// and the recheck budget on each transient failure, letting a flaky task escape
/// all three ceilings and hang its waiters forever. Carrying all three together
/// makes a working status that omits a counter unrepresentable.
///
/// Terminal (`Completed`/`PermanentFailure`) and `Pending` statuses carry none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptCounts {
    /// Resume-class retries: network blips / crash recovery — re-poll the same
    /// request. Bounded by `RetryConfig::max_retries`.
    pub retry: u32,
    /// Resubmit-class retries: a dead remote request resubmitted fresh (each
    /// re-runs the whole proof). Bounded by `RetryConfig::max_resubmits`.
    pub resubmit: u32,
    /// Dependency rechecks accrued while `Blocked`. Bounded by
    /// `RetryConfig::max_blocked_rechecks`.
    pub recheck: u32,
}

/// Status of a proof task in the lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    /// Task registered but not yet picked up for proving.
    Pending,
    /// Actively being proved. Carries the [`AttemptCounts`] so they survive the
    /// `TransientFailure → Proving → (crash)` transition: if the process dies
    /// mid-attempt (OOM, SIGKILL, panic) the persisted record still reflects how
    /// many attempts the task has already burned, and `recover` can bump
    /// correctly instead of resetting to zero.
    Proving { counts: AttemptCounts },
    /// Proof completed successfully, receipt available.
    Completed,
    /// Parked waiting for an input dependency (e.g. an upstream chunk proof
    /// not yet produced). Not a failure: rechecked on a steady cadence via
    /// `retry_after`, and does NOT consume the retry/resubmit budget — but the
    /// [`AttemptCounts`] (including `recheck`) travel through so the
    /// `Blocked → Proving → Blocked` loop, and any interleaved transient
    /// failures, stay bounded.
    Blocked {
        reason: String,
        counts: AttemptCounts,
    },
    /// Temporary failure; will be retried after backoff.
    TransientFailure {
        counts: AttemptCounts,
        error: String,
    },
    /// Unrecoverable failure; task will not be retried.
    PermanentFailure { error: String },
}

impl TaskStatus {
    /// The retry/resubmit/recheck counters this status carries, or all-zero for
    /// terminal and `Pending` statuses. Snapshotted before the `Proving`
    /// overwrite in `run_task` so counters survive every status transition.
    pub fn counts(&self) -> AttemptCounts {
        match self {
            Self::Proving { counts }
            | Self::Blocked { counts, .. }
            | Self::TransientFailure { counts, .. } => *counts,
            Self::Pending | Self::Completed | Self::PermanentFailure { .. } => {
                AttemptCounts::default()
            }
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::PermanentFailure { .. })
    }

    pub fn is_retriable(&self) -> bool {
        matches!(self, Self::TransientFailure { .. })
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::Blocked { .. })
    }

    /// Statuses the scanner re-spawns once their `retry_after` elapses:
    /// transient failures (retry) and blocked tasks (dependency recheck).
    pub fn wants_rescan(&self) -> bool {
        self.is_retriable() || self.is_blocked()
    }

    pub fn is_in_progress(&self) -> bool {
        matches!(self, Self::Proving { .. })
    }

    /// True for any status that should be re-spawned on startup recovery:
    /// tasks that were submitted but never finished (Pending / Proving).
    /// Transient failures are handled separately by the retry scanner via
    /// [`Self::is_retriable`].
    pub fn is_unfinished(&self) -> bool {
        matches!(self, Self::Pending | Self::Proving { .. })
    }
}

/// Outcome of a completed (or failed) task. Returned by `execute` and `wait_for_tasks`.
#[derive(Debug, Clone)]
pub enum TaskResult<T> {
    Completed { task: T },
    Failed { task: T, error: String },
}

impl<T> TaskResult<T> {
    pub fn completed(task: T) -> Self {
        Self::Completed { task }
    }

    pub fn failed(task: T, error: impl Into<String>) -> Self {
        Self::Failed {
            task,
            error: error.into(),
        }
    }

    pub fn is_completed(&self) -> bool {
        matches!(self, Self::Completed { .. })
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }

    pub fn task(&self) -> &T {
        match self {
            Self::Completed { task } | Self::Failed { task, .. } => task,
        }
    }
}

// ============================================================================
// Stored record shape + time helpers
// ============================================================================

/// Current wall-clock seconds since UNIX epoch.
///
/// Internal helper — timestamps in task records are plain `u64` seconds
/// since epoch so the record encodes stably.
pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// The mutable state associated with a stored task, separate from its key.
///
/// Splitting the key bytes from the value fields makes the dataflow
/// explicit: storage backends store `TaskRecordData` against a `Vec<u8>`
/// key, and [`TaskRecord`] is just the key-value pair surfaced to callers.
///
/// All time fields are `u64` seconds since UNIX epoch so the record serializes
/// directly — persistent backends store this type as-is, no on-disk shadow
/// type, no conversion. Sub-second precision isn't needed anywhere in the
/// prover.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecordData {
    status: TaskStatus,
    updated_at_secs: u64,
    retry_after_secs: Option<u64>,
    /// Opaque bytes for strategy-specific state (e.g. remote ProofId for crash recovery).
    #[serde(with = "serde_bytes")]
    metadata: Option<Vec<u8>>,
}

impl TaskRecordData {
    pub fn new(status: TaskStatus) -> Self {
        Self {
            status,
            updated_at_secs: now_secs(),
            retry_after_secs: None,
            metadata: None,
        }
    }

    pub fn status(&self) -> &TaskStatus {
        &self.status
    }

    pub fn updated_at_secs(&self) -> u64 {
        self.updated_at_secs
    }

    pub fn retry_after_secs(&self) -> Option<u64> {
        self.retry_after_secs
    }

    pub fn metadata(&self) -> Option<&[u8]> {
        self.metadata.as_deref()
    }

    pub fn set_status(&mut self, status: TaskStatus) {
        self.status = status;
        self.updated_at_secs = now_secs();
    }

    pub fn set_retry_after_secs(&mut self, when: Option<u64>) {
        self.retry_after_secs = when;
        self.updated_at_secs = now_secs();
    }

    pub fn set_metadata(&mut self, data: Option<Vec<u8>>) {
        self.metadata = data;
        self.updated_at_secs = now_secs();
    }
}

/// A stored task: the opaque byte key plus its associated [`TaskRecordData`].
#[derive(Debug, Clone)]
pub struct TaskRecord {
    key: Vec<u8>,
    data: TaskRecordData,
}

impl TaskRecord {
    pub fn new(key: Vec<u8>, status: TaskStatus) -> Self {
        Self {
            key,
            data: TaskRecordData::new(status),
        }
    }

    pub fn from_parts(key: Vec<u8>, data: TaskRecordData) -> Self {
        Self { key, data }
    }

    pub fn key(&self) -> &[u8] {
        &self.key
    }

    pub fn data(&self) -> &TaskRecordData {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut TaskRecordData {
        &mut self.data
    }

    pub fn status(&self) -> &TaskStatus {
        self.data.status()
    }

    pub fn retry_after_secs(&self) -> Option<u64> {
        self.data.retry_after_secs()
    }

    pub fn metadata(&self) -> Option<&[u8]> {
        self.data.metadata()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every working status must round-trip all three counters through
    /// `counts()`, and terminal/pending statuses must report all-zero. This is
    /// the invariant `run_task`'s snapshot relies on to keep the retry,
    /// resubmit, and recheck budgets bounded across `Blocked ↔ Proving ↔
    /// TransientFailure` transitions — if any working variant dropped a counter,
    /// a flaky-dependency task could reset it and bypass its ceiling forever.
    #[test]
    fn counts_survive_every_working_status() {
        let c = AttemptCounts {
            retry: 4,
            resubmit: 2,
            recheck: 7,
        };

        assert_eq!(TaskStatus::Proving { counts: c }.counts(), c);
        assert_eq!(
            TaskStatus::Blocked {
                reason: "dep not ready".into(),
                counts: c,
            }
            .counts(),
            c
        );
        assert_eq!(
            TaskStatus::TransientFailure {
                counts: c,
                error: "rpc down".into(),
            }
            .counts(),
            c
        );

        // Terminal / pending statuses carry no counters.
        assert_eq!(TaskStatus::Pending.counts(), AttemptCounts::default());
        assert_eq!(TaskStatus::Completed.counts(), AttemptCounts::default());
        assert_eq!(
            TaskStatus::PermanentFailure { error: "x".into() }.counts(),
            AttemptCounts::default()
        );
    }
}
