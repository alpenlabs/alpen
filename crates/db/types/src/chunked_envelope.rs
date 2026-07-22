//! Chunked envelope database interface and its entry record types.

use std::fmt;

// TODO(trey): split ChunkedEnvelopeEntry into different parts for the different types of enveloped and the different stages of processing

use serde::Serialize;
#[cfg(feature = "proxies")]
use strata_db_macros::gen_proxy;
use strata_l1_txfmt::MagicBytes;

use crate::common::{L1TxId, L1WtxId};
#[cfg(feature = "proxies")]
use crate::DbError;
use crate::DbResult;

/// A chunked envelope entry representing a commit tx funding N reveal txs.
///
/// Used for posting large DA blobs that exceed single-transaction limits.
/// The commit tx publishes the EE DA marker via an OP_RETURN at output 0
/// (`magic + version`); each subsequent P2TR output funds a
/// reveal whose tapscript carries one chunk under `<sequencer_pk> OP_CHECKSIG`.
/// Reveals do NOT reference each other; entries are independent across batches.
// FIXME(trey): this merges information from different stages into a single entry, which creates issues where multiple services are writing to the same database object
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChunkedEnvelopeEntry {
    /// Raw chunk payloads, ordered by commit-output index.
    pub chunk_data: Vec<Vec<u8>>,
    /// OP_RETURN magic bytes (4) used in the commit tx.
    pub magic_bytes: MagicBytes,
    /// DA blob version carried in the commit OP_RETURN.
    pub da_blob_version: u32,
    /// Commit transaction ID. Zero if unsigned.
    pub commit_txid: L1TxId,
    /// Witness transaction ID of the commit. Zero if unsigned.
    pub commit_wtxid: L1WtxId,
    /// Per-reveal metadata, ordered by output index. Empty if unsigned.
    pub reveals: Vec<RevealTxMeta>,
    /// Lifecycle status.
    pub status: ChunkedEnvelopeStatus,
}

impl ChunkedEnvelopeEntry {
    /// Creates a new unsigned entry with no transaction metadata.
    ///
    /// Transaction IDs and reveal metadata are populated at signing time by
    /// the watcher.
    pub fn new_unsigned(
        chunk_data: Vec<Vec<u8>>,
        magic_bytes: MagicBytes,
        da_blob_version: u32,
    ) -> Self {
        Self {
            chunk_data,
            magic_bytes,
            da_blob_version,
            commit_txid: L1TxId::zero(),
            commit_wtxid: L1WtxId::zero(),
            reveals: Vec::new(),
            status: ChunkedEnvelopeStatus::Unsigned,
        }
    }
}

impl fmt::Display for ChunkedEnvelopeEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ChunkedEnvelopeEntry(status={}, chunk_count={}, commit_txid={:?}, reveals=[",
            self.status,
            self.chunk_data.len(),
            self.commit_txid
        )?;

        for (idx, reveal) in self.reveals.iter().enumerate() {
            if idx > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{reveal}")?;
        }

        f.write_str("])")
    }
}

/// Metadata for a single reveal transaction within a chunked envelope.
// FIXME(trey): this "meta" type contains non-meta data, the serialized transaction
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RevealTxMeta {
    /// Output index in the commit tx that this reveal spends.
    pub vout_index: u32,
    /// Reveal transaction ID.
    pub txid: L1TxId,
    /// Reveal witness transaction ID.
    pub wtxid: L1WtxId,
    /// Raw signed reveal transaction bytes (consensus-encoded).
    /// Stored here until the commit is published, then added to broadcast DB.
    pub tx_bytes: Vec<u8>,
}

impl fmt::Display for RevealTxMeta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}/{:?}", self.txid, self.wtxid)
    }
}

