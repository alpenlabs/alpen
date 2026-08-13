//! Persistent bookkeeping for writer-side fee bumping.
//!
//! A logical writer transaction (for example "the reveal tx of chunked envelope 3") can be
//! published several times with increasing fee rates. Each concrete transaction is a
//! [`TxAttempt`], and the ordered chain of attempts for one logical transaction is a
//! [`TxNodeRecord`] keyed by a [`TxNodeId`] derived from its [`TxNodeKind`].
//!
//! Records are persisted with CBOR so new fields stay forward compatible. [`TxNodeId`] is
//! deliberately derived from an explicit, versioned encoding of the kind, because the key must
//! stay byte-stable across releases and is independent of the value codec.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use strata_identifiers::Buf32;
use strata_primitives::L1Height;

use crate::common::{L1TxId, L1WtxId};

/// Domain separator for [`TxNodeId`] derivation.
const TX_NODE_ID_DOMAIN: &[u8] = b"alpen.btcio.tx-node.v1";

/// Deterministic identifier for one logical writer transaction replacement chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TxNodeId(pub Buf32);

impl TxNodeId {
    /// Derives a stable id from the logical transaction kind.
    pub fn from_kind(kind: &TxNodeKind) -> Self {
        let mut bytes = Vec::with_capacity(TX_NODE_ID_DOMAIN.len() + 13);
        bytes.extend_from_slice(TX_NODE_ID_DOMAIN);
        kind.extend_id_material(&mut bytes);

        Self(Buf32(Sha256::digest(&bytes).into()))
    }
}

/// Logical BTCIO writer transaction kind.
///
/// The explicit encoding of this enum feeds [`TxNodeId::from_kind`], so variant tags and field
/// encodings are persistent identifiers. New variants must use new tags, and existing encodings
/// must not change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl TxNodeKind {
    /// Appends the stable identifier material for this logical transaction kind.
    fn extend_id_material(&self, bytes: &mut Vec<u8>) {
        match self {
            Self::SingleEnvelopeCommit { payload_idx } => {
                bytes.push(0);
                bytes.extend_from_slice(&payload_idx.to_le_bytes());
            }
            Self::SingleEnvelopeReveal { payload_idx } => {
                bytes.push(1);
                bytes.extend_from_slice(&payload_idx.to_le_bytes());
            }
            Self::ChunkedEnvelopeCommit { envelope_idx } => {
                bytes.push(2);
                bytes.extend_from_slice(&envelope_idx.to_le_bytes());
            }
            Self::ChunkedEnvelopeReveal {
                envelope_idx,
                reveal_idx,
            } => {
                bytes.push(3);
                bytes.extend_from_slice(&envelope_idx.to_le_bytes());
                bytes.extend_from_slice(&reveal_idx.to_le_bytes());
            }
        }
    }
}

/// Replacement-attempt lifecycle within a logical transaction node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// Concrete transaction material for one attempt.
///
/// Produced by Bitcoin-aware callers (see `strata_btcio::tx_attempt`); this crate treats the
/// transaction bytes and ids as opaque so it stays free of a Bitcoin dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxAttemptParts {
    /// `consensus::serialize()` of the attempt transaction.
    pub raw_tx: Vec<u8>,

    /// Transaction id of the attempt.
    pub txid: L1TxId,

    /// Witness transaction id of the attempt.
    pub wtxid: L1WtxId,

    /// Fee rate the attempt was built at, in sat/vB.
    pub fee_rate_sat_vb: u64,

    /// Absolute fee the attempt pays, in satoshis.
    pub fee_sats: u64,
}

