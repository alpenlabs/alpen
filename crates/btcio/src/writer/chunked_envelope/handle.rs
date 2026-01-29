//! Handle for chunked envelope publication operations.

use std::sync::Arc;

use bitcoin::{hashes::Hash as _, Network, Txid};
use bitcoind_async_client::traits::{Broadcaster, Reader, Signer, Wallet};
use strata_db_types::{
    traits::L1DaBlobDatabase,
    types::{DaBlobEntry, DaBlobStatusDb, DaChunkEntry, L1BundleStatus, L1TxEntry},
};
use strata_primitives::buf::Buf32;
use tracing::{debug, info};

use super::{
    builder::{build_chunked_envelope_txs, ChunkedEnvelopeConfig},
    types::{ChunkedPayloadIntent, ChunkedSubmissionResult, DaBlobStatus, DaStatus},
    ChunkedEnvelopeError,
};
use crate::broadcaster::L1BroadcastHandle;

/// Handle for submitting and tracking chunked envelope publications.
///
/// This is the main entry point for sequencer to publish DA payloads.
#[derive(Clone)]
#[expect(
    missing_debug_implementations,
    reason = "L1BroadcastHandle doesn't implement Debug"
)]
pub struct ChunkedEnvelopeHandle<C, D> {
    /// Bitcoin RPC client.
    client: Arc<C>,
    /// DA blob database.
    da_db: Arc<D>,
    /// Broadcast handle for submitting transactions.
    broadcast_handle: Arc<L1BroadcastHandle>,
    /// Sequencer address for change outputs.
    sequencer_address: bitcoin::Address,
    /// Bitcoin network.
    network: Network,
}

