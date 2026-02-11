//! Chunked envelope handle and lifecycle driver.
//!
//! Polls entries by sequential index and advances them through the status
//! lifecycle: Unsigned → Unpublished → CommitPublished → Published → Confirmed → Finalized.
//!
//! The `CommitPublished` intermediate state ensures reveal txs are broadcast directly
//! (all at once) when the commit tx is on-chain. For single-reveal entries, broadcast
//! happens as soon as the commit is in the mempool. For multi-reveal entries, we wait
//! for the commit to be confirmed in a block to avoid hitting Bitcoin Core's mempool
//! descendant size limit (default 101 KB).

use std::{future::Future, sync::Arc, time::Duration};

use bitcoin::{consensus::encode::deserialize as btc_deserialize, Address, Transaction};
use bitcoind_async_client::{
    traits::{Broadcaster, Reader, Signer, Wallet},
    Client,
};
use strata_config::btcio::WriterConfig;
use strata_db_types::types::{ChunkedEnvelopeEntry, ChunkedEnvelopeStatus, L1TxEntry, L1TxStatus};
use strata_primitives::buf::Buf32;
use strata_storage::ops::chunked_envelope::ChunkedEnvelopeOps;
use tokio::time::interval;
use tracing::*;

use super::{context::ChunkedWriterContext, signer::sign_chunked_envelope};
use crate::{broadcaster::L1BroadcastHandle, writer::builder::EnvelopeError, BtcioParams};

/// Handle for submitting chunked envelope entries.
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have debug impls"
)]
pub struct ChunkedEnvelopeHandle {
    ops: Arc<ChunkedEnvelopeOps>,
}