/// One concrete transaction attempt in a logical replacement chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxAttempt {
    /// Zero-based position of this attempt within its chain.
    pub attempt_no: u32,

    /// `consensus::serialize()` of the attempt transaction.
    ///
    /// Emptied once the attempt is `Replaced` or `Discarded`, since nothing rebroadcasts or
    /// re-signs an attempt in either state and these bytes dominate a record's size.
    pub raw_tx: Vec<u8>,

    /// Transaction id of the attempt.
    pub txid: L1TxId,

    /// Witness transaction id of the attempt.
    pub wtxid: L1WtxId,

    /// Fee rate the attempt was built at, in sat/vB.
    pub fee_rate_sat_vb: u64,

    /// Absolute fee the attempt pays, in satoshis.
    pub fee_sats: u64,

    /// Wall-clock creation time, used only for operational logging.
    pub created_at_unix_secs: u64,

    /// L1 height at which the attempt was first observed as published.
    pub first_published_l1_height: Option<L1Height>,

    /// Lifecycle status of the attempt.
    pub status: TxAttemptStatus,

    /// Txid of the attempt that superseded this one.
    pub replaced_by: Option<L1TxId>,
}

impl TxAttempt {
    /// Creates an active attempt for a transaction.
    pub fn active(parts: TxAttemptParts, attempt_no: u32) -> Self {
        Self::new(parts, attempt_no, TxAttemptStatus::Active)
    }

    /// Creates an attempt that is waiting for an external reveal signature.
    pub fn pending_signature(parts: TxAttemptParts, attempt_no: u32) -> Self {
        Self::new(parts, attempt_no, TxAttemptStatus::PendingSignature)
    }

    /// Creates an attempt for a transaction with the provided status.
    pub fn new(parts: TxAttemptParts, attempt_no: u32, status: TxAttemptStatus) -> Self {
        Self {
            attempt_no,
            raw_tx: parts.raw_tx,
            txid: parts.txid,
            wtxid: parts.wtxid,
            fee_rate_sat_vb: parts.fee_rate_sat_vb,
            fee_sats: parts.fee_sats,
            created_at_unix_secs: unix_secs_now(),
            first_published_l1_height: None,
            status,
            replaced_by: None,
        }
    }

    /// Drops the raw transaction bytes of an attempt that can no longer be broadcast.
    ///
    /// A record keeps every attempt of its chain forever, and `raw_tx` is by far the largest field
    /// in one: a chunked EE-DA reveal carries its chunk payload in the witness, up to a few hundred
    /// kilobytes. Retaining that for superseded attempts multiplies a record's size by its attempt
    /// count, and the replacement pass loads every record it has. The metadata that outlives the
    /// bytes — txid, wtxid, fee, status, `replaced_by` — is what the chain is actually reconstructed
    /// and audited from.
    ///
    /// Only ever called on `Replaced` and `Discarded` attempts, and on every attempt of a chain
    /// retired via [`TxNodeRecord::forget_all_raw_txs`]. Everything that deserializes an attempt
    /// reads it through [`TxNodeRecord::active_attempt`] or
    /// [`TxNodeRecord::pending_signature_attempt`], and neither can return a superseded or retired
    /// one.
    fn forget_raw_tx(&mut self) {
        self.raw_tx = Vec::new();
    }

    fn refresh_tx(&mut self, parts: TxAttemptParts) {
        self.raw_tx = parts.raw_tx;
        self.txid = parts.txid;
        self.wtxid = parts.wtxid;
        self.fee_rate_sat_vb = parts.fee_rate_sat_vb;
        self.fee_sats = parts.fee_sats;
    }
}

