//! Drives RBF replacements for the transactions a writer owns.
//!
//! This is not a service. [`run_replacement_pass`] does one pass over the writer's tx-node records
//! and is called from the writer's existing watcher tick, so replacement runs at the writer's
//! cadence and inside its task. The broadcaster is what notices a transaction has gone stale.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{bail, Context};
use bitcoin::{
    consensus::{deserialize, serialize},
    hashes::Hash,
    key::Keypair,
    Amount, FeeRate, Transaction, TxOut,
};
use bitcoind_async_client::traits::{Reader, Signer};
use strata_config::btcio::{FeeBumpingConfig, WriterConfig};
use strata_db_types::{
    chunked_envelope::{ChunkedEnvelopeEntry, RevealTxMeta},
    common::{L1TxId, L1WtxId},
    fee_bump::{TerminalError, TxAttempt, TxAttemptStatus, TxNodeId, TxNodeKind, TxNodeRecord},
    l1_broadcast::{L1TxEntry, L1TxStatus},
    l1_writer::L1BundleStatus,
};
use strata_primitives::{buf::Buf32, L1Height};
use strata_storage::ops::{chunked_envelope::ChunkedEnvelopeOps, writer::EnvelopeDataOps};
use tracing::*;

use super::build::{
    build_chunked_reveal_replacement, build_pending_single_reveal_replacement,
    build_wallet_commit_replacement, chunked_commit_change_index, ensure_reveal_signable,
    extract_reveal_pubkey, rebuild_reveal_for_replaced_commit,
    validate_chunked_commit_replacement_layout, ReplacementError,
};
use crate::{
    broadcaster::{
        fee_bump::{evaluate_fee_bump, FeeBumpDecision, FeeBumpEvaluation},
        L1BroadcastHandle,
    },
    writer::{
        builder::BITCOIN_DUST_LIMIT, chunked_envelope::CommitPhaseLatch, fees::resolve_fee_rate,
        EnvelopeSigningMode, EnvelopeSigningModeProvider,
    },
};

/// Bitcoin Core's default `incrementalrelayfee`, used when the node does not report one.
const DEFAULT_INCREMENTAL_RELAY_FEE_RATE: FeeRate = FeeRate::from_sat_per_vb_u32(1);

/// Writer-supplied storage and signing handles the replacement path needs.
///
/// Each field is populated by the writer that owns the corresponding transaction kind: the
/// checkpoint writer fills the single-envelope fields, the EE-DA chunked writer fills the chunked
/// ones. A process runs one writer or the other, so the unused half stays `None`.
#[derive(Clone, Default)]
pub(crate) struct ReplacementContext {
    /// Chunked-envelope storage used by Alpen EE DA reveal replacements.
    pub chunked_ops: Option<Arc<ChunkedEnvelopeOps>>,
    /// Single-envelope payload storage used by checkpoint reveal replacements.
    pub envelope_ops: Option<Arc<EnvelopeDataOps>>,
    /// Sequencer keypair used to re-sign Alpen EE DA reveal replacements.
    pub sequencer_keypair: Option<Keypair>,
    /// Resolves whether single-envelope reveals can be re-signed by an external signer.
    ///
    /// Held as a provider rather than a resolved value because the signing mode tracks canonical
    /// ASM state and rotates at runtime. Resolving once at startup would let a rotation strand
    /// every reveal node behind a sticky [`TerminalError::UnsupportedRbfKind`].
    pub signing_mode_provider: Option<Arc<dyn EnvelopeSigningModeProvider>>,

    /// Serialises chunked commit replacement against reveal enqueueing.
    ///
    /// Must be the latch the chunked-envelope writer holds
    /// ([`ChunkedEnvelopeHandle::commit_phase_latch`]). The default is only correct when there is
    /// no chunked writer in this process, in which case no chunked commit is ever replaced.
    ///
    /// [`ChunkedEnvelopeHandle::commit_phase_latch`]: crate::writer::ChunkedEnvelopeHandle::commit_phase_latch
    pub commit_phase: CommitPhaseLatch,

    /// Paces the pass independently of the watcher tick that drives it.
    pub pacer: Arc<ReplacementPacer>,
}

/// Rate-limits the replacement pass.
///
/// The pass runs inside a writer's watcher tick, which is paced for payload processing:
/// `write_poll_dur_ms` is commonly 200. Rescanning every tx-node record and re-resolving the fee
/// estimate at that rate is pure waste, and under `fee_policy = "mempool"` it means an external
/// HTTP request several times a second. The work is driven by L1 blocks, so it only needs to
/// happen on the order of `check_interval_ms`.
#[derive(Debug)]
pub(crate) struct ReplacementPacer {
    interval: Duration,
    last_run: Mutex<Option<Instant>>,
}

impl ReplacementPacer {
    pub(crate) fn new(interval: Duration) -> Self {
        Self {
            interval,
            last_run: Mutex::new(None),
        }
    }

    /// Reports whether a pass is due, recording the run when it is.
    ///
    /// The first call always runs, so a freshly started writer does not wait an interval before
    /// looking at records a previous process left behind.
    fn claim(&self) -> bool {
        let mut last_run = self
            .last_run
            .lock()
            .expect("replacement pacer lock poisoned");
        let now = Instant::now();
        match *last_run {
            Some(previous) if now.duration_since(previous) < self.interval => false,
            _ => {
                *last_run = Some(now);
                true
            }
        }
    }
}

impl Default for ReplacementPacer {
    fn default() -> Self {
        Self::new(FeeBumpingConfig::default().check_interval())
    }
}

/// Runs one replacement pass over the writer's tx-node records.
///
/// Called from the writer's watcher tick. Errors are returned to the caller, which logs them and
/// carries on to the next tick; a failed pass is retried by the next one.
pub(crate) async fn run_replacement_pass<C>(
    client: &C,
    writer_config: &WriterConfig,
    broadcast_handle: &L1BroadcastHandle,
    context: &ReplacementContext,
) -> anyhow::Result<()>
where
    C: Reader + Signer,
{
    if !context.pacer.claim() {
        return Ok(());
    }

    // Read the records before any network call. On a writer with nothing in flight, or one whose
    // records have all reached a terminal state, there is nothing for the estimate to price and
    // the whole pass costs one local DB read.
    //
    // TODO(STR-4153): bound this by the active set. Records are never removed once their
    // transaction finalizes, so the scan loads and decodes every historical checkpoint and reveal
    // the node has ever published; `check_interval_ms` caps how often that runs, not how big it
    // gets. Needs an index keyed by node id that a record leaves once its active transaction is
    // confirmed, finalized or terminal — deleting a record outright is not safe while the watchers
    // still resolve progress through it.
    //
    // What is bounded already: a record's size no longer grows with its attempt count, because
    // superseded and discarded attempts drop their raw transaction bytes
    // (`TxAttempt::forget_raw_tx`). That matters most for chunked EE-DA reveals, whose witness
    // carries the chunk payload. The remaining growth is one live transaction per historical
    // record, which is what the index above removes.
    let records = broadcast_handle.get_all_tx_nodes().await?;
    if records.iter().all(|record| record.terminal_error.is_some()) {
        return Ok(());
    }

    let current_l1_tip = client.get_block_count().await? as L1Height;
    // Price the replacement through the same fee policy the original was built with. Calling
    // `estimatesmartfee` directly here would silently ignore a `fee_policy = "mempool"` node's
    // configured estimator and bump against a source it does not otherwise use.
    let estimate_fee_rate = resolve_fee_rate(client, writer_config).await?;
    // BIP-125 rule 4 is priced at the node's configured incremental relay fee, so read it rather
    // than assuming Core's 1 sat/vB default. Fall back to that default when the field is absent.
    let incremental_relay_fee_rate = client
        .get_mempool_info()
        .await?
        .incremental_relay_fee
        .unwrap_or(DEFAULT_INCREMENTAL_RELAY_FEE_RATE);

    for mut record in records {
        if record.terminal_error.is_some() {
            continue;
        }
        if record.pending_signature_attempt().is_some() {
            if pending_attempt_is_orphaned(context, &record).await? {
                warn!(node_id = ?record.node_id, "discarding a pending reveal replacement nothing can complete");
                let snapshot_active_txid = record.active_txid;
                record.discard_pending_signature_replacement();
                put_tx_node_if_active_unchanged(broadcast_handle, snapshot_active_txid, record)
                    .await?;
            } else {
                trace!(node_id = ?record.node_id, "tx-node is waiting for external signature");
            }
            continue;
        }
        // Per-record failures are logged and skipped rather than propagated. A record that fails
        // deterministically, an undecodable raw tx say, would otherwise abort the poll at the same
        // point every tick and starve every record ordered after it.
        let node_id = record.node_id;
        if let Err(error) = process_record(
            client,
            writer_config,
            broadcast_handle,
            ReplacementPolicyInputs {
                current_l1_tip,
                estimate_fee_rate,
                incremental_relay_fee_rate,
            },
            record,
            context,
        )
        .await
        {
            warn!(?node_id, %error, "skipping tx-node after fee-bump processing failed");
        }
    }

    Ok(())
}

/// Reports whether the row that owns a logical transaction still names its active attempt.
///
/// Used to gate re-inserting a broadcast entry the node has but the broadcaster does not. That
/// state has two causes with opposite remedies: a stop between the two writes, where re-inserting
/// is the recovery, and a writer rebuild that has already moved the row to a fresh transaction,
/// where re-inserting resurrects an envelope the writer abandoned.
///
/// Fails closed. Without the owning row, or the storage handle to read it, there is no way to tell
/// the two apart, and re-inserting is the destructive guess.
async fn active_attempt_is_still_owned(
    context: &ReplacementContext,
    record: &TxNodeRecord,
) -> anyhow::Result<bool> {
    match record.kind {
        TxNodeKind::SingleEnvelopeCommit { payload_idx }
        | TxNodeKind::SingleEnvelopeReveal { payload_idx } => {
            let Some(envelope_ops) = context.envelope_ops.as_ref() else {
                return Ok(false);
            };
            let Some(entry) = envelope_ops
                .get_payload_entry_by_idx_async(payload_idx)
                .await?
            else {
                return Ok(false);
            };
            let owned_txid = match record.kind {
                TxNodeKind::SingleEnvelopeCommit { .. } => entry.commit_txid,
                _ => entry.reveal_txid,
            };
            Ok(owned_txid == record.active_txid)
        }
        TxNodeKind::ChunkedEnvelopeCommit { envelope_idx } => {
            let Some(chunked_ops) = context.chunked_ops.as_ref() else {
                return Ok(false);
            };
            let Some(entry) = chunked_ops
                .get_chunked_envelope_entry_async(envelope_idx)
                .await?
            else {
                return Ok(false);
            };
            Ok(entry.commit_txid == record.active_txid)
        }
        TxNodeKind::ChunkedEnvelopeReveal {
            envelope_idx,
            reveal_idx,
        } => {
            let Some(chunked_ops) = context.chunked_ops.as_ref() else {
                return Ok(false);
            };
            let Some(entry) = chunked_ops
                .get_chunked_envelope_entry_async(envelope_idx)
                .await?
            else {
                return Ok(false);
            };
            Ok(entry
                .reveals
                .get(reveal_idx as usize)
                .is_some_and(|reveal| reveal.txid == record.active_txid))
        }
    }
}

