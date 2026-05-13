#![expect(deprecated, reason = "legacy old code is retained for compatibility")] // I have no idea how to make clippy be happy with precise expects in this module
//! Module for database local types

use std::{
    fmt,
    time::{SystemTime, UNIX_EPOCH},
};

use arbitrary::Arbitrary;
use bitcoin::{
    consensus::{self, deserialize, serialize},
    hashes::{sha256, Hash},
    Amount, FeeRate, Transaction,
};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_checkpoint_types::Checkpoint;
use strata_csm_types::{CheckpointL1Ref, L1Payload, PayloadIntent};
use strata_identifiers::{Buf32, Buf64, OLTxId, RBuf32};
use strata_l1_txfmt::MagicBytes;
use strata_primitives::L1Height;

/// Taproot script-spend sighash for the reveal transaction.
pub type Sighash = Buf32;

/// Bitcoin transaction ID displayed in Bitcoin byte order.
pub type L1TxId = RBuf32;

/// Bitcoin witness transaction ID displayed in Bitcoin byte order.
pub type L1WtxId = RBuf32;

/// Bitcoin block hash displayed in Bitcoin byte order.
pub type L1BlockHash = RBuf32;

/// Deterministic identifier for one logical writer transaction replacement chain.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
)]
pub struct TxNodeId(pub Buf32);

impl TxNodeId {
    /// Derives a stable id from the logical transaction kind.
    pub fn from_kind(kind: &TxNodeKind) -> Self {
        const DOMAIN: &[u8] = b"alpen.btcio.tx-node.v1";

        let mut bytes = Vec::with_capacity(DOMAIN.len() + 64);
        bytes.extend_from_slice(DOMAIN);
        bytes.extend_from_slice(
            &borsh::to_vec(kind).expect("tx-node kind serialization cannot fail"),
        );

        Self(Buf32(sha256::Hash::hash(&bytes).to_byte_array()))
    }
}

/// Logical BTCIO writer transaction kind.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub enum TxNodeKind {
    /// Commit transaction for a single-envelope payload row.
    SingleEnvelopeCommit { payload_idx: u64 },
    /// Reveal transaction for a single-envelope payload row.
    SingleEnvelopeReveal { payload_idx: u64 },
    /// Commit transaction for a chunked-envelope row.
    ChunkedEnvelopeCommit { envelope_idx: u64 },
    /// One reveal transaction for a chunked-envelope row.
    ChunkedEnvelopeReveal { envelope_idx: u64, reveal_idx: u32 },
}

/// Replacement-attempt lifecycle within a logical transaction node.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub enum TxAttemptStatus {
    /// The attempt is the currently broadcastable transaction.
    Active,
    /// The attempt has been superseded by another txid.
    Replaced,
    /// The attempt was abandoned before becoming active.
    Discarded,
    /// The attempt is waiting for an external reveal signature.
    PendingSignature,
}

/// One concrete transaction attempt in a logical replacement chain.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct TxAttempt {
    pub attempt_no: u32,
    pub raw_tx: Vec<u8>,
    pub txid: L1TxId,
    pub wtxid: L1WtxId,
    pub fee_rate_sat_vb: u64,
    pub fee_sats: u64,
    pub created_at_unix_secs: u64,
    pub first_published_l1_height: Option<L1Height>,
    pub status: TxAttemptStatus,
    pub replaced_by: Option<L1TxId>,
}

impl TxAttempt {
    /// Creates an active attempt for a transaction.
    pub fn active(tx: &Transaction, fee_rate: FeeRate, fee_sats: Amount, attempt_no: u32) -> Self {
        Self::new(tx, fee_rate, fee_sats, attempt_no, TxAttemptStatus::Active)
    }

    /// Creates an attempt that is waiting for an external reveal signature.
    pub fn pending_signature(
        tx: &Transaction,
        fee_rate: FeeRate,
        fee_sats: Amount,
        attempt_no: u32,
    ) -> Self {
        Self::new(
            tx,
            fee_rate,
            fee_sats,
            attempt_no,
            TxAttemptStatus::PendingSignature,
        )
    }