impl ChunkedEnvelopeHandle {
    pub fn new(ops: Arc<ChunkedEnvelopeOps>) -> Self {
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

/// Creates the chunked envelope lifecycle driver.
///
/// Returns a `(handle, future)` pair. The caller is responsible for spawning the
/// future on whatever executor it uses (e.g. alpen ee `task_executor`).
pub fn create_chunked_envelope_task(
    bitcoin_client: Arc<Client>,
    config: Arc<WriterConfig>,
    btcio_params: BtcioParams,
    sequencer_address: Address,
    ops: Arc<ChunkedEnvelopeOps>,
    broadcast_handle: Arc<L1BroadcastHandle>,
) -> anyhow::Result<(Arc<ChunkedEnvelopeHandle>, impl Future<Output = ()>)> {
    let start_idx = find_first_pending_idx(ops.as_ref())?;
    let handle = Arc::new(ChunkedEnvelopeHandle::new(ops.clone()));

    let ctx = Arc::new(ChunkedWriterContext::new(
        btcio_params,
        config,
        sequencer_address,
        bitcoin_client,
    ));

    let task = async move {
        if let Err(e) = watcher_task(start_idx, ctx, ops, broadcast_handle).await {
            error!(%e, "chunked envelope watcher exited with error");
        }
    };

    Ok((handle, task))
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
///
/// The lifecycle is:
/// 1. `Unsigned`/`NeedsResign` → sign commit+reveals, store commit in broadcast DB → `Unpublished`
/// 2. `Unpublished` → wait for commit to be on-chain, then broadcast ALL reveals →
///    `CommitPublished`
/// 3. `CommitPublished` → wait for all reveals to be published → `Published`
/// 4. `Published` → wait for confirmation → `Confirmed`
/// 5. `Confirmed` → wait for finalization → `Finalized`
async fn watcher_task<R: Reader + Signer + Wallet + Broadcaster>(
    start_idx: u64,
    ctx: Arc<ChunkedWriterContext<R>>,
    ops: Arc<ChunkedEnvelopeOps>,
    broadcast_handle: Arc<L1BroadcastHandle>,
) -> anyhow::Result<()> {
    info!("starting chunked envelope watcher");
    let tick = interval(Duration::from_millis(ctx.config.write_poll_dur_ms));
    tokio::pin!(tick);

    let mut curr = start_idx;
    loop {
        tick.as_mut().tick().await;
        let span_curr = curr;

        async {
            let Some(entry) = ops.get_chunked_envelope_entry_async(curr).await? else {
                trace!("no entry at current index, waiting");
                return Ok::<(), anyhow::Error>(());
            };

            match entry.status {
                ChunkedEnvelopeStatus::Unsigned | ChunkedEnvelopeStatus::NeedsResign => {
                    debug!(status = ?entry.status, "entry needs signing");

                    let prev_tail_wtxid = resolve_prev_tail_wtxid(curr, &ops).await?;

                    match sign_chunked_envelope(
                        &entry,
                        prev_tail_wtxid,
                        &broadcast_handle,
                        ctx.clone(),
                    )
                    .await
                    {
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

                ChunkedEnvelopeStatus::Unpublished => {
                    // Check if commit is published. If so, broadcast ALL reveals together.
                    let new_status = check_commit_and_broadcast_reveals(
                        &entry,
                        &broadcast_handle,
                        ctx.client.as_ref(),
                    )
                    .await?;
                    if new_status != entry.status {
                        debug!(?new_status, "status changed");
                        let mut updated = entry.clone();
                        updated.status = new_status.clone();
                        ops.put_chunked_envelope_entry_async(curr, updated).await?;
                    }
                }

                ChunkedEnvelopeStatus::CommitPublished
                | ChunkedEnvelopeStatus::Published
                | ChunkedEnvelopeStatus::Confirmed => {
                    // Reveals are in broadcast DB, check their status.
                    let new_status = check_full_broadcast_status(&entry, &broadcast_handle).await?;
                    if new_status != entry.status {
                        debug!(?new_status, "status changed");
                        let mut updated = entry;
                        updated.status = new_status.clone();
                        ops.put_chunked_envelope_entry_async(curr, updated).await?;
                    }
                    if new_status == ChunkedEnvelopeStatus::Finalized {
                        curr += 1;
                    }
                }
            }

            Ok(())
        }
        .instrument(debug_span!("chunked_envelope", curr = %span_curr))
        .await?;
    }
}

/// Resolves the correct `prev_tail_wtxid` for the entry at index `curr`.
///
/// For index 0, returns [`Buf32::zero`] (first entry in chain). For all others,
/// reads entry[curr-1] from the DB and returns its
/// [`tail_wtxid()`](ChunkedEnvelopeEntry::tail_wtxid). The watcher's sequential
/// processing guarantees entry[curr-1] is signed, so `tail_wtxid()` returns the
/// actual last-reveal wtxid rather than the stale fallback.
async fn resolve_prev_tail_wtxid(curr: u64, ops: &ChunkedEnvelopeOps) -> anyhow::Result<Buf32> {
    if curr == 0 {
        return Ok(Buf32::zero());
    }
    Ok(ops
        .get_chunked_envelope_entry_async(curr - 1)
        .await?
        .map(|e| e.tail_wtxid())
        .unwrap_or(Buf32::zero()))
}

/// Checks commit tx status and broadcasts reveals once it is safe to do so.
///
/// Called when status is `Unpublished`. Returns:
/// - `CommitPublished` if commit is on-chain and reveals are broadcast and stored in DB
/// - `NeedsResign` if commit has invalid inputs or any reveal fails to broadcast
/// - `Unsigned` if commit is missing
/// - `Unpublished` if commit is still waiting
///
/// For single-reveal entries the reveal is broadcast as soon as the commit is
/// published (in mempool) — one reveal's ~99 KB vsize fits within Bitcoin
/// Core's default 101 KB descendant-size limit.
///
/// For multi-reveal entries we wait until the commit is **confirmed** (in a
/// block) before broadcasting. Multiple large reveals would otherwise exceed
/// the descendant-size limit and be rejected by the mempool.
async fn check_commit_and_broadcast_reveals(
    entry: &ChunkedEnvelopeEntry,
    bcast: &L1BroadcastHandle,
    client: &impl Broadcaster,
) -> anyhow::Result<ChunkedEnvelopeStatus> {
    let Some(commit) = bcast.get_tx_entry_by_id_async(entry.commit_txid).await? else {
        warn!("commit tx missing from broadcast db, will re-sign");
        return Ok(ChunkedEnvelopeStatus::Unsigned);
    };

    // A single reveal fits within the default 101 KB mempool descendant-size
    // limit, so it can be broadcast as soon as the commit is in the mempool.
    // Multiple reveals would exceed that limit, so we wait for the commit to
    // be confirmed in a block first.
    let needs_commit_confirmed = entry.reveals.len() > 1;

    let ready = match commit.status {
        L1TxStatus::InvalidInputs => return Ok(ChunkedEnvelopeStatus::NeedsResign),
        L1TxStatus::Unpublished => false,
        L1TxStatus::Published => !needs_commit_confirmed,
        L1TxStatus::Confirmed { .. } | L1TxStatus::Finalized { .. } => true,
    };

    if !ready {
        return Ok(ChunkedEnvelopeStatus::Unpublished);
    }

    info!(
        commit_txid = %entry.commit_txid,
        reveal_count = entry.reveals.len(),
        "commit on-chain, broadcasting all reveals"
    );

    // Deserialize all reveal transactions.
    let mut reveal_txs = Vec::with_capacity(entry.reveals.len());
    for reveal in &entry.reveals {
        let tx: Transaction = btc_deserialize(&reveal.tx_bytes)
            .map_err(|e| anyhow::anyhow!("failed to deserialize reveal tx: {}", e))?;
        reveal_txs.push((reveal.txid, tx));
    }

    // Broadcast all reveals.
    for (txid, tx) in &reveal_txs {
        match client.send_raw_transaction(tx).await {
            Ok(_) => {
                debug!(%txid, "reveal tx broadcast successfully");
            }
            Err(e) if e.is_missing_or_invalid_input() => {
                warn!(%txid, ?e, "reveal tx has invalid inputs, will re-sign");
                return Ok(ChunkedEnvelopeStatus::NeedsResign);
            }
            Err(e) => {
                // Could be "already in mempool" which is fine, or a network error.
                // We'll verify actual status on the next poll.
                warn!(%txid, ?e, "broadcast returned error (may already be in mempool)");
            }
        }
    }

    info!(
        commit_txid = %entry.commit_txid,
        reveal_count = entry.reveals.len(),
        "completed reveal broadcast attempt"
    );

    // Store all reveals in broadcast DB for tracking.
    for (txid, tx) in reveal_txs {
        let mut tx_entry = L1TxEntry::from_tx(&tx);
        tx_entry.status = L1TxStatus::Published;
        bcast
            .put_tx_entry(txid, tx_entry)
            .await
            .map_err(|e| anyhow::anyhow!("failed to store reveal tx: {}", e))?;
    }

    Ok(ChunkedEnvelopeStatus::CommitPublished)
}

/// Checks broadcast status of commit + all reveals (after reveals are in broadcast DB).
///
/// Called when status is `CommitPublished`, `Published`, or `Confirmed`.
/// The least-progressed transaction determines the overall envelope status.
async fn check_full_broadcast_status(
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
            warn!(txid = %reveal.txid, "reveal tx missing from broadcast db, will re-sign");
            return Ok(ChunkedEnvelopeStatus::Unsigned);
        };
        if rtx.status == L1TxStatus::InvalidInputs {
            // This shouldn't happen if we waited for commit to be published first,
            // but handle it gracefully by re-signing.
            warn!(txid = %reveal.txid, "reveal has InvalidInputs despite commit being published");
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
/// Called after reveals are in broadcast DB (from `CommitPublished` state onwards).
/// Returns `CommitPublished` for `Unpublished` to avoid regressing the envelope status.
/// `InvalidInputs` is excluded — the caller must handle it separately since it
/// maps to [`ChunkedEnvelopeStatus::NeedsResign`], which has no `L1TxStatus`
/// counterpart.
fn to_envelope_status(s: &L1TxStatus) -> ChunkedEnvelopeStatus {
    match s {
        // Reveals may still be unpublished even though they're in broadcast DB.
        // Stay at CommitPublished until all are Published.
        L1TxStatus::Unpublished => ChunkedEnvelopeStatus::CommitPublished,
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
    use bitcoin::{
        absolute::LockTime, consensus::encode::serialize as btc_serialize, hashes::Hash,
        transaction::Version, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut,
        Witness,
    };
    use strata_db_types::types::RevealTxMeta;
    use strata_l1_txfmt::MagicBytes;
    use strata_primitives::buf::Buf32;

    use super::*;
    use crate::{
        test_utils::{SendRawTransactionMode, TestBitcoinClient},
        writer::test_utils::{get_broadcast_handle, get_chunked_envelope_ops},
    };

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
            MagicBytes::new([0xAA, 0xBB, 0xCC, 0xDD]),
        );
        e0.status = ChunkedEnvelopeStatus::Finalized;
        ops.put_chunked_envelope_entry_blocking(0, e0).unwrap();

        let mut e1 = ChunkedEnvelopeEntry::new_unsigned(
            vec![vec![0x02; 50]],
            MagicBytes::new([0xAA, 0xBB, 0xCC, 0xDD]),
        );
        e1.status = ChunkedEnvelopeStatus::Published;
        ops.put_chunked_envelope_entry_blocking(1, e1).unwrap();

        let mut e2 = ChunkedEnvelopeEntry::new_unsigned(
            vec![vec![0x03; 50]],
            MagicBytes::new([0xAA, 0xBB, 0xCC, 0xDD]),
        );
        e2.status = ChunkedEnvelopeStatus::Unsigned;
        ops.put_chunked_envelope_entry_blocking(2, e2).unwrap();

        let mut e3 = ChunkedEnvelopeEntry::new_unsigned(
            vec![vec![0x04; 50]],
            MagicBytes::new([0xAA, 0xBB, 0xCC, 0xDD]),
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
        // Unpublished maps to CommitPublished (to avoid regressing the envelope status
        // after reveals are stored in broadcast DB).
        assert_eq!(
            to_envelope_status(&L1TxStatus::Unpublished),
            ChunkedEnvelopeStatus::CommitPublished,
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
        // All unpublished → CommitPublished (waiting for reveals to be published).
        assert_eq!(
            to_envelope_status(&L1TxStatus::Unpublished),
            ChunkedEnvelopeStatus::CommitPublished,
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

    // Async state-machine tests for commit/reveal status transitions.

    /// Creates a minimal valid transaction for test database entries.
    fn make_test_tx() -> Transaction {
        Transaction {
            version: Version(2),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint {
                    txid: bitcoin::Txid::all_zeros(),
                    vout: 0,
                },
                script_sig: ScriptBuf::new(),
                witness: Witness::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            }],
            output: vec![TxOut {
                value: Amount::from_sat(1000),
                script_pubkey: ScriptBuf::new(),
            }],
        }
    }

    /// Creates a test entry with N reveal transactions containing valid tx_bytes.
    fn make_entry_with_reveals(n: usize) -> ChunkedEnvelopeEntry {
        let mut entry = ChunkedEnvelopeEntry::new_unsigned(
            vec![vec![0xAA; 100]; n],
            MagicBytes::new([0x01, 0x02, 0x03, 0x04]),
        );
        entry.commit_txid = Buf32::from([0x11; 32]);
        entry.reveals = (0..n)
            .map(|i| {
                let tx = make_test_tx();
                RevealTxMeta {
                    vout_index: i as u32,
                    txid: Buf32::from([(0x20 + i as u8); 32]),
                    wtxid: Buf32::from([(0x30 + i as u8); 32]),
                    tx_bytes: btc_serialize(&tx),
                }
            })
            .collect();
        entry.status = ChunkedEnvelopeStatus::Unpublished;
        entry
    }

    #[tokio::test]
    async fn test_check_commit_unpublished_stays_waiting() {
        let bcast = get_broadcast_handle();
        let client = TestBitcoinClient::new(1);
        let entry = make_entry_with_reveals(2);

        // Store commit with Unpublished status — reveals should NOT be broadcast.
        let commit_entry = L1TxEntry::from_tx(&make_test_tx());
        bcast
            .put_tx_entry(entry.commit_txid, commit_entry)
            .await
            .unwrap();

        let result = check_commit_and_broadcast_reveals(&entry, &bcast, &client)
            .await
            .unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::Unpublished,
            "should stay Unpublished while commit is not yet published"
        );

        // Ensure reveals are not inserted in broadcast DB before commit is published.
        for reveal in &entry.reveals {
            let rtx = bcast.get_tx_entry_by_id_async(reveal.txid).await.unwrap();
            assert!(
                rtx.is_none(),
                "reveal should not be stored before commit publish"
            );
        }
    }

    #[tokio::test]
    async fn test_check_commit_missing_returns_unsigned() {
        let bcast = get_broadcast_handle();
        let client = TestBitcoinClient::new(1);
        let entry = make_entry_with_reveals(2);

        // Don't store commit at all — should return Unsigned for re-signing.
        let result = check_commit_and_broadcast_reveals(&entry, &bcast, &client)
            .await
            .unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::Unsigned,
            "missing commit should trigger re-sign"
        );
    }

    #[tokio::test]
    async fn test_check_commit_invalid_inputs_returns_needs_resign() {
        let bcast = get_broadcast_handle();
        let client = TestBitcoinClient::new(1);
        let entry = make_entry_with_reveals(2);

        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = L1TxStatus::InvalidInputs;
        bcast
            .put_tx_entry(entry.commit_txid, commit_entry)
            .await
            .unwrap();

        let result = check_commit_and_broadcast_reveals(&entry, &bcast, &client)
            .await
            .unwrap();
        assert_eq!(result, ChunkedEnvelopeStatus::NeedsResign);
    }

    #[tokio::test]
    async fn test_check_commit_published_broadcasts_reveals() {
        let bcast = get_broadcast_handle();
        let client = TestBitcoinClient::new(1);
        let entry = make_entry_with_reveals(3);

        // Store commit as Published.
        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = L1TxStatus::Published;
        bcast
            .put_tx_entry(entry.commit_txid, commit_entry)
            .await
            .unwrap();

        let result = check_commit_and_broadcast_reveals(&entry, &bcast, &client)
            .await
            .unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::CommitPublished,
            "should broadcast reveals and transition to CommitPublished"
        );

        // Verify all reveals were stored in broadcast DB with Published status.
        for reveal in &entry.reveals {
            let rtx = bcast
                .get_tx_entry_by_id_async(reveal.txid)
                .await
                .unwrap()
                .expect("reveal should be in broadcast DB");
            assert_eq!(
                rtx.status,
                L1TxStatus::Published,
                "reveal should be marked Published"
            );
        }
    }

    #[tokio::test]
    async fn test_check_commit_broadcast_missing_input_returns_needs_resign() {
        let bcast = get_broadcast_handle();
        let client = TestBitcoinClient::new(1)
            .with_send_raw_transaction_mode(SendRawTransactionMode::MissingOrInvalidInput);
        let entry = make_entry_with_reveals(2);

        // Store commit as Published.
        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = L1TxStatus::Published;
        bcast
            .put_tx_entry(entry.commit_txid, commit_entry)
            .await
            .unwrap();

        let result = check_commit_and_broadcast_reveals(&entry, &bcast, &client)
            .await
            .unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::NeedsResign,
            "missing/invalid input during reveal broadcast should trigger re-sign"
        );
    }

    #[tokio::test]
    async fn test_check_commit_broadcast_generic_error_keeps_commit_published_state() {
        let bcast = get_broadcast_handle();
        let client = TestBitcoinClient::new(1)
            .with_send_raw_transaction_mode(SendRawTransactionMode::GenericError);
        let entry = make_entry_with_reveals(2);

        // Store commit as Published.
        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = L1TxStatus::Published;
        bcast
            .put_tx_entry(entry.commit_txid, commit_entry)
            .await
            .unwrap();

        let result = check_commit_and_broadcast_reveals(&entry, &bcast, &client)
            .await
            .unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::CommitPublished,
            "generic reveal broadcast errors should still move to CommitPublished"
        );

        // Reveals are inserted for tracking even when RPC returned generic broadcast errors.
        for reveal in &entry.reveals {
            let rtx = bcast
                .get_tx_entry_by_id_async(reveal.txid)
                .await
                .unwrap()
                .expect("reveal should be in broadcast DB");
            assert_eq!(rtx.status, L1TxStatus::Published);
        }
    }

    #[tokio::test]
    async fn test_full_status_all_finalized() {
        let bcast = get_broadcast_handle();
        let entry = make_entry_with_reveals(2);

        let finalized = L1TxStatus::Finalized {
            confirmations: 6,
            block_hash: Buf32::from([0xAA; 32]),
            block_height: 100,
        };

        // Store commit as Finalized.
        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = finalized.clone();
        bcast
            .put_tx_entry(entry.commit_txid, commit_entry)
            .await
            .unwrap();

        // Store all reveals as Finalized.
        for reveal in &entry.reveals {
            let mut rtx = L1TxEntry::from_tx(&make_test_tx());
            rtx.status = finalized.clone();
            bcast.put_tx_entry(reveal.txid, rtx).await.unwrap();
        }

        let result = check_full_broadcast_status(&entry, &bcast).await.unwrap();
        assert_eq!(result, ChunkedEnvelopeStatus::Finalized);
    }

    #[tokio::test]
    async fn test_full_status_least_progressed_wins() {
        let bcast = get_broadcast_handle();
        let entry = make_entry_with_reveals(3);

        let confirmed = L1TxStatus::Confirmed {
            confirmations: 3,
            block_hash: Buf32::from([0xBB; 32]),
            block_height: 100,
        };

        // Commit is Confirmed.
        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = confirmed.clone();
        bcast
            .put_tx_entry(entry.commit_txid, commit_entry)
            .await
            .unwrap();

        // Reveal 0: Confirmed.
        let mut r0 = L1TxEntry::from_tx(&make_test_tx());
        r0.status = confirmed.clone();
        bcast.put_tx_entry(entry.reveals[0].txid, r0).await.unwrap();

        // Reveal 1: Published (least progressed).
        let mut r1 = L1TxEntry::from_tx(&make_test_tx());
        r1.status = L1TxStatus::Published;
        bcast.put_tx_entry(entry.reveals[1].txid, r1).await.unwrap();

        // Reveal 2: Confirmed.
        let mut r2 = L1TxEntry::from_tx(&make_test_tx());
        r2.status = confirmed;
        bcast.put_tx_entry(entry.reveals[2].txid, r2).await.unwrap();

        let result = check_full_broadcast_status(&entry, &bcast).await.unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::Published,
            "least progressed (Published) should determine overall status"
        );
    }

    #[tokio::test]
    async fn test_full_status_commit_missing_returns_unsigned() {
        let bcast = get_broadcast_handle();
        let entry = make_entry_with_reveals(2);

        let result = check_full_broadcast_status(&entry, &bcast).await.unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::Unsigned,
            "missing commit should trigger re-sign"
        );
    }