/// Reports whether a node's pending-signature attempt can no longer be completed.
///
/// Only the watcher activates a pending replacement, and only while the payload row sits in
/// [`L1BundleStatus::PendingRevealTxSign`]. If the row never reached that state, because the
/// process stopped between persisting the node and updating the payload, or has already moved past
/// it, nothing will ever advance the attempt. Since the poll skips every node that carries one, the
/// chain would stop being bumped for good.
async fn pending_attempt_is_orphaned(
    context: &ReplacementContext,
    record: &TxNodeRecord,
) -> anyhow::Result<bool> {
    let TxNodeKind::SingleEnvelopeReveal { payload_idx } = record.kind else {
        return Ok(false);
    };
    let Some(envelope_ops) = context.envelope_ops.as_ref() else {
        return Ok(false);
    };
    let Some(payload_entry) = envelope_ops
        .get_payload_entry_by_idx_async(payload_idx)
        .await?
    else {
        return Ok(false);
    };
    Ok(!matches!(
        payload_entry.status,
        L1BundleStatus::PendingRevealTxSign(_)
    ))
}

async fn resolve_reveal_commit_output(
    broadcast_handle: &L1BroadcastHandle,
    context: &ReplacementContext,
    kind: &TxNodeKind,
) -> anyhow::Result<Option<TxOut>> {
    let commit_output = match kind {
        TxNodeKind::SingleEnvelopeReveal { payload_idx } => {
            let Some(envelope_ops) = context.envelope_ops.as_ref() else {
                return Ok(None);
            };
            let Some(payload_entry) = envelope_ops
                .get_payload_entry_by_idx_async(*payload_idx)
                .await?
            else {
                warn!(
                    payload_idx,
                    "payload entry missing while resolving single-envelope reveal funding"
                );
                return Ok(None);
            };
            let Some((_, commit_entry)) = broadcast_handle
                .get_active_tx_entry_by_id_async(to_raw_buf32(payload_entry.commit_txid))
                .await?
            else {
                debug!(
                    payload_idx,
                    "commit entry missing while resolving single-envelope reveal funding"
                );
                return Ok(None);
            };
            commit_entry.try_to_tx()?.output.first().cloned()
        }
        TxNodeKind::ChunkedEnvelopeReveal {
            envelope_idx,
            reveal_idx,
        } => {
            let Some(chunked_ops) = context.chunked_ops.as_ref() else {
                return Ok(None);
            };
            let Some(envelope_entry) = chunked_ops
                .get_chunked_envelope_entry_async(*envelope_idx)
                .await?
            else {
                warn!(
                    envelope_idx,
                    "chunked envelope entry missing while resolving reveal funding"
                );
                return Ok(None);
            };
            let Some(reveal_meta) = envelope_entry.reveals.get(*reveal_idx as usize) else {
                warn!(
                    envelope_idx,
                    reveal_idx, "chunked reveal metadata missing while resolving funding"
                );
                return Ok(None);
            };
            let Some((_, commit_entry)) = broadcast_handle
                .get_active_tx_entry_by_id_async(to_raw_buf32(envelope_entry.commit_txid))
                .await?
            else {
                debug!(
                    envelope_idx,
                    reveal_idx, "commit entry missing while resolving chunked reveal funding"
                );
                return Ok(None);
            };
            commit_entry
                .try_to_tx()?
                .output
                .get(reveal_meta.vout_index as usize)
                .cloned()
        }
        TxNodeKind::SingleEnvelopeCommit { .. } | TxNodeKind::ChunkedEnvelopeCommit { .. } => None,
    };

    Ok(commit_output)
}

async fn resolve_reveal_fee_budget(
    broadcast_handle: &L1BroadcastHandle,
    context: &ReplacementContext,
    kind: &TxNodeKind,
    active_reveal_tx: &Transaction,
) -> anyhow::Result<Option<Amount>> {
    let Some(commit_output) = resolve_reveal_commit_output(broadcast_handle, context, kind).await?
    else {
        return Ok(None);
    };

    Ok(Some(reveal_fee_budget(&commit_output, active_reveal_tx)))
}

fn reveal_fee_budget(commit_output: &TxOut, active_reveal_tx: &Transaction) -> Amount {
    let other_output_value = active_reveal_tx
        .output
        .iter()
        .take(active_reveal_tx.output.len().saturating_sub(1))
        .try_fold(Amount::ZERO, |total, output| {
            total.checked_add(output.value)
        });
    let Some(other_output_value) = other_output_value else {
        return Amount::ZERO;
    };
    commit_output
        .value
        .checked_sub(other_output_value)
        .and_then(|remaining| remaining.checked_sub(Amount::from_sat(BITCOIN_DUST_LIMIT)))
        .unwrap_or(Amount::ZERO)
}

#[derive(Debug, Clone, Copy)]
struct ReplacementPolicyInputs {
    current_l1_tip: L1Height,
    estimate_fee_rate: FeeRate,
    incremental_relay_fee_rate: FeeRate,
}

async fn process_record<C>(
    client: &C,
    writer_config: &WriterConfig,
    broadcast_handle: &L1BroadcastHandle,
    policy_inputs: ReplacementPolicyInputs,
    mut record: TxNodeRecord,
    context: &ReplacementContext,
) -> anyhow::Result<()>
where
    C: Reader + Signer,
{
    let ReplacementPolicyInputs {
        current_l1_tip,
        estimate_fee_rate,
        incremental_relay_fee_rate,
    } = policy_inputs;
    let active_txid = to_raw_buf32(record.active_txid);
    let Some(active_entry) = broadcast_handle
        .get_tx_entry_by_id_async(active_txid)
        .await?
    else {
        if matches!(
            record.active_attempt().map(|attempt| attempt.status),
            Some(TxAttemptStatus::PendingSignature)
        ) {
            trace!(node_id = ?record.node_id, active_txid = ?record.active_txid, "tx-node is waiting for external signature");
            return Ok(());
        }
        // A crash between persisting the tx-node record and inserting its broadcast entry
        // leaves the record pointing at a transaction the broadcaster never saw. The record
        // holds the raw tx, so re-insert it rather than stalling the chain forever.
        //
        // Only while the owning row still names this transaction, though. The writer reacts to the
        // same missing entry by rebuilding the whole envelope, and a re-insert racing that rebuild
        // hands the broadcaster a second, complete envelope for one payload: the original commit
        // entry was written before the node, so both would publish and the payload would land on
        // L1 twice.
        if !active_attempt_is_still_owned(context, &record).await? {
            debug!(node_id = ?record.node_id, active_txid = ?record.active_txid, "not re-inserting a broadcast entry the owning row has moved past");
            return Ok(());
        }
        let Some(active_attempt) = record.active_attempt() else {
            warn!(node_id = ?record.node_id, "tx-node record has no active attempt");
            return Ok(());
        };
        let active_tx = active_attempt.try_to_tx()?;
        let fee_rate = active_attempt
            .fee_rate()
            .unwrap_or_else(|| estimate_fee_rate.max(FeeRate::from_sat_per_vb_u32(1)));
        warn!(node_id = ?record.node_id, active_txid = ?record.active_txid, "re-inserting broadcast entry missing for active tx-node");
        broadcast_handle
            .put_tx_entry(
                active_txid,
                L1TxEntry::from_tx_with_fee(&active_tx, fee_rate, active_attempt.fee()),
            )
            .await?;
        return Ok(());
    };

    // A crash between marking the original `Replaced` and persisting the updated record leaves
    // the record pointing at a superseded entry, which would otherwise stall the chain forever.
    // Adopt the replacement the broadcast DB already knows about.
    if matches!(active_entry.status, L1TxStatus::Replaced { .. }) {
        let resolved_txid = broadcast_handle
            .get_active_tx_entry_by_id_async(active_txid)
            .await?
            .map(|(txid, _)| txid);

        match resolved_txid {
            Some(resolved_txid) if resolved_txid != active_txid => {
                let resolved = L1TxId::from(resolved_txid.0);
                warn!(node_id = ?record.node_id, ?resolved, "recovering tx-node stranded on a superseded attempt");

                // Snapshot before the rebuild below, which calls `append_replacement` and moves
                // `active_txid` onto the new attempt. Taken afterwards it would already equal
                // `resolved`, never match the durable record's superseded txid, and the write
                // that repairs the node would be skipped on every pass.
                let snapshot_active_txid = record.active_txid;

                if !record
                    .attempts
                    .iter()
                    .any(|attempt| attempt.txid == resolved)
                {
                    // The crash landed between marking the original replaced and appending the
                    // replacement, so the record has never seen it. Rebuild the attempt from the
                    // broadcast entry, which holds the raw tx and the fee rate it was built at.
                    let Some(resolved_entry) = broadcast_handle
                        .get_tx_entry_by_id_async(resolved_txid)
                        .await?
                    else {
                        warn!(node_id = ?record.node_id, ?resolved, "replacement entry vanished; leaving tx-node untouched");
                        return Ok(());
                    };
                    let resolved_tx = resolved_entry.try_to_tx()?;
                    // Read both from the replacement's own metadata. Copying the superseded
                    // attempt's fee would under-report it and make the next bump undershoot the
                    // BIP-125 absolute-fee floor.
                    let Some(rbf) = resolved_entry.rbf else {
                        warn!(node_id = ?record.node_id, ?resolved, "replacement entry has no fee metadata; leaving tx-node untouched");
                        return Ok(());
                    };
                    let fee_rate =
                        FeeRate::from_sat_per_vb(rbf.fee_rate_sat_vb).unwrap_or(estimate_fee_rate);
                    let fee = Amount::from_sat(rbf.fee_sats);
                    let attempt_no = record.next_attempt_no();
                    record.append_replacement(TxAttempt::active(
                        &resolved_tx,
                        fee_rate,
                        fee,
                        attempt_no,
                    ));
                }

                record.active_txid = resolved;
                put_tx_node_if_active_unchanged(broadcast_handle, snapshot_active_txid, record)
                    .await?;
            }
            _ => {
                warn!(node_id = ?record.node_id, "tx-node active attempt is replaced but its replacement is unreachable");
            }
        }
        return Ok(());
    }

    // A crash between persisting the node and refreshing the envelope row leaves the row pointing
    // at a superseded commit. The writer refuses to enqueue reveals in that state, so finish the
    // refresh here rather than leaving the envelope wedged.
    if let TxNodeKind::ChunkedEnvelopeCommit { envelope_idx } = record.kind {
        retry_stale_chunked_commit_metadata(broadcast_handle, context, &record, envelope_idx)
            .await?;
    }

    // The same partial write on the reveal side leaves the envelope row naming a superseded
    // reveal. Nothing else repairs it, and the DA provider reads that row directly.
    if let TxNodeKind::ChunkedEnvelopeReveal {
        envelope_idx,
        reveal_idx,
    } = record.kind
    {
        retry_stale_chunked_reveal_metadata(
            broadcast_handle,
            context,
            &record,
            envelope_idx,
            reveal_idx,
        )
        .await?;
    }

    if active_entry.status != L1TxStatus::Published {
        return Ok(());
    }

    if mark_first_published_height(&mut record, current_l1_tip) {
        let snapshot_active_txid = record.active_txid;
        put_tx_node_if_active_unchanged(broadcast_handle, snapshot_active_txid, record).await?;
        return Ok(());
    }

    let Some(active_attempt) = record.active_attempt() else {
        warn!(node_id = ?record.node_id, "tx-node record has no active attempt");
        return Ok(());
    };
    let active_tx = active_attempt.try_to_tx()?;

    let reveal_fee_budget =
        resolve_reveal_fee_budget(broadcast_handle, context, &record.kind, &active_tx).await?;
    if matches!(
        &record.kind,
        TxNodeKind::SingleEnvelopeReveal { .. } | TxNodeKind::ChunkedEnvelopeReveal { .. }
    ) && reveal_fee_budget.is_none()
    {
        return Ok(());
    }

    let decision = evaluate_fee_bump(
        &writer_config.fee_bumping,
        &record,
        active_attempt,
        FeeBumpEvaluation {
            current_l1_tip,
            estimate_fee_rate,
            incremental_relay_fee_rate,
            replacement_vsize: active_tx.vsize(),
            reveal_fee_budget,
        },
    );

    match decision {
        FeeBumpDecision::Wait => Ok(()),
        FeeBumpDecision::BlockedByCeiling { estimate, ceiling } => {
            // Warn rather than trace: the chain is stuck until the estimate falls below the
            // effective ceiling, and nothing else surfaces that.
            warn!(
                node_id = ?record.node_id,
                estimate_sat_vb = estimate.to_sat_per_vb_ceil(),
                ceiling_sat_vb = ceiling.to_sat_per_vb_ceil(),
                "fee estimate is above the effective ceiling; deferring the bump until it falls back"
            );
            Ok(())
        }
        FeeBumpDecision::Terminal(error) => {
            // Logged once per node: every later pass skips a record that carries a terminal error,
            // so this is the only place an operator learns the chain has stopped advancing.
            warn!(
                node_id = ?record.node_id,
                kind = ?record.kind,
                %error,
                "fee bumping is disabled for this transaction; it stays at its current fee rate"
            );
            mark_terminal(broadcast_handle, record, error).await?;
            Ok(())
        }
        FeeBumpDecision::Replace(request) => match record.kind.clone() {
            TxNodeKind::SingleEnvelopeCommit { .. } => {
                // The checkpoint writer stores a payload's commit and reveal broadcast rows within
                // the same call, so there is no commit-only phase to bump in: by the time a commit
                // is old enough to be stale, its reveal is already queued and replacing the commit
                // would orphan it. Bumping the reveal is what actually unsticks a checkpoint, and
                // that path is unaffected.
                mark_terminal(broadcast_handle, record, TerminalError::UnsupportedRbfKind).await?;
                Ok(())
            }
            TxNodeKind::SingleEnvelopeReveal { .. } => {
                replace_single_envelope_reveal(
                    writer_config,
                    broadcast_handle,
                    context,
                    record,
                    request.target_fee_rate,
                    request.attempt_no,
                )
                .await
            }
            TxNodeKind::ChunkedEnvelopeCommit { .. } => {
                replace_wallet_commit(
                    client,
                    writer_config,
                    broadcast_handle,
                    context,
                    record,
                    request.target_fee_rate,
                    request.attempt_no,
                )
                .await
            }
            TxNodeKind::ChunkedEnvelopeReveal { .. } => {
                replace_chunked_reveal(
                    writer_config,
                    broadcast_handle,
                    context,
                    record,
                    request.target_fee_rate,
                    request.attempt_no,
                )
                .await
            }
        },
    }
}

