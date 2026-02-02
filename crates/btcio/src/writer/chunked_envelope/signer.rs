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

use bitcoin::hashes::Hash;
use bitcoind_async_client::traits::{Reader, Signer, Wallet};
use strata_config::btcio::FeePolicy;
use strata_db_types::types::{
    ChunkedEnvelopeEntry, ChunkedEnvelopeStatus, L1TxEntry, RevealTxMeta,
};
use strata_primitives::buf::Buf32;
use tracing::*;

use super::builder::build_chunked_envelope_txs;
use crate::{
    broadcaster::L1BroadcastHandle,
    writer::{
        builder::{EnvelopeConfig, EnvelopeError, BITCOIN_DUST_LIMIT},
        context::WriterContext,
    },
};

/// Builds and signs a chunked envelope's commit + N reveal transactions.
///
/// The commit tx is signed via wallet RPC. All transactions are inserted into
/// the broadcast database. Returns the updated entry with status
/// [`Unpublished`](ChunkedEnvelopeStatus::Unpublished).
pub(crate) async fn sign_chunked_envelope<R: Reader + Signer + Wallet>(
    entry: &ChunkedEnvelopeEntry,
    broadcast_handle: &L1BroadcastHandle,
    ctx: Arc<WriterContext<R>>,
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
        ctx.params.clone(),
        ctx.sequencer_address.clone(),
        network,
        fee_rate,
        BITCOIN_DUST_LIMIT,
    );

    let built = build_chunked_envelope_txs(
        &env_config,
        &entry.chunk_data,
        &entry.magic_bytes,
        &entry.prev_tail_wtxid,
        utxos,
    )?;

    // Sign commit via bitcoind wallet RPC.
    let signed_commit = ctx
        .client
        .sign_raw_transaction_with_wallet(&built.commit_tx, None)
        .await
        .map_err(|e| EnvelopeError::SignRawTransaction(e.to_string()))?
        .tx;
    let commit_txid: Buf32 = signed_commit.compute_txid().into();

    // Store commit and all reveals in broadcast DB. Not atomic — the watcher
    // handles partial state by falling back to re-signing.
    broadcast_handle
        .put_tx_entry(commit_txid, L1TxEntry::from_tx(&signed_commit))
        .await
        .map_err(|e| EnvelopeError::Other(e.into()))?;

    let mut reveals = Vec::with_capacity(built.reveal_txs.len());
    for (i, reveal_tx) in built.reveal_txs.iter().enumerate() {
        let txid: Buf32 = reveal_tx.compute_txid().into();
        let wtxid: Buf32 = reveal_tx.compute_wtxid().as_byte_array().into();

        broadcast_handle
            .put_tx_entry(txid, L1TxEntry::from_tx(reveal_tx))
            .await
            .map_err(|e| EnvelopeError::Other(e.into()))?;

        reveals.push(RevealTxMeta {
            vout_index: i as u32,
            txid,
            wtxid,
        });
    }

    let mut updated = entry.clone();
    updated.commit_txid = commit_txid;
    updated.reveals = reveals;
    updated.status = ChunkedEnvelopeStatus::Unpublished;
    Ok(updated)
}
