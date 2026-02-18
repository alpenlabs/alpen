#![expect(deprecated, reason = "legacy old code is retained for compatibility")] // I have no idea how to make clippy be happy with precise expects in this module
//! Module for database local types

use std::fmt;

use arbitrary::Arbitrary;
use bitcoin::{
    consensus::{self, deserialize, serialize},
    Transaction,
};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_checkpoint_types::{BatchInfo, BatchTransition, Checkpoint, CheckpointSidecar};
use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_csm_types::{CheckpointL1Ref, L1Payload, PayloadIntent};
use strata_identifiers::OLTxId;
use strata_l1_txfmt::MagicBytes;
use strata_ol_chainstate_types::Chainstate;
use strata_primitives::buf::Buf32;
use zkaleido::Proof;

/// Represents an intent to publish to some DA, which will be bundled for efficiency.
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct IntentEntry {
    pub intent: PayloadIntent,
    pub status: IntentStatus,
}

impl IntentEntry {
    pub fn new_unbundled(intent: PayloadIntent) -> Self {
        Self {
            intent,
            status: IntentStatus::Unbundled,
        }
    }

    pub fn new_bundled(intent: PayloadIntent, bundle_idx: u64) -> Self {
        Self {
            intent,
            status: IntentStatus::Bundled(bundle_idx),
        }
    }

    pub fn payload(&self) -> &L1Payload {
        self.intent.payload()
    }
}

/// Status of Intent indicating various stages of being bundled to L1 transaction.
/// Unbundled Intents are collected and bundled to create [`BundledPayloadEntry`].
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub enum IntentStatus {
    // It is not bundled yet, and thus will be collected and processed by bundler.
    Unbundled,
    // It has been bundled to [`BundledPayloadEntry`] with given bundle idx.
    Bundled(u64),
}

/// Represents data for a payload we're still planning to post to L1.
#[derive(Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct BundledPayloadEntry {
    pub payload: L1Payload,
    pub commit_txid: Buf32,
    pub reveal_txid: Buf32,
    pub status: L1BundleStatus,
}

impl BundledPayloadEntry {
    pub fn new(
        payload: L1Payload,
        commit_txid: Buf32,
        reveal_txid: Buf32,
        status: L1BundleStatus,
    ) -> Self {
        Self {
            payload,
            commit_txid,
            reveal_txid,
            status,
        }
    }

    /// Create new unsigned [`BundledPayloadEntry`].
    ///
    /// NOTE: This won't have commit - reveal pairs associated with it.
    ///   Because it is better to defer gathering utxos as late as possible to prevent being spent
    ///   by others. Those will be created and signed in a single step.
    pub fn new_unsigned(payload: L1Payload) -> Self {
        let cid = Buf32::zero();
        let rid = Buf32::zero();
        Self::new(payload, cid, rid, L1BundleStatus::Unsigned)
    }
}

// Custom debug implementation to print commit_txid and reveal_txid in little endian
impl fmt::Debug for BundledPayloadEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let commit_txid_le = {
            let mut bytes = self.commit_txid.0;
            bytes.reverse();
            bytes
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>()
        };
        let reveal_txid_le = {
            let mut bytes = self.reveal_txid.0;
            bytes.reverse();
            bytes
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>()
        };

        f.debug_struct("BundledPayloadEntry")
            .field("payload", &self.payload)
            .field("commit_txid", &commit_txid_le)
            .field("reveal_txid", &reveal_txid_le)
            .field("status", &self.status)
            .finish()
    }
}

// Custom display implementation to print commit_txid and reveal_txid in little endian
impl fmt::Display for BundledPayloadEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let commit_txid_le = {
            let mut bytes = self.commit_txid.0;
            bytes.reverse();
            bytes
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>()
        };
        let reveal_txid_le = {
            let mut bytes = self.reveal_txid.0;
            bytes.reverse();
            bytes
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>()
        };

        write!(
            f,
            "BundledPayloadEntry {{ payload: {:?}, commit_txid: {}, reveal_txid: {}, status: {:?} }}",
            self.payload, commit_txid_le, reveal_txid_le, self.status
        )
    }
}