/// Persistent replacement-chain record for one logical writer transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxNodeRecord {
    /// Identifier derived from [`TxNodeRecord::kind`].
    pub node_id: TxNodeId,

    /// Logical transaction this chain publishes.
    pub kind: TxNodeKind,

    /// Txid of the attempt that is currently broadcastable.
    pub active_txid: L1TxId,

    /// Attempts in creation order.
    pub attempts: Vec<TxAttempt>,

    /// Set once the chain can no longer be bumped.
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
    ///
    /// Used when the underlying transaction is rebuilt from scratch rather than fee bumped, for
    /// example when a chunked-envelope commit replacement forces its reveals to be regenerated.
    pub fn replace_initial_attempt(&mut self, mut attempt: TxAttempt) {
        attempt.attempt_no = 0;
        attempt.status = TxAttemptStatus::Active;
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

    /// Returns the number of the next attempt to append to this chain.
    pub fn next_attempt_no(&self) -> u32 {
        self.attempts
            .iter()
            .map(|attempt| attempt.attempt_no)
            .max()
            .map(|highest| highest.saturating_add(1))
            .unwrap_or(0)
    }

    /// Returns the pending externally-signed replacement attempt, if any.
    ///
    /// A chain with a pending attempt is skipped by the fee bumper until the signature arrives or
    /// the attempt is discarded. There is deliberately no timeout: the previous attempt stays
    /// active and broadcastable throughout, so a silent signer delays further bumping rather than
    /// stalling publication.
    pub fn pending_signature_attempt(&self) -> Option<&TxAttempt> {
        self.attempts
            .iter()
            .rev()
            .find(|attempt| attempt.status == TxAttemptStatus::PendingSignature)
    }

    /// Appends a replacement attempt and marks the previous active attempt as replaced.
    pub fn append_replacement(&mut self, mut replacement: TxAttempt) {
        if let Some(active) = self.active_attempt_mut() {
            active.status = TxAttemptStatus::Replaced;
            active.replaced_by = Some(replacement.txid);
            active.forget_raw_tx();
        }
        replacement.status = TxAttemptStatus::Active;
        self.active_txid = replacement.txid;
        self.attempts.push(replacement);
    }

    /// Restores a superseded attempt that became the live transaction again.
    ///
    /// The broadcaster can reverse a replacement chain when a miner confirms an earlier attempt.
    /// Superseded attempts normally discard their transaction bytes, so adopting one requires
    /// restoring those bytes and fee metadata before making it active. The previous chain head is
    /// marked as replaced by the adopted transaction.
    ///
    /// Returns `false` when the transaction in `parts` is not already present in this record.
    pub fn restore_attempt_as_active(&mut self, parts: TxAttemptParts) -> bool {
        let restored_txid = parts.txid;
        let Some(restored_idx) = self
            .attempts
            .iter()
            .position(|attempt| attempt.txid == restored_txid)
        else {
            return false;
        };

        if let Some(previous_active_idx) = self
            .attempts
            .iter()
            .position(|attempt| attempt.txid == self.active_txid)
        {
            if previous_active_idx != restored_idx {
                let previous_active = &mut self.attempts[previous_active_idx];
                previous_active.status = TxAttemptStatus::Replaced;
                previous_active.replaced_by = Some(restored_txid);
                previous_active.forget_raw_tx();
            }
        }

        let restored = &mut self.attempts[restored_idx];
        restored.refresh_tx(parts);
        restored.status = TxAttemptStatus::Active;
        restored.replaced_by = None;
        self.active_txid = restored_txid;
        true
    }

    /// Appends a replacement attempt that still needs an external signature.
    ///
    /// The previous active attempt stays active, because the unsigned replacement cannot be
    /// broadcast until the external signer returns its witness.
    pub fn append_pending_signature_replacement(&mut self, mut replacement: TxAttempt) {
        replacement.status = TxAttemptStatus::PendingSignature;
        self.attempts
            .retain(|attempt| attempt.status != TxAttemptStatus::PendingSignature);
        self.attempts.push(replacement);
    }

    /// Activates the current pending-signature attempt after the final witness is attached.
    ///
    /// Returns whether a pending attempt was found and activated.
    pub fn activate_pending_signature(&mut self, parts: TxAttemptParts) -> bool {
        let Some(pending_idx) = self
            .attempts
            .iter()
            .position(|attempt| attempt.status == TxAttemptStatus::PendingSignature)
        else {
            return false;
        };

        let previous_active_txid = self.active_txid;
        self.attempts[pending_idx].refresh_tx(parts);
        let active_txid = self.attempts[pending_idx].txid;

        if let Some(active_idx) = self
            .attempts
            .iter()
            .position(|attempt| attempt.txid == previous_active_txid)
        {
            self.attempts[active_idx].status = TxAttemptStatus::Replaced;
            self.attempts[active_idx].replaced_by = Some(active_txid);
            self.attempts[active_idx].forget_raw_tx();
        }

        self.attempts[pending_idx].status = TxAttemptStatus::Active;
        self.active_txid = active_txid;
        true
    }

    /// Discards any unsigned pending-signature replacement attempts.
    ///
    /// Returns whether anything was discarded.
    pub fn discard_pending_signature_replacement(&mut self) -> bool {
        let mut discarded = false;
        for attempt in &mut self.attempts {
            if attempt.status == TxAttemptStatus::PendingSignature {
                attempt.status = TxAttemptStatus::Discarded;
                attempt.forget_raw_tx();
                discarded = true;
            }
        }
        discarded
    }

    /// Marks the replacement chain permanently terminal.
    pub fn set_terminal_error(&mut self, error: TerminalError) {
        self.terminal_error = Some(error);
    }

    /// Drops the raw transaction bytes of every attempt in the chain.
    ///
    /// For a retired chain — one whose active transaction is finalized on L1 — nothing ever
    /// rebroadcasts or re-signs any attempt again, active one included, so the bytes are dead
    /// weight. The record itself is kept forever for crash-recovery point lookups, and without
    /// this the active attempt of every finalized chain would retain its full serialized
    /// transaction — for chunked EE-DA reveals a few hundred kilobytes each — growing the
    /// broadcaster database without bound.
    pub fn forget_all_raw_txs(&mut self) {
        for attempt in &mut self.attempts {
            attempt.forget_raw_tx();
        }
    }
}

