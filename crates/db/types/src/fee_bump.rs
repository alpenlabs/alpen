//! Persistent bookkeeping for writer-side fee bumping.
//!
//! A logical writer transaction (for example "the reveal tx of chunked envelope 3") can be
//! published several times with increasing fee rates. Each concrete transaction is a
//! [`TxAttempt`], and the ordered chain of attempts for one logical transaction is a
//! [`TxNodeRecord`] keyed by a [`TxNodeId`] derived from its [`TxNodeKind`].
//!
//! Records are persisted with CBOR so new fields stay forward compatible. [`TxNodeId`] is
//! deliberately derived from a Borsh encoding of the kind instead, because the key must stay
//! byte-stable across releases and is independent of the value codec.

use std::time::{SystemTime, UNIX_EPOCH};

use bitcoin::consensus::{self, deserialize, serialize};
use bitcoin::hashes::{sha256, Hash};
use bitcoin::{Amount, FeeRate, Transaction};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_identifiers::Buf32;
use strata_primitives::L1Height;

use crate::common::{L1TxId, L1WtxId};

/// Domain separator for [`TxNodeId`] derivation.
const TX_NODE_ID_DOMAIN: &[u8] = b"alpen.btcio.tx-node.v1";

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
        let mut bytes = Vec::with_capacity(TX_NODE_ID_DOMAIN.len() + 64);
        bytes.extend_from_slice(TX_NODE_ID_DOMAIN);
        bytes.extend_from_slice(&borsh::to_vec(kind).expect("db: tx-node kind must serialize"));

        Self(Buf32(sha256::Hash::hash(&bytes).to_byte_array()))
    }
}

