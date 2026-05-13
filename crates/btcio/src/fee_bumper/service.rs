//! Fee-bumper polling service.

use std::{sync::Arc, time::Duration};

use anyhow::bail;
use bitcoin::{
    consensus::{deserialize, serialize},
    hashes::Hash,
    key::Keypair,
    FeeRate, Transaction,
};
use bitcoind_async_client::traits::{Reader, Signer};
use strata_config::btcio::{FeeBumpPolicy, WriterConfig};
use strata_db_types::types::{
    ChunkedEnvelopeEntry, L1TxEntry, L1TxId, L1TxStatus, L1WtxId, RevealTxMeta, TerminalError,
    TxNodeId, TxNodeKind, TxNodeRecord,
};
use strata_primitives::{buf::Buf32, L1Height};
use strata_storage::ops::chunked_envelope::ChunkedEnvelopeOps;
use tokio::time::interval;
use tracing::*;

use super::{
    policy::{evaluate_fee_bump, fee_rate_from_sat_vb, FeeBumpDecision},
    replacement::{
        build_chunked_reveal_replacement, build_wallet_commit_replacement,
        rebuild_reveal_for_replaced_commit, ReplacementError,
    },
};
use crate::broadcaster::L1BroadcastHandle;

/// Optional replacement coordinators for writer-specific transaction kinds.
#[derive(Clone, Default)]
#[expect(
    missing_debug_implementations,
    reason = "ChunkedEnvelopeOps does not expose a useful Debug representation"
)]
pub struct FeeBumperContext {
    /// Chunked-envelope storage used by Alpen EE DA reveal replacements.
    pub chunked_ops: Option<Arc<ChunkedEnvelopeOps>>,
    /// Sequencer keypair used to re-sign Alpen EE DA reveal replacements.
    pub sequencer_keypair: Option<Keypair>,
}

/// Runs the BTCIO fee-bumper loop until the task is cancelled.
pub async fn fee_bumper_task<C>(
    client: Arc<C>,
    writer_config: WriterConfig,
    broadcast_handle: Arc<L1BroadcastHandle>,
    context: FeeBumperContext,
) -> anyhow::Result<()>
where
    C: Reader + Signer + Send + Sync + 'static,
{
    if !matches!(writer_config.fee_bumping.policy, FeeBumpPolicy::Rbf) {
        debug!("BTCIO fee bumper disabled");
        return Ok(());
    }

    let mut ticks = interval(Duration::from_millis(writer_config.write_poll_dur_ms));
    loop {
        ticks.tick().await;
        if let Err(error) = poll_fee_bumper(
            client.as_ref(),
            &writer_config,
            broadcast_handle.as_ref(),
            &context,
        )
        .await
        {
            warn!(%error, "BTCIO fee bumper poll failed");
        }
    }
}

async fn poll_fee_bumper<C>(
    client: &C,
    writer_config: &WriterConfig,
    broadcast_handle: &L1BroadcastHandle,
    context: &FeeBumperContext,
) -> anyhow::Result<()>
where
    C: Reader + Signer,
{
    let current_l1_tip = client.get_block_count().await? as L1Height;
    let estimate_fee_rate_raw_sat_vb = client
        .estimate_smart_fee(writer_config.fee_bumping.target_inclusion_blocks.get())
        .await?;
    let Some(estimate_fee_rate) = fee_rate_from_sat_vb(estimate_fee_rate_raw_sat_vb) else {
        bail!(
            "estimated fee rate {estimate_fee_rate_raw_sat_vb} sat/vB does not fit bitcoin fee-rate type"
        );
    };
    let records = broadcast_handle.get_all_tx_nodes().await?;

    for record in records {
        if record.terminal_error.is_some() {
            continue;
        }
        if record.pending_signature_attempt().is_some() {
            trace!(node_id = ?record.node_id, "tx-node is waiting for external signature");
            continue;
        }
        process_record(
            client,
            writer_config,
            broadcast_handle,
            current_l1_tip,
            estimate_fee_rate,
            record,
            context,
        )
        .await?;
    }

    Ok(())
}