/// Various status that transactions corresponding to a payload can be in L1
#[derive(
    Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary, Serialize, Deserialize,
)]
pub enum L1BundleStatus {
    /// The payload has not been signed yet, i.e commit-reveal transactions have not been created
    /// yet.
    Unsigned,

    /// The commit-reveal transactions for payload are signed and waiting to be published
    Unpublished,

    /// The transactions are published
    Published,

    /// The transactions are confirmed
    Confirmed,

    /// The transactions are finalized
    Finalized,

    /// The transactions need to be resigned.
    /// This could be due to transactions input UTXOs already being spent.
    NeedsResign,
}

/// This is the entry that gets saved to the database corresponding to a bitcoin transaction that
/// the broadcaster will publish and watches for until finalization
#[derive(
    Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary, Serialize, Deserialize,
)]
pub struct L1TxEntry {
    /// Raw serialized transaction. This is basically `consensus::serialize()` of [`Transaction`]
    tx_raw: Vec<u8>,

    /// The status of the transaction in bitcoin
    pub status: L1TxStatus,
}

impl L1TxEntry {
    /// Create a new [`L1TxEntry`] from a [`Transaction`].
    pub fn from_tx(tx: &Transaction) -> Self {
        Self {
            tx_raw: serialize(tx),
            status: L1TxStatus::Unpublished,
        }
    }

    /// Returns the raw serialized transaction.
    ///
    /// # Note
    ///
    /// Whenever possible use [`try_to_tx()`](L1TxEntry::try_to_tx) to deserialize the transaction.
    /// This imposes more strict type checks.
    pub fn tx_raw(&self) -> &[u8] {
        &self.tx_raw
    }

    /// Deserializes the raw transaction into a [`Transaction`].
    pub fn try_to_tx(&self) -> Result<Transaction, consensus::encode::Error> {
        deserialize(&self.tx_raw)
    }

    pub fn is_valid(&self) -> bool {
        !matches!(self.status, L1TxStatus::InvalidInputs)
    }

    pub fn is_finalized(&self) -> bool {
        matches!(self.status, L1TxStatus::Finalized { .. })
    }
}

/// The possible statuses of a publishable transaction
#[derive(
    Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary, Serialize, Deserialize,
)]
#[serde(tag = "status")]
pub enum L1TxStatus {
    /// The transaction is waiting to be published
    Unpublished,

    /// The transaction is published
    Published,

    /// The transaction is included in L1 with the given number of confirmations.
    ///
    /// `block_hash` and `block_height` identify the L1 block the transaction was included in.
    Confirmed {
        confirmations: u64,
        block_hash: Buf32,
        block_height: u64,
    },

    /// The transaction is finalized in L1 with the given number of confirmations.
    ///
    /// `block_hash` and `block_height` identify the L1 block the transaction was included in.
    Finalized {
        confirmations: u64,
        block_hash: Buf32,
        block_height: u64,
    },

    /// The transaction is not included in L1 because it's inputs were invalid
    InvalidInputs,
}

/// Entry corresponding to a BatchCommitment
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
#[deprecated(note = "use `OLCheckpointEntry` for OL/EE-decoupled checkpoint storage")]
pub struct CheckpointEntry {
    /// The batch checkpoint containing metadata, state transitions, and proof data.
    pub checkpoint: Checkpoint,

    /// Proving Status
    #[expect(
        deprecated,
        reason = "legacy old code CheckpointProvingStatus is retained for compatibility"
    )]
    pub proving_status: CheckpointProvingStatus,

    /// Confirmation Status
    #[expect(
        deprecated,
        reason = "legacy old code CheckpointConfStatus is retained for compatibility"
    )]
    pub confirmation_status: CheckpointConfStatus,
}

