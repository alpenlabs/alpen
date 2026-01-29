//! Status monitoring task for chunked envelope publications.
//!
//! The watcher periodically checks the confirmation status of pending payloads
//! and updates their status in the database.

// Placeholders for future implementation when database traits are added.
// The core logic (update_blob_status) is tested via unit tests.
#![expect(dead_code, reason = "placeholder module - will be used when DB traits are added")]

use std::time::Duration;

use tokio::time::sleep;
use tracing::{debug, info};

use super::types::DaBlobStatus;

/// Configuration for the watcher task.
#[derive(Debug, Clone)]
pub(crate) struct WatcherConfig {
    /// Interval between status checks.
    pub poll_interval: Duration,
    /// Number of confirmations required for "finalized" status.
    pub finalization_depth: u32,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(30),
            finalization_depth: 6,
        }
    }
}

/// Placeholder for watcher task.
///
/// The full implementation requires:
/// - Database access to get pending payloads
/// - Broadcaster to check tx confirmation status
/// - Database updates for status changes
///
/// This will be implemented when database traits are added.
pub(crate) async fn chunked_envelope_watcher_task(
    _config: WatcherConfig,
    // TODO: Add database ops
    // TODO: Add broadcast handle
) {
    info!("Chunked envelope watcher task starting");

    loop {
        debug!("Checking chunked envelope status...");

        // TODO: Implement status checking loop:
        // 1. Get pending payloads from database
        // 2. For each payload:
        //    - Check commit tx status
        //    - Check reveal tx statuses
        //    - Update payload status in database
        // 3. Sleep for poll_interval

        sleep(Duration::from_secs(30)).await;
    }
}

/// Updates payload status based on transaction confirmations.
///
/// Status transitions:
/// - Pending → CommitConfirmed (when commit has 1+ confirmations)
/// - CommitConfirmed → AllRevealsConfirmed (when all reveals have 1+ confirmations)
/// - AllRevealsConfirmed → Finalized (when all reveals have finalization_depth confirmations)
fn update_blob_status(
    current: &DaBlobStatus,
    commit_confirmations: u32,
    reveal_confirmations: &[u32],
    finalization_depth: u32,
) -> Option<DaBlobStatus> {
    let total_chunks = reveal_confirmations.len() as u16;

    match current {
        DaBlobStatus::Pending => {
            if commit_confirmations >= 1 {
                Some(DaBlobStatus::CommitConfirmed {
                    reveals_confirmed: 0,
                })
            } else {
                None
            }
        }

        DaBlobStatus::CommitConfirmed { reveals_confirmed } => {
            let confirmed_count = reveal_confirmations
                .iter()
                .filter(|&&c| c >= 1)
                .count() as u16;

            if confirmed_count == total_chunks {
                Some(DaBlobStatus::AllRevealsConfirmed)
            } else if confirmed_count > *reveals_confirmed {
                Some(DaBlobStatus::CommitConfirmed {
                    reveals_confirmed: confirmed_count,
                })
            } else {
                None
            }
        }

        DaBlobStatus::AllRevealsConfirmed => {
            let all_finalized = reveal_confirmations
                .iter()
                .all(|&c| c >= finalization_depth);

            if all_finalized {
                Some(DaBlobStatus::Finalized)
            } else {
                None
            }
        }

        DaBlobStatus::Finalized | DaBlobStatus::Failed(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_transitions() {
        // Pending → CommitConfirmed
        let new = update_blob_status(
            &DaBlobStatus::Pending,
            1,
            &[0, 0],
            6,
        );
        assert!(matches!(new, Some(DaBlobStatus::CommitConfirmed { reveals_confirmed: 0 })));

        // CommitConfirmed → update reveals_confirmed
        let new = update_blob_status(
            &DaBlobStatus::CommitConfirmed { reveals_confirmed: 0 },
            1,
            &[1, 0],
            6,
        );
        assert!(matches!(new, Some(DaBlobStatus::CommitConfirmed { reveals_confirmed: 1 })));

        // CommitConfirmed → AllRevealsConfirmed
        let new = update_blob_status(
            &DaBlobStatus::CommitConfirmed { reveals_confirmed: 1 },
            1,
            &[1, 1],
            6,
        );
        assert!(matches!(new, Some(DaBlobStatus::AllRevealsConfirmed)));

        // AllRevealsConfirmed → Finalized
        let new = update_blob_status(
            &DaBlobStatus::AllRevealsConfirmed,
            6,
            &[6, 6],
            6,
        );
        assert!(matches!(new, Some(DaBlobStatus::Finalized)));

        // Finalized stays Finalized
        let new = update_blob_status(
            &DaBlobStatus::Finalized,
            10,
            &[10, 10],
            6,
        );
        assert!(new.is_none());
    }
}