/// Terminal reason that prevents further fee bumping for a logical transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum TerminalError {
    /// The configured replacement budget for the chain is exhausted.
    #[error("reached the configured maximum number of fee-bump attempts")]
    MaxAttemptsReached,

    /// The next replacement would exceed the configured fee-rate ceiling.
    #[error("required fee rate is above the configured maximum")]
    AboveMaxFeeRate,

    /// BIP-125 rule 3/4 cannot be satisfied below the configured fee-rate ceiling.
    #[error("BIP-125 replacement fee rules cannot be satisfied below the configured maximum")]
    Bip125FeeRuleUnsatisfiable,

    /// The wallet does not hold enough funds to pay for the replacement.
    #[error("wallet has insufficient funds for the replacement")]
    WalletInsufficient,

    /// The replacement would push an output below the dust threshold.
    #[error("replacement would create a dust output")]
    ReplacementWouldDustOutput,

    /// The logical transaction kind cannot be replaced in place.
    #[error("transaction kind does not support RBF replacement")]
    UnsupportedRbfKind,

    /// The wallet funded the replacement with inputs the original did not spend.
    ///
    /// Not a funding shortfall. The wallet had spare UTXOs and pulled them in, but `psbtbumpfee`
    /// returns a PSBT rather than committing it, so those inputs stay unlocked and a concurrent
    /// envelope build could select the same one. Refusing is the conservative choice until
    /// `lockunspent` is available.
    #[error("replacement would spend wallet inputs the original did not")]
    ReplacementAddsInputs,

    /// The reveal cannot pay the required replacement fee while keeping its final output above dust.
    #[error("reveal fee headroom is exhausted")]
    RevealFeeHeadroomExhausted,
}