#[expect(
    deprecated,
    reason = "legacy old code CheckpointEntry is retained for compatibility"
)]
impl CheckpointEntry {
    #[expect(
        deprecated,
        reason = "legacy old code CheckpointProvingStatus and CheckpointConfStatus are retained for compatibility"
    )]
    pub fn new(
        checkpoint: Checkpoint,
        proving_status: CheckpointProvingStatus,
        confirmation_status: CheckpointConfStatus,
    ) -> Self {
        Self {
            checkpoint,
            proving_status,
            confirmation_status,
        }
    }

    #[expect(
        deprecated,
        reason = "legacy old code CheckpointEntry is retained for compatibility"
    )]
    pub fn into_batch_checkpoint(self) -> Checkpoint {
        self.checkpoint
    }

    /// Creates a new instance for a freshly defined checkpoint.
    #[expect(
        deprecated,
        reason = "legacy old code CheckpointEntry is retained for compatibility"
    )]
    pub fn new_pending_proof(
        info: BatchInfo,
        transition: BatchTransition,
        chainstate: &Chainstate,
    ) -> Self {
        let sidecar =
            CheckpointSidecar::new(borsh::to_vec(chainstate).expect("serialize chainstate"));
        let checkpoint = Checkpoint::new(info, transition, Proof::default(), sidecar);
        Self::new(
            checkpoint,
            CheckpointProvingStatus::PendingProof,
            CheckpointConfStatus::Pending,
        )
    }
    #[expect(
        deprecated,
        reason = "legacy old code CheckpointEntry is retained for compatibility"
    )]
    pub fn is_proof_ready(&self) -> bool {
        self.proving_status == CheckpointProvingStatus::ProofReady
    }
}

#[expect(
    deprecated,
    reason = "legacy old code CheckpointEntry is retained for compatibility"
)]
impl From<CheckpointEntry> for Checkpoint {
    fn from(entry: CheckpointEntry) -> Checkpoint {
        entry.into_batch_checkpoint()
    }
}

/// Status of the commmitment
#[deprecated(note = "use `OLCheckpointStatus` for OL/EE-decoupled checkpoint signing status")]
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary, Serialize)]
pub enum CheckpointProvingStatus {
    /// Proof has not been created for this checkpoint
    PendingProof,
    /// Proof is ready
    ProofReady,
}

#[deprecated(note = "use `OLCheckpointStatus` for OL/EE-decoupled checkpoint confirmation flow")]
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary, Serialize)]
pub enum CheckpointConfStatus {
    /// Pending to be posted on L1
    Pending,
    /// Confirmed on L1, with reference.
    Confirmed(CheckpointL1Ref),
    /// Finalized on L1, with reference
    Finalized(CheckpointL1Ref),
}

/// Stored mempool transaction with ordering metadata.
///
/// Used by [`MempoolDatabase`](crate::traits::MempoolDatabase) trait for storage and retrieval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MempoolTxData {
    /// Transaction ID.
    pub txid: OLTxId,
    /// Raw transaction bytes.
    pub tx_bytes: Vec<u8>,
    /// Timestamp (microseconds since UNIX epoch) for FIFO ordering.
    ///
    /// Persists across restarts.
    pub timestamp_micros: u64,
}

impl MempoolTxData {
    /// Create new mempool transaction data.
    pub fn new(txid: OLTxId, tx_bytes: Vec<u8>, timestamp_micros: u64) -> Self {
        Self {
            txid,
            tx_bytes,
            timestamp_micros,
        }
    }
}

/// Index into the L1 payload intent store.
pub type L1PayloadIntentIndex = u64;

/// Entry for an OL checkpoint.
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct OLCheckpointEntry {
    /// The checkpoint payload to be posted to L1.
    pub checkpoint: CheckpointPayload,

    /// Signing status.
    pub status: OLCheckpointStatus,
}

impl OLCheckpointEntry {
    pub fn new(checkpoint: CheckpointPayload, status: OLCheckpointStatus) -> Self {
        Self { checkpoint, status }
    }

    pub fn new_unsigned(checkpoint: CheckpointPayload) -> Self {
        Self::new(checkpoint, OLCheckpointStatus::Unsigned)
    }
}

/// Signing status of an OL checkpoint.
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Serialize)]
pub enum OLCheckpointStatus {
    /// Not signed yet.
    Unsigned,
    /// Signed and stored as L1PayloadIntent with given index.
    Signed(L1PayloadIntentIndex),
}