async fn replace_wallet_commit<C>(
    client: &C,
    writer_config: &WriterConfig,
    broadcast_handle: &L1BroadcastHandle,
    context: &ReplacementContext,
    mut record: TxNodeRecord,
    target_fee_rate: FeeRate,
    attempt_no: u32,
) -> anyhow::Result<()>
where
    C: Signer,
{
    // Claim the envelope before checking anything. The durable guard below is a read, and the
    // writes that follow it are not in the same transaction, so without exclusion a reveal can be
    // enqueued in the gap and the replacement would orphan it. The claim is released on drop.
    let _commit_claim = match record.kind {
        TxNodeKind::ChunkedEnvelopeCommit { envelope_idx } => {
            let Some(claim) = context.commit_phase.try_claim(envelope_idx) else {
                debug!(
                    envelope_idx,
                    "commit replacement skipped: reveal enqueueing is in flight"
                );
                return Ok(());
            };
            Some(claim)
        }
        _ => None,
    };

    if !commit_replacement_allowed(&record, broadcast_handle, context).await? {
        debug!(node_id = ?record.node_id, kind = ?record.kind, "commit replacement skipped after dependent reveal activity");
        return Ok(());
    }

    let active_txid = to_raw_buf32(record.active_txid);
    let Some(active_entry) = broadcast_handle
        .get_tx_entry_by_id_async(active_txid)
        .await?
    else {
        return Ok(());
    };
    let original_commit_tx = active_entry.try_to_tx()?;

    // Tell Core which output to recycle rather than letting it guess. Its own change detection
    // skips anything in the wallet's address book, which is where the sequencer address lives, so
    // an unguided bump would add inputs and be refused every time.
    let chunked_envelope_idx = match record.kind {
        TxNodeKind::ChunkedEnvelopeCommit { envelope_idx } => Some(envelope_idx),
        _ => None,
    };
    let original_change_index = if let Some(envelope_idx) = chunked_envelope_idx {
        let Some(chunked_ops) = context.chunked_ops.as_ref() else {
            bail!("chunked commit replacement requires chunked envelope context");
        };
        let Some(envelope_entry) = chunked_ops
            .get_chunked_envelope_entry_async(envelope_idx)
            .await?
        else {
            bail!("chunked envelope {envelope_idx} missing");
        };
        // Replacing the commit re-points every reveal at the new commit output and re-signs it,
        // while each reveal keeps its original tapscript. Under a rotated sequencer key those
        // signatures could never satisfy those scripts, and the refusal only surfaces at broadcast,
        // by which point the original commit is already `Replaced` and the envelope has nothing
        // live left. Refuse before anything is written; the writer rebuilds under the new key, and
        // that fresh initial attempt clears the terminal error.
        if let Err(error) = chunked_reveals_signable(context, &envelope_entry) {
            warn!(envelope_idx, %error, "sequencer key rotated since the envelope was built; refusing to bump its commit");
            mark_terminal(broadcast_handle, record, error.terminal_error()).await?;
            return Ok(());
        }
        chunked_commit_change_index(&original_commit_tx, envelope_entry.reveals.len())
    } else {
        None
    };

    let replacement = match build_wallet_commit_replacement(
        client,
        &record.kind,
        &original_commit_tx,
        record.active_txid,
        target_fee_rate,
        writer_config.fee_bumping.max_fee_rate(),
        attempt_no,
        original_change_index,
    )
    .await
    {
        Ok(replacement) => replacement,
        Err(error) if error.is_retryable() => {
            warn!(node_id = ?record.node_id, kind = ?record.kind, %error, "RBF replacement build failed transiently, retrying next poll");
            return Ok(());
        }
        // Discard rather than terminate. The overshoot comes from the wallet's coin selection, not
        // from our escalation policy, so a later candidate built against a different UTXO set can
        // land under the ceiling; marking the node terminal would strand the envelope for good.
        Err(error @ ReplacementError::ExceedsMaxFeeRate { .. }) => {
            warn!(node_id = ?record.node_id, kind = ?record.kind, %error, "discarding replacement commit that breaches the configured fee-rate ceiling");
            return Ok(());
        }
        Err(error) => {
            warn!(node_id = ?record.node_id, kind = ?record.kind, %error, "failed to build RBF replacement");
            mark_terminal(broadcast_handle, record, error.terminal_error()).await?;
            return Ok(());
        }
    };

    let active_entry_after_build = broadcast_handle
        .get_tx_entry_by_id_async(active_txid)
        .await?
        .unwrap_or(active_entry);
    if matches!(
        active_entry_after_build.status,
        L1TxStatus::Confirmed { .. } | L1TxStatus::Finalized { .. }
    ) {
        debug!(node_id = ?record.node_id, "original transaction confirmed before replacement was persisted");
        return Ok(());
    }

    let replacement_tx = replacement.try_to_tx()?;
    let is_chunked_commit = matches!(record.kind, TxNodeKind::ChunkedEnvelopeCommit { .. });

    // Validate before writing anything: an incompatible layout must not leave partial state.
    if is_chunked_commit {
        // A storage or decode failure here propagates: it is our own state that is broken, and
        // silently retrying would rebuild and re-sign a PSBT every poll while hiding the fault.
        match validate_chunked_commit_layout(
            context,
            &record,
            &active_entry_after_build,
            &replacement_tx,
        )
        .await?
        {
            CommitLayoutCheck::Ok => {}
            CommitLayoutCheck::IncompatibleCandidate(error) => {
                // A different candidate (different fee rate, different change handling) may well
                // be compatible, so discard this one rather than disabling the chain permanently.
                warn!(node_id = ?record.node_id, %error, "discarding replacement commit whose layout is incompatible with the envelope");
                return Ok(());
            }
        }
    }

    // Write order across the three trees:
    //
    // 1. the replacement's broadcast entry, so anything that later points at the replacement
    //    resolves to a real entry (an envelope row referencing a commit the broadcaster has never
    //    seen reads as corruption to the watcher and exits it permanently);
    // 2. the original's `Replaced` transition, which is atomic and reports whether we actually won
    //    the race against a confirmation;
    // 3. the envelope metadata, only once step 2 says the replacement is the live commit;
    // 4. the tx-node record.
    //
    // Rewriting metadata before step 2 is what lets a confirmation land in between and leave the
    // envelope tracking a replacement that can never confirm.
    let replacement_txid = replacement.txid;
    let replacement_txid_raw = to_raw_buf32(replacement_txid);
    // The attempt's own rate, not the target: the wallet prices the fee off the transaction it
    // ends up building, and the recovery path in `process_record` rebuilds attempts from this
    // metadata, so the two must agree.
    let replacement_entry = L1TxEntry::from_tx_with_fee(
        &replacement_tx,
        replacement.fee_rate().unwrap_or(target_fee_rate),
        replacement.fee(),
    );
    if !broadcast_handle
        .put_replacement_tx_entry(active_txid, replacement_txid_raw, replacement_entry)
        .await?
    {
        debug!(node_id = ?record.node_id, "original transaction left the publishable state before it could be superseded");
        return Ok(());
    }

    // Persist the node before the metadata refresh. The refresh can fail, and a node that still
    // points at the superseded txid is the one state that is not self-healing; with the node
    // written, `process_record` adopts the replacement and the refresh is retried below.
    record.append_replacement(replacement);
    broadcast_handle.put_tx_node(record.clone()).await?;

    if is_chunked_commit {
        // Deliberately not terminal. Terminal records are skipped by every later poll, so a
        // transient storage failure here would strand the envelope permanently. Propagating
        // instead surfaces the fault and retries on the next poll, and until the refresh lands the
        // writer refuses to enqueue reveals built against the superseded commit.
        update_chunked_commit_replacement_metadata(context, &record, &replacement_tx)
            .await
            .context("refreshing chunked envelope metadata after commit replacement")?;
    }

    Ok(())
}