    /// Creates an attempt for a transaction with the provided status.
    pub fn new(
        tx: &Transaction,
        fee_rate: FeeRate,
        fee_sats: Amount,
        attempt_no: u32,
        status: TxAttemptStatus,
    ) -> Self {
        Self {
            attempt_no,
            raw_tx: serialize(tx),
            txid: L1TxId::from(tx.compute_txid().to_byte_array()),
            wtxid: L1WtxId::from(tx.compute_wtxid().to_byte_array()),
            fee_rate_sat_vb: fee_rate.to_sat_per_vb_ceil(),
            fee_sats: fee_sats.to_sat(),
            created_at_unix_secs: unix_secs_now(),
            first_published_l1_height: None,
            status,
            replaced_by: None,
        }
    }

    /// Deserializes the raw transaction for this attempt.
    pub fn try_to_tx(&self) -> Result<Transaction, consensus::encode::Error> {
        deserialize(&self.raw_tx)
    }

    fn refresh_tx(&mut self, tx: &Transaction, fee_rate: FeeRate, fee_sats: Amount) {
        self.raw_tx = serialize(tx);
        self.txid = L1TxId::from(tx.compute_txid().to_byte_array());
        self.wtxid = L1WtxId::from(tx.compute_wtxid().to_byte_array());
        self.fee_rate_sat_vb = fee_rate.to_sat_per_vb_ceil();
        self.fee_sats = fee_sats.to_sat();
    }
}

/// Persistent replacement-chain record for one logical writer transaction.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct TxNodeRecord {
    pub node_id: TxNodeId,
    pub kind: TxNodeKind,
    pub active_txid: L1TxId,
    pub attempts: Vec<TxAttempt>,
    pub terminal_error: Option<TerminalError>,
}

impl TxNodeRecord {
    /// Creates a replacement-chain record from its first active attempt.
    pub fn new(kind: TxNodeKind, first_attempt: TxAttempt) -> Self {
        let node_id = TxNodeId::from_kind(&kind);
        let active_txid = first_attempt.txid;
        Self {
            node_id,
            kind,
            active_txid,
            attempts: vec![first_attempt],
            terminal_error: None,
        }
    }

    /// Replaces the chain with a fresh initial attempt for the same logical node.
    pub fn replace_initial_attempt(&mut self, mut attempt: TxAttempt) {
        attempt.attempt_no = 0;
        self.active_txid = attempt.txid;
        self.attempts = vec![attempt];
        self.terminal_error = None;
    }

    /// Returns the active attempt.
    pub fn active_attempt(&self) -> Option<&TxAttempt> {
        self.attempts
            .iter()
            .find(|attempt| attempt.txid == self.active_txid)
    }

    /// Returns the mutable active attempt.
    pub fn active_attempt_mut(&mut self) -> Option<&mut TxAttempt> {
        let active_txid = self.active_txid;
        self.attempts
            .iter_mut()
            .find(|attempt| attempt.txid == active_txid)
    }

    /// Returns the pending externally-signed replacement attempt, if any.
    pub fn pending_signature_attempt(&self) -> Option<&TxAttempt> {
        self.attempts
            .iter()
            .rev()
            .find(|attempt| attempt.status == TxAttemptStatus::PendingSignature)
    }

    /// Appends a replacement attempt and marks the previous active attempt as replaced.
    pub fn append_replacement(&mut self, mut replacement: TxAttempt) {
        let previous_active = self.active_txid;
        if let Some(active) = self.active_attempt_mut() {
            active.status = TxAttemptStatus::Replaced;
            active.replaced_by = Some(replacement.txid);
        }
        replacement.status = TxAttemptStatus::Active;
        self.active_txid = replacement.txid;
        self.attempts.push(replacement);

        debug_assert_ne!(self.active_txid, previous_active);
    }

    /// Appends a replacement attempt that still needs an external signature.
    pub fn append_pending_signature_replacement(&mut self, mut replacement: TxAttempt) {
        replacement.status = TxAttemptStatus::PendingSignature;
        self.attempts
            .retain(|attempt| attempt.status != TxAttemptStatus::PendingSignature);
        self.attempts.push(replacement);
    }