/// A chunked envelope entry representing a commit tx funding N reveal txs.
///
/// Used for posting large DA blobs that exceed single-transaction limits.
/// Each reveal contains a chunk of the original blob with header metadata
/// for reassembly.
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct ChunkedEnvelopeEntry {
    /// Opaque witness data per reveal, ordered by output index.
    pub chunk_data: Vec<Vec<u8>>,
    /// OP_RETURN tag magic bytes.
    pub magic_bytes: MagicBytes,
    /// Wtxid of the last reveal in the preceding envelope, or zero for the first.
    pub prev_tail_wtxid: Buf32,
    /// Commit transaction ID. Zero if unsigned.
    pub commit_txid: Buf32,
    /// Per-reveal metadata, ordered by output index. Empty if unsigned.
    pub reveals: Vec<RevealTxMeta>,
    /// Lifecycle status.
    pub status: ChunkedEnvelopeStatus,
}

impl ChunkedEnvelopeEntry {
    /// Creates a new unsigned entry with no transaction metadata.
    ///
    /// Transaction IDs, reveal metadata, and `prev_tail_wtxid` are populated
    /// at signing time by the watcher (which guarantees the predecessor entry
    /// is already signed).
    pub fn new_unsigned(chunk_data: Vec<Vec<u8>>, magic_bytes: MagicBytes) -> Self {
        Self {
            chunk_data,
            magic_bytes,
            prev_tail_wtxid: Buf32::zero(),
            commit_txid: Buf32::zero(),
            reveals: Vec::new(),
            status: ChunkedEnvelopeStatus::Unsigned,
        }
    }

    /// Returns the wtxid of the last reveal, or [`prev_tail_wtxid`](Self::prev_tail_wtxid) if
    /// unsigned.
    pub fn tail_wtxid(&self) -> Buf32 {
        self.reveals
            .last()
            .map(|r| r.wtxid)
            .unwrap_or(self.prev_tail_wtxid)
    }
}

/// Metadata for a single reveal transaction within a chunked envelope.
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct RevealTxMeta {
    /// Output index in the commit tx that this reveal spends.
    pub vout_index: u32,
    /// Reveal transaction ID.
    pub txid: Buf32,
    /// Reveal witness transaction ID.
    pub wtxid: Buf32,
    /// Raw signed reveal transaction bytes (consensus-encoded).
    /// Stored here until the commit is published, then added to broadcast DB.
    pub tx_bytes: Vec<u8>,
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
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Serialize)]
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

#[cfg(test)]
mod tests {
    use serde_json;

    use super::*;

    #[test]
    fn check_serde_of_l1txstatus() {
        let test_cases: Vec<(L1TxStatus, &str)> = vec![
            (L1TxStatus::Unpublished, r#"{"status":"Unpublished"}"#),
            (L1TxStatus::Published, r#"{"status":"Published"}"#),
            (
                L1TxStatus::Confirmed {
                    confirmations: 10,
                    block_hash: Buf32::zero(),
                    block_height: 42,
                },
                r#"{"status":"Confirmed","confirmations":10,"block_hash":"0000000000000000000000000000000000000000000000000000000000000000","block_height":42}"#,
            ),
            (
                L1TxStatus::Finalized {
                    confirmations: 100,
                    block_hash: Buf32::zero(),
                    block_height: 42,
                },
                r#"{"status":"Finalized","confirmations":100,"block_hash":"0000000000000000000000000000000000000000000000000000000000000000","block_height":42}"#,
            ),
            (L1TxStatus::InvalidInputs, r#"{"status":"InvalidInputs"}"#),
        ];

        // check serialization and deserialization
        for (l1_tx_status, serialized) in test_cases {
            let actual = serde_json::to_string(&l1_tx_status).unwrap();
            assert_eq!(actual, serialized);

            let actual: L1TxStatus = serde_json::from_str(serialized).unwrap();
            assert_eq!(actual, l1_tx_status);
        }
    }
}