async fn replace_single_envelope_reveal(
    writer_config: &WriterConfig,
    broadcast_handle: &L1BroadcastHandle,
    context: &ReplacementContext,
    mut record: TxNodeRecord,
    target_fee_rate: FeeRate,
    attempt_no: u32,
) -> anyhow::Result<()> {
    let payload_idx = match &record.kind {
        TxNodeKind::SingleEnvelopeReveal { payload_idx } => *payload_idx,
        _ => return Ok(()),
    };
    let Some(envelope_ops) = context.envelope_ops.as_ref() else {
        mark_terminal(broadcast_handle, record, TerminalError::UnsupportedRbfKind).await?;
        return Ok(());
    };
    let Some(provider) = context.signing_mode_provider.as_ref() else {
        mark_terminal(broadcast_handle, record, TerminalError::UnsupportedRbfKind).await?;
        return Ok(());
    };
    let signer_pubkey = match provider.signing_mode() {
        // Only an external signer can re-sign a reveal replacement.
        Ok(EnvelopeSigningMode::External { pubkey }) => pubkey,
        Ok(EnvelopeSigningMode::InProcess) => {
            mark_terminal(broadcast_handle, record, TerminalError::UnsupportedRbfKind).await?;
            return Ok(());
        }
        // The mode tracks canonical state, so a transient resolution failure must not mark the
        // node terminal: that is sticky and would strand it permanently. Defer instead, matching
        // the watcher's `resolve_signing_mode`.
        Err(err) => {
            warn!(%err, "could not resolve envelope signing mode; deferring to next tick");
            return Ok(());
        }
    };

    let Some(commit_output) =
        resolve_reveal_commit_output(broadcast_handle, context, &record.kind).await?
    else {
        return Ok(());
    };

    let active_txid = to_raw_buf32(record.active_txid);
    let Some(active_entry) = broadcast_handle
        .get_tx_entry_by_id_async(active_txid)
        .await?
    else {
        return Ok(());
    };
    let active_reveal_tx = active_entry.try_to_tx()?;

    // The replacement reuses the original tapscript, so its witness only validates under the key
    // that script commits to. If the predicate has rotated to a different external key, the signer
    // would sign the replacement sighash with the new key and produce an invalid witness, while
    // the original entry was already marked `Replaced`. Refuse instead; the watcher rebuilds the
    // envelope under the new key, and that fresh initial attempt clears this terminal error.
    match extract_reveal_pubkey(&active_reveal_tx) {
        Ok(reveal_pubkey) if reveal_pubkey == signer_pubkey => {}
        Ok(reveal_pubkey) => {
            warn!(
                payload_idx,
                %reveal_pubkey,
                %signer_pubkey,
                "envelope signing key rotated since the reveal was built; refusing to bump it"
            );
            mark_terminal(broadcast_handle, record, TerminalError::UnsupportedRbfKind).await?;
            return Ok(());
        }
        Err(error) => {
            mark_terminal(broadcast_handle, record, error.terminal_error()).await?;
            return Ok(());
        }
    }

    let (replacement, sighash) = match build_pending_single_reveal_replacement(
        &active_reveal_tx,
        &commit_output,
        target_fee_rate,
        attempt_no,
    ) {
        Ok(replacement) => replacement,
        Err(error) if error.is_retryable() => {
            warn!(node_id = ?record.node_id, kind = ?record.kind, %error, "RBF replacement build failed transiently, retrying next poll");
            return Ok(());
        }
        Err(error) => {
            mark_terminal(broadcast_handle, record, error.terminal_error()).await?;
            return Ok(());
        }
    };

    let active_entry_after_build = broadcast_handle
        .get_tx_entry_by_id_async(active_txid)
        .await?
        .unwrap_or(active_entry);
    if matches!(
        active_entry_after_build.status,
        L1TxStatus::Confirmed { .. } | L1TxStatus::Finalized { .. }
    ) {
        debug!(
            payload_idx,
            "original reveal confirmed before replacement was persisted"
        );
        return Ok(());
    }

    // Both writes below come from a snapshot taken before the replacement was built, and the writer
    // rebuilds this payload concurrently whenever the original reveal goes invalid. Re-read the row
    // and give up if it has moved on: writing the stale row back would restore a sighash computed
    // over the superseded reveal, and the signer would sign that for the rebuilt envelope, whose
    // witness could then never validate.
    let Some(mut current_payload_entry) = envelope_ops
        .get_payload_entry_by_idx_async(payload_idx)
        .await?
    else {
        return Ok(());
    };
    if current_payload_entry.reveal_txid != record.active_txid {
        debug!(
            payload_idx,
            "payload rebuilt while its reveal replacement was being built; discarding the replacement"
        );
        return Ok(());
    }

    // Node before metadata. The watcher treats `PendingRevealTxSign` without a pending node
    // attempt as a stalled signature and resets the payload to `Unsigned`, which rebuilds against
    // fresh UTXOs while the original reveal is still live. Writing the node first means that state
    // is never observable. The reverse interleaving only stalls this node's bumping.
    let snapshot_active_txid = record.active_txid;
    record.append_pending_signature_replacement(replacement);
    put_tx_node_if_active_unchanged(broadcast_handle, snapshot_active_txid, record).await?;

    current_payload_entry.payload_signature = None;
    current_payload_entry.status = L1BundleStatus::PendingRevealTxSign(sighash);
    envelope_ops
        .put_payload_entry_async(payload_idx, current_payload_entry)
        .await?;

    debug!(
        payload_idx,
        target_fee_rate_sat_vb = target_fee_rate.to_sat_per_vb_ceil(),
        max_fee_rate_sat_vb = writer_config.fee_bumping.max_fee_rate_sat_vb.get(),
        "single-envelope reveal replacement awaiting external signature"
    );
    Ok(())
}

async fn replace_chunked_reveal(
    writer_config: &WriterConfig,
    broadcast_handle: &L1BroadcastHandle,
    context: &ReplacementContext,
    mut record: TxNodeRecord,
    target_fee_rate: FeeRate,
    attempt_no: u32,
) -> anyhow::Result<()> {
    let (envelope_idx, reveal_idx) = match &record.kind {
        TxNodeKind::ChunkedEnvelopeReveal {
            envelope_idx,
            reveal_idx,
        } => (*envelope_idx, *reveal_idx),
        _ => return Ok(()),
    };
    let (Some(chunked_ops), Some(sequencer_keypair)) = (
        context.chunked_ops.as_ref(),
        context.sequencer_keypair.as_ref(),
    ) else {
        mark_terminal(broadcast_handle, record, TerminalError::UnsupportedRbfKind).await?;
        return Ok(());
    };
    let Some(mut envelope_entry) = chunked_ops
        .get_chunked_envelope_entry_async(envelope_idx)
        .await?
    else {
        warn!(
            envelope_idx,
            "chunked envelope entry missing for reveal replacement"
        );
        return Ok(());
    };
    let Some(commit_output) =
        resolve_reveal_commit_output(broadcast_handle, context, &record.kind).await?
    else {
        return Ok(());
    };
    let active_txid = to_raw_buf32(record.active_txid);
    let Some(active_entry) = broadcast_handle
        .get_tx_entry_by_id_async(active_txid)
        .await?
    else {
        return Ok(());
    };
    let active_reveal_tx = active_entry.try_to_tx()?;
    let replacement = match build_chunked_reveal_replacement(
        &active_reveal_tx,
        &commit_output,
        target_fee_rate,
        attempt_no,
        sequencer_keypair,
    ) {
        Ok(replacement) => replacement,
        Err(error) if error.is_retryable() => {
            warn!(node_id = ?record.node_id, kind = ?record.kind, %error, "RBF replacement build failed transiently, retrying next poll");
            return Ok(());
        }
        Err(error) => {
            warn!(envelope_idx, reveal_idx, %error, "failed to build chunked reveal replacement");
            mark_terminal(broadcast_handle, record, error.terminal_error()).await?;
            return Ok(());
        }
    };

    let active_entry_after_build = broadcast_handle
        .get_tx_entry_by_id_async(active_txid)
        .await?
        .unwrap_or(active_entry);
    if matches!(
        active_entry_after_build.status,
        L1TxStatus::Confirmed { .. } | L1TxStatus::Finalized { .. }
    ) {
        debug!(
            envelope_idx,
            reveal_idx, "original reveal confirmed before replacement was persisted"
        );
        return Ok(());
    }

    let replacement_tx = replacement.try_to_tx()?;
    let replacement_txid = replacement.txid;
    let replacement_txid_raw = to_raw_buf32(replacement_txid);
    let replacement_entry =
        L1TxEntry::from_tx_with_fee(&replacement_tx, target_fee_rate, replacement.fee());
    if !broadcast_handle
        .put_replacement_tx_entry(active_txid, replacement_txid_raw, replacement_entry)
        .await?
    {
        debug!(
            envelope_idx,
            reveal_idx, "reveal left the publishable state before it could be superseded"
        );
        return Ok(());
    }

    update_chunked_reveal_meta(&mut envelope_entry, reveal_idx, &replacement_tx);
    chunked_ops
        .put_chunked_envelope_entry_async(envelope_idx, envelope_entry)
        .await?;

    record.append_replacement(replacement);
    broadcast_handle.put_tx_node(record).await?;

    debug!(
        envelope_idx,
        reveal_idx,
        txid = ?replacement_txid,
        target_fee_rate_sat_vb = target_fee_rate.to_sat_per_vb_ceil(),
        max_fee_rate_sat_vb = writer_config.fee_bumping.max_fee_rate_sat_vb.get(),
        "chunked reveal replacement persisted"
    );
    Ok(())
}

