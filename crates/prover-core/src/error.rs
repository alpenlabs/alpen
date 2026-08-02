//! Error types.
//!
//! Variants are typed (no erased `anyhow::Error` wrapped inside the enum).
//! Wrapping `anyhow` inside a `thiserror` enum muddies downcast behavior
//! for callers — keep the library boundary typed and let applications
//! decide whether to erase upstream.

pub type ProverResult<T> = Result<T, ProverError>;

#[derive(Debug, thiserror::Error)]
pub enum ProverError {
    #[error("task not found: {0}")]
    TaskNotFound(String),

    #[error("task already exists: {0}")]
    TaskAlreadyExists(String),

    #[error("no receipt store configured")]
    NoReceiptStore,

    #[error("transient: {0}")]
    TransientFailure(String),

    #[error("permanent: {0}")]
    PermanentFailure(String),

    /// Backend IO failure (sled, filesystem, tokio runtime, ...).
    #[error("storage: {0}")]
    Storage(String),

    /// Encode or decode of a stored record failed.
    #[error("codec: {0}")]
    Codec(String),

    /// Command channel failure (send/recv/cancelled).
    #[error("command channel: {0}")]
    Command(String),
}

impl ProverError {
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::TransientFailure(_))
    }

    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::PermanentFailure(_))
    }
}

/// Substrings that mark a proving failure as an infrastructure fault rather
/// than a property of the task. Matched case-insensitively.
///
/// `signal: 7` / `signal 7` is the Linux SIGBUS exit status reported by an
/// out-of-process proving backend (SP1 server, docker); `bus error` is the
/// shell rendering of the same kill.
const TRANSIENT_PROVE_FAILURE_MARKERS: &[&str] = &["sigbus", "signal: 7", "signal 7", "bus error"];

/// Classifies a proving-backend failure message.
///
/// Resource-exhaustion kills (SIGBUS from a full `/dev/shm`, bus errors from
/// mmap-backed proving) are retryable infrastructure faults, not properties of
/// the task, so they map to [`ProverError::TransientFailure`] and go through
/// bounded retry with backoff. Everything else stays
/// [`ProverError::PermanentFailure`].
///
/// The failure reaches this layer only as a string, hence the substring match.
/// A typed exit-status channel through zkaleido would be the clean fix.
pub(crate) fn classify_prove_failure(msg: String) -> ProverError {
    let lower = msg.to_lowercase();
    if TRANSIENT_PROVE_FAILURE_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
    {
        ProverError::TransientFailure(msg)
    } else {
        ProverError::PermanentFailure(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::{classify_prove_failure, ProverError};

    #[test]
    fn sigbus_kills_classify_as_transient() {
        for msg in [
            "SIGBUS",
            "prove failed: exit status: signal: 7 (SIGBUS)",
            "worker died with signal 7",
            "Bus error (core dumped)",
        ] {
            let err = classify_prove_failure(msg.to_string());
            assert!(
                err.is_transient(),
                "expected transient classification for {msg:?}, got {err:?}"
            );
            assert!(matches!(err, ProverError::TransientFailure(inner) if inner == msg));
        }
    }

    #[test]
    fn ordinary_prove_failures_stay_permanent() {
        for msg in [
            "invalid public values",
            "guest program panicked: assertion failed",
            "signal: 11 (SIGSEGV)",
            "",
        ] {
            let err = classify_prove_failure(msg.to_string());
            assert!(
                err.is_permanent(),
                "expected permanent classification for {msg:?}, got {err:?}"
            );
        }
    }
}