    /// Activates the current pending-signature attempt after the final witness is attached.
    pub fn activate_pending_signature(
        &mut self,
        signed_tx: &Transaction,
        fee_rate: FeeRate,
        fee_sats: Amount,
    ) -> bool {
        let Some(pending_idx) = self
            .attempts
            .iter()
            .position(|attempt| attempt.status == TxAttemptStatus::PendingSignature)
        else {
            return false;
        };

        let previous_active_txid = self.active_txid;
        self.attempts[pending_idx].refresh_tx(signed_tx, fee_rate, fee_sats);
        let active_txid = self.attempts[pending_idx].txid;

        if let Some(active_idx) = self
            .attempts
            .iter()
            .position(|attempt| attempt.txid == previous_active_txid)
        {
            self.attempts[active_idx].status = TxAttemptStatus::Replaced;
            self.attempts[active_idx].replaced_by = Some(active_txid);
        }

        self.attempts[pending_idx].status = TxAttemptStatus::Active;
        self.active_txid = active_txid;
        true
    }

    /// Discards any unsigned pending-signature replacement attempts.
    pub fn discard_pending_signature_replacement(&mut self) -> bool {
        let mut discarded = false;
        for attempt in &mut self.attempts {
            if attempt.status == TxAttemptStatus::PendingSignature {
                attempt.status = TxAttemptStatus::Discarded;
                discarded = true;
            }
        }
        discarded
    }

    /// Marks the replacement chain permanently terminal.
    pub fn set_terminal_error(&mut self, error: TerminalError) {
        self.terminal_error = Some(error);
    }
}

/// Terminal reason that prevents further fee bumping for a logical transaction.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub enum TerminalError {
    MaxAttemptsReached,
    AboveMaxFeeRate,
    Bip125FeeRuleUnsatisfiable,
    WalletInsufficient,
    ReplacementWouldDustOutput,
    UnsupportedRbfKind,
    SignerTimeout,
}

fn unix_secs_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is after Unix epoch")
        .as_secs()
}

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
    pub commit_txid: L1TxId,
    pub reveal_txid: L1TxId,
    pub status: L1BundleStatus,
    /// Schnorr signature provided by the external signer for envelope reveal tx.
    ///
    /// Populated when the signer calls `complete_payload_signature` RPC.
    pub payload_signature: Option<Buf64>,
}

impl BundledPayloadEntry {
    pub fn new(
        payload: L1Payload,
        commit_txid: L1TxId,
        reveal_txid: L1TxId,
        status: L1BundleStatus,
    ) -> Self {
        Self {
            payload,
            commit_txid,
            reveal_txid,
            status,
            payload_signature: None,
        }
    }

    /// Create new unsigned [`BundledPayloadEntry`].
    ///
    /// NOTE: This won't have commit - reveal pairs associated with it.
    ///   Because it is better to defer gathering utxos as late as possible to prevent being spent
    ///   by others. Those will be created and signed in a single step.
    pub fn new_unsigned(payload: L1Payload) -> Self {
        let cid = L1TxId::zero();
        let rid = L1TxId::zero();
        Self::new(payload, cid, rid, L1BundleStatus::Unsigned)
    }
}

// Custom debug implementation to print commit_txid and reveal_txid in Bitcoin order.
impl fmt::Debug for BundledPayloadEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let commit_txid = format!("{:?}", self.commit_txid);
        let reveal_txid = format!("{:?}", self.reveal_txid);

        f.debug_struct("BundledPayloadEntry")
            .field("payload", &self.payload)
            .field("commit_txid", &commit_txid)
            .field("reveal_txid", &reveal_txid)
            .field("status", &self.status)
            .finish()
    }
}

// Custom display implementation to print commit_txid and reveal_txid in Bitcoin order.
impl fmt::Display for BundledPayloadEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "BundledPayloadEntry {{ payload: {:?}, commit_txid: {:?}, reveal_txid: {:?}, status: {:?} }}",
            self.payload, self.commit_txid, self.reveal_txid, self.status
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

    /// The envelope has been built and the commit tx signed; waiting for the external signer to
    /// provide a Schnorr signature for the reveal tx sighash.
    PendingRevealTxSign(Sighash),

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
#[derive(Debug, Clone, PartialEq, Arbitrary, Serialize, Deserialize)]
pub struct L1TxEntry {
    /// Raw serialized transaction. This is basically `consensus::serialize()` of [`Transaction`]
    tx_raw: Vec<u8>,

    /// The status of the transaction in bitcoin
    pub status: L1TxStatus,

    /// Optional metadata used by writer-side RBF replacement logic.
    pub rbf: Option<L1TxRbfInfo>,
}

impl L1TxEntry {
    /// Create a new [`L1TxEntry`] from a [`Transaction`].
    pub fn from_tx(tx: &Transaction) -> Self {
        Self {
            tx_raw: serialize(tx),
            status: L1TxStatus::Unpublished,
            rbf: None,
        }
    }