/// Lifecycle status of a chunked envelope.
///
/// The lifecycle ensures reveals are not broadcast before their parent commit tx
/// is accepted into the mempool. This prevents `InvalidInputs` errors when the
/// commit's outputs aren't yet spendable.
///
/// ```text
/// Unsigned → Unpublished → CommitPublished → Published → Confirmed → Finalized
///                 ↓              ↓
///            NeedsResign    NeedsResign
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChunkedEnvelopeStatus {
    /// Chunk data prepared, transactions not yet created.
    Unsigned,
    /// Commit tx signed and stored in broadcast DB. Reveals are signed but held
    /// locally until commit is published to ensure they don't fail with
    /// `InvalidInputs` due to the commit outputs not yet being spendable.
    Unpublished,
    /// Commit tx is published/confirmed. Reveals are now stored in broadcast DB
    /// and waiting to be published.
    CommitPublished,
    /// All transactions (commit + reveals) broadcast to the mempool.
    Published,
    /// Transactions confirmed with sufficient depth.
    Confirmed,
    /// Fully finalized on L1.
    Finalized,
    /// Input UTXOs were spent; needs fresh signing.
    NeedsResign,
}

impl fmt::Display for ChunkedEnvelopeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsigned => f.write_str("unsigned"),
            Self::Unpublished => f.write_str("unpublished"),
            Self::CommitPublished => f.write_str("commit_published"),
            Self::Published => f.write_str("published"),
            Self::Confirmed => f.write_str("confirmed"),
            Self::Finalized => f.write_str("finalized"),
            Self::NeedsResign => f.write_str("needs_resign"),
        }
    }
}

/// Storage for chunked envelope entries.
///
/// Each entry represents one commit tx funding N reveal txs, tracked through
/// signing, broadcasting, and L1 confirmation.
#[cfg_attr(
    feature = "proxies",
    gen_proxy(error = DbError, tracing_component = "storage:chunked_envelope")
)]
pub trait L1ChunkedEnvelopeDatabase: Send + Sync + 'static {
    /// Stores a [`ChunkedEnvelopeEntry`] at the given index.
    fn put_chunked_envelope_entry(&self, idx: u64, entry: ChunkedEnvelopeEntry) -> DbResult<()>;

    /// Gets a [`ChunkedEnvelopeEntry`] by its index.
    fn get_chunked_envelope_entry(&self, idx: u64) -> DbResult<Option<ChunkedEnvelopeEntry>>;

    /// Gets chunked envelope entries starting from a given index up to a maximum count.
    ///
    /// Returns entries in ascending index order. If `start_idx` doesn't exist,
    /// starts from the next available entry after it.
    fn get_chunked_envelope_entries_from(
        &self,
        start_idx: u64,
        max_count: usize,
    ) -> DbResult<Vec<(u64, ChunkedEnvelopeEntry)>>;

    /// Gets the next available index.
    fn get_next_chunked_envelope_idx(&self) -> DbResult<u64>;

    /// Deletes a single entry by index.
    ///
    /// Returns true if the entry existed and was deleted.
    fn del_chunked_envelope_entry(&self, idx: u64) -> DbResult<bool>;

    /// Deletes all entries from the given index onwards (inclusive).
    ///
    /// Returns indices of deleted entries.
    fn del_chunked_envelope_entries_from_idx(&self, start_idx: u64) -> DbResult<Vec<u64>>;
}

#[cfg(test)]
mod tests {
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
    fn display_chunked_envelope_entry_includes_commit_and_reveals() {
        let commit_bytes = bytes_from_start(0x01);
        let commit_witness_bytes = bytes_from_start(0x21);
        let reveal_bytes = bytes_from_start(0x41);
        let reveal_witness_bytes = bytes_from_start(0x61);
        let entry = ChunkedEnvelopeEntry {
            chunk_data: vec![vec![1], vec![2]],
            magic_bytes: MagicBytes::new([0; 4]),
            da_blob_version: 1,
            commit_txid: L1TxId::from(commit_bytes),
            commit_wtxid: L1WtxId::from(commit_witness_bytes),
            reveals: vec![RevealTxMeta {
                vout_index: 0,
                txid: L1TxId::from(reveal_bytes),
                wtxid: L1WtxId::from(reveal_witness_bytes),
                tx_bytes: Vec::new(),
            }],
            status: ChunkedEnvelopeStatus::Published,
        };

        let display = entry.to_string();
        assert!(display.contains(&reversed_hex(commit_bytes)));
        assert!(display.contains(&reversed_hex(reveal_bytes)));
        assert!(display.contains(&reversed_hex(reveal_witness_bytes)));
        assert!(!display.contains(".."));
    }
}
