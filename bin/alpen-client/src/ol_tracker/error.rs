use alpen_ee_common::StorageError;
use thiserror::Error;

use crate::traits::error::OlClientError;

/// Error type for OL tracker operations.
///
/// Errors are categorized into:
/// - **Recoverable**: Transient failures that can be retried (network issues, temporary DB
///   failures)
/// - **NonRecoverable**: Fatal errors requiring intervention (no fork point found, data corruption)
#[derive(Debug, Error)]
pub(crate) enum OlTrackerError {
    /// Storage operation failed (recoverable - may be transient DB issue)
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    /// OL client operation failed (recoverable - network/RPC issues)
    #[error("OL client error: {0}")]
    OlClient(#[from] OlClientError),

    /// Failed to build tracker state from storage data (recoverable - may succeed on retry)
    #[error("failed to build tracker state: {0}")]
    BuildStateFailed(String),

    /// No common fork point found between local chain and OL chain.
    /// This is a FATAL error indicating complete chain divergence.
    /// Recovery requires manual intervention: wipe DB and resync from genesis.
    #[error("no fork point found back to genesis slot {genesis_slot}")]
    NoForkPointFound { genesis_slot: u64 },

    /// Expected block data is missing from storage (potentially recoverable)
    #[error("missing expected block: {block_id}")]
    MissingBlock { block_id: String },

    /// Generic error for unexpected conditions (may be recoverable depending on cause)
    #[error("{0}")]
    Other(String),
}

impl OlTrackerError {
    /// Returns true if this error is recoverable and the operation can be retried.
    pub(crate) fn is_recoverable(&self) -> bool {
        match self {
            // Non-recoverable: requires manual intervention
            OlTrackerError::NoForkPointFound { .. } => false,

            // All others are potentially recoverable
            OlTrackerError::Storage(_)
            | OlTrackerError::OlClient(_)
            | OlTrackerError::BuildStateFailed(_)
            | OlTrackerError::MissingBlock { .. }
            | OlTrackerError::Other(_) => true,
        }
    }

    /// Returns true if this is a fatal error that should cause the task to panic.
    pub(crate) fn is_fatal(&self) -> bool {
        !self.is_recoverable()
    }

    /// Creates a detailed panic message for non-recoverable errors.
    pub(crate) fn panic_message(&self) -> String {
        match self {
            OlTrackerError::NoForkPointFound { genesis_slot } => {
                format!(
                    "FATAL: OL tracker cannot recover - no common fork point found.\n\
                     \n\
                     The local chain has completely diverged from the OL chain.\n\
                     No common ancestor exists between local state and OL chain history\n\
                     going back to genesis slot {}.",
                    genesis_slot
                )
            }
            _ => format!("FATAL: Unexpected non-recoverable error: {}", self),
        }
    }
}

impl From<eyre::Error> for OlTrackerError {
    fn from(e: eyre::Error) -> Self {
        OlTrackerError::Other(e.to_string())
    }
}

pub(crate) type Result<T> = std::result::Result<T, OlTrackerError>;