/// Logical BTCIO writer transaction kind.
///
/// The Borsh encoding of this enum feeds [`TxNodeId::from_kind`], so variants must only ever be
/// appended and existing fields must not be reordered or retyped.
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

    /// Returns the fee rate the attempt was built at.
    pub fn fee_rate(&self) -> Option<FeeRate> {
        FeeRate::from_sat_per_vb(self.fee_rate_sat_vb)
    }

    /// Returns the absolute fee the attempt pays.
    pub fn fee(&self) -> Amount {
        Amount::from_sat(self.fee_sats)
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
    /// Only ever called on `Replaced` and `Discarded` attempts. Everything that deserializes an
    /// attempt reads it through [`TxNodeRecord::active_attempt`] or
    /// [`TxNodeRecord::pending_signature_attempt`], and neither can return one of those.
    fn forget_raw_tx(&mut self) {
        self.raw_tx = Vec::new();
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
    use bitcoin::absolute::LockTime;
    use bitcoin::transaction::Version;
    use bitcoin::{OutPoint, ScriptBuf, Sequence, TxIn, TxOut, Witness};

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

    fn fee_rate(sat_per_vb: u64) -> FeeRate {
        FeeRate::from_sat_per_vb(sat_per_vb).expect("valid fee rate")
    }

    fn record_with_initial_attempt(kind: TxNodeKind) -> TxNodeRecord {
        let initial = TxAttempt::active(
            &tx_with_output(1_000),
            fee_rate(2),
            Amount::from_sat(100),
            0,
        );
        TxNodeRecord::new(kind, initial)
    }

    #[test]
    fn tx_node_id_is_stable_and_kind_separated() {
        let commit = TxNodeKind::ChunkedEnvelopeCommit { envelope_idx: 3 };
        let reveal = TxNodeKind::ChunkedEnvelopeReveal {
            envelope_idx: 3,
            reveal_idx: 0,
        };

        assert_eq!(TxNodeId::from_kind(&commit), TxNodeId::from_kind(&commit));
        assert_ne!(TxNodeId::from_kind(&commit), TxNodeId::from_kind(&reveal));
        assert_ne!(
            TxNodeId::from_kind(&TxNodeKind::SingleEnvelopeCommit { payload_idx: 3 }),
            TxNodeId::from_kind(&TxNodeKind::SingleEnvelopeReveal { payload_idx: 3 }),
        );
    }

    #[test]
    fn append_replacement_marks_previous_attempt_replaced() {
        let mut record =
            record_with_initial_attempt(TxNodeKind::SingleEnvelopeCommit { payload_idx: 7 });
        let initial_txid = record.active_txid;
        let replacement = TxAttempt::active(
            &tx_with_output(900),
            fee_rate(3),
            Amount::from_sat(200),
            record.next_attempt_no(),
        );
        let replacement_txid = replacement.txid;

        record.append_replacement(replacement);

        assert_eq!(record.active_txid, replacement_txid);
        assert_eq!(record.attempts[0].status, TxAttemptStatus::Replaced);
        assert_eq!(record.attempts[0].replaced_by, Some(replacement_txid));
        assert_ne!(initial_txid, replacement_txid);
        assert_eq!(record.next_attempt_no(), 2);
    }

    #[test]
    fn pending_signature_replacement_keeps_previous_attempt_active() {
        let mut record =
            record_with_initial_attempt(TxNodeKind::SingleEnvelopeReveal { payload_idx: 7 });
        let initial_txid = record.active_txid;
        let replacement = TxAttempt::pending_signature(
            &tx_with_output(900),
            fee_rate(2),
            Amount::from_sat(200),
            1,
        );
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
        let unsigned = tx_with_output(900);
        record.append_pending_signature_replacement(TxAttempt::pending_signature(
            &unsigned,
            fee_rate(2),
            Amount::from_sat(200),
            1,
        ));

        let mut signed = unsigned;
        signed.input[0].witness.push([1u8; 64]);
        let activated =
            record.activate_pending_signature(&signed, fee_rate(3), Amount::from_sat(300));

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
        let mut record =
            record_with_initial_attempt(TxNodeKind::SingleEnvelopeReveal { payload_idx: 7 });
        let initial_txid = record.active_txid;
        record.append_pending_signature_replacement(TxAttempt::pending_signature(
            &tx_with_output(900),
            fee_rate(2),
            Amount::from_sat(200),
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
        let replacement =
            TxAttempt::active(&tx_with_output(900), fee_rate(4), Amount::from_sat(200), 1);
        let replacement_txid = replacement.txid;

        record.append_replacement(replacement);

        assert_eq!(record.attempts[0].status, TxAttemptStatus::Replaced);
        assert!(record.attempts[0].raw_tx.is_empty());
        // The metadata the chain is reconstructed and audited from outlives the bytes.
        assert_eq!(record.attempts[0].replaced_by, Some(replacement_txid));
        assert_eq!(record.attempts[0].fee_sats, 100);
        // The live attempt keeps its bytes: everything that rebroadcasts or re-signs reads them.
        assert!(record
            .active_attempt()
            .expect("replacement is active")
            .try_to_tx()
            .is_ok());
    }

    #[test]
    fn activating_a_pending_signature_drops_the_superseded_attempt_bytes() {
        let mut record =
            record_with_initial_attempt(TxNodeKind::SingleEnvelopeReveal { payload_idx: 4 });
        record.append_pending_signature_replacement(TxAttempt::pending_signature(
            &tx_with_output(900),
            fee_rate(3),
            Amount::from_sat(300),
            1,
        ));

        assert!(record.activate_pending_signature(
            &tx_with_output(880),
            fee_rate(3),
            Amount::from_sat(320)
        ));

        assert_eq!(record.attempts[0].status, TxAttemptStatus::Replaced);
        assert!(record.attempts[0].raw_tx.is_empty());
        assert!(record
            .active_attempt()
            .expect("activated attempt is active")
            .try_to_tx()
            .is_ok());
    }

    #[test]
    fn replace_initial_attempt_clears_terminal_errors() {
        let mut record =
            record_with_initial_attempt(TxNodeKind::ChunkedEnvelopeCommit { envelope_idx: 1 });
        record.set_terminal_error(TerminalError::MaxAttemptsReached);
        let rebuilt =
            TxAttempt::active(&tx_with_output(800), fee_rate(5), Amount::from_sat(400), 9);
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
