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
    ChunkedEnvelopeEntry, ChunkedEnvelopeStatus, L1TxEntry, L1TxId, L1WtxId, RevealTxMeta,
};
use tracing::*;

use super::{builder::build_chunked_envelope_txs, context::ChunkedWriterContext};
use crate::writer::{
    builder::{EnvelopeConfig, EnvelopeError, BITCOIN_DUST_LIMIT},
    fees::resolve_fee_rate,
};

fn format_reveal_refs(reveals: &[RevealTxMeta]) -> Vec<String> {
    reveals
        .iter()
        .map(|reveal| format!("{:?}/{:?}", reveal.txid, reveal.wtxid))
        .collect()
}

fn to_l1_txid(txid: bitcoin::Txid) -> L1TxId {
    L1TxId::from(txid.to_buf32().0)
}

fn to_l1_wtxid(wtxid: bitcoin::Wtxid) -> L1WtxId {
    L1WtxId::from(wtxid.to_buf32().0)
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
            .map_err(|e| EnvelopeError::PrereqFetch(e.into()))?;

        // NOTE: passing `min_conf = 0` would also include unconfirmed UTXOs and mitigate the
        // lack-of-UTXO problem, but it complicates fee bumping (RBF/CPFP over chained unconfirmed
        // ancestors) and has mempool-policy and reorg-safety implications, so it is left as a
        // future consideration.
        let utxos = ctx
            .client
            .list_unspent(None, None, None, None, None)
            .await
            .map_err(|e| EnvelopeError::PrereqFetch(e.into()))?
            .0;

        let spendable_utxo_count = utxos
            .iter()
            .filter(|u| u.spendable && u.solvable && u.amount.to_sat() > BITCOIN_DUST_LIMIT as i64)
            .count();

        let spendable_value_sats: i64 = utxos
            .iter()
            .filter(|u| u.spendable && u.solvable && u.amount.to_sat() > BITCOIN_DUST_LIMIT as i64)
            .map(|u| u.amount.to_sat())
            .sum();

        let fee_rate = resolve_fee_rate(ctx.client.as_ref(), ctx.config.as_ref())
            .await
            .map_err(EnvelopeError::PrereqFetch)?;

        debug!(
            envelope_idx,
            chunk_count = entry.chunk_data.len(),
            utxo_count = utxos.len(),
            spendable_utxo_count,
            spendable_value_sats,
            fee_rate,
            "loaded wallet state for chunked envelope signing"
        );

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
            .map_err(EnvelopeError::SignRawTransaction)?
            .tx;
        let commit_txid = to_l1_txid(signed_commit.compute_txid());
        let commit_wtxid = to_l1_wtxid(signed_commit.compute_wtxid());

        // Store reveal metadata and raw bytes locally. They'll be added to broadcast
        // DB by the watcher after commit is published.
        let mut reveals = Vec::with_capacity(built.reveal_txs.len());
        for (i, reveal_tx) in built.reveal_txs.iter().enumerate() {
            let txid = to_l1_txid(reveal_tx.compute_txid());
            let wtxid = to_l1_wtxid(reveal_tx.compute_wtxid());
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
            ?commit_txid,
            ?commit_wtxid,
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

#[cfg(test)]
mod tests {
    use strata_db_types::types::RevealTxMeta;

    use super::*;

    fn bytes_from_start(start: u8) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        for (idx, byte) in bytes.iter_mut().enumerate() {
            *byte = start.wrapping_add(idx as u8);
        }
        bytes
    }

    fn reversed_hex(bytes: [u8; 32]) -> String {
        bytes
            .into_iter()
            .rev()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    #[test]
    fn format_reveal_refs_uses_full_reversed_hex() {
        let txid_bytes = bytes_from_start(0x10);
        let wtxid_bytes = bytes_from_start(0x40);
        let reveals = vec![RevealTxMeta {
            vout_index: 1,
            txid: L1TxId::from(txid_bytes),
            wtxid: L1WtxId::from(wtxid_bytes),
            tx_bytes: Vec::new(),
        }];

        assert_eq!(
            format_reveal_refs(&reveals),
            vec![format!(
                "{}/{}",
                reversed_hex(txid_bytes),
                reversed_hex(wtxid_bytes)
            )]
        );
    }
}