impl<C, D> ChunkedEnvelopeHandle<C, D>
where
    C: Reader + Broadcaster + Wallet + Signer + Send + Sync + 'static,
    D: L1DaBlobDatabase + Send + Sync + 'static,
{
    /// Creates a new chunked envelope handle.
    pub fn new(
        client: Arc<C>,
        da_db: Arc<D>,
        broadcast_handle: Arc<L1BroadcastHandle>,
        sequencer_address: bitcoin::Address,
        network: Network,
    ) -> Self {
        Self {
            client,
            da_db,
            broadcast_handle,
            sequencer_address,
            network,
        }
    }

    /// Submits a payload for chunked publication.
    ///
    /// This will:
    /// 1. Compute payload_hash = sha256(payload)
    /// 2. Split into chunks if payload > MAX_CHUNK_PAYLOAD
    /// 3. Build batched commit tx (1 commit with N outputs)
    /// 4. Build all reveal txs sequentially (for prev_chunk_wtxid linking)
    /// 5. Submit transactions to broadcaster
    ///
    /// Idempotent: if payload_hash already exists, returns existing submission.
    pub async fn submit_payload(
        &self,
        intent: ChunkedPayloadIntent,
    ) -> Result<ChunkedSubmissionResult, ChunkedEnvelopeError> {
        let payload_hash = intent.compute_payload_hash();
        let blob_id: Buf32 = payload_hash;
        let total_chunks = intent.chunk_count();

        info!(
            %payload_hash,
            total_chunks,
            payload_size = intent.payload().len(),
            "Submitting chunked envelope"
        );

        // Check if payload_hash already exists in DB (idempotent)
        if let Some(entry) = self
            .da_db
            .get_da_blob(&blob_id)
            .map_err(ChunkedEnvelopeError::Database)?
        {
            debug!(
                %payload_hash,
                "Payload already exists in DB, returning existing submission"
            );
            return entry_to_submission(&entry, self.da_db.as_ref());
        }

        // Get fee rate
        let fee_rate = self
            .client
            .estimate_smart_fee(1)
            .await
            .map_err(|e| ChunkedEnvelopeError::Other(e.into()))?;

        // Get UTXOs
        let utxos = self
            .client
            .list_unspent(None, None, None, None, None)
            .await
            .map_err(|e| ChunkedEnvelopeError::Other(e.into()))?
            .0;

        // Build config
        let config = ChunkedEnvelopeConfig::new(
            self.sequencer_address.clone(),
            self.network,
            fee_rate * 2, // 2x for faster confirmation
        );

        // Build all transactions
        let built = build_chunked_envelope_txs(&intent, utxos, &config)?;

        let commit_txid_bitcoin: Txid = built.commit_tx.compute_txid();
        let commit_txid: Buf32 = commit_txid_bitcoin.into();

        info!(
            %payload_hash,
            %commit_txid,
            total_chunks,
            "Built chunked envelope transactions"
        );

        // Sign commit transaction using wallet
        let signed_commit = self
            .client
            .sign_raw_transaction_with_wallet(&built.commit_tx, None)
            .await
            .map_err(|e| ChunkedEnvelopeError::Other(e.into()))?;

        if !signed_commit.complete {
            return Err(ChunkedEnvelopeError::Signing(
                "commit tx signing incomplete".to_string(),
            ));
        }

        let commit_tx = signed_commit.tx;
        let commit_txid_bitcoin: Txid = commit_tx.compute_txid();
        let commit_txid: Buf32 = commit_txid_bitcoin.into();

        // Submit commit transaction
        let commit_entry = L1TxEntry::from_tx(&commit_tx);
        self.broadcast_handle
            .put_tx_entry(commit_txid, commit_entry)
            .await
            .map_err(ChunkedEnvelopeError::Database)?;

        // Get last chunk wtxid from previous blob (for cross-blob linking)
        let prev_chunk_wtxid = self
            .da_db
            .get_da_last_chunk_wtxid(intent.op_return_tag())
            .map_err(ChunkedEnvelopeError::Database)?
            .unwrap_or([0u8; 32]);

        // Store chunk entries and submit reveal transactions
        let mut da_chunk_indices = Vec::with_capacity(built.reveal_txs.len());
        let mut chunk_wtxids = Vec::with_capacity(built.reveal_txs.len());

        for (i, reveal_tx) in built.reveal_txs.iter().enumerate() {
            let reveal_txid: Buf32 = reveal_tx.compute_txid().into();
            let reveal_wtxid = reveal_tx.compute_wtxid();
            let reveal_wtxid_bytes: [u8; 32] = reveal_wtxid.to_byte_array();
            let reveal_entry = L1TxEntry::from_tx(reveal_tx);

            // Submit to broadcaster
            self.broadcast_handle
                .put_tx_entry(reveal_txid, reveal_entry)
                .await
                .map_err(ChunkedEnvelopeError::Database)?;

            // Get next chunk index and store chunk entry
            let chunk_idx = self
                .da_db
                .get_next_da_chunk_idx()
                .map_err(ChunkedEnvelopeError::Database)?;

            let chunk_entry = DaChunkEntry {
                blob_id,
                chunk_index: i as u16,
                total_chunks,
                payload_hash: payload_hash.0,
                prev_chunk_wtxid: if i == 0 {
                    prev_chunk_wtxid
                } else {
                    // Previous chunk's wtxid in this blob
                    built.reveal_txs[i - 1].compute_wtxid().to_byte_array()
                },
                reveal_wtxid: Some(reveal_wtxid_bytes),
                commit_txid,
                reveal_txid,
                status: L1BundleStatus::Unpublished,
                confirmed_height: None,
            };

            self.da_db
                .put_da_chunk(chunk_idx, chunk_entry)
                .map_err(ChunkedEnvelopeError::Database)?;
            da_chunk_indices.push(chunk_idx);
            chunk_wtxids.push(reveal_wtxid);

            info!(
                %payload_hash,
                chunk_index = i,
                %reveal_txid,
                "Submitted reveal transaction"
            );
        }

        // Get last chunk wtxid for cross-blob linking
        let last_chunk_wtxid = built
            .reveal_txs
            .last()
            .map(|tx| tx.compute_wtxid().to_byte_array())
            .unwrap_or([0u8; 32]);

        // Store blob entry
        let blob_entry = DaBlobEntry {
            blob_id,
            blob_hash: payload_hash.0,
            blob_size: intent.payload().len() as u64,
            total_chunks,
            tag: intent.op_return_tag(),
            status: DaBlobStatusDb::Pending,
            da_chunk_indices,
            commit_txid,
            last_chunk_wtxid,
            retry_count: 0,
            last_error: None,
        };

        self.da_db
            .put_da_blob(&blob_id, blob_entry)
            .map_err(ChunkedEnvelopeError::Database)?;

        // Update last chunk wtxid for this tag
        self.da_db
            .put_da_last_chunk_wtxid(intent.op_return_tag(), last_chunk_wtxid)
            .map_err(ChunkedEnvelopeError::Database)?;

        let submission = ChunkedSubmissionResult {
            payload_hash,
            total_chunks,
            chunk_wtxids,
            commit_txid: commit_txid_bitcoin,
        };

        info!(
            %payload_hash,
            commit_txid = %submission.commit_txid,
            total_chunks = submission.total_chunks,
            "Chunked envelope submission complete"
        );

        Ok(submission)
    }

    /// Checks publication status of a payload (simple status for sequencer).
    ///
    /// Returns `Published` when all reveals have at least 1 confirmation.
    pub async fn check_status(
        &self,
        payload_hash: Buf32,
    ) -> Result<DaStatus, ChunkedEnvelopeError> {
        let entry = self
            .da_db
            .get_da_blob(&payload_hash)
            .map_err(ChunkedEnvelopeError::Database)?;

        match entry {
            Some(e) => Ok(status_db_to_da_status(&e.status)),
            None => Ok(DaStatus::Pending), // Not found = not yet submitted
        }
    }

    /// Gets detailed internal status (for monitoring/debugging).
    pub async fn get_detailed_status(
        &self,
        payload_hash: Buf32,
    ) -> Result<Option<DaBlobStatus>, ChunkedEnvelopeError> {
        let entry = self
            .da_db
            .get_da_blob(&payload_hash)
            .map_err(ChunkedEnvelopeError::Database)?;

        match entry {
            Some(e) => Ok(Some(status_db_to_blob_status(&e.status))),
            None => Ok(None),
        }
    }

    /// Gets submission details for a payload.
    pub async fn get_submission(
        &self,
        payload_hash: Buf32,
    ) -> Result<Option<ChunkedSubmissionResult>, ChunkedEnvelopeError> {
        let entry = self
            .da_db
            .get_da_blob(&payload_hash)
            .map_err(ChunkedEnvelopeError::Database)?;

        match entry {
            Some(e) => Ok(Some(entry_to_submission(&e, self.da_db.as_ref())?)),
            None => Ok(None),
        }
    }
}

