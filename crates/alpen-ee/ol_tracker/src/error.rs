use alpen_ee_common::{OLClientError, StorageError};
use thiserror::Error;

/// Error type for OL tracker operations.
///
/// Errors are categorized into:
/// - **Recoverable**: Transient failures that can be retried (network issues, temporary DB
///   failures)
/// - **NonRecoverable**: Fatal errors requiring intervention (no fork point found, data corruption)
#[derive(Debug, Error)]
pub enum OLTrackerError {
    /// Storage operation failed (recoverable - may be transient DB issue)
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    /// OL client operation failed (recoverable - network/RPC issues)
    #[error("OL client error: {0}")]
    OLClient(#[from] OLClientError),

    /// Failed to build tracker state from storage data (recoverable - may succeed on retry)
    #[error("failed to build tracker state: {0}")]
    BuildStateFailed(String),

    /// No common fork point found between local chain and OL chain.
    /// This is a FATAL error indicating complete chain divergence.
    /// Recovery requires manual intervention: wipe DB and resync from genesis.
    #[error("no fork point found back to genesis epoch {genesis_epoch}")]
    NoForkPointFound { genesis_epoch: u64 },

    /// Expected block data is missing from storage (potentially recoverable)
    #[error("missing expected block: {block_id}")]
    MissingBlock { block_id: String },

    /// Generic error for unexpected conditions (may be recoverable depending on cause)
    #[error("{0}")]
    Other(String),
}

impl OLTrackerError {
    /// Returns true if this error is recoverable and the operation can be retried.
    pub fn is_recoverable(&self) -> bool {
        match self {
            // Non-recoverable: requires manual intervention
            OLTrackerError::NoForkPointFound { .. } => false,

            // All others are potentially recoverable
            OLTrackerError::Storage(_)
            | OLTrackerError::OLClient(_)
            | OLTrackerError::BuildStateFailed(_)
            | OLTrackerError::MissingBlock { .. }
            | OLTrackerError::Other(_) => true,
        }
    }

    /// Returns true if this is a fatal error that should cause the task to panic.
    pub fn is_fatal(&self) -> bool {
        !self.is_recoverable()
    }

    /// Creates a detailed panic message for non-recoverable errors.
    pub fn panic_message(&self) -> String {
        match self {
            OLTrackerError::NoForkPointFound { genesis_epoch } => {
                format!(
                    "FATAL: OL tracker cannot recover - no common fork point found.\n\
                     \n\
                     The local chain has completely diverged from the OL chain.\n\
                     No common ancestor exists between local state and OL chain history\n\
                     going back to genesis epoch {}.",
                    genesis_epoch
                )
            }
            _ => format!("FATAL: Unexpected non-recoverable error: {}", self),
        }
    }
}

impl From<eyre::Error> for OLTrackerError {
    fn from(e: eyre::Error) -> Self {
        OLTrackerError::Other(e.to_string())
    }
}

pub(crate) type Result<T> = std::result::Result<T, OLTrackerError>;