    /// Create a new writer-owned [`L1TxEntry`] with RBF metadata.
    pub fn from_tx_with_fee_rate(tx: &Transaction, fee_rate: FeeRate) -> Self {
        Self {
            tx_raw: serialize(tx),
            status: L1TxStatus::Unpublished,
            rbf: Some(L1TxRbfInfo {
                first_published_l1_height: None,
                fee_rate_sat_vb: fee_rate.to_sat_per_vb_ceil(),
                bump_count: 0,
            }),
        }
    }

    /// Creates an entry from persisted raw transaction bytes and metadata.
    pub fn from_raw_parts(tx_raw: Vec<u8>, status: L1TxStatus, rbf: Option<L1TxRbfInfo>) -> Self {
        Self {
            tx_raw,
            status,
            rbf,
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
        !matches!(
            self.status,
            L1TxStatus::InvalidInputs | L1TxStatus::Replaced { .. }
        )
    }

    pub fn is_finalized(&self) -> bool {
        matches!(self.status, L1TxStatus::Finalized { .. })
    }
}

/// RBF metadata for one concrete broadcast transaction.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    BorshSerialize,
    BorshDeserialize,
    Arbitrary,
    Serialize,
    Deserialize,
)]
pub struct L1TxRbfInfo {
    /// L1 height where this transaction was first observed as published.
    pub first_published_l1_height: Option<L1Height>,

    /// Fee rate used to construct this transaction in sat/vB.
    pub fee_rate_sat_vb: u64,

    /// Number of replacements already made for this logical writer transaction.
    pub bump_count: u32,
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
        block_hash: L1BlockHash,
        block_height: L1Height,
    },

    /// The transaction is finalized in L1 with the given number of confirmations.
    ///
    /// `block_hash` and `block_height` identify the L1 block the transaction was included in.
    Finalized {
        confirmations: u64,
        block_hash: L1BlockHash,
        block_height: L1Height,
    },

    /// The transaction is not included in L1 because it's inputs were invalid
    InvalidInputs,

    /// The transaction has been superseded by an RBF replacement.
    Replaced {
        /// Replacement transaction id.
        by: L1TxId,
    },
}

impl fmt::Display for L1TxStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unpublished => f.write_str("unpublished"),
            Self::Published => f.write_str("published"),
            Self::Confirmed {
                confirmations,
                block_hash,
                block_height,
            } => {
                write!(
                    f,
                    "confirmed@{block_height}/{block_hash} ({confirmations} confs)"
                )
            }
            Self::Finalized {
                confirmations,
                block_hash,
                block_height,
            } => {
                write!(
                    f,
                    "finalized@{block_height}/{block_hash} ({confirmations} confs)"
                )
            }
            Self::InvalidInputs => f.write_str("invalid_inputs"),
            Self::Replaced { by } => write!(f, "replaced_by({by})"),
        }
    }
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
#[deprecated(
    note = "use `OLCheckpointEntry::signing_status` for OL/EE-decoupled checkpoint signing status"
)]
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary, Serialize)]
pub enum CheckpointProvingStatus {
    /// Proof has not been created for this checkpoint
    PendingProof,
    /// Proof is ready
    ProofReady,
}

#[deprecated(
    note = "use `OLCheckpointEntry::confirmation_status` for OL/EE-decoupled checkpoint confirmation flow"
)]
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

/// A chunked envelope entry representing a commit tx funding N reveal txs.
///
/// Used for posting large DA blobs that exceed single-transaction limits.
/// The commit tx publishes the EE DA marker via an OP_RETURN at output 0
/// (`magic + version`); each subsequent P2TR output funds a
/// reveal whose tapscript carries one chunk under `<sequencer_pk> OP_CHECKSIG`.
/// Reveals do NOT reference each other; entries are independent across batches.
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize)]
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
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize)]
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

#[cfg(test)]
mod tests {
    use bitcoin::{
        absolute::LockTime, transaction::Version, OutPoint, ScriptBuf, Sequence, TxIn, TxOut,
        Witness,
    };
    use proptest::{
        strategy::{Strategy, ValueTree},
        test_runner::TestRunner,
    };
    use serde_json;
    use strata_identifiers::test_utils::{buf32_strategy, l1_block_commitment_strategy};
    use strata_l1_txfmt::TagData;

