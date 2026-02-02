//! Chunked envelope handle and lifecycle driver.
//!
//! Polls entries by sequential index and advances them through the status
//! lifecycle: Unsigned → Unpublished → Published → Confirmed → Finalized.

use std::{sync::Arc, time::Duration};

use bitcoin::Address;
use bitcoind_async_client::{
    traits::{Reader, Signer, Wallet},
    Client,
};
use strata_config::btcio::WriterConfig;
use strata_db_types::{
    traits::L1ChunkedEnvelopeDatabase,
    types::{ChunkedEnvelopeEntry, ChunkedEnvelopeStatus, L1TxStatus},
};
use strata_params::Params;
use strata_status::StatusChannel;
use strata_storage::ops::chunked_envelope::{ChunkedEnvelopeOps, Context};
use strata_tasks::TaskExecutor;
use tokio::time::interval;
use tracing::*;

use super::signer::sign_chunked_envelope;
use crate::{
    broadcaster::L1BroadcastHandle,
    writer::{builder::EnvelopeError, context::WriterContext},
};

/// Handle for submitting chunked envelope entries.
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have debug impls"
)]
pub struct ChunkedEnvelopeHandle {
    ops: Arc<ChunkedEnvelopeOps>,
}

impl ChunkedEnvelopeHandle {
    pub(crate) fn new(ops: Arc<ChunkedEnvelopeOps>) -> Self {
        Self { ops }
    }

    /// Stores a new unsigned entry and returns the assigned index.
    pub async fn submit_entry(&self, entry: ChunkedEnvelopeEntry) -> anyhow::Result<u64> {
        let idx = self.ops.get_next_chunked_envelope_idx_async().await?;
        self.ops
            .put_chunked_envelope_entry_async(idx, entry)
            .await?;
        debug!(%idx, "submitted chunked envelope entry");
        Ok(idx)
    }

    /// Blocking variant of [`submit_entry`](Self::submit_entry).
    pub fn submit_entry_blocking(&self, entry: ChunkedEnvelopeEntry) -> anyhow::Result<u64> {
        let idx = self.ops.get_next_chunked_envelope_idx_blocking()?;
        self.ops.put_chunked_envelope_entry_blocking(idx, entry)?;
        debug!(%idx, "submitted chunked envelope entry");
        Ok(idx)
    }

    /// Returns the inner ops for direct reads.
    pub fn ops(&self) -> &Arc<ChunkedEnvelopeOps> {
        &self.ops
    }
}

/// Spawns the lifecycle driver and returns a [`ChunkedEnvelopeHandle`].
#[expect(clippy::too_many_arguments, reason = "used for starting envelope task")]
pub fn start_chunked_envelope_task<D: L1ChunkedEnvelopeDatabase>(
    executor: &TaskExecutor,
    bitcoin_client: Arc<Client>,
    config: Arc<WriterConfig>,
    params: Arc<Params>,
    sequencer_address: Address,
    db: Arc<D>,
    status_channel: StatusChannel,
    pool: threadpool::ThreadPool,
    broadcast_handle: Arc<L1BroadcastHandle>,
) -> anyhow::Result<Arc<ChunkedEnvelopeHandle>> {
    let ops = Arc::new(Context::new(db).into_ops(pool));
    let start_idx = find_first_pending_idx(ops.as_ref())?;
    let handle = Arc::new(ChunkedEnvelopeHandle::new(ops.clone()));

    let ctx = Arc::new(WriterContext::new(
        params,
        config,
        sequencer_address,
        bitcoin_client,
        status_channel,
    ));

    executor.spawn_critical_async("btcio::chunked_envelope_watcher", async move {
        watcher_task(start_idx, ctx, ops, broadcast_handle).await
    });

    Ok(handle)
}

/// Scans backwards to find the earliest non-finalized entry index.
fn find_first_pending_idx(ops: &ChunkedEnvelopeOps) -> anyhow::Result<u64> {
    let mut idx = ops.get_next_chunked_envelope_idx_blocking()?;
    while idx > 0 {
        let Some(entry) = ops.get_chunked_envelope_entry_blocking(idx - 1)? else {
            break;
        };
        if entry.status == ChunkedEnvelopeStatus::Finalized {
            break;
        }
        idx -= 1;
    }
    Ok(idx)
}

