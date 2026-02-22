//! Builds and signs a chunked envelope, then stores transactions in the broadcast database.
//!
//! This module orchestrates the full build-sign-store pipeline for a
//! chunked envelope entry — matching the pattern used by the parent single-reveal
//! [`signer`](super::super::signer) module.
//!
//! **Signing strategy:**
//! - **Commit tx**: Signed via bitcoind wallet RPC (`sign_raw_transaction_with_wallet`) because its
//!   inputs are wallet-managed UTXOs.
//! - **Reveal txs**: Pre-signed in [`build_chunked_envelope_txs`] using ephemeral in-memory
//!   keypairs (one per reveal). Each reveal spends a P2TR output created by the commit, so the
//!   ephemeral key only needs to live long enough to produce the tapscript spend signature. This
//!   matches the existing single-reveal approach.

use std::sync::Arc;

use bitcoin::consensus::encode::serialize as btc_serialize;
use bitcoind_async_client::traits::{Reader, Signer, Wallet};
use strata_btc_types::{TxidExt, WtxidExt};
use strata_config::btcio::FeePolicy;
use strata_db_types::types::{
    ChunkedEnvelopeEntry, ChunkedEnvelopeStatus, L1TxEntry, RevealTxMeta,
};
use strata_primitives::buf::Buf32;
use tracing::*;

use super::{builder::build_chunked_envelope_txs, context::ChunkedWriterContext};
use crate::{
    broadcaster::L1BroadcastHandle,
    writer::builder::{EnvelopeConfig, EnvelopeError, BITCOIN_DUST_LIMIT},
};

/// Builds and signs a chunked envelope's commit + N reveal transactions.
///
/// The commit tx is signed via wallet RPC and stored in the broadcast database.
/// Reveal txs are signed and stored in the entry's `reveals` field (with raw bytes),
/// but are NOT added to the broadcast DB yet. The watcher will add them after
/// the commit tx is published, preventing `InvalidInputs` errors.
///
/// Returns the updated entry with status [`Unpublished`](ChunkedEnvelopeStatus::Unpublished).
pub(crate) async fn sign_chunked_envelope<R: Reader + Signer + Wallet>(
    entry: &ChunkedEnvelopeEntry,
    prev_tail_wtxid: Buf32,
    broadcast_handle: &L1BroadcastHandle,
    ctx: Arc<ChunkedWriterContext<R>>,
) -> Result<ChunkedEnvelopeEntry, EnvelopeError> {
    trace!(
        chunk_count = entry.chunk_data.len(),
        "signing chunked envelope"
    );

    let network = ctx
        .client
        .network()
        .await
        .map_err(|e| EnvelopeError::Other(e.into()))?;
    let utxos = ctx
        .client
        .list_unspent(None, None, None, None, None)
        .await
        .map_err(|e| EnvelopeError::Other(e.into()))?
        .0;

    let fee_rate = match ctx.config.fee_policy {
        FeePolicy::Smart => {
            ctx.client
                .estimate_smart_fee(1)
                .await
                .map_err(|e| EnvelopeError::Other(e.into()))?
                * 2
        }
        FeePolicy::Fixed(val) => val,
    };

    let env_config = EnvelopeConfig::new(
        ctx.btcio_params.magic_bytes,
        ctx.sequencer_address.clone(),
        network,
        fee_rate,
        BITCOIN_DUST_LIMIT,
    );

    let built = build_chunked_envelope_txs(
        &env_config,
        &entry.chunk_data,
        &entry.magic_bytes,
        &prev_tail_wtxid,
        utxos,
    )?;

    // Sign commit via bitcoind wallet RPC.
    let signed_commit = ctx
        .client
        .sign_raw_transaction_with_wallet(&built.commit_tx, None)
        .await
        .map_err(|e| EnvelopeError::SignRawTransaction(e.to_string()))?
        .tx;
    let commit_txid: Buf32 = signed_commit.compute_txid().to_buf32();

    // Store ONLY the commit tx in broadcast DB. Reveals will be added after
    // commit is published to prevent InvalidInputs errors.
    broadcast_handle
        .put_tx_entry(commit_txid, L1TxEntry::from_tx(&signed_commit))
        .await
        .map_err(|e| EnvelopeError::Other(e.into()))?;

    // Store reveal metadata and raw bytes locally. They'll be added to broadcast
    // DB by the watcher after commit is published.
    let mut reveals = Vec::with_capacity(built.reveal_txs.len());
    for (i, reveal_tx) in built.reveal_txs.iter().enumerate() {
        let txid: Buf32 = reveal_tx.compute_txid().to_buf32();
        let wtxid: Buf32 = reveal_tx.compute_wtxid().to_buf32();
        let tx_bytes = btc_serialize(reveal_tx);

        reveals.push(RevealTxMeta {
            vout_index: i as u32,
            txid,
            wtxid,
            tx_bytes,
        });
    }

    debug!(
        commit_txid = %commit_txid,
        reveal_count = reveals.len(),
        "signed chunked envelope, commit stored, reveals pending"
    );

    let mut updated = entry.clone();
    updated.prev_tail_wtxid = prev_tail_wtxid;
    updated.commit_txid = commit_txid;
    updated.reveals = reveals;
    updated.status = ChunkedEnvelopeStatus::Unpublished;
    Ok(updated)
}