    use super::*;

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
    fn check_serde_of_l1txstatus() {
        let test_cases: Vec<(L1TxStatus, &str)> = vec![
            (L1TxStatus::Unpublished, r#"{"status":"Unpublished"}"#),
            (L1TxStatus::Published, r#"{"status":"Published"}"#),
            (
                L1TxStatus::Confirmed {
                    confirmations: 10,
                    block_hash: L1BlockHash::zero(),
                    block_height: 42,
                },
                r#"{"status":"Confirmed","confirmations":10,"block_hash":"0000000000000000000000000000000000000000000000000000000000000000","block_height":42}"#,
            ),
            (
                L1TxStatus::Finalized {
                    confirmations: 100,
                    block_hash: L1BlockHash::zero(),
                    block_height: 42,
                },
                r#"{"status":"Finalized","confirmations":100,"block_hash":"0000000000000000000000000000000000000000000000000000000000000000","block_height":42}"#,
            ),
            (L1TxStatus::InvalidInputs, r#"{"status":"InvalidInputs"}"#),
            (
                L1TxStatus::Replaced { by: L1TxId::zero() },
                r#"{"status":"Replaced","by":"0000000000000000000000000000000000000000000000000000000000000000"}"#,
            ),
        ];

        // check serialization and deserialization
        for (l1_tx_status, serialized) in test_cases {
            let actual = serde_json::to_string(&l1_tx_status).unwrap();
            assert_eq!(actual, serialized);

            let actual: L1TxStatus = serde_json::from_str(serialized).unwrap();
            assert_eq!(actual, l1_tx_status);
        }
    }

    #[test]
    fn display_l1txstatus_uses_log_friendly_format() {
        let status = L1TxStatus::Confirmed {
            confirmations: 12,
            block_hash: L1BlockHash::zero(),
            block_height: 42,
        };

        assert_eq!(status.to_string(), "confirmed@42/000000..000000 (12 confs)");
    }

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
    fn bundled_payload_entry_formats_full_reversed_txids() {
        let commit_bytes = bytes_from_start(0x10);
        let reveal_bytes = bytes_from_start(0x40);
        let payload = L1Payload::new(vec![vec![1, 2, 3]], TagData::new(1, 1, vec![]).unwrap());
        let entry = BundledPayloadEntry::new(
            payload,
            L1TxId::from(commit_bytes),
            L1TxId::from(reveal_bytes),
            L1BundleStatus::Unpublished,
        );

        let display = entry.to_string();
        let debug = format!("{entry:?}");
        let expected_commit = reversed_hex(commit_bytes);
        let expected_reveal = reversed_hex(reveal_bytes);

        assert!(display.contains(&expected_commit));
        assert!(display.contains(&expected_reveal));
        assert!(!display.contains(".."));
        assert!(debug.contains(&expected_commit));
        assert!(debug.contains(&expected_reveal));
        assert!(!debug.contains(".."));
    }

    #[test]
    fn pending_signature_replacement_keeps_previous_attempt_active() {
        let fee_rate = FeeRate::from_sat_per_vb(2).expect("valid fee rate");
        let initial = TxAttempt::active(&tx_with_output(1_000), fee_rate, Amount::from_sat(100), 0);
        let initial_txid = initial.txid;
        let mut record =
            TxNodeRecord::new(TxNodeKind::SingleEnvelopeReveal { payload_idx: 7 }, initial);
        let replacement =
            TxAttempt::pending_signature(&tx_with_output(900), fee_rate, Amount::from_sat(200), 1);
        let replacement_txid = replacement.txid;

        record.append_pending_signature_replacement(replacement);

        assert_eq!(record.active_txid, initial_txid);
        assert_eq!(
            record.active_attempt().map(|attempt| attempt.status),
            Some(TxAttemptStatus::Active)
        );
        assert_eq!(
            record
                .pending_signature_attempt()
                .map(|attempt| attempt.txid),
            Some(replacement_txid)
        );
        assert_eq!(record.attempts[0].replaced_by, None);
    }

    #[test]
    fn pending_signature_attempt_becomes_active_after_signature() {
        let fee_rate = FeeRate::from_sat_per_vb(2).expect("valid fee rate");
        let initial = TxAttempt::active(&tx_with_output(1_000), fee_rate, Amount::from_sat(100), 0);
        let mut record =
            TxNodeRecord::new(TxNodeKind::SingleEnvelopeReveal { payload_idx: 7 }, initial);
        let unsigned = tx_with_output(900);
        let replacement =
            TxAttempt::pending_signature(&unsigned, fee_rate, Amount::from_sat(200), 1);
        record.append_pending_signature_replacement(replacement);

        let mut signed = unsigned;
        signed.input[0].witness.push([1u8; 64]);
        let activated = record.activate_pending_signature(
            &signed,
            FeeRate::from_sat_per_vb(3).expect("valid fee rate"),
            Amount::from_sat(300),
        );

        let active = record.active_attempt().expect("active attempt");
        assert!(activated);
        assert_eq!(active.status, TxAttemptStatus::Active);
        assert_eq!(active.fee_rate_sat_vb, 3);
        assert_eq!(active.fee_sats, 300);
        assert_eq!(
            active.wtxid,
            L1WtxId::from(signed.compute_wtxid().to_byte_array())
        );
        assert_eq!(record.attempts[0].status, TxAttemptStatus::Replaced);
        assert_eq!(record.attempts[0].replaced_by, Some(active.txid));
    }

    #[test]
    fn pending_signature_attempt_can_be_discarded() {
        let fee_rate = FeeRate::from_sat_per_vb(2).expect("valid fee rate");
        let initial = TxAttempt::active(&tx_with_output(1_000), fee_rate, Amount::from_sat(100), 0);
        let initial_txid = initial.txid;
        let mut record =
            TxNodeRecord::new(TxNodeKind::SingleEnvelopeReveal { payload_idx: 7 }, initial);
        let replacement =
            TxAttempt::pending_signature(&tx_with_output(900), fee_rate, Amount::from_sat(200), 1);
        record.append_pending_signature_replacement(replacement);

        assert!(record.discard_pending_signature_replacement());

        assert_eq!(record.active_txid, initial_txid);
        assert_eq!(record.pending_signature_attempt(), None);
        assert_eq!(record.attempts[1].status, TxAttemptStatus::Discarded);
    }

    #[test]
    fn replace_initial_attempt_clears_terminal_errors() {
        let fee_rate = FeeRate::from_sat_per_vb(2).expect("valid fee rate");
        let initial = TxAttempt::active(&tx_with_output(1_000), fee_rate, Amount::from_sat(100), 0);
        let mut record =
            TxNodeRecord::new(TxNodeKind::SingleEnvelopeCommit { payload_idx: 7 }, initial);
        record.set_terminal_error(TerminalError::WalletInsufficient);

        let fresh = TxAttempt::active(&tx_with_output(900), fee_rate, Amount::from_sat(120), 3);
        let fresh_txid = fresh.txid;
        record.replace_initial_attempt(fresh);

        assert_eq!(record.active_txid, fresh_txid);
        assert_eq!(record.terminal_error, None);
        assert_eq!(record.attempts.len(), 1);
        assert_eq!(record.attempts[0].attempt_no, 0);
        assert_eq!(record.attempts[0].status, TxAttemptStatus::Active);
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
                vout_index: 1,
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

    #[test]
    fn l1_payload_intent_index_borsh_roundtrip() {
        let idx: L1PayloadIntentIndex = 99;
        let bytes = borsh::to_vec(&idx).unwrap();
        let decoded: L1PayloadIntentIndex = borsh::from_slice(&bytes).unwrap();
        assert_eq!(decoded, idx);
    }

    #[test]
    fn checkpoint_l1_ref_borsh_roundtrip() {
        let mut runner = TestRunner::default();
        let l1_commitment = l1_block_commitment_strategy()
            .new_tree(&mut runner)
            .expect("failed to generate L1BlockCommitment")
            .current();
        let txid = buf32_strategy()
            .new_tree(&mut runner)
            .expect("failed to generate txid")
            .current();
        let wtxid = buf32_strategy()
            .new_tree(&mut runner)
            .expect("failed to generate wtxid")
            .current();

        let observation =
            CheckpointL1Ref::new(l1_commitment, RBuf32::from(txid.0), RBuf32::from(wtxid.0));
        let bytes = borsh::to_vec(&observation).unwrap();
        let decoded: CheckpointL1Ref = borsh::from_slice(&bytes).unwrap();
        assert_eq!(decoded, observation);
    }
}