fn unix_secs_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("db: system time is after Unix epoch")
        .as_secs()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn parts(seed: u8, fee_rate_sat_vb: u64, fee_sats: u64) -> TxAttemptParts {
        TxAttemptParts {
            raw_tx: vec![seed; 8],
            txid: L1TxId::from([seed; 32]),
            wtxid: L1WtxId::from([seed.wrapping_add(0x80); 32]),
            fee_rate_sat_vb,
            fee_sats,
        }
    }

    fn record_with_initial_attempt(kind: TxNodeKind) -> TxNodeRecord {
        let initial = TxAttempt::active(parts(1, 2, 100), 0);
        TxNodeRecord::new(kind, initial)
    }

    #[test]
    fn tx_node_id_matches_stable_vectors() {
        #[rustfmt::skip]
        let cases = [
            (
                TxNodeKind::SingleEnvelopeCommit { payload_idx: 7 },
                [0x4d, 0xa7, 0xe7, 0x28, 0x05, 0x6f, 0x7e, 0xfe, 0x88, 0x49, 0x5d, 0x6a, 0x27, 0xe1, 0x3d, 0x3e, 0x23, 0xf0, 0x87, 0xb2, 0xaa, 0xf1, 0xa2, 0x8f, 0x64, 0xb8, 0xea, 0x26, 0xa4, 0x8a, 0x86, 0xa8],
            ),
            (
                TxNodeKind::SingleEnvelopeReveal { payload_idx: 7 },
                [0x5b, 0x9a, 0x32, 0xfe, 0x9f, 0x33, 0xf0, 0x62, 0x6e, 0x9c, 0x55, 0xd9, 0xcb, 0xde, 0x3a, 0x19, 0xf5, 0x66, 0x6b, 0x98, 0x73, 0x00, 0x8b, 0xf6, 0x44, 0xb2, 0x59, 0xba, 0xfc, 0x57, 0x37, 0xd6],
            ),
            (
                TxNodeKind::ChunkedEnvelopeCommit { envelope_idx: 7 },
                [0xa2, 0x3d, 0xca, 0xbc, 0xbc, 0x04, 0xae, 0x74, 0xfb, 0x5b, 0xa2, 0xef, 0x83, 0xa7, 0xf2, 0xac, 0x49, 0x5d, 0x99, 0xb3, 0xb9, 0x14, 0x4e, 0xbd, 0x99, 0x70, 0xb4, 0x87, 0xaa, 0x49, 0x60, 0x60],
            ),
            (
                TxNodeKind::ChunkedEnvelopeReveal { envelope_idx: 7, reveal_idx: 3 },
                [0xa9, 0xf9, 0x2f, 0xe3, 0x9f, 0xde, 0x2f, 0xd1, 0x1c, 0x07, 0x4a, 0xef, 0x2e, 0x1f, 0x63, 0x8d, 0x0b, 0xca, 0x8d, 0xbf, 0x09, 0xb1, 0x2b, 0xce, 0x07, 0x37, 0xe1, 0xfc, 0xd3, 0xa4, 0x5e, 0xeb],
            ),
        ];

        for (kind, expected) in cases {
            assert_eq!(TxNodeId::from_kind(&kind), TxNodeId(Buf32(expected)));
        }
    }

    #[test]
    fn tx_node_id_separates_kinds_and_indices() {
        let kinds = [
            TxNodeKind::SingleEnvelopeCommit { payload_idx: 3 },
            TxNodeKind::SingleEnvelopeCommit { payload_idx: 4 },
            TxNodeKind::SingleEnvelopeReveal { payload_idx: 3 },
            TxNodeKind::SingleEnvelopeReveal { payload_idx: 4 },
            TxNodeKind::ChunkedEnvelopeCommit { envelope_idx: 3 },
            TxNodeKind::ChunkedEnvelopeCommit { envelope_idx: 4 },
            TxNodeKind::ChunkedEnvelopeReveal {
                envelope_idx: 3,
                reveal_idx: 0,
            },
            TxNodeKind::ChunkedEnvelopeReveal {
                envelope_idx: 3,
                reveal_idx: 1,
            },
        ];
        let ids = kinds
            .iter()
            .map(TxNodeId::from_kind)
            .collect::<HashSet<_>>();

        assert_eq!(ids.len(), kinds.len());
    }

    #[test]
    fn append_replacement_marks_previous_attempt_replaced() {
        let mut record =
            record_with_initial_attempt(TxNodeKind::SingleEnvelopeCommit { payload_idx: 7 });
        let initial_txid = record.active_txid;
        let replacement = TxAttempt::active(parts(2, 3, 200), record.next_attempt_no());
        let replacement_txid = replacement.txid;

        record.append_replacement(replacement);

        assert_eq!(record.active_txid, replacement_txid);
        assert_eq!(record.attempts[0].status, TxAttemptStatus::Replaced);
        assert_eq!(record.attempts[0].replaced_by, Some(replacement_txid));
        assert_ne!(initial_txid, replacement_txid);
        assert_eq!(record.next_attempt_no(), 2);
    }

    #[test]
    fn restoring_superseded_attempt_rehydrates_and_activates_it() {
        let original = parts(1, 2, 100);
        let original_txid = original.txid;
        let mut record = TxNodeRecord::new(
            TxNodeKind::SingleEnvelopeCommit { payload_idx: 7 },
            TxAttempt::active(original.clone(), 0),
        );
        let replacement = parts(2, 3, 200);
        let replacement_txid = replacement.txid;
        record.append_replacement(TxAttempt::active(replacement, 1));

        assert!(record.attempts[0].raw_tx.is_empty());
        assert!(record.restore_attempt_as_active(original.clone()));

        let restored = record.active_attempt().expect("restored attempt is active");
        assert_eq!(record.active_txid, original_txid);
        assert_eq!(restored.status, TxAttemptStatus::Active);
        assert_eq!(restored.replaced_by, None);
        assert_eq!(restored.raw_tx, original.raw_tx);
        assert_eq!(record.attempts[1].status, TxAttemptStatus::Replaced);
        assert_eq!(record.attempts[1].replaced_by, Some(original_txid));
        assert!(record.attempts[1].raw_tx.is_empty());
        assert_ne!(original_txid, replacement_txid);
    }

    #[test]
    fn pending_signature_replacement_keeps_previous_attempt_active() {
        let mut record =
            record_with_initial_attempt(TxNodeKind::SingleEnvelopeReveal { payload_idx: 7 });
        let initial_txid = record.active_txid;
        let replacement = TxAttempt::pending_signature(parts(2, 2, 200), 1);
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
        let mut record =
            record_with_initial_attempt(TxNodeKind::SingleEnvelopeReveal { payload_idx: 7 });
        let unsigned = parts(2, 2, 200);
        record.append_pending_signature_replacement(TxAttempt::pending_signature(
            unsigned.clone(),
            1,
        ));

        // Attaching the witness keeps the txid but changes the bytes and wtxid.
        let signed = TxAttemptParts {
            raw_tx: vec![3; 12],
            wtxid: L1WtxId::from([0xee; 32]),
            fee_rate_sat_vb: 3,
            fee_sats: 300,
            ..unsigned
        };
        let signed_wtxid = signed.wtxid;
        let activated = record.activate_pending_signature(signed);

        let active = record.active_attempt().expect("active attempt");
        assert!(activated);
        assert_eq!(active.status, TxAttemptStatus::Active);
        assert_eq!(active.fee_rate_sat_vb, 3);
        assert_eq!(active.fee_sats, 300);
        assert_eq!(active.wtxid, signed_wtxid);
        assert_eq!(record.attempts[0].status, TxAttemptStatus::Replaced);
        assert_eq!(record.attempts[0].replaced_by, Some(active.txid));
    }

    #[test]
    fn pending_signature_attempt_can_be_discarded() {
        let mut record =
            record_with_initial_attempt(TxNodeKind::SingleEnvelopeReveal { payload_idx: 7 });
        let initial_txid = record.active_txid;
        record.append_pending_signature_replacement(TxAttempt::pending_signature(
            parts(2, 2, 200),
            1,
        ));

        assert!(record.discard_pending_signature_replacement());

        assert_eq!(record.active_txid, initial_txid);
        assert_eq!(record.pending_signature_attempt(), None);
        assert_eq!(record.attempts[1].status, TxAttemptStatus::Discarded);
        assert!(record.attempts[1].raw_tx.is_empty());
    }

    /// A record keeps every attempt forever and the replacement pass loads every record, so an
    /// attempt that can no longer be broadcast must not keep carrying its serialized transaction —
    /// a chunked EE-DA reveal's is a few hundred kilobytes of witness.
    #[test]
    fn superseded_attempts_drop_their_raw_transaction() {
        let mut record =
            record_with_initial_attempt(TxNodeKind::ChunkedEnvelopeCommit { envelope_idx: 2 });
        let replacement = TxAttempt::active(parts(2, 4, 200), 1);
        let replacement_txid = replacement.txid;

        record.append_replacement(replacement);

        assert_eq!(record.attempts[0].status, TxAttemptStatus::Replaced);
        assert!(record.attempts[0].raw_tx.is_empty());
        // The metadata the chain is reconstructed and audited from outlives the bytes.
        assert_eq!(record.attempts[0].replaced_by, Some(replacement_txid));
        assert_eq!(record.attempts[0].fee_sats, 100);
        // The live attempt keeps its bytes: everything that rebroadcasts or re-signs reads them.
        assert!(!record
            .active_attempt()
            .expect("replacement is active")
            .raw_tx
            .is_empty());
    }

    #[test]
    fn activating_a_pending_signature_drops_the_superseded_attempt_bytes() {
        let mut record =
            record_with_initial_attempt(TxNodeKind::SingleEnvelopeReveal { payload_idx: 4 });
        record.append_pending_signature_replacement(TxAttempt::pending_signature(
            parts(2, 3, 300),
            1,
        ));

        assert!(record.activate_pending_signature(parts(2, 3, 320)));

        assert_eq!(record.attempts[0].status, TxAttemptStatus::Replaced);
        assert!(record.attempts[0].raw_tx.is_empty());
        assert!(!record
            .active_attempt()
            .expect("activated attempt is active")
            .raw_tx
            .is_empty());
    }

    /// A retired chain keeps its record forever, so every attempt — the active one included —
    /// must shed its serialized transaction while the audit metadata survives.
    #[test]
    fn forgetting_all_raw_txs_strips_every_attempt_but_keeps_metadata() {
        let mut record = record_with_initial_attempt(TxNodeKind::ChunkedEnvelopeReveal {
            envelope_idx: 5,
            reveal_idx: 1,
        });
        record.append_replacement(TxAttempt::active(parts(2, 4, 200), 1));
        let active_txid = record.active_txid;

        record.forget_all_raw_txs();

        assert!(record
            .attempts
            .iter()
            .all(|attempt| attempt.raw_tx.is_empty()));
        assert_eq!(record.active_txid, active_txid);
        assert_eq!(record.attempts[0].status, TxAttemptStatus::Replaced);
        assert_eq!(record.attempts[0].replaced_by, Some(active_txid));
        assert_eq!(record.attempts[1].status, TxAttemptStatus::Active);
        assert_eq!(record.attempts[1].fee_sats, 200);
    }

    #[test]
    fn replace_initial_attempt_clears_terminal_errors() {
        let mut record =
            record_with_initial_attempt(TxNodeKind::ChunkedEnvelopeCommit { envelope_idx: 1 });
        record.set_terminal_error(TerminalError::MaxAttemptsReached);
        let rebuilt = TxAttempt::active(parts(3, 5, 400), 9);
        let rebuilt_txid = rebuilt.txid;

        record.replace_initial_attempt(rebuilt);

        assert_eq!(record.terminal_error, None);
        assert_eq!(record.active_txid, rebuilt_txid);
        assert_eq!(record.attempts.len(), 1);
        assert_eq!(record.attempts[0].attempt_no, 0);
        assert_eq!(record.attempts[0].status, TxAttemptStatus::Active);
        assert_eq!(record.next_attempt_no(), 1);
    }
}