fn update_chunked_reveal_meta(
    envelope_entry: &mut ChunkedEnvelopeEntry,
    reveal_idx: u32,
    replacement_tx: &Transaction,
) {
    if let Some(reveal) = envelope_entry.reveals.get_mut(reveal_idx as usize) {
        *reveal = RevealTxMeta {
            vout_index: reveal.vout_index,
            txid: L1TxId::from(replacement_tx.compute_txid().to_byte_array()),
            wtxid: L1WtxId::from(replacement_tx.compute_wtxid().to_byte_array()),
            tx_bytes: serialize(replacement_tx),
        };
    }
}

/// Outcome of checking a replacement chunked commit before envelope metadata is rewritten.
enum CommitLayoutCheck {
    /// The replacement is safe to adopt.
    Ok,
    /// This particular candidate is unusable, but a later one may not be.
    IncompatibleCandidate(ReplacementError),
}

/// Re-runs the envelope metadata refresh when it did not complete after a commit replacement.
async fn retry_stale_chunked_commit_metadata(
    broadcast_handle: &L1BroadcastHandle,
    context: &ReplacementContext,
    record: &TxNodeRecord,
    envelope_idx: u64,
) -> anyhow::Result<()> {
    let Some(chunked_ops) = context.chunked_ops.as_ref() else {
        return Ok(());
    };
    let Some(entry) = chunked_ops
        .get_chunked_envelope_entry_async(envelope_idx)
        .await?
    else {
        return Ok(());
    };
    if entry.commit_txid == record.active_txid {
        return Ok(());
    }
    // Forward-only, same rule as the reveal side. A resign that crashed between writing the row and
    // writing the node leaves the row on the newer commit and the node on the dead one; without
    // this the refresh would drag the row back onto the invalidated commit and re-sign every reveal
    // against it, once per poll, until the watcher's own resign won.
    let resolves_to_active = broadcast_handle
        .get_active_tx_entry_by_id_async(to_raw_buf32(entry.commit_txid))
        .await?
        .is_some_and(|(resolved_txid, _)| L1TxId::from(resolved_txid.0) == record.active_txid);
    if !resolves_to_active {
        return Ok(());
    }
    let Some(active_attempt) = record.active_attempt() else {
        return Ok(());
    };

    warn!(
        envelope_idx,
        stale_commit_txid = ?entry.commit_txid,
        active_commit_txid = ?record.active_txid,
        "completing chunked envelope metadata refresh left undone by a commit replacement"
    );
    let replacement_tx = active_attempt.try_to_tx()?;
    update_chunked_commit_replacement_metadata(context, record, &replacement_tx).await
}

/// Re-points a chunked envelope's reveal metadata when a replacement did not finish rewriting it.
///
/// [`replace_chunked_reveal`] supersedes the reveal in the broadcast DB before it rewrites the
/// envelope row, so a stop in between leaves the row naming a `Replaced` txid. Unlike the commit
/// path, nothing downstream repairs that: the chunked watcher follows the replacement chain without
/// rewriting the row, while the EE DA provider looks the row's txid up directly and refuses
/// anything that is not `Finalized`, which blocks the envelope's DA refs forever.
async fn retry_stale_chunked_reveal_metadata(
    broadcast_handle: &L1BroadcastHandle,
    context: &ReplacementContext,
    record: &TxNodeRecord,
    envelope_idx: u64,
    reveal_idx: u32,
) -> anyhow::Result<()> {
    let Some(chunked_ops) = context.chunked_ops.as_ref() else {
        return Ok(());
    };
    let Some(mut entry) = chunked_ops
        .get_chunked_envelope_entry_async(envelope_idx)
        .await?
    else {
        return Ok(());
    };
    let Some(stale_reveal_txid) = entry
        .reveals
        .get(reveal_idx as usize)
        .map(|reveal| reveal.txid)
    else {
        return Ok(());
    };
    if stale_reveal_txid == record.active_txid {
        return Ok(());
    }

    // Only ever move the row forward: adopt the node's txid when the row's reveal resolves *through
    // the replacement chain* to it. The inverse partial write, where the row is newer than the
    // node, is healed by the `Replaced` adoption above and must not be dragged back here.
    let resolves_to_active = broadcast_handle
        .get_active_tx_entry_by_id_async(to_raw_buf32(stale_reveal_txid))
        .await?
        .is_some_and(|(resolved_txid, _)| L1TxId::from(resolved_txid.0) == record.active_txid);
    if !resolves_to_active {
        return Ok(());
    }
    let Some(active_attempt) = record.active_attempt() else {
        return Ok(());
    };
    let replacement_tx = active_attempt.try_to_tx()?;

    warn!(
        envelope_idx,
        reveal_idx,
        ?stale_reveal_txid,
        active_reveal_txid = ?record.active_txid,
        "completing chunked envelope metadata refresh left undone by a reveal replacement"
    );
    update_chunked_reveal_meta(&mut entry, reveal_idx, &replacement_tx);
    chunked_ops
        .put_chunked_envelope_entry_async(envelope_idx, entry)
        .await?;
    Ok(())
}

/// Verifies a replacement chunked commit before any envelope metadata is rewritten.
///
/// Errors are reserved for problems with our own state (missing context, missing envelope row,
/// undecodable transaction, storage failure). A replacement that is simply shaped wrong comes back
/// as [`CommitLayoutCheck::IncompatibleCandidate`] so it can be discarded without masking a real
/// fault as a routine retry.
async fn validate_chunked_commit_layout(
    context: &ReplacementContext,
    record: &TxNodeRecord,
    active_commit_entry: &L1TxEntry,
    replacement_commit_tx: &Transaction,
) -> anyhow::Result<CommitLayoutCheck> {
    let TxNodeKind::ChunkedEnvelopeCommit { envelope_idx } = record.kind else {
        return Ok(CommitLayoutCheck::Ok);
    };
    let Some(chunked_ops) = context.chunked_ops.as_ref() else {
        bail!("chunked commit replacement requires chunked envelope context");
    };
    let Some(envelope_entry) = chunked_ops
        .get_chunked_envelope_entry_async(envelope_idx)
        .await?
    else {
        bail!("chunked envelope {envelope_idx} missing");
    };

    let original_commit_tx = active_commit_entry.try_to_tx()?;
    match validate_chunked_commit_replacement_layout(
        &original_commit_tx,
        replacement_commit_tx,
        envelope_entry.reveals.len(),
    ) {
        Ok(()) => Ok(CommitLayoutCheck::Ok),
        Err(error) => Ok(CommitLayoutCheck::IncompatibleCandidate(error)),
    }
}

/// Reports whether every reveal of a chunked envelope still commits to the sequencer's current key.
///
/// A commit replacement re-signs all of them, so one rotated reveal is enough to make the whole
/// envelope unspendable. Reveals whose stored bytes do not decode are left alone here; the metadata
/// refresh surfaces that as the state corruption it is rather than as a routine refusal to bump.
fn chunked_reveals_signable(
    context: &ReplacementContext,
    envelope_entry: &ChunkedEnvelopeEntry,
) -> Result<(), ReplacementError> {
    let Some(sequencer_keypair) = context.sequencer_keypair.as_ref() else {
        return Ok(());
    };
    for reveal in &envelope_entry.reveals {
        let Ok(reveal_tx) = deserialize::<Transaction>(&reveal.tx_bytes) else {
            continue;
        };
        ensure_reveal_signable(&reveal_tx, sequencer_keypair)?;
    }
    Ok(())
}

async fn update_chunked_commit_replacement_metadata(
    context: &ReplacementContext,
    record: &TxNodeRecord,
    replacement_commit_tx: &Transaction,
) -> anyhow::Result<()> {
    let TxNodeKind::ChunkedEnvelopeCommit { envelope_idx } = record.kind else {
        return Ok(());
    };
    let (Some(chunked_ops), Some(sequencer_keypair)) = (
        context.chunked_ops.as_ref(),
        context.sequencer_keypair.as_ref(),
    ) else {
        bail!("chunked commit replacement requires chunked envelope context");
    };
    let Some(mut envelope_entry) = chunked_ops
        .get_chunked_envelope_entry_async(envelope_idx)
        .await?
    else {
        bail!("chunked envelope {envelope_idx} missing");
    };

    let replacement_commit_txid = replacement_commit_tx.compute_txid();
    envelope_entry.commit_txid = L1TxId::from(replacement_commit_txid.to_byte_array());
    envelope_entry.commit_wtxid =
        L1WtxId::from(replacement_commit_tx.compute_wtxid().to_byte_array());

    for reveal in &mut envelope_entry.reveals {
        let old_reveal_tx: Transaction = deserialize(&reveal.tx_bytes)?;
        let Some(commit_output) = replacement_commit_tx.output.get(reveal.vout_index as usize)
        else {
            bail!(
                "replacement commit missing reveal output {}",
                reveal.vout_index
            );
        };
        let replacement_reveal = rebuild_reveal_for_replaced_commit(
            &old_reveal_tx,
            replacement_commit_txid,
            commit_output,
            sequencer_keypair,
        )?;
        reveal.txid = L1TxId::from(replacement_reveal.compute_txid().to_byte_array());
        reveal.wtxid = L1WtxId::from(replacement_reveal.compute_wtxid().to_byte_array());
        reveal.tx_bytes = serialize(&replacement_reveal);
    }

    chunked_ops
        .put_chunked_envelope_entry_async(envelope_idx, envelope_entry)
        .await?;
    Ok(())
}

fn mark_first_published_height(record: &mut TxNodeRecord, current_l1_tip: L1Height) -> bool {
    let Some(active_attempt) = record.active_attempt_mut() else {
        return false;
    };
    if active_attempt.first_published_l1_height.is_some() {
        return false;
    }
    active_attempt.first_published_l1_height = Some(current_l1_tip);
    true
}

async fn mark_terminal(
    broadcast_handle: &L1BroadcastHandle,
    mut record: TxNodeRecord,
    error: TerminalError,
) -> anyhow::Result<()> {
    let snapshot_active_txid = record.active_txid;
    record.set_terminal_error(error);
    put_tx_node_if_active_unchanged(broadcast_handle, snapshot_active_txid, record).await
}

/// Persists a record only while the durable node still names the same active attempt.
///
/// The poll works from a snapshot taken before its per-record I/O, and the writer rebuilds a
/// logical transaction concurrently whenever its original goes `InvalidInputs`:
/// `put_tx_node_if_enabled` calls [`TxNodeRecord::replace_initial_attempt`], which installs a fresh
/// attempt under a new txid and clears any terminal error. Writing the snapshot back on top of that
/// would revive the superseded txid, and because the payload tracks the rebuilt transaction, every
/// later poll would read a non-`Published` entry and skip the node for good.
///
/// This narrows the window to two adjacent reads rather than closing it; the compare and the write
/// are still separate operations. Making it atomic needs a compare-and-swap in the broadcast DB,
/// which belongs with the journalling work in known gap 2.
///
/// Replacement writes do not need this: they only run once `put_replacement_tx_entry` has
/// transitioned the original, and that check is atomic and refuses a txid a rebuild has already
/// invalidated.
///
/// `snapshot_active_txid` is the active txid the caller read, which is not always `record`'s own:
/// the adoption path rewrites it before persisting.
async fn put_tx_node_if_active_unchanged(
    broadcast_handle: &L1BroadcastHandle,
    snapshot_active_txid: L1TxId,
    record: TxNodeRecord,
) -> anyhow::Result<()> {
    if let Some(current) = broadcast_handle.get_tx_node(record.node_id).await? {
        if current.active_txid != snapshot_active_txid {
            debug!(
                node_id = ?record.node_id,
                ?snapshot_active_txid,
                active_txid = ?current.active_txid,
                "skipping tx-node write: the writer rebuilt this transaction during the poll"
            );
            return Ok(());
        }
    }
    broadcast_handle.put_tx_node(record).await?;
    Ok(())
}

