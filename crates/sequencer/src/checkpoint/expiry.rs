//! Handle timeouts for checkpoints in ProofPublishMode::Timeout

use std::{cmp::Reverse, collections::BinaryHeap, sync::Arc};

use strata_db::types::CheckpointProvingStatus;
use tokio::{
    select,
    time::{self, Duration, Instant},
};
use tracing::{debug, error, warn};

use crate::checkpoint::CheckpointHandle;

// FIXME WHAT DO THESE FIELDS REPRESENT????????
#[derive(Debug, PartialEq, Eq)]
struct CheckpointExpiry(u64, Instant);

impl CheckpointExpiry {
    fn expiry(&self) -> Instant {
        self.1
    }

    fn checkpoint_idx(&self) -> u64 {
        self.0
    }
}

impl PartialOrd for CheckpointExpiry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CheckpointExpiry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.1.cmp(&other.1)
    }
}

/// Worker to handle checkpoint prover expiry and submit empty proof after timeout.
pub async fn checkpoint_expiry_worker(
    checkpoint_handle: Arc<CheckpointHandle>,
    proof_timeout: Duration,
) -> anyhow::Result<()> {
    // min heap of checkpoint expiry items
    // Note: there will normally only be a single item we need to track in absence of forks
    let mut expiry_queue = BinaryHeap::<Reverse<CheckpointExpiry>>::new();
    let mut subscription = checkpoint_handle.subscribe();

    // check for pending checkpoint in db
    if let Some(last_checkpoint_idx) = checkpoint_handle.get_last_checkpoint_idx().await? {
        let checkpoint = match checkpoint_handle
            .get_checkpoint(last_checkpoint_idx)
            .await?
        {
            Some(entry) => entry,
            None => {
                warn!(%last_checkpoint_idx, "Expected checkpoint not found in db");
                return Ok(());
            }
        };

        if checkpoint.proving_status == CheckpointProvingStatus::PendingProof {
            let expiry_time = Instant::now() + proof_timeout;
            expiry_queue.push(Reverse(CheckpointExpiry(last_checkpoint_idx, expiry_time)));
        }
    }

    loop {
        select! {
            Ok(new_checkpoint_idx) = subscription.recv() => {
                let checkpoint = match checkpoint_handle.get_checkpoint(new_checkpoint_idx).await {
                    Ok(Some(entry)) => entry,
                    Ok(None) => {
                        warn!(%new_checkpoint_idx, "Expected checkpoint not found in db");
                        continue;
                    }
                    Err(e) => {
                        error!(%new_checkpoint_idx, ?e, "DB error occurred while fetching checkpoint ");
                        continue;
                    }
                };

                if checkpoint.proving_status != CheckpointProvingStatus::PendingProof {
                    continue;
                }

                let expiry_time = time::Instant::now() + proof_timeout;

                expiry_queue.push(Reverse(CheckpointExpiry(new_checkpoint_idx, expiry_time)));
            }

            _ = time::sleep_until(expiry_queue
                .peek()
                .map(|Reverse(CheckpointExpiry(_, expiry_time))| *expiry_time)
                .unwrap_or_else(|| Instant::now() + Duration::from_millis(500))) => {
                    let now = Instant::now();
                    while let Some(Reverse(checkpoint_expiry)) = expiry_queue.peek() {
                        if checkpoint_expiry.expiry() > now {
                            break;
                        }

                        handle_pending_checkpoint_expiry(&checkpoint_handle, checkpoint_expiry.checkpoint_idx()).await;
                        expiry_queue.pop();
                    }
            }
        }
    }
}

async fn handle_pending_checkpoint_expiry(checkpoint_handle: &CheckpointHandle, idx: u64) {
    match checkpoint_handle.get_checkpoint(idx).await {
        Ok(Some(mut entry)) => {
            if entry.proving_status != CheckpointProvingStatus::PendingProof {
                warn!("Got request for already ready proof");
                return;
            }
            debug!(%idx, "proof is pending, setting proof ready");

            entry.proving_status = CheckpointProvingStatus::ProofReady;
            if let Err(e) = checkpoint_handle.put_checkpoint(idx, entry).await {
                error!(%e, "could not update checkpoint after timeout");
            }

            debug!(%idx, "successfully submitted proof after timeout");
        }
        Ok(None) => {
            error!(%idx, "Expected checkpoint not found in db");
        }
        Err(e) => {
            error!(%idx, ?e, "DB error occurred while fetching checkpoint ");
        }
    }
}