    #[tokio::test]
    async fn test_full_status_reveal_missing_returns_unsigned() {
        let bcast = get_broadcast_handle();
        let entry = make_entry_with_reveals(2);

        // Store commit.
        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = L1TxStatus::Published;
        bcast
            .put_tx_entry(entry.commit_txid, commit_entry)
            .await
            .unwrap();

        // Store only first reveal.
        let mut r0 = L1TxEntry::from_tx(&make_test_tx());
        r0.status = L1TxStatus::Published;
        bcast.put_tx_entry(entry.reveals[0].txid, r0).await.unwrap();

        // Second reveal is missing.
        let result = check_full_broadcast_status(&entry, &bcast).await.unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::Unsigned,
            "missing reveal should trigger re-sign"
        );
    }

    #[tokio::test]
    async fn test_full_status_reveal_invalid_inputs_returns_needs_resign() {
        let bcast = get_broadcast_handle();
        let entry = make_entry_with_reveals(2);

        // Store commit as Published.
        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = L1TxStatus::Published;
        bcast
            .put_tx_entry(entry.commit_txid, commit_entry)
            .await
            .unwrap();

        // Reveal 0 is fine.
        let mut r0 = L1TxEntry::from_tx(&make_test_tx());
        r0.status = L1TxStatus::Published;
        bcast.put_tx_entry(entry.reveals[0].txid, r0).await.unwrap();

        // Reveal 1 has invalid inputs.
        let mut r1 = L1TxEntry::from_tx(&make_test_tx());
        r1.status = L1TxStatus::InvalidInputs;
        bcast.put_tx_entry(entry.reveals[1].txid, r1).await.unwrap();

        let result = check_full_broadcast_status(&entry, &bcast).await.unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::NeedsResign,
            "InvalidInputs on any reveal should trigger re-sign"
        );
    }

    #[tokio::test]
    async fn test_full_status_unpublished_maps_to_commit_published() {
        let bcast = get_broadcast_handle();
        let entry = make_entry_with_reveals(2);

        // Commit is Published.
        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = L1TxStatus::Published;
        bcast
            .put_tx_entry(entry.commit_txid, commit_entry)
            .await
            .unwrap();

        // Reveals are Unpublished (stored in DB but not yet in mempool).
        for reveal in &entry.reveals {
            let rtx = L1TxEntry::from_tx(&make_test_tx());
            // from_tx creates with Unpublished status by default
            bcast.put_tx_entry(reveal.txid, rtx).await.unwrap();
        }

        let result = check_full_broadcast_status(&entry, &bcast).await.unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::CommitPublished,
            "Unpublished L1TxStatus should map to CommitPublished to avoid status regression"
        );
    }

    #[tokio::test]
    async fn test_resolve_prev_tail_wtxid_from_signed_predecessor() {
        let ops = get_chunked_envelope_ops();

        // Entry[0]: simulate signed state (reveals populated with known wtxid).
        let mut e0 = ChunkedEnvelopeEntry::new_unsigned(
            vec![vec![0x01; 50]],
            MagicBytes::new([0xAA, 0xBB, 0xCC, 0xDD]),
        );
        let real_tail = Buf32::from([0x42; 32]);
        e0.reveals = vec![RevealTxMeta {
            vout_index: 0,
            txid: Buf32::from([0x11; 32]),
            wtxid: real_tail,
            tx_bytes: vec![0xDE, 0xAD],
        }];
        e0.status = ChunkedEnvelopeStatus::Unpublished;
        ops.put_chunked_envelope_entry_async(0, e0).await.unwrap();

        // Entry[1]: prev_tail_wtxid is zero at creation (deferred to signing).
        let e1 = ChunkedEnvelopeEntry::new_unsigned(
            vec![vec![0x02; 50]],
            MagicBytes::new([0xAA, 0xBB, 0xCC, 0xDD]),
        );
        ops.put_chunked_envelope_entry_async(1, e1).await.unwrap();

        // resolve_prev_tail_wtxid should return entry[0]'s real tail_wtxid.
        let resolved = resolve_prev_tail_wtxid(1, &ops).await.unwrap();
        assert_eq!(resolved, real_tail);

        // Index 0 should return zero (first in chain).
        let resolved_zero = resolve_prev_tail_wtxid(0, &ops).await.unwrap();
        assert_eq!(resolved_zero, Buf32::zero());
    }
}