/// Reports whether a commit transaction may still be fee bumped.
///
/// Replacing a commit changes its txid, which invalidates every reveal that spends one of its
/// outputs. So a commit is only replaceable while the envelope is still in the commit-only phase.
/// This check is deliberately fail-closed: anything it cannot positively confirm as "no reveal has
/// been handed to the broadcaster" blocks the replacement.
async fn commit_replacement_allowed(
    record: &TxNodeRecord,
    broadcast_handle: &L1BroadcastHandle,
    context: &ReplacementContext,
) -> anyhow::Result<bool> {
    match record.kind {
        TxNodeKind::SingleEnvelopeCommit { payload_idx } => {
            reveal_node_not_handed_to_broadcaster(
                TxNodeId::from_kind(&TxNodeKind::SingleEnvelopeReveal { payload_idx }),
                broadcast_handle,
            )
            .await
        }
        TxNodeKind::ChunkedEnvelopeCommit { envelope_idx } => {
            chunked_reveals_not_handed_to_broadcaster(envelope_idx, broadcast_handle, context).await
        }
        TxNodeKind::SingleEnvelopeReveal { .. } | TxNodeKind::ChunkedEnvelopeReveal { .. } => {
            Ok(true)
        }
    }
}

/// Reports whether no reveal of `envelope_idx` has reached the broadcaster yet.
///
/// Two independent sources are consulted so the answer does not depend on the order in which the
/// writer persists a reveal's tx-node record and its broadcast entry:
///
/// 1. the persisted envelope row, whose `reveals` carry the authoritative reveal txids, and
/// 2. the tx-node records for `(envelope_idx, reveal_idx)`, looked up one per reveal, which cover
///    reveals recorded before the envelope row was refreshed.
///
/// A missing envelope row is treated as disqualifying rather than permissive: without it there is
/// no way to enumerate the reveals that a commit replacement would orphan.
async fn chunked_reveals_not_handed_to_broadcaster(
    envelope_idx: u64,
    broadcast_handle: &L1BroadcastHandle,
    context: &ReplacementContext,
) -> anyhow::Result<bool> {
    let Some(chunked_ops) = context.chunked_ops.as_ref() else {
        return Ok(false);
    };
    let Some(entry) = chunked_ops
        .get_chunked_envelope_entry_async(envelope_idx)
        .await?
    else {
        warn!(
            envelope_idx,
            "chunked envelope row missing; refusing commit replacement"
        );
        return Ok(false);
    };

    for reveal in &entry.reveals {
        if broadcast_handle
            .get_tx_entry_by_id_async(to_raw_buf32(reveal.txid))
            .await?
            .is_some()
        {
            return Ok(false);
        }
    }

    // Point lookups rather than a tree scan. Tx-node ids are content-derived, and every chunked
    // reveal node is created from this same row's reveal enumeration with
    // `reveal_idx = vout_index - 1`, so the row names exactly the ids that can exist. Scanning
    // instead would cost a full decode of every node the writer has ever published, once per
    // eligible commit per pass.
    //
    // Reading them here rather than from a snapshot taken at pass start also keeps the check
    // fail-closed: a reveal enqueued between pass start and the commit-phase claim would be
    // invisible to a snapshot.
    for reveal_idx in 0..entry.reveals.len() {
        let node_id = TxNodeId::from_kind(&TxNodeKind::ChunkedEnvelopeReveal {
            envelope_idx,
            reveal_idx: reveal_idx as u32,
        });
        if broadcast_handle.get_tx_node(node_id).await?.is_some() {
            return Ok(false);
        }
    }

    Ok(true)
}

/// Reports whether this reveal has not yet been handed to the broadcaster.
///
/// "Handed over" means a broadcast row exists, not that the row says `Published`. An `Unpublished`
/// row is already queued and the broadcaster can publish it on any tick, so a commit replacement
/// that reads it as safe would race the publication and orphan the reveal.
async fn reveal_node_not_handed_to_broadcaster(
    node_id: TxNodeId,
    broadcast_handle: &L1BroadcastHandle,
) -> anyhow::Result<bool> {
    let Some(reveal_node) = broadcast_handle.get_tx_node(node_id).await? else {
        return Ok(true);
    };
    Ok(broadcast_handle
        .get_active_tx_entry_by_id_async(to_raw_buf32(reveal_node.active_txid))
        .await?
        .is_none())
}

fn to_raw_buf32(txid: L1TxId) -> Buf32 {
    Buf32(txid.0)
}

#[cfg(test)]
mod pacer_tests {
    use super::*;

    #[test]
    fn first_claim_runs_then_the_interval_holds() {
        let pacer = ReplacementPacer::new(Duration::from_secs(30));

        // A fresh writer must look at records a previous process left behind rather than idling
        // for a full interval first.
        assert!(pacer.claim());
        assert!(!pacer.claim());
        assert!(!pacer.claim());
    }

    #[test]
    fn a_zero_interval_runs_every_time() {
        let pacer = ReplacementPacer::new(Duration::ZERO);

        assert!(pacer.claim());
        assert!(pacer.claim());
    }

    #[test]
    fn the_default_interval_matches_the_config_default() {
        assert_eq!(
            ReplacementPacer::default().interval,
            FeeBumpingConfig::default().check_interval()
        );
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        absolute::LockTime, consensus::serialize, transaction::Version, Amount, OutPoint,
        ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
    };
    use strata_csm_types::L1Payload;
    use strata_db_types::{
        chunked_envelope::{ChunkedEnvelopeEntry, ChunkedEnvelopeStatus},
        common::L1WtxId,
        fee_bump::TxAttempt,
        l1_writer::BundledPayloadEntry,
    };
    use strata_l1_txfmt::{MagicBytes, TagData};

    use super::*;
    use crate::writer::test_utils::{
        get_broadcast_handle, get_chunked_envelope_ops, get_envelope_ops,
    };

    const ENVELOPE_IDX: u64 = 3;
    const PAYLOAD_IDX: u64 = 7;

    /// Signing mode provider that yields a fixed mode, or fails when none is set.
    #[derive(Debug)]
    struct TestSigningModeProvider {
        mode: Option<EnvelopeSigningMode>,
    }

    impl TestSigningModeProvider {
        fn returning(mode: EnvelopeSigningMode) -> Arc<dyn EnvelopeSigningModeProvider> {
            Arc::new(Self { mode: Some(mode) })
        }

        fn failing() -> Arc<dyn EnvelopeSigningModeProvider> {
            Arc::new(Self { mode: None })
        }
    }

    impl EnvelopeSigningModeProvider for TestSigningModeProvider {
        fn signing_mode(&self) -> anyhow::Result<EnvelopeSigningMode> {
            self.mode
                .ok_or_else(|| anyhow::anyhow!("canonical ASM state unavailable"))
        }
    }