/// Polls entries and drives them through signing, broadcast, and confirmation.
async fn watcher_task<R: Reader + Signer + Wallet>(
    start_idx: u64,
    ctx: Arc<WriterContext<R>>,
    ops: Arc<ChunkedEnvelopeOps>,
    broadcast_handle: Arc<L1BroadcastHandle>,
) -> anyhow::Result<()> {
    info!("starting chunked envelope watcher");
    let tick = interval(Duration::from_millis(ctx.config.write_poll_dur_ms));
    tokio::pin!(tick);

    let mut curr = start_idx;
    loop {
        tick.as_mut().tick().await;

        let dspan = debug_span!("chunked_envelope", %curr);
        let _g = dspan.enter();

        let Some(entry) = ops.get_chunked_envelope_entry_async(curr).await? else {
            trace!("no entry at current index, waiting");
            continue;
        };

        match entry.status {
            ChunkedEnvelopeStatus::Unsigned | ChunkedEnvelopeStatus::NeedsResign => {
                debug!(status = ?entry.status, "entry needs signing");
                match sign_chunked_envelope(&entry, &broadcast_handle, ctx.clone()).await {
                    Ok(updated) => {
                        ops.put_chunked_envelope_entry_async(curr, updated).await?;
                        debug!("entry signed successfully");
                    }
                    Err(EnvelopeError::NotEnoughUtxos(need, have)) => {
                        error!(%need, %have, "waiting for sufficient utxos");
                    }
                    Err(e) => return Err(e.into()),
                }
            }

            ChunkedEnvelopeStatus::Finalized => {
                curr += 1;
            }

            ChunkedEnvelopeStatus::Unpublished
            | ChunkedEnvelopeStatus::Published
            | ChunkedEnvelopeStatus::Confirmed => {
                let new_status = check_broadcast_status(&entry, &broadcast_handle).await?;
                if new_status != entry.status {
                    debug!(?new_status, "status changed");
                    let mut updated = entry.clone();
                    updated.status = new_status.clone();
                    ops.put_chunked_envelope_entry_async(curr, updated).await?;
                }
                if new_status == ChunkedEnvelopeStatus::Finalized {
                    curr += 1;
                }
            }
        }
    }
}

/// Checks the broadcast database for commit + all reveal tx statuses and
/// returns the aggregate. The least-progressed transaction determines the
/// overall envelope status.
async fn check_broadcast_status(
    entry: &ChunkedEnvelopeEntry,
    bcast: &L1BroadcastHandle,
) -> anyhow::Result<ChunkedEnvelopeStatus> {
    let Some(commit) = bcast.get_tx_entry_by_id_async(entry.commit_txid).await? else {
        warn!("commit tx missing from broadcast db, will re-sign");
        return Ok(ChunkedEnvelopeStatus::Unsigned);
    };
    if commit.status == L1TxStatus::InvalidInputs {
        return Ok(ChunkedEnvelopeStatus::NeedsResign);
    }

    let mut min_progress = commit.status;
    for reveal in &entry.reveals {
        let Some(rtx) = bcast.get_tx_entry_by_id_async(reveal.txid).await? else {
            warn!(txid = %reveal.txid, "reveal tx missing, will re-sign");
            return Ok(ChunkedEnvelopeStatus::Unsigned);
        };
        if rtx.status == L1TxStatus::InvalidInputs {
            return Ok(ChunkedEnvelopeStatus::NeedsResign);
        }
        if is_less_progressed(&rtx.status, &min_progress) {
            min_progress = rtx.status;
        }
    }

    Ok(to_envelope_status(&min_progress))
}

/// Returns a progress ordinal for comparing [`L1TxStatus`] values.
///
/// Only used for ordering — never converted back into an enum.
/// `InvalidInputs` is excluded because the caller handles it via early return.
fn progress_ordinal(s: &L1TxStatus) -> u8 {
    match s {
        L1TxStatus::Unpublished => 0,
        L1TxStatus::Published => 1,
        L1TxStatus::Confirmed { .. } => 2,
        L1TxStatus::Finalized { .. } => 3,
        L1TxStatus::InvalidInputs => {
            unreachable!("InvalidInputs is handled before aggregation")
        }
    }
}

/// Returns `true` if `a` has made less broadcast progress than `b`.
fn is_less_progressed(a: &L1TxStatus, b: &L1TxStatus) -> bool {
    progress_ordinal(a) < progress_ordinal(b)
}

/// Maps a broadcast-layer [`L1TxStatus`] to the corresponding [`ChunkedEnvelopeStatus`].
///
/// `InvalidInputs` is excluded — the caller must handle it separately since it
/// maps to [`ChunkedEnvelopeStatus::NeedsResign`], which has no `L1TxStatus`
/// counterpart.
fn to_envelope_status(s: &L1TxStatus) -> ChunkedEnvelopeStatus {
    match s {
        L1TxStatus::Unpublished => ChunkedEnvelopeStatus::Unpublished,
        L1TxStatus::Published => ChunkedEnvelopeStatus::Published,
        L1TxStatus::Confirmed { .. } => ChunkedEnvelopeStatus::Confirmed,
        L1TxStatus::Finalized { .. } => ChunkedEnvelopeStatus::Finalized,
        L1TxStatus::InvalidInputs => {
            unreachable!("InvalidInputs is handled before aggregation")
        }
    }
}