async fn process_record<C>(
    client: &C,
    writer_config: &WriterConfig,
    broadcast_handle: &L1BroadcastHandle,
    current_l1_tip: L1Height,
    estimate_fee_rate: FeeRate,
    mut record: TxNodeRecord,
    context: &FeeBumperContext,
) -> anyhow::Result<()>
where
    C: Reader + Signer,
{
    let active_txid = to_raw_buf32(record.active_txid);
    let Some(active_entry) = broadcast_handle
        .get_tx_entry_by_id_async(active_txid)
        .await?
    else {
        warn!(node_id = ?record.node_id, active_txid = ?record.active_txid, "active tx-node entry missing from broadcast db");
        return Ok(());
    };

    if active_entry.status != L1TxStatus::Published {
        return Ok(());
    }

    if mark_first_published_height(&mut record, current_l1_tip) {
        broadcast_handle.put_tx_node(record).await?;
        return Ok(());
    }

    let Some(active_attempt) = record.active_attempt() else {
        warn!(node_id = ?record.node_id, "tx-node record has no active attempt");
        return Ok(());
    };
    let active_tx = active_attempt.try_to_tx()?;

    let decision = evaluate_fee_bump(
        &writer_config.fee_bumping,
        &record,
        active_attempt,
        current_l1_tip,
        estimate_fee_rate,
        active_tx.vsize(),
    );

    match decision {
        FeeBumpDecision::Wait => Ok(()),
        FeeBumpDecision::Terminal(error) => {
            mark_terminal(broadcast_handle, record, error).await?;
            Ok(())
        }
        FeeBumpDecision::Replace(request) => {
            if matches!(record.kind, TxNodeKind::ChunkedEnvelopeReveal { .. }) {
                return replace_chunked_reveal(
                    writer_config,
                    broadcast_handle,
                    context,
                    record,
                    request.target_fee_rate,
                    request.attempt_no,
                )
                .await;
            }

            if !commit_replacement_allowed(&record, broadcast_handle).await? {
                debug!(node_id = ?record.node_id, kind = ?record.kind, "commit replacement skipped after dependent reveal activity");
                return Ok(());
            }

            let replacement = match build_wallet_commit_replacement(
                client,
                &record.kind,
                record.active_txid,
                request.target_fee_rate_sat_vb,
                request.attempt_no,
            )
            .await
            {
                Ok(replacement) => replacement,
                Err(error @ ReplacementError::UnsupportedKind(_)) => {
                    mark_terminal(broadcast_handle, record, error.terminal_error()).await?;
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
            if matches!(record.kind, TxNodeKind::ChunkedEnvelopeCommit { .. }) {
                if let Err(error) =
                    update_chunked_commit_replacement_metadata(context, &record, &replacement_tx)
                        .await
                {
                    warn!(node_id = ?record.node_id, %error, "failed to update chunked commit replacement metadata");
                    mark_terminal(broadcast_handle, record, TerminalError::UnsupportedRbfKind)
                        .await?;
                    return Ok(());
                }
            }
            let replacement_txid = to_raw_buf32(replacement.txid);
            let replacement_entry = L1TxEntry::from_tx_with_fee_rate(
                &replacement_tx,
                FeeRate::from_sat_per_vb(request.target_fee_rate_sat_vb)
                    .expect("target fee rate was validated by replacement builder"),
            );
            broadcast_handle
                .put_tx_entry(replacement_txid, replacement_entry)
                .await?;

            let mut replaced_entry = active_entry_after_build;
            replaced_entry.status = L1TxStatus::Replaced {
                by: replacement_txid,
            };
            broadcast_handle
                .update_tx_entry_by_id_async(active_txid, replaced_entry)
                .await?;

            record.append_replacement(replacement);
            broadcast_handle.put_tx_node(record).await?;
            Ok(())
        }
    }
}

async fn replace_chunked_reveal(
    writer_config: &WriterConfig,
    broadcast_handle: &L1BroadcastHandle,
    context: &FeeBumperContext,
    mut record: TxNodeRecord,
    target_fee_rate: FeeRate,
    attempt_no: u32,
) -> anyhow::Result<()> {
    let TxNodeKind::ChunkedEnvelopeReveal {
        envelope_idx,
        reveal_idx,
    } = record.kind
    else {
        return Ok(());
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
    let Some(reveal_meta) = envelope_entry.reveals.get(reveal_idx as usize).cloned() else {
        warn!(
            envelope_idx,
            reveal_idx, "chunked reveal metadata missing for replacement"
        );
        return Ok(());
    };
    let Some((_, commit_entry)) = broadcast_handle
        .get_active_tx_entry_by_id_async(to_raw_buf32(envelope_entry.commit_txid))
        .await?
    else {
        debug!(
            envelope_idx,
            reveal_idx, "commit entry missing for reveal replacement"
        );
        return Ok(());
    };
    let commit_tx = commit_entry.try_to_tx()?;
    let Some(commit_output) = commit_tx.output.get(reveal_meta.vout_index as usize) else {
        mark_terminal(broadcast_handle, record, TerminalError::UnsupportedRbfKind).await?;
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
        commit_output,
        target_fee_rate,
        attempt_no,
        sequencer_keypair,
    ) {
        Ok(replacement) => replacement,
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
            envelope_idx,
            reveal_idx, "original reveal confirmed before replacement was persisted"
        );
        return Ok(());
    }

    let replacement_tx = replacement.try_to_tx()?;
    let replacement_txid = replacement.txid;
    let replacement_txid_raw = to_raw_buf32(replacement_txid);
    let replacement_entry = L1TxEntry::from_tx_with_fee_rate(&replacement_tx, target_fee_rate);
    broadcast_handle
        .put_tx_entry(replacement_txid_raw, replacement_entry)
        .await?;

    let mut replaced_entry = active_entry_after_build;
    replaced_entry.status = L1TxStatus::Replaced {
        by: replacement_txid,
    };
    broadcast_handle
        .update_tx_entry_by_id_async(active_txid, replaced_entry)
        .await?;

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

async fn update_chunked_commit_replacement_metadata(
    context: &FeeBumperContext,
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
    record.set_terminal_error(error);
    broadcast_handle.put_tx_node(record).await?;
    Ok(())
}

async fn commit_replacement_allowed(
    record: &TxNodeRecord,
    broadcast_handle: &L1BroadcastHandle,
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
            let nodes = broadcast_handle.get_all_tx_nodes().await?;
            Ok(!nodes.iter().any(|node| {
                matches!(
                    node.kind,
                    TxNodeKind::ChunkedEnvelopeReveal {
                        envelope_idx: node_envelope_idx,
                        ..
                    } if node_envelope_idx == envelope_idx
                )
            }))
        }
        TxNodeKind::SingleEnvelopeReveal { .. } | TxNodeKind::ChunkedEnvelopeReveal { .. } => {
            Ok(true)
        }
    }
}

async fn reveal_node_not_handed_to_broadcaster(
    node_id: TxNodeId,
    broadcast_handle: &L1BroadcastHandle,
) -> anyhow::Result<bool> {
    let Some(reveal_node) = broadcast_handle.get_tx_node(node_id).await? else {
        return Ok(true);
    };
    let Some((_, entry)) = broadcast_handle
        .get_active_tx_entry_by_id_async(to_raw_buf32(reveal_node.active_txid))
        .await?
    else {
        return Ok(true);
    };
    Ok(matches!(entry.status, L1TxStatus::Unpublished))
}

fn to_raw_buf32(txid: L1TxId) -> Buf32 {
    Buf32(txid.0)
}