    fn tx_with_output(value: u64) -> Transaction {
        Transaction {
            version: Version(2),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(value),
                script_pubkey: ScriptBuf::new(),
            }],
        }
    }

    #[test]
    fn reveal_budget_reserves_other_outputs_and_final_output_dust() {
        let mut reveal = tx_with_output(900);
        reveal.output.insert(
            0,
            TxOut {
                value: Amount::from_sat(100),
                script_pubkey: ScriptBuf::new(),
            },
        );
        let commit_output = TxOut {
            value: Amount::from_sat(2_000),
            script_pubkey: ScriptBuf::new(),
        };

        assert_eq!(
            reveal_fee_budget(&commit_output, &reveal),
            Amount::from_sat(1_354)
        );
    }

    #[test]
    fn reveal_budget_saturates_to_zero_when_funding_is_below_reserved_value() {
        let reveal = tx_with_output(BITCOIN_DUST_LIMIT);
        let commit_output = TxOut {
            value: Amount::from_sat(BITCOIN_DUST_LIMIT - 1),
            script_pubkey: ScriptBuf::new(),
        };

        assert_eq!(reveal_fee_budget(&commit_output, &reveal), Amount::ZERO);
    }

    fn fee_rate() -> FeeRate {
        FeeRate::from_sat_per_vb(2).expect("test: valid fee rate")
    }

    fn commit_record() -> TxNodeRecord {
        let attempt = TxAttempt::active(
            &tx_with_output(10_000),
            fee_rate(),
            Amount::from_sat(500),
            0,
        );
        TxNodeRecord::new(
            TxNodeKind::ChunkedEnvelopeCommit {
                envelope_idx: ENVELOPE_IDX,
            },
            attempt,
        )
    }

    /// Builds an envelope row carrying one reveal, returning the row and the reveal tx.
    fn envelope_entry_with_reveal() -> (ChunkedEnvelopeEntry, Transaction) {
        let reveal_tx = tx_with_output(900);
        let mut entry =
            ChunkedEnvelopeEntry::new_unsigned(vec![vec![1u8; 8]], MagicBytes::new(*b"ALPN"), 0);
        entry.status = ChunkedEnvelopeStatus::CommitPublished;
        entry.reveals = vec![RevealTxMeta {
            vout_index: 1,
            txid: L1TxId::from(reveal_tx.compute_txid().to_byte_array()),
            wtxid: L1WtxId::from(reveal_tx.compute_wtxid().to_byte_array()),
            tx_bytes: serialize(&reveal_tx),
        }];
        (entry, reveal_tx)
    }

    /// The commit-only-phase guard is a read, and the writes that follow it are not in the same
    /// transaction. The latch is what makes the pair safe: while the writer is enqueueing reveals
    /// for an envelope, a commit replacement for it must not even begin.
    #[tokio::test(flavor = "multi_thread")]
    async fn commit_replacement_is_refused_while_reveal_enqueue_holds_the_envelope() {
        let bcast = get_broadcast_handle();
        let (entry, _reveal_tx) = envelope_entry_with_reveal();
        let mut context = context_with(Some(entry)).await;
        let latch = CommitPhaseLatch::new();
        context.commit_phase = latch.clone();

        // Nothing has been enqueued yet, so the durable guard alone would allow the replacement.
        assert!(
            commit_replacement_allowed(&commit_record(), &bcast, &context)
                .await
                .unwrap()
        );

        // The writer claims the envelope to enqueue its reveals.
        let enqueue_claim = latch
            .try_claim(ENVELOPE_IDX)
            .expect("writer claims the envelope");
        assert!(
            latch.try_claim(ENVELOPE_IDX).is_none(),
            "the fee bumper must not be able to claim the same envelope"
        );

        drop(enqueue_claim);
        assert!(
            latch.try_claim(ENVELOPE_IDX).is_some(),
            "the envelope must be claimable again once enqueueing finishes"
        );
    }

    fn single_reveal_record() -> TxNodeRecord {
        TxNodeRecord::new(
            TxNodeKind::SingleEnvelopeReveal {
                payload_idx: PAYLOAD_IDX,
            },
            TxAttempt::active(&tx_with_output(900), fee_rate(), Amount::from_sat(100), 0),
        )
    }

    /// Runs a single-envelope reveal replacement against `provider` and returns the persisted
    /// terminal error, if any.
    async fn terminal_error_after_reveal_replacement(
        provider: Option<Arc<dyn EnvelopeSigningModeProvider>>,
    ) -> Option<TerminalError> {
        let bcast = get_broadcast_handle();
        let record = single_reveal_record();
        let node_id = record.node_id;
        bcast
            .put_tx_node(record.clone())
            .await
            .expect("test: reveal node persists");

        let context = ReplacementContext {
            envelope_ops: Some(get_envelope_ops()),
            signing_mode_provider: provider,
            ..ReplacementContext::default()
        };

        replace_single_envelope_reveal(
            &WriterConfig::default(),
            &bcast,
            &context,
            record,
            fee_rate(),
            1,
        )
        .await
        .expect("test: replacement attempt returns");

        bcast
            .get_tx_node(node_id)
            .await
            .expect("test: node is readable")
            .expect("test: node still exists")
            .terminal_error
    }

    /// An in-process signing mode cannot re-sign a reveal, so the node is terminal.
    #[tokio::test(flavor = "multi_thread")]
    async fn single_reveal_replacement_is_terminal_for_in_process_signing() {
        let terminal = terminal_error_after_reveal_replacement(Some(
            TestSigningModeProvider::returning(EnvelopeSigningMode::InProcess),
        ))
        .await;

        assert_eq!(terminal, Some(TerminalError::UnsupportedRbfKind));
    }

    /// Without a provider there is nothing to resolve, so the node is terminal.
    #[tokio::test(flavor = "multi_thread")]
    async fn single_reveal_replacement_is_terminal_without_a_provider() {
        assert_eq!(
            terminal_error_after_reveal_replacement(None).await,
            Some(TerminalError::UnsupportedRbfKind)
        );
    }

    /// Regression: the signing mode tracks canonical ASM state, so a transient resolution failure
    /// must defer to the next tick. Marking the node terminal here is sticky and would strand it
    /// for good, even once the mode resolves again.
    #[tokio::test(flavor = "multi_thread")]
    async fn single_reveal_replacement_defers_when_signing_mode_is_unresolvable() {
        let terminal =
            terminal_error_after_reveal_replacement(Some(TestSigningModeProvider::failing())).await;

        assert_eq!(terminal, None, "a provider error must not be terminal");
    }

    async fn context_with(entry: Option<ChunkedEnvelopeEntry>) -> ReplacementContext {
        let chunked_ops = get_chunked_envelope_ops();
        if let Some(entry) = entry {
            chunked_ops
                .put_chunked_envelope_entry_async(ENVELOPE_IDX, entry)
                .await
                .expect("test: envelope row persists");
        }
        ReplacementContext {
            chunked_ops: Some(chunked_ops),
            ..ReplacementContext::default()
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn commit_replacement_allowed_before_any_reveal_is_enqueued() {
        let bcast = get_broadcast_handle();
        let (entry, _reveal_tx) = envelope_entry_with_reveal();
        let context = context_with(Some(entry)).await;

        // The envelope row lists a reveal, but nothing has been handed to the broadcaster and no
        // reveal tx-node exists: the envelope is still in the commit-only phase.
        assert!(
            commit_replacement_allowed(&commit_record(), &bcast, &context)
                .await
                .unwrap()
        );
    }

    /// Regression: a crash between inserting a reveal's broadcast entry and writing its tx-node
    /// record must not let the commit be replaced, which would orphan the published reveal.
    #[tokio::test(flavor = "multi_thread")]
    async fn commit_replacement_blocked_when_reveal_entry_exists_without_tx_node() {
        let bcast = get_broadcast_handle();
        let (entry, reveal_tx) = envelope_entry_with_reveal();
        let context = context_with(Some(entry)).await;

        bcast
            .put_tx_entry(
                Buf32(reveal_tx.compute_txid().to_byte_array()),
                L1TxEntry::from_tx_with_fee(&reveal_tx, fee_rate(), Amount::from_sat(100)),
            )
            .await
            .expect("test: reveal entry persists");

        assert!(
            !commit_replacement_allowed(&commit_record(), &bcast, &context)
                .await
                .unwrap()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn commit_replacement_blocked_when_reveal_tx_node_exists() {
        let bcast = get_broadcast_handle();
        let (entry, reveal_tx) = envelope_entry_with_reveal();
        let context = context_with(Some(entry)).await;

        let reveal_record = TxNodeRecord::new(
            TxNodeKind::ChunkedEnvelopeReveal {
                envelope_idx: ENVELOPE_IDX,
                reveal_idx: 0,
            },
            TxAttempt::active(&reveal_tx, fee_rate(), Amount::from_sat(100), 0),
        );
        bcast
            .put_tx_node(reveal_record)
            .await
            .expect("test: reveal node persists");

        assert!(
            !commit_replacement_allowed(&commit_record(), &bcast, &context)
                .await
                .unwrap()
        );
    }

    /// The tx-node check enumerates `reveal_idx` from the envelope row rather than scanning the
    /// node tree, so an off-by-one in that enumeration would silently stop seeing the reveals it
    /// exists to catch. Pin the last index of a multi-reveal envelope, which is the one a
    /// half-open/inclusive slip drops.
    #[tokio::test(flavor = "multi_thread")]
    async fn commit_replacement_blocked_by_a_tx_node_at_the_last_reveal_index() {
        let bcast = get_broadcast_handle();
        let (mut entry, reveal_tx) = envelope_entry_with_reveal();
        // Three reveals, so the last index is 2.
        let extra = |vout: u32| RevealTxMeta {
            vout_index: vout,
            txid: L1TxId::from(reveal_tx.compute_txid().to_byte_array()),
            wtxid: L1WtxId::from(reveal_tx.compute_wtxid().to_byte_array()),
            tx_bytes: serialize(&reveal_tx),
        };
        entry.reveals.push(extra(2));
        entry.reveals.push(extra(3));
        let context = context_with(Some(entry)).await;

        let reveal_record = TxNodeRecord::new(
            TxNodeKind::ChunkedEnvelopeReveal {
                envelope_idx: ENVELOPE_IDX,
                reveal_idx: 2,
            },
            TxAttempt::active(&reveal_tx, fee_rate(), Amount::from_sat(100), 0),
        );
        bcast
            .put_tx_node(reveal_record)
            .await
            .expect("test: reveal node persists");

        assert!(
            !commit_replacement_allowed(&commit_record(), &bcast, &context)
                .await
                .unwrap()
        );
    }

    /// Without the envelope row there is no way to enumerate the reveals a replacement would
    /// orphan, so the guard must refuse rather than assume the commit-only phase.
    #[tokio::test(flavor = "multi_thread")]
    async fn commit_replacement_blocked_when_envelope_row_is_missing() {
        let bcast = get_broadcast_handle();
        let context = context_with(None).await;

        assert!(
            !commit_replacement_allowed(&commit_record(), &bcast, &context)
                .await
                .unwrap()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn commit_replacement_blocked_without_chunked_ops() {
        let bcast = get_broadcast_handle();

        assert!(!commit_replacement_allowed(
            &commit_record(),
            &bcast,
            &ReplacementContext::default()
        )
        .await
        .unwrap());
    }

    /// Builds a chunked-reveal node whose replacement is live in the broadcast DB while the
    /// envelope row still names the superseded reveal, the state a stop between the two writes in
    /// [`replace_chunked_reveal`] leaves behind.
    async fn chunked_reveal_stranded_after_partial_replacement() -> (
        Arc<L1BroadcastHandle>,
        ReplacementContext,
        TxNodeRecord,
        L1TxId,
    ) {
        let bcast = get_broadcast_handle();
        let (entry, reveal_tx) = envelope_entry_with_reveal();
        let context = context_with(Some(entry)).await;

        let replacement_tx = tx_with_output(800);
        let replacement_txid = L1TxId::from(replacement_tx.compute_txid().to_byte_array());
        bcast
            .put_tx_entry(
                to_raw_buf32(L1TxId::from(reveal_tx.compute_txid().to_byte_array())),
                L1TxEntry::from_tx_with_fee(&reveal_tx, fee_rate(), Amount::from_sat(100)),
            )
            .await
            .expect("test: original reveal entry persists");
        assert!(
            bcast
                .put_replacement_tx_entry(
                    to_raw_buf32(L1TxId::from(reveal_tx.compute_txid().to_byte_array())),
                    to_raw_buf32(replacement_txid),
                    L1TxEntry::from_tx_with_fee(&replacement_tx, fee_rate(), Amount::from_sat(200)),
                )
                .await
                .expect("test: replacement swap runs"),
            "test: the original must be replaceable"
        );

        let mut record = TxNodeRecord::new(
            TxNodeKind::ChunkedEnvelopeReveal {
                envelope_idx: ENVELOPE_IDX,
                reveal_idx: 0,
            },
            TxAttempt::active(&reveal_tx, fee_rate(), Amount::from_sat(100), 0),
        );
        record.append_replacement(TxAttempt::active(
            &replacement_tx,
            fee_rate(),
            Amount::from_sat(200),
            1,
        ));

        (bcast, context, record, replacement_txid)
    }

    /// Regression: the envelope row is rewritten after the broadcast swap, so a stop in between
    /// leaves it naming a `Replaced` reveal. Nothing downstream repairs that, and the EE DA
    /// provider reads the row directly, so the envelope's DA refs would never be built.
    #[tokio::test(flavor = "multi_thread")]
    async fn stale_chunked_reveal_metadata_is_refreshed_from_the_node() {
        let (bcast, context, record, replacement_txid) =
            chunked_reveal_stranded_after_partial_replacement().await;

        retry_stale_chunked_reveal_metadata(&bcast, &context, &record, ENVELOPE_IDX, 0)
            .await
            .expect("test: metadata refresh runs");

        let refreshed = context
            .chunked_ops
            .as_ref()
            .expect("test: chunked ops configured")
            .get_chunked_envelope_entry_async(ENVELOPE_IDX)
            .await
            .expect("test: envelope row is readable")
            .expect("test: envelope row exists");
        assert_eq!(refreshed.reveals[0].txid, replacement_txid);
        assert_eq!(refreshed.reveals[0].vout_index, 1, "vout must be preserved");
    }

    /// The row is only ever moved forward. A reveal that does not resolve through the replacement
    /// chain to the node's active attempt is left alone, so the inverse partial write, where the
    /// row is newer than the node, is not dragged back to a superseded reveal.
    #[tokio::test(flavor = "multi_thread")]
    async fn unrelated_chunked_reveal_metadata_is_left_alone() {
        let (bcast, context, mut record, _) =
            chunked_reveal_stranded_after_partial_replacement().await;
        let unrelated_tx = tx_with_output(700);
        record.append_replacement(TxAttempt::active(
            &unrelated_tx,
            fee_rate(),
            Amount::from_sat(300),
            2,
        ));

        retry_stale_chunked_reveal_metadata(&bcast, &context, &record, ENVELOPE_IDX, 0)
            .await
            .expect("test: metadata refresh runs");

        let chunked_ops = context
            .chunked_ops
            .as_ref()
            .expect("test: chunked ops configured");
        let unchanged = chunked_ops
            .get_chunked_envelope_entry_async(ENVELOPE_IDX)
            .await
            .expect("test: envelope row is readable")
            .expect("test: envelope row exists");
        let (original_entry, _) = envelope_entry_with_reveal();
        assert_eq!(unchanged.reveals[0].txid, original_entry.reveals[0].txid);
    }

    /// Builds a payload row naming `reveal_txid`, and the reveal node for the same payload.
    async fn payload_row_and_reveal_node(
        reveal_tx: &Transaction,
    ) -> (ReplacementContext, TxNodeRecord) {
        let envelope_ops = get_envelope_ops();
        let tag = TagData::new(1, 1, vec![]).expect("test: valid tag");
        let payload = L1Payload::new(vec![vec![1u8; 8]], tag).expect("test: valid payload");
        let mut payload_entry = BundledPayloadEntry::new_unsigned(payload);
        payload_entry.reveal_txid = L1TxId::from(reveal_tx.compute_txid().to_byte_array());
        payload_entry.status = L1BundleStatus::PendingRevealTxSign(Buf32::zero());
        envelope_ops
            .put_payload_entry_async(PAYLOAD_IDX, payload_entry)
            .await
            .expect("test: payload row persists");

        let record = TxNodeRecord::new(
            TxNodeKind::SingleEnvelopeReveal {
                payload_idx: PAYLOAD_IDX,
            },
            TxAttempt::active(reveal_tx, fee_rate(), Amount::from_sat(100), 0),
        );
        (
            ReplacementContext {
                envelope_ops: Some(envelope_ops),
                ..ReplacementContext::default()
            },
            record,
        )
    }

    /// The re-insert recovery only applies while the payload row still names the node's
    /// transaction, which is the case when the process simply stopped between the two writes.
    #[tokio::test(flavor = "multi_thread")]
    async fn missing_entry_is_re_inserted_while_the_payload_still_owns_it() {
        let reveal_tx = tx_with_output(900);
        let (context, record) = payload_row_and_reveal_node(&reveal_tx).await;

        assert!(active_attempt_is_still_owned(&context, &record)
            .await
            .expect("test: ownership check runs"));
    }

    /// Regression: the writer reacts to a missing reveal entry by rebuilding the whole envelope.
    /// Re-inserting the abandoned reveal then hands the broadcaster a second complete envelope for
    /// one payload, since the original commit entry was written before the node, and the payload
    /// lands on L1 twice.
    #[tokio::test(flavor = "multi_thread")]
    async fn missing_entry_is_not_re_inserted_after_the_payload_was_rebuilt() {
        let reveal_tx = tx_with_output(900);
        let (context, record) = payload_row_and_reveal_node(&reveal_tx).await;

        // The writer rebuilt the envelope: the row now names a different reveal.
        let envelope_ops = context
            .envelope_ops
            .as_ref()
            .expect("test: envelope ops configured");
        let mut rebuilt = envelope_ops
            .get_payload_entry_by_idx_async(PAYLOAD_IDX)
            .await
            .expect("test: payload row is readable")
            .expect("test: payload row exists");
        rebuilt.reveal_txid = L1TxId::from(tx_with_output(800).compute_txid().to_byte_array());
        envelope_ops
            .put_payload_entry_async(PAYLOAD_IDX, rebuilt)
            .await
            .expect("test: rebuilt row persists");

        assert!(!active_attempt_is_still_owned(&context, &record)
            .await
            .expect("test: ownership check runs"));
    }

    /// Without the owning row there is no way to tell a stopped write from an abandoned envelope,
    /// and re-inserting is the destructive guess.
    #[tokio::test(flavor = "multi_thread")]
    async fn missing_entry_is_not_re_inserted_without_the_owning_row() {
        let record = single_reveal_record();

        assert!(
            !active_attempt_is_still_owned(&ReplacementContext::default(), &record)
                .await
                .expect("test: ownership check runs")
        );
    }

    /// Regression: the poll writes back a record it read before its per-record I/O. If the writer
    /// rebuilt the transaction in that window, persisting the snapshot would revive the superseded
    /// txid and clear the rebuild, and since the payload tracks the rebuilt transaction the node
    /// would read a non-`Published` entry on every later poll and never be bumped again.
    #[tokio::test(flavor = "multi_thread")]
    async fn stale_snapshot_does_not_clobber_a_rebuilt_node() {
        let bcast = get_broadcast_handle();
        let snapshot = single_reveal_record();
        let node_id = snapshot.node_id;

        // The writer rebuilds the logical transaction under a new txid while the poll is
        // mid-flight.
        let mut rebuilt = snapshot.clone();
        let rebuilt_attempt =
            TxAttempt::active(&tx_with_output(700), fee_rate(), Amount::from_sat(300), 0);
        let rebuilt_txid = rebuilt_attempt.txid;
        rebuilt.replace_initial_attempt(rebuilt_attempt);
        bcast
            .put_tx_node(rebuilt)
            .await
            .expect("test: rebuilt node persists");

        // The poll now finishes with the record it read before the rebuild.
        let snapshot_active_txid = snapshot.active_txid;
        put_tx_node_if_active_unchanged(&bcast, snapshot_active_txid, snapshot)
            .await
            .expect("test: guarded write runs");

        let stored = bcast
            .get_tx_node(node_id)
            .await
            .expect("test: node is readable")
            .expect("test: node exists");
        assert_eq!(
            stored.active_txid, rebuilt_txid,
            "the rebuilt attempt must survive the stale write"
        );
    }

    /// A terminal decision made about a transaction the writer has since rebuilt must not land on
    /// the rebuild, which `replace_initial_attempt` deliberately cleared.
    #[tokio::test(flavor = "multi_thread")]
    async fn terminal_error_does_not_land_on_a_rebuilt_node() {
        let bcast = get_broadcast_handle();
        let snapshot = single_reveal_record();
        let node_id = snapshot.node_id;

        let mut rebuilt = snapshot.clone();
        rebuilt.replace_initial_attempt(TxAttempt::active(
            &tx_with_output(700),
            fee_rate(),
            Amount::from_sat(300),
            0,
        ));
        bcast
            .put_tx_node(rebuilt)
            .await
            .expect("test: rebuilt node persists");

        mark_terminal(&bcast, snapshot, TerminalError::UnsupportedRbfKind)
            .await
            .expect("test: terminal marking runs");

        let stored = bcast
            .get_tx_node(node_id)
            .await
            .expect("test: node is readable")
            .expect("test: node exists");
        assert_eq!(stored.terminal_error, None);
    }

    /// The guard must not block the ordinary case, where nothing has touched the node.
    #[tokio::test(flavor = "multi_thread")]
    async fn unchanged_node_is_persisted() {
        let bcast = get_broadcast_handle();
        let record = single_reveal_record();
        let node_id = record.node_id;
        bcast
            .put_tx_node(record.clone())
            .await
            .expect("test: node persists");

        mark_terminal(&bcast, record, TerminalError::UnsupportedRbfKind)
            .await
            .expect("test: terminal marking runs");

        assert_eq!(
            bcast
                .get_tx_node(node_id)
                .await
                .expect("test: node is readable")
                .expect("test: node exists")
                .terminal_error,
            Some(TerminalError::UnsupportedRbfKind)
        );
    }

    /// Builds a reveal node carrying a pending-signature attempt, plus the payload row it belongs
    /// to at `status`.
    async fn reveal_node_pending_signature(
        status: L1BundleStatus,
    ) -> (ReplacementContext, TxNodeRecord) {
        let envelope_ops = get_envelope_ops();
        let tag = TagData::new(1, 1, vec![]).expect("test: valid tag");
        let payload = L1Payload::new(vec![vec![1u8; 8]], tag).expect("test: valid payload");
        let mut payload_entry = BundledPayloadEntry::new_unsigned(payload);
        payload_entry.status = status;
        envelope_ops
            .put_payload_entry_async(PAYLOAD_IDX, payload_entry)
            .await
            .expect("test: payload row persists");

        let mut record = single_reveal_record();
        record.append_pending_signature_replacement(TxAttempt::active(
            &tx_with_output(800),
            fee_rate(),
            Amount::from_sat(200),
            1,
        ));

        (
            ReplacementContext {
                envelope_ops: Some(envelope_ops),
                ..ReplacementContext::default()
            },
            record,
        )
    }

    /// Regression: the node is written before the payload row, so a stop in between leaves an
    /// attempt only the watcher could advance, and only from `PendingRevealTxSign`. The poll skips
    /// every node carrying one, so without this the chain would never be bumped again.
    #[tokio::test(flavor = "multi_thread")]
    async fn pending_attempt_is_orphaned_when_the_payload_never_reached_pending_sign() {
        let (context, record) = reveal_node_pending_signature(L1BundleStatus::Published).await;

        assert!(pending_attempt_is_orphaned(&context, &record)
            .await
            .expect("test: orphan check runs"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pending_attempt_is_kept_while_the_payload_awaits_its_signature() {
        let (context, record) =
            reveal_node_pending_signature(L1BundleStatus::PendingRevealTxSign(Buf32::zero())).await;

        assert!(!pending_attempt_is_orphaned(&context, &record)
            .await
            .expect("test: orphan check runs"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reveal_kinds_are_always_replaceable() {
        let bcast = get_broadcast_handle();
        let reveal_record = TxNodeRecord::new(
            TxNodeKind::ChunkedEnvelopeReveal {
                envelope_idx: ENVELOPE_IDX,
                reveal_idx: 0,
            },
            TxAttempt::active(&tx_with_output(900), fee_rate(), Amount::from_sat(100), 0),
        );

        assert!(
            commit_replacement_allowed(&reveal_record, &bcast, &ReplacementContext::default())
                .await
                .unwrap()
        );
    }
}
