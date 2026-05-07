//! Builds and signs a chunked envelope for lifecycle persistence.
//!
//! **Signing strategy:**
//! - **Commit tx**: Signed via bitcoind wallet RPC (`sign_raw_transaction_with_wallet`) because its
//!   inputs are wallet-managed UTXOs.
//! - **Reveal txs**: Signed in [`build_chunked_envelope_txs`] under the EE sequencer's BIP340
//!   Schnorr key (the same key used for gossip preconfirmation packages). Each reveal's tapscript
//!   is `<sequencer_pk> OP_CHECKSIG OP_FALSE OP_IF <chunk> OP_ENDIF`. The spend signature
//!   transitively authenticates the commit because reveals spend commit outputs.

use std::sync::Arc;

use bitcoin::consensus::encode::serialize as btc_serialize;
use bitcoind_async_client::traits::{Reader, Signer, Wallet};
use strata_btc_types::{TxidExt, WtxidExt};
use strata_db_types::types::{
    ChunkedEnvelopeEntry, ChunkedEnvelopeStatus, L1TxEntry, RevealTxMeta,
};
use strata_primitives::buf::Buf32;
use tracing::*;

use super::{builder::build_chunked_envelope_txs, context::ChunkedWriterContext};
use crate::writer::{
    builder::{EnvelopeConfig, EnvelopeError, BITCOIN_DUST_LIMIT},
    fees::resolve_fee_rate,
};

fn format_reveal_refs(reveals: &[RevealTxMeta]) -> Vec<String> {
    reveals
        .iter()
        .map(|reveal| format!("{}/{}", reveal.txid, reveal.wtxid))
        .collect()
}

/// Signed chunked-envelope metadata ready for lifecycle persistence.
pub(crate) struct SignedChunkedEnvelope {
    /// Updated chunked-envelope row containing txids, reveal bytes, and status.
    pub entry: ChunkedEnvelopeEntry,
    /// Wallet-signed commit tx entry, initially stored as unpublished.
    pub commit_tx_entry: L1TxEntry,
}

/// Builds and signs a chunked envelope's commit + N reveal transactions.
///
/// The commit tx is signed via wallet RPC and returned as a broadcast DB entry.
/// Reveal txs are signed under the sequencer keypair and stored in the returned
/// chunked-envelope row with raw bytes. The lifecycle driver persists these
/// artifacts before attempting immediate commit publication.
///
/// Returns the updated entry with status [`Unpublished`](ChunkedEnvelopeStatus::Unpublished).
pub(crate) async fn sign_chunked_envelope<R: Reader + Signer + Wallet>(
    envelope_idx: u64,
    entry: &ChunkedEnvelopeEntry,
    ctx: Arc<ChunkedWriterContext<R>>,
) -> Result<SignedChunkedEnvelope, EnvelopeError> {
    let sign_chunked_envelope_span = debug_span!(
        "btcio_chunked_envelope_sign",
        envelope_idx,
        chunk_count = entry.chunk_data.len(),
    );

    async {
        trace!("signing chunked envelope");

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

        let fee_rate = resolve_fee_rate(ctx.client.as_ref(), ctx.config.as_ref())
            .await
            .map_err(EnvelopeError::Other)?;

        let env_config = EnvelopeConfig::new(
            ctx.btcio_params.magic_bytes,
            ctx.sequencer_address.clone(),
            network,
            fee_rate,
            BITCOIN_DUST_LIMIT,
            None,
        );

        let built = build_chunked_envelope_txs(
            &env_config,
            &entry.chunk_data,
            &entry.magic_bytes,
            entry.da_blob_version,
            &ctx.sequencer_keypair,
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
        let commit_wtxid: Buf32 = signed_commit.compute_wtxid().to_buf32();

        // Store reveal metadata and raw bytes locally. They'll be added to broadcast
        // DB by the watcher after commit is published.
        let mut reveals = Vec::with_capacity(built.reveal_txs.len());
        for (i, reveal_tx) in built.reveal_txs.iter().enumerate() {
            let txid: Buf32 = reveal_tx.compute_txid().to_buf32();
            let wtxid: Buf32 = reveal_tx.compute_wtxid().to_buf32();
            let tx_bytes = btc_serialize(reveal_tx);

            // vout_index is i+1 because vout 0 is the commit OP_RETURN.
            reveals.push(RevealTxMeta {
                vout_index: (i + 1) as u32,
                txid,
                wtxid,
                tx_bytes,
            });
        }

        let reveal_refs = format_reveal_refs(&reveals);
        debug!(
            %commit_txid,
            %commit_wtxid,
            reveal_count = reveals.len(),
            ?reveal_refs,
            "signed chunked envelope, ready for persistence"
        );

        let mut updated = entry.clone();
        updated.commit_txid = commit_txid;
        updated.commit_wtxid = commit_wtxid;
        updated.reveals = reveals;
        updated.status = ChunkedEnvelopeStatus::Unpublished;
        Ok(SignedChunkedEnvelope {
            entry: updated,
            commit_tx_entry: L1TxEntry::from_tx(&signed_commit),
        })
    }
    .instrument(sign_chunked_envelope_span)
    .await
}