#[cfg(test)]
mod tests {
    use strata_primitives::buf::Buf32;

    use super::*;
    use crate::writer::test_utils::get_chunked_envelope_ops;

    #[test]
    fn test_find_first_pending_idx_empty() {
        let ops = get_chunked_envelope_ops();
        assert_eq!(ops.get_next_chunked_envelope_idx_blocking().unwrap(), 0);
        assert_eq!(find_first_pending_idx(&ops).unwrap(), 0);
    }

    #[test]
    fn test_find_first_pending_idx_with_entries() {
        let ops = get_chunked_envelope_ops();

        let mut e0 = ChunkedEnvelopeEntry::new_unsigned(
            vec![vec![0x01; 50]],
            [0xAA, 0xBB, 0xCC, 0xDD],
            Buf32::zero(),
        );
        e0.status = ChunkedEnvelopeStatus::Finalized;
        ops.put_chunked_envelope_entry_blocking(0, e0).unwrap();

        let mut e1 = ChunkedEnvelopeEntry::new_unsigned(
            vec![vec![0x02; 50]],
            [0xAA, 0xBB, 0xCC, 0xDD],
            Buf32::zero(),
        );
        e1.status = ChunkedEnvelopeStatus::Published;
        ops.put_chunked_envelope_entry_blocking(1, e1).unwrap();

        let mut e2 = ChunkedEnvelopeEntry::new_unsigned(
            vec![vec![0x03; 50]],
            [0xAA, 0xBB, 0xCC, 0xDD],
            Buf32::zero(),
        );
        e2.status = ChunkedEnvelopeStatus::Unsigned;
        ops.put_chunked_envelope_entry_blocking(2, e2).unwrap();

        let mut e3 = ChunkedEnvelopeEntry::new_unsigned(
            vec![vec![0x04; 50]],
            [0xAA, 0xBB, 0xCC, 0xDD],
            Buf32::zero(),
        );
        e3.status = ChunkedEnvelopeStatus::Unsigned;
        ops.put_chunked_envelope_entry_blocking(3, e3).unwrap();

        // e0 is Finalized, so the first non-finalized is e1 at index 1.
        let idx = find_first_pending_idx(&ops).unwrap();
        assert_eq!(idx, 1);
    }

    #[test]
    fn test_progress_ordering_is_monotonic() {
        let statuses = [
            L1TxStatus::Unpublished,
            L1TxStatus::Published,
            L1TxStatus::Confirmed {
                confirmations: 1,
                block_hash: Buf32::zero(),
                block_height: 100,
            },
            L1TxStatus::Finalized {
                confirmations: 6,
                block_hash: Buf32::zero(),
                block_height: 100,
            },
        ];
        for window in statuses.windows(2) {
            assert!(
                is_less_progressed(&window[0], &window[1]),
                "{:?} should be less progressed than {:?}",
                window[0],
                window[1]
            );
        }
    }

    #[test]
    fn test_to_envelope_status_mapping() {
        assert_eq!(
            to_envelope_status(&L1TxStatus::Unpublished),
            ChunkedEnvelopeStatus::Unpublished,
        );
        assert_eq!(
            to_envelope_status(&L1TxStatus::Published),
            ChunkedEnvelopeStatus::Published,
        );
        assert_eq!(
            to_envelope_status(&L1TxStatus::Confirmed {
                confirmations: 3,
                block_hash: Buf32::zero(),
                block_height: 100,
            }),
            ChunkedEnvelopeStatus::Confirmed,
        );
        assert_eq!(
            to_envelope_status(&L1TxStatus::Finalized {
                confirmations: 6,
                block_hash: Buf32::zero(),
                block_height: 100,
            }),
            ChunkedEnvelopeStatus::Finalized,
        );
    }

    #[test]
    fn test_least_progressed_determines_aggregate() {
        // All unpublished → Unpublished.
        assert_eq!(
            to_envelope_status(&L1TxStatus::Unpublished),
            ChunkedEnvelopeStatus::Unpublished,
        );

        // All finalized → Finalized.
        assert_eq!(
            to_envelope_status(&L1TxStatus::Finalized {
                confirmations: 6,
                block_hash: Buf32::zero(),
                block_height: 100,
            }),
            ChunkedEnvelopeStatus::Finalized,
        );

        // One published, rest confirmed → published is least progressed.
        assert!(is_less_progressed(
            &L1TxStatus::Published,
            &L1TxStatus::Confirmed {
                confirmations: 3,
                block_hash: Buf32::zero(),
                block_height: 100,
            },
        ));
        assert_eq!(
            to_envelope_status(&L1TxStatus::Published),
            ChunkedEnvelopeStatus::Published,
        );
    }
}