/// Converts a database blob entry to a submission result.
fn entry_to_submission<D: L1DaBlobDatabase>(
    entry: &DaBlobEntry,
    da_db: &D,
) -> Result<ChunkedSubmissionResult, ChunkedEnvelopeError> {
    // Collect wtxids from chunks
    let mut chunk_wtxids = Vec::with_capacity(entry.da_chunk_indices.len());
    for &chunk_idx in &entry.da_chunk_indices {
        if let Some(chunk) = da_db.get_da_chunk(chunk_idx).map_err(ChunkedEnvelopeError::Database)? {
            if let Some(wtxid_bytes) = chunk.reveal_wtxid {
                chunk_wtxids.push(bitcoin::Wtxid::from_byte_array(wtxid_bytes));
            }
        }
    }

    Ok(ChunkedSubmissionResult {
        payload_hash: entry.blob_hash.into(),
        total_chunks: entry.total_chunks,
        chunk_wtxids,
        commit_txid: Txid::from_byte_array(entry.commit_txid.0),
    })
}

/// Converts database status to DA status.
fn status_db_to_da_status(status: &DaBlobStatusDb) -> DaStatus {
    match status {
        DaBlobStatusDb::Pending => DaStatus::Pending,
        DaBlobStatusDb::CommitConfirmed { .. } => DaStatus::Pending,
        DaBlobStatusDb::AllRevealsConfirmed => DaStatus::Published,
        DaBlobStatusDb::Finalized => DaStatus::Published,
        DaBlobStatusDb::Failed(reason) => DaStatus::Failed {
            reason: reason.clone(),
        },
    }
}

/// Converts database status to blob status.
fn status_db_to_blob_status(status: &DaBlobStatusDb) -> DaBlobStatus {
    match status {
        DaBlobStatusDb::Pending => DaBlobStatus::Pending,
        DaBlobStatusDb::CommitConfirmed { reveals_confirmed } => DaBlobStatus::CommitConfirmed {
            reveals_confirmed: *reveals_confirmed,
        },
        DaBlobStatusDb::AllRevealsConfirmed => DaBlobStatus::AllRevealsConfirmed,
        DaBlobStatusDb::Finalized => DaBlobStatus::Finalized,
        DaBlobStatusDb::Failed(reason) => DaBlobStatus::Failed(reason.clone()),
    }
}
