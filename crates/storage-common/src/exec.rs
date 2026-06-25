//! Errors for the database operation execution layer.

use thiserror::Error;

/// Errors specific to the ops execution layer.
///
/// These errors represent failures in the operation execution infrastructure itself,
/// not database-level errors. The most common case is when a worker drops its
/// response channel before a result is sent (e.g. a coalesced cache fetch whose
/// primary load was aborted).
#[derive(Debug, Clone, Error)]
pub enum OpsError {
    #[error("worker failed strangely")]
    WorkerFailedStrangely,
}
